//! Order management and execution.

use super::core::Engine;
use super::results::{EngineError, OrderResult};
use crate::events::{CancelReason, EventPayload, FillEvent, OrderCanceledEvent, OrderPlacedEvent};
use crate::margin::calculate_margin_requirement;
use crate::market::MarketConfig;
use crate::order::{match_order, Fill, Order, TimeInForce, OrderType};
use crate::types::{AccountId, MarketId, OrderId, Price, Quote, Side, SignedSize};
use rust_decimal::Decimal;

impl Engine {
    /// Generate a new order ID.
    fn next_order_id(&mut self) -> OrderId {
        let id = OrderId(self.next_order_id);
        self.next_order_id += 1;
        id
    }

    /// Place a market order.
    pub fn place_market_order(
        &mut self,
        account_id: AccountId,
        market_id: MarketId,
        side: Side,
        size: Decimal,
    ) -> Result<OrderResult, EngineError> {
        let order_id = self.next_order_id();

        let market = self
            .markets
            .get(&market_id)
            .ok_or(EngineError::MarketNotFound(market_id))?;

        if !market.is_active() {
            return Err(EngineError::MarketNotActive(market_id));
        }

        if !self.accounts.contains_key(&account_id) {
            return Err(EngineError::AccountNotFound(account_id));
        }

        market.config.validate_size(size).map_err(EngineError::Market)?;

        let order = Order::new_market(
            order_id,
            account_id,
            market_id,
            side,
            size,
            self.current_time,
        );

        self.emit_event(EventPayload::OrderPlaced(OrderPlacedEvent {
            market_id,
            order_id,
            account_id,
            side,
            size,
            price: None,
            reduce_only: false,
        }));

        self.execute_order(order)
    }

    /// Place a limit order.
    pub fn place_limit_order(
        &mut self,
        account_id: AccountId,
        market_id: MarketId,
        side: Side,
        size: Decimal,
        price: Price,
        time_in_force: TimeInForce,
    ) -> Result<OrderResult, EngineError> {
        let order_id = self.next_order_id();

        let market = self
            .markets
            .get(&market_id)
            .ok_or(EngineError::MarketNotFound(market_id))?;

        if !market.is_active() {
            return Err(EngineError::MarketNotActive(market_id));
        }

        if !self.accounts.contains_key(&account_id) {
            return Err(EngineError::AccountNotFound(account_id));
        }

        market.config.validate_size(size).map_err(EngineError::Market)?;
        let validated_price = market.config.validate_price(price).map_err(EngineError::Market)?;

        let order = Order::new_limit(
            order_id,
            account_id,
            market_id,
            side,
            size,
            validated_price,
            time_in_force,
            self.current_time,
        );

        self.emit_event(EventPayload::OrderPlaced(OrderPlacedEvent {
            market_id,
            order_id,
            account_id,
            side,
            size,
            price: Some(validated_price),
            reduce_only: false,
        }));

        self.execute_order(order)
    }

    /// Cancel an order.
    pub fn cancel_order(&mut self, market_id: MarketId, order_id: OrderId) -> Result<(), EngineError> {
        let market = self
            .markets
            .get_mut(&market_id)
            .ok_or(EngineError::MarketNotFound(market_id))?;

        let order = market
            .order_book
            .remove(order_id)
            .ok_or(EngineError::OrderNotFound(order_id))?;

        self.emit_event(EventPayload::OrderCanceled(OrderCanceledEvent {
            market_id,
            order_id,
            account_id: order.account_id,
            reason: CancelReason::UserRequested,
        }));

        Ok(())
    }

    /// Execute an order by matching against the book and updating positions.
    fn execute_order(&mut self, order: Order) -> Result<OrderResult, EngineError> {
        let market_id = order.market_id;
        let account_id = order.account_id;
        let order_id = order.id;
        let order_side = order.side;
        let order_type = order.order_type;
        let time_in_force = order.time_in_force;

        let market = self
            .markets
            .get_mut(&market_id)
            .ok_or(EngineError::MarketNotFound(market_id))?;

        let match_result = match_order(&mut market.order_book, order.clone());

        let mut total_filled = Decimal::ZERO;
        let mut total_cost = Decimal::ZERO;
        let mut fill_events = Vec::new();

        for fill in &match_result.fills {
            total_filled += fill.size;
            total_cost += fill.size * fill.price.value();
            market.record_trade(fill.price, fill.size);
            fill_events.push(fill.clone());
        }

        let market_config = market.config.clone();

        for fill in fill_events {
            self.process_fill(&fill, &market_config)?;
        }

        let remaining = match_result.remaining_size;
        let order_posted = if !remaining.is_zero() {
            match order_type {
                OrderType::Market => false,
                OrderType::Limit => {
                    match time_in_force {
                        TimeInForce::GTC => {
                            if self.check_margin_for_order(account_id, market_id, order_side, remaining, order.price.unwrap())? {
                                let mut resting_order = order.clone();
                                resting_order.remaining_size = remaining;
                                let market = self.markets.get_mut(&market_id).unwrap();
                                market.order_book.insert(resting_order);
                                true
                            } else {
                                self.emit_event(EventPayload::OrderCanceled(OrderCanceledEvent {
                                    market_id,
                                    order_id,
                                    account_id,
                                    reason: CancelReason::InsufficientMargin,
                                }));
                                false
                            }
                        }
                        TimeInForce::IOC | TimeInForce::FOK => false,
                        TimeInForce::PostOnly => {
                            if total_filled > Decimal::ZERO {
                                self.emit_event(EventPayload::OrderCanceled(OrderCanceledEvent {
                                    market_id,
                                    order_id,
                                    account_id,
                                    reason: CancelReason::PostOnlyWouldTake,
                                }));
                                false
                            } else {
                                let mut resting_order = order.clone();
                                resting_order.remaining_size = remaining;
                                let market = self.markets.get_mut(&market_id).unwrap();
                                market.order_book.insert(resting_order);
                                true
                            }
                        }
                    }
                }
            }
        } else {
            false
        };

        let avg_price = if total_filled > Decimal::ZERO {
            Some(Price::new_unchecked(total_cost / total_filled))
        } else {
            None
        };

        Ok(OrderResult {
            order_id,
            filled_size: total_filled,
            remaining_size: remaining,
            average_price: avg_price,
            is_posted: order_posted,
            fills: match_result.fills,
        })
    }

    /// Check if account has enough margin for a resting order.
    fn check_margin_for_order(
        &self,
        account_id: AccountId,
        market_id: MarketId,
        side: Side,
        size: Decimal,
        price: Price,
    ) -> Result<bool, EngineError> {
        let account = self
            .accounts
            .get(&account_id)
            .ok_or(EngineError::AccountNotFound(account_id))?;

        let market = self
            .markets
            .get(&market_id)
            .ok_or(EngineError::MarketNotFound(market_id))?;

        let signed_size = match side {
            Side::Long => SignedSize::new(size),
            Side::Short => SignedSize::new(-size),
        };

        let max_leverage = market.config.margin_params.max_leverage;
        let margin_req = calculate_margin_requirement(
            signed_size,
            price,
            max_leverage,
            &market.config.margin_params,
        );

        Ok(account.balance.value() >= margin_req.initial.value())
    }

    /// Process a fill by updating positions for both maker and taker.
    fn process_fill(&mut self, fill: &Fill, config: &MarketConfig) -> Result<(), EngineError> {
        self.update_position_for_fill(
            fill.taker_account_id,
            config,
            fill.taker_side,
            fill.size,
            fill.price,
        )?;

        let maker_side = match fill.taker_side {
            Side::Long => Side::Short,
            Side::Short => Side::Long,
        };

        self.update_position_for_fill(
            fill.maker_account_id,
            config,
            maker_side,
            fill.size,
            fill.price,
        )?;

        self.emit_event(EventPayload::Fill(FillEvent {
            market_id: config.id,
            order_id: fill.taker_order_id,
            account_id: fill.taker_account_id,
            side: fill.taker_side,
            size: fill.size,
            price: fill.price,
            fee: Quote::zero(),
            is_maker: false,
        }));

        self.emit_event(EventPayload::Fill(FillEvent {
            market_id: config.id,
            order_id: fill.maker_order_id,
            account_id: fill.maker_account_id,
            side: maker_side,
            size: fill.size,
            price: fill.price,
            fee: Quote::zero(),
            is_maker: true,
        }));

        Ok(())
    }
}
