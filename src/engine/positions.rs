//! Position management for fills.

use super::core::Engine;
use super::results::EngineError;
use crate::events::{
    CloseReason, EventPayload, PositionClosedEvent, PositionOpenedEvent, PositionUpdatedEvent,
};
use crate::margin::calculate_margin_requirement;
use crate::market::MarketConfig;
use crate::position::{increase_position, reduce_position, Position};
use crate::types::{AccountId, Price, Quote, Side, SignedSize};
use rust_decimal::Decimal;

impl Engine {
    /// Update a position based on a fill.
    pub(super) fn update_position_for_fill(
        &mut self,
        account_id: AccountId,
        config: &MarketConfig,
        side: Side,
        size: Decimal,
        price: Price,
    ) -> Result<(), EngineError> {
        let market_id = config.id;

        let funding_index = {
            let market = self.markets.get(&market_id).unwrap();
            market.funding_state.cumulative_funding
        };

        let signed_size = match side {
            Side::Long => SignedSize::new(size),
            Side::Short => SignedSize::new(-size),
        };

        let mut events_to_emit: Vec<EventPayload> = Vec::new();

        {
            let account = self
                .accounts
                .get_mut(&account_id)
                .ok_or(EngineError::AccountNotFound(account_id))?;

            let existing_position = account.get_position(market_id).cloned();

            match existing_position {
                Some(position) => {
                    let is_same_direction = (position.size.is_long() && side == Side::Long)
                        || (position.size.is_short() && side == Side::Short);

                    if is_same_direction {
                        self.handle_position_increase(
                            account_id,
                            config,
                            &position,
                            signed_size,
                            price,
                            funding_index,
                            &mut events_to_emit,
                        )?;
                    } else {
                        self.handle_position_reduce_or_flip(
                            account_id,
                            config,
                            &position,
                            side,
                            size,
                            price,
                            funding_index,
                            &mut events_to_emit,
                        )?;
                    }
                }
                None => {
                    self.handle_new_position(
                        account_id,
                        config,
                        side,
                        size,
                        signed_size,
                        price,
                        funding_index,
                        &mut events_to_emit,
                    )?;
                }
            }
        }

        let market = self.markets.get_mut(&market_id).unwrap();
        match side {
            Side::Long => market.update_open_interest(size, Decimal::ZERO),
            Side::Short => market.update_open_interest(Decimal::ZERO, size),
        }

        for event in events_to_emit {
            self.emit_event(event);
        }

        Ok(())
    }

    fn handle_position_increase(
        &mut self,
        account_id: AccountId,
        config: &MarketConfig,
        position: &Position,
        signed_size: SignedSize,
        price: Price,
        funding_index: Decimal,
        events: &mut Vec<EventPayload>,
    ) -> Result<(), EngineError> {
        let market_id = config.id;
        let max_leverage = config.margin_params.max_leverage;

        let margin_req = calculate_margin_requirement(
            signed_size,
            price,
            max_leverage,
            &config.margin_params,
        );

        let account = self.accounts.get_mut(&account_id).unwrap();
        account.reserve_collateral(margin_req.initial).map_err(EngineError::Account)?;

        let new_position = increase_position(
            position,
            signed_size.value(),
            price,
            margin_req.initial,
            funding_index,
            self.current_time,
        );

        account.set_position(new_position.clone());

        events.push(EventPayload::PositionUpdated(PositionUpdatedEvent {
            market_id,
            account_id,
            old_size: position.size,
            new_size: new_position.size,
            old_entry_price: position.entry_price,
            new_entry_price: new_position.entry_price,
            realized_pnl: Quote::zero(),
        }));

        Ok(())
    }

    fn handle_position_reduce_or_flip(
        &mut self,
        account_id: AccountId,
        config: &MarketConfig,
        position: &Position,
        side: Side,
        size: Decimal,
        price: Price,
        funding_index: Decimal,
        events: &mut Vec<EventPayload>,
    ) -> Result<(), EngineError> {
        let position_abs = position.size.abs();

        if size >= position_abs {
            self.handle_full_close_or_flip(
                account_id, config, position, side, size, price, funding_index, events,
            )
        } else {
            self.handle_partial_close(
                account_id, config, position, size, price, funding_index, events,
            )
        }
    }

    fn handle_full_close_or_flip(
        &mut self,
        account_id: AccountId,
        config: &MarketConfig,
        position: &Position,
        side: Side,
        size: Decimal,
        price: Price,
        funding_index: Decimal,
        events: &mut Vec<EventPayload>,
    ) -> Result<(), EngineError> {
        let market_id = config.id;
        let position_abs = position.size.abs();

        let close_update = reduce_position(
            position,
            position_abs,
            price,
            funding_index,
            self.current_time,
        );

        let account = self.accounts.get_mut(&account_id).unwrap();
        account.realize_pnl(close_update.realized_pnl);
        account.return_collateral(close_update.collateral_returned);

        events.push(EventPayload::PositionClosed(PositionClosedEvent {
            market_id,
            account_id,
            exit_price: price,
            realized_pnl: close_update.realized_pnl,
            collateral_returned: close_update.collateral_returned,
            close_reason: CloseReason::UserClosed,
        }));

        account.remove_position(market_id);

        let flip_size = size - position_abs;
        if flip_size > Decimal::ZERO {
            let flip_signed = match side {
                Side::Long => SignedSize::new(flip_size),
                Side::Short => SignedSize::new(-flip_size),
            };

            let max_leverage = config.margin_params.max_leverage;
            let margin_req = calculate_margin_requirement(
                flip_signed,
                price,
                max_leverage,
                &config.margin_params,
            );

            account.reserve_collateral(margin_req.initial).map_err(EngineError::Account)?;

            let new_position = Position::new(
                market_id,
                flip_signed,
                price,
                margin_req.initial,
                max_leverage,
                funding_index,
                self.current_time,
            );

            account.set_position(new_position.clone());

            events.push(EventPayload::PositionOpened(PositionOpenedEvent {
                market_id,
                account_id,
                side,
                size: flip_size,
                entry_price: price,
                leverage: max_leverage.value(),
                collateral: margin_req.initial,
            }));
        }

        Ok(())
    }

    fn handle_partial_close(
        &mut self,
        account_id: AccountId,
        config: &MarketConfig,
        position: &Position,
        size: Decimal,
        price: Price,
        funding_index: Decimal,
        events: &mut Vec<EventPayload>,
    ) -> Result<(), EngineError> {
        let market_id = config.id;

        let close_update = reduce_position(
            position,
            size,
            price,
            funding_index,
            self.current_time,
        );

        let account = self.accounts.get_mut(&account_id).unwrap();
        account.realize_pnl(close_update.realized_pnl);
        account.return_collateral(close_update.collateral_returned);

        if let Some(new_pos) = close_update.new_position {
            account.set_position(new_pos.clone());

            events.push(EventPayload::PositionUpdated(PositionUpdatedEvent {
                market_id,
                account_id,
                old_size: position.size,
                new_size: new_pos.size,
                old_entry_price: position.entry_price,
                new_entry_price: new_pos.entry_price,
                realized_pnl: close_update.realized_pnl,
            }));
        }

        Ok(())
    }

    fn handle_new_position(
        &mut self,
        account_id: AccountId,
        config: &MarketConfig,
        side: Side,
        size: Decimal,
        signed_size: SignedSize,
        price: Price,
        funding_index: Decimal,
        events: &mut Vec<EventPayload>,
    ) -> Result<(), EngineError> {
        let market_id = config.id;
        let max_leverage = config.margin_params.max_leverage;

        let margin_req = calculate_margin_requirement(
            signed_size,
            price,
            max_leverage,
            &config.margin_params,
        );

        let account = self.accounts.get_mut(&account_id).unwrap();
        account.reserve_collateral(margin_req.initial).map_err(EngineError::Account)?;

        let new_position = Position::new(
            market_id,
            signed_size,
            price,
            margin_req.initial,
            max_leverage,
            funding_index,
            self.current_time,
        );

        account.set_position(new_position.clone());

        events.push(EventPayload::PositionOpened(PositionOpenedEvent {
            market_id,
            account_id,
            side,
            size,
            entry_price: price,
            leverage: max_leverage.value(),
            collateral: margin_req.initial,
        }));

        Ok(())
    }
}
