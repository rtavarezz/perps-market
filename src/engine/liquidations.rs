//! Liquidation detection and execution.

use super::core::Engine;
use super::results::{EngineError, LiquidationResult};
use crate::events::{BadDebtEvent, EventPayload, LiquidationEvent};
use crate::liquidation::{calculate_liquidation_penalty, evaluate_liquidation, LiquidationStatus};
use crate::margin::{calculate_margin_requirement, MarginRequirement};
use crate::position::Position;
use crate::types::{AccountId, MarketId, Price, Quote, Side};
use rust_decimal::Decimal;

impl Engine {
    /// Check and execute liquidations for a market.
    pub fn check_liquidations(&mut self, market_id: MarketId) -> Result<Vec<LiquidationResult>, EngineError> {
        let market = self
            .markets
            .get(&market_id)
            .ok_or(EngineError::MarketNotFound(market_id))?;

        let Some(mark_price) = market.mark_price else {
            return Err(EngineError::NoMarkPrice(market_id));
        };

        let margin_params = market.config.margin_params.clone();
        let liq_params = market.config.liquidation_params.clone();
        let funding_index = market.funding_state.cumulative_funding;

        let mut liquidatable: Vec<(AccountId, Position, MarginRequirement)> = Vec::new();

        for (account_id, account) in &self.accounts {
            if let Some(position) = account.get_position(market_id) {
                let margin_req = calculate_margin_requirement(
                    position.size,
                    mark_price,
                    position.leverage,
                    &margin_params,
                );

                let equity = position.equity(mark_price, funding_index);
                let notional = position.notional_value(mark_price);

                let status = evaluate_liquidation(
                    equity,
                    &margin_req,
                    notional,
                    position.entry_price,
                    mark_price,
                    position.side().unwrap(),
                );

                match status {
                    LiquidationStatus::Liquidatable { .. } | LiquidationStatus::Bankrupt { .. } => {
                        liquidatable.push((*account_id, position.clone(), margin_req));
                    }
                    _ => {}
                }
            }
        }

        let mut results = Vec::new();

        for (account_id, position, margin_req) in liquidatable {
            let result = self.execute_liquidation(
                account_id,
                market_id,
                position,
                margin_req,
                mark_price,
                &liq_params,
            )?;
            results.push(result);
        }

        Ok(results)
    }

    /// Execute a liquidation.
    fn execute_liquidation(
        &mut self,
        account_id: AccountId,
        market_id: MarketId,
        position: Position,
        _margin_req: MarginRequirement,
        mark_price: Price,
        liq_params: &crate::liquidation::LiquidationParams,
    ) -> Result<LiquidationResult, EngineError> {
        let funding_index = {
            let market = self.markets.get(&market_id).unwrap();
            market.funding_state.cumulative_funding
        };

        let equity = position.equity(mark_price, funding_index);
        let position_value = position.notional_value(mark_price);
        let penalty = calculate_liquidation_penalty(position_value, liq_params);
        let remaining_equity = equity.value() - penalty.total.value();

        let bad_debt = if remaining_equity < Decimal::ZERO {
            Quote::new(-remaining_equity)
        } else {
            Quote::zero()
        };

        let mut events_to_emit: Vec<EventPayload> = Vec::new();

        {
            let account = self
                .accounts
                .get_mut(&account_id)
                .ok_or(EngineError::AccountNotFound(account_id))?;

            if remaining_equity > Decimal::ZERO {
                account.return_collateral(Quote::new(remaining_equity));
            }

            account.remove_position(market_id);
        }

        if bad_debt.value() > Decimal::ZERO {
            let covered = self.insurance_fund.cover_bad_debt(bad_debt);
            let uncovered = Quote::new(bad_debt.value() - covered.value());

            if uncovered.value() > Decimal::ZERO {
                events_to_emit.push(EventPayload::BadDebt(BadDebtEvent {
                    market_id,
                    account_id,
                    debt_amount: bad_debt,
                    covered_by_insurance: covered,
                    socialized_loss: uncovered,
                }));
            }
        }

        self.insurance_fund.deposit(penalty.insurance_contribution);

        let market = self.markets.get_mut(&market_id).unwrap();
        match position.side().unwrap() {
            Side::Long => market.update_open_interest(-position.size.abs(), Decimal::ZERO),
            Side::Short => market.update_open_interest(Decimal::ZERO, -position.size.abs()),
        }

        let realized_pnl = position.unrealized_pnl(mark_price);

        events_to_emit.push(EventPayload::Liquidation(LiquidationEvent {
            market_id,
            account_id,
            liquidated_size: position.size,
            liquidation_price: mark_price,
            penalty: penalty.total,
            liquidator_account: None,
        }));

        for event in events_to_emit {
            self.emit_event(event);
        }

        Ok(LiquidationResult {
            account_id,
            market_id,
            position_size: position.size,
            liquidation_price: mark_price,
            penalty: penalty.total,
            bad_debt,
            realized_pnl,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::EngineConfig;
    use crate::market::MarketConfig;
    use crate::order::TimeInForce;
    use rust_decimal_macros::dec;

    fn setup_engine() -> Engine {
        let mut engine = Engine::new(EngineConfig::default());
        engine.add_market(MarketConfig::btc_perp());
        engine
    }

    #[test]
    fn create_account_and_deposit() {
        let mut engine = setup_engine();

        let account_id = engine.create_account();
        engine.deposit(account_id, Quote::new(dec!(10000))).unwrap();

        let account = engine.get_account(account_id).unwrap();
        assert_eq!(account.balance.value(), dec!(10000));
    }

    #[test]
    fn place_market_order_no_liquidity() {
        let mut engine = setup_engine();

        let account_id = engine.create_account();
        engine.deposit(account_id, Quote::new(dec!(10000))).unwrap();
        engine.update_index_price(MarketId(1), Price::new_unchecked(dec!(50000))).unwrap();

        let result = engine
            .place_market_order(account_id, MarketId(1), Side::Long, dec!(0.1))
            .unwrap();

        assert_eq!(result.filled_size, Decimal::ZERO);
        assert_eq!(result.remaining_size, dec!(0.1));
    }

    #[test]
    fn limit_order_creates_position_on_match() {
        let mut engine = setup_engine();

        let buyer = engine.create_account();
        let seller = engine.create_account();

        engine.deposit(buyer, Quote::new(dec!(10000))).unwrap();
        engine.deposit(seller, Quote::new(dec!(10000))).unwrap();

        engine.update_index_price(MarketId(1), Price::new_unchecked(dec!(50000))).unwrap();

        let ask_result = engine
            .place_limit_order(
                seller,
                MarketId(1),
                Side::Short,
                dec!(0.1),
                Price::new_unchecked(dec!(50000)),
                TimeInForce::GTC,
            )
            .unwrap();

        assert!(ask_result.is_posted);

        let bid_result = engine
            .place_limit_order(
                buyer,
                MarketId(1),
                Side::Long,
                dec!(0.1),
                Price::new_unchecked(dec!(50100)),
                TimeInForce::GTC,
            )
            .unwrap();

        assert_eq!(bid_result.filled_size, dec!(0.1));
        assert_eq!(bid_result.fills.len(), 1);

        let buyer_account = engine.get_account(buyer).unwrap();
        let buyer_pos = buyer_account.get_position(MarketId(1)).unwrap();
        assert!(buyer_pos.size.is_long());
        assert_eq!(buyer_pos.size.abs(), dec!(0.1));

        let seller_account = engine.get_account(seller).unwrap();
        let seller_pos = seller_account.get_position(MarketId(1)).unwrap();
        assert!(seller_pos.size.is_short());
        assert_eq!(seller_pos.size.abs(), dec!(0.1));
    }

    #[test]
    fn liquidation_on_price_drop() {
        let mut engine = setup_engine();

        let buyer = engine.create_account();
        let seller = engine.create_account();

        engine.deposit(buyer, Quote::new(dec!(1000))).unwrap();
        engine.deposit(seller, Quote::new(dec!(100000))).unwrap();

        engine.update_index_price(MarketId(1), Price::new_unchecked(dec!(50000))).unwrap();

        engine
            .place_limit_order(
                seller,
                MarketId(1),
                Side::Short,
                dec!(0.1),
                Price::new_unchecked(dec!(50000)),
                TimeInForce::GTC,
            )
            .unwrap();

        engine
            .place_market_order(buyer, MarketId(1), Side::Long, dec!(0.1))
            .unwrap();

        assert!(engine.get_account(buyer).unwrap().get_position(MarketId(1)).is_some());

        engine.update_index_price(MarketId(1), Price::new_unchecked(dec!(40000))).unwrap();

        let liquidations = engine.check_liquidations(MarketId(1)).unwrap();

        assert!(!liquidations.is_empty());
        assert_eq!(liquidations[0].account_id, buyer);

        assert!(engine.get_account(buyer).unwrap().get_position(MarketId(1)).is_none());
    }

    #[test]
    fn funding_settlement() {
        let mut engine = setup_engine();

        let long_trader = engine.create_account();
        let short_trader = engine.create_account();

        engine.deposit(long_trader, Quote::new(dec!(10000))).unwrap();
        engine.deposit(short_trader, Quote::new(dec!(10000))).unwrap();

        engine.update_index_price(MarketId(1), Price::new_unchecked(dec!(50000))).unwrap();

        engine
            .place_limit_order(
                short_trader,
                MarketId(1),
                Side::Short,
                dec!(1.0),
                Price::new_unchecked(dec!(50000)),
                TimeInForce::GTC,
            )
            .unwrap();

        engine
            .place_market_order(long_trader, MarketId(1), Side::Long, dec!(1.0))
            .unwrap();

        engine.advance_time(8 * 60 * 60 * 1000);

        let funding_result = engine.settle_funding(MarketId(1)).unwrap();

        assert_eq!(funding_result.accounts_affected, 2);
        let net = funding_result.total_long_payments.value() + funding_result.total_short_payments.value();
        assert!(net.abs() < dec!(0.01));
    }
}
