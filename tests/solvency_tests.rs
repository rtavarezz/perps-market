//! Solvency invariant tests.
//!
//! These tests verify critical invariants that must hold for the exchange
//! to remain solvent under all conditions.

use perps_core::*;
use proptest::prelude::*;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

proptest! {
    /// Open interest must balance. Total long OI should equal total short OI.
    #[test]
    fn open_interest_always_balanced(
        num_traders in 2..20usize,
        trade_sizes in proptest::collection::vec(1i64..100i64, 2..20),
    ) {
        let mut engine = Engine::new(EngineConfig::default());
        engine.add_market(MarketConfig::btc_perp());

        let entry_price = dec!(50000);
        engine.update_index_price(MarketId(1), Price::new_unchecked(entry_price)).unwrap();

        // Create accounts
        let mut accounts: Vec<AccountId> = Vec::new();
        for _ in 0..num_traders {
            let id = engine.create_account();
            engine.deposit(id, Quote::new(dec!(1_000_000))).unwrap();
            accounts.push(id);
        }

        // Execute trades (pairs of long/short)
        for (i, &size_raw) in trade_sizes.iter().enumerate() {
            if i + 1 >= accounts.len() { break; }

            let size = Decimal::new(size_raw, 2);
            let buyer = accounts[i % accounts.len()];
            let seller = accounts[(i + 1) % accounts.len()];

            // Seller posts ask
            let _ = engine.place_limit_order(
                seller,
                MarketId(1),
                Side::Short,
                size,
                Price::new_unchecked(entry_price),
                TimeInForce::GTC,
            );

            // Buyer takes
            let _ = engine.place_market_order(buyer, MarketId(1), Side::Long, size);
        }

        let market = engine.get_market(MarketId(1)).unwrap();

        // OI must balance (within small rounding)
        let diff = (market.open_interest_long - market.open_interest_short).abs();
        prop_assert!(
            diff < dec!(0.0001),
            "OI imbalanced: long={}, short={}, diff={}",
            market.open_interest_long,
            market.open_interest_short,
            diff
        );
    }

    /// Equity conservation. Sum of all account equities plus insurance should be constant.
    #[test]
    fn equity_conserved_through_pnl_changes(
        initial_price in 40000i64..60000i64,
        price_changes in proptest::collection::vec(-5000i64..5000i64, 1..10),
    ) {
        let mut engine = Engine::new(EngineConfig::default());
        engine.add_market(MarketConfig::btc_perp());
        engine.fund_insurance(Quote::new(dec!(100_000)));

        let initial = Price::new_unchecked(Decimal::from(initial_price));
        engine.update_index_price(MarketId(1), initial).unwrap();

        let long_trader = engine.create_account();
        let short_trader = engine.create_account();

        engine.deposit(long_trader, Quote::new(dec!(50000))).unwrap();
        engine.deposit(short_trader, Quote::new(dec!(50000))).unwrap();

        // Create matched positions
        engine.place_limit_order(
            short_trader,
            MarketId(1),
            Side::Short,
            dec!(1),
            initial,
            TimeInForce::GTC,
        ).unwrap();
        engine.place_market_order(long_trader, MarketId(1), Side::Long, dec!(1)).unwrap();

        // Calculate initial total equity
        let calc_total_equity = |e: &Engine| -> Decimal {
            let mut total = Decimal::ZERO;
            for (_, account) in e.accounts_iter() {
                total += account.balance.value();
                for pos in account.positions.values() {
                    let market = e.get_market(pos.market_id).unwrap();
                    if let Some(mark) = market.mark_price {
                        total += pos.unrealized_pnl(mark).value();
                    }
                    total += pos.collateral.value();
                }
            }
            total += e.insurance_fund_balance().value();
            total
        };

        let initial_total = calc_total_equity(&engine);

        // Apply price changes (skip liquidations for pure PnL test)
        for delta in price_changes {
            let new_val = Decimal::from(initial_price + delta).max(dec!(1000));
            engine.update_index_price(MarketId(1), Price::new_unchecked(new_val)).unwrap();

            let current_total = calc_total_equity(&engine);

            // Should be conserved (PnL is zero-sum)
            let diff = (initial_total - current_total).abs();
            prop_assert!(
                diff < dec!(1),
                "Equity not conserved: initial={}, current={}, diff={}",
                initial_total,
                current_total,
                diff
            );
        }
    }

    /// Funding payments must be zero-sum across all accounts.
    #[test]
    fn funding_zero_sum_invariant(
        num_longs in 1..5usize,
        num_shorts in 1..5usize,
        premium_bps in -100i32..100i32,
    ) {
        let mut engine = Engine::new(EngineConfig::default());
        engine.add_market(MarketConfig::btc_perp());

        let mark = dec!(50000) + Decimal::from(premium_bps);
        let index = dec!(50000);

        engine.update_index_price(MarketId(1), Price::new_unchecked(index)).unwrap();

        // Create accounts
        let mut all_accounts = Vec::new();
        let mut long_accounts = Vec::new();
        let mut short_accounts = Vec::new();

        for _ in 0..num_longs {
            let id = engine.create_account();
            engine.deposit(id, Quote::new(dec!(100000))).unwrap();
            all_accounts.push(id);
            long_accounts.push(id);
        }

        for _ in 0..num_shorts {
            let id = engine.create_account();
            engine.deposit(id, Quote::new(dec!(100000))).unwrap();
            all_accounts.push(id);
            short_accounts.push(id);
        }

        // Long accounts post bids, short accounts post asks
        for &id in &short_accounts {
            engine.place_limit_order(
                id,
                MarketId(1),
                Side::Short,
                dec!(1),
                Price::new_unchecked(index),
                TimeInForce::GTC,
            ).unwrap();
        }

        // Long accounts take those orders
        for &id in &long_accounts {
            engine.place_market_order(id, MarketId(1), Side::Long, dec!(1)).unwrap();
        }

        // Set mark price with premium
        engine.update_index_price(MarketId(1), Price::new_unchecked(mark)).unwrap();

        // Record balances before funding for all accounts
        let balances_before: Vec<(AccountId, Decimal)> = all_accounts.iter()
            .map(|&id| (id, engine.get_account(id).unwrap().balance.value()))
            .collect();

        // Settle funding
        engine.advance_time(8 * 60 * 60 * 1000);
        let result = engine.settle_funding(MarketId(1)).unwrap();

        // Record balances after funding
        let balances_after: Vec<Decimal> = all_accounts.iter()
            .map(|&id| engine.get_account(id).unwrap().balance.value())
            .collect();

        // Sum of all funding payments should be zero
        let total_change: Decimal = balances_after.iter()
            .zip(balances_before.iter())
            .map(|(after, (_, before))| after - before)
            .sum();

        prop_assert!(
            total_change.abs() < dec!(0.01),
            "Funding not zero-sum: total change={}, rate={}, affected={}",
            total_change,
            result.funding_rate,
            result.accounts_affected
        );
    }
}

/// Non-proptest solvency tests.
#[cfg(test)]
mod deterministic_solvency {
    use super::*;

    #[test]
    fn insurance_fund_covers_bad_debt() {
        let mut engine = Engine::new(EngineConfig::default());
        engine.add_market(MarketConfig::btc_perp());
        engine.fund_insurance(Quote::new(dec!(50_000)));

        let trader = engine.create_account();
        let counterparty = engine.create_account();

        // Minimal margin for high leverage
        engine.deposit(trader, Quote::new(dec!(1000))).unwrap();
        engine.deposit(counterparty, Quote::new(dec!(1_000_000))).unwrap();

        engine
            .update_index_price(MarketId(1), Price::new_unchecked(dec!(50000)))
            .unwrap();

        engine
            .place_limit_order(
                counterparty,
                MarketId(1),
                Side::Short,
                dec!(10),
                Price::new_unchecked(dec!(50000)),
                TimeInForce::GTC,
            )
            .unwrap();

        // Open leveraged position
        engine
            .place_market_order(trader, MarketId(1), Side::Long, dec!(0.5))
            .unwrap();

        let insurance_before = engine.insurance_fund_balance().value();

        // Crash price to create bad debt scenario
        engine
            .update_index_price(MarketId(1), Price::new_unchecked(dec!(35000)))
            .unwrap();

        let liqs = engine.check_liquidations(MarketId(1)).unwrap();

        if !liqs.is_empty() {
            let total_bad_debt: Decimal = liqs.iter().map(|l| l.bad_debt.value()).sum();

            // Insurance should have covered what it could
            let insurance_after = engine.insurance_fund_balance().value();

            // If there was bad debt, insurance should have decreased or stayed same
            // (it also receives liquidation fees)
            assert!(
                insurance_after <= insurance_before + total_bad_debt,
                "Insurance fund behavior unexpected"
            );
        }
    }

    #[test]
    fn no_negative_balances_after_liquidation() {
        let mut engine = Engine::new(EngineConfig::default());
        engine.add_market(MarketConfig::btc_perp());
        engine.fund_insurance(Quote::new(dec!(1_000_000)));

        // Create multiple accounts with varying margin
        let accounts: Vec<AccountId> = (0..10)
            .map(|i| {
                let id = engine.create_account();
                let deposit = dec!(2000) + Decimal::from(i) * dec!(500);
                engine.deposit(id, Quote::new(deposit)).unwrap();
                id
            })
            .collect();

        let counterparty = engine.create_account();
        engine.deposit(counterparty, Quote::new(dec!(10_000_000))).unwrap();

        engine
            .update_index_price(MarketId(1), Price::new_unchecked(dec!(50000)))
            .unwrap();

        engine
            .place_limit_order(
                counterparty,
                MarketId(1),
                Side::Short,
                dec!(100),
                Price::new_unchecked(dec!(50000)),
                TimeInForce::GTC,
            )
            .unwrap();

        // Everyone goes long
        for &id in &accounts {
            let _ = engine.place_market_order(id, MarketId(1), Side::Long, dec!(0.5));
        }

        // Price crash
        for price in [dec!(45000), dec!(40000), dec!(35000), dec!(30000)] {
            engine
                .update_index_price(MarketId(1), Price::new_unchecked(price))
                .unwrap();
            engine.check_liquidations(MarketId(1)).unwrap();
        }

        // No account should have negative balance
        for &id in &accounts {
            let balance = engine.get_account(id).unwrap().balance.value();
            assert!(
                balance >= Decimal::ZERO,
                "Account {:?} has negative balance: {}",
                id,
                balance
            );
        }
    }

    #[test]
    fn collateral_always_returned_on_close() {
        let mut engine = Engine::new(EngineConfig::default());
        engine.add_market(MarketConfig::btc_perp());

        let trader = engine.create_account();
        let counterparty = engine.create_account();

        let initial_deposit = dec!(50000);
        engine.deposit(trader, Quote::new(initial_deposit)).unwrap();
        engine.deposit(counterparty, Quote::new(dec!(1_000_000))).unwrap();

        engine
            .update_index_price(MarketId(1), Price::new_unchecked(dec!(50000)))
            .unwrap();

        // Place short limit order at a higher price (will be ask)
        // Use 10 BTC which requires ~$10k margin at 50x leverage
        let short_result = engine
            .place_limit_order(
                counterparty,
                MarketId(1),
                Side::Short,
                dec!(10),
                Price::new_unchecked(dec!(50100)),  // Ask at 50100
                TimeInForce::GTC,
            )
            .expect("Should place short order");

        assert!(short_result.is_posted, "Short order should be posted");

        // Place long limit order at a lower price (will be bid)
        let long_result = engine
            .place_limit_order(
                counterparty,
                MarketId(1),
                Side::Long,
                dec!(10),
                Price::new_unchecked(dec!(49900)),  // Bid at 49900
                TimeInForce::GTC,
            )
            .expect("Should place long order");

        assert!(long_result.is_posted, "Long order should be posted");

        // Open position (market buy will match the ask at 50100)
        let open_result = engine
            .place_market_order(trader, MarketId(1), Side::Long, dec!(1))
            .expect("Should open position");

        assert!(
            open_result.filled_size > Decimal::ZERO,
            "Order should have filled. Result: {:?}",
            open_result
        );

        let balance_after_open = engine.get_account(trader).unwrap().balance.value();
        let pos = engine
            .get_account(trader)
            .unwrap()
            .get_position(MarketId(1))
            .expect("Position should exist after opening");
        let collateral_locked = pos.collateral.value();

        assert!(
            collateral_locked > Decimal::ZERO,
            "Collateral should be locked"
        );

        // Close position (market sell will match the bid at 49900)
        let close_result = engine
            .place_market_order(trader, MarketId(1), Side::Short, dec!(1))
            .expect("Should close position");

        assert!(
            close_result.filled_size > Decimal::ZERO,
            "Close order should have filled"
        );

        let balance_after_close = engine.get_account(trader).unwrap().balance.value();

        // Collateral should be returned (balance increases by locked amount, minus the loss from spread)
        // Entry was at 50100, exit at 49900, so loss = 200
        let returned = balance_after_close - balance_after_open;
        let expected_return = collateral_locked - dec!(200);  // Minus the loss from spread
        assert!(
            (returned - expected_return).abs() < dec!(1),
            "Collateral not properly handled: locked={}, returned={}, expected_return={}",
            collateral_locked,
            returned,
            expected_return
        );

        // Position should be closed (either None or size is zero)
        let maybe_pos = engine.get_account(trader).unwrap().get_position(MarketId(1));
        let pos_closed = match maybe_pos {
            None => true,
            Some(p) => p.size.is_zero(),
        };
        assert!(pos_closed, "Position should be closed");
    }
}
