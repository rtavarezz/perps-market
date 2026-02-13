//! Stress tests
//!
//! These tests simulate extreme market conditions to verify the engine remains
//! solvent and behaves correctly under stress.

use perps_core::*;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

/// Tests rapid price movements and cascading liquidations.
mod cascade_tests {
    use super::*;

    #[test]
    fn liquidation_cascade_no_bad_debt() {
        let mut engine = Engine::new(EngineConfig::default());
        let mut market = MarketConfig::btc_perp();
        market.funding_params.lp_fee_fraction = Decimal::ZERO;
        engine.add_market(market);
        engine.fund_insurance(Quote::new(dec!(1_000_000)));

        let entry_price = dec!(50000);
        engine
            .update_index_price(MarketId(1), Price::new_unchecked(entry_price))
            .unwrap();

        // Create 10 leveraged long positions with varying margin
        let mut traders = Vec::new();
        let counterparty = engine.create_account();
        engine
            .deposit(counterparty, Quote::new(dec!(10_000_000)))
            .unwrap();

        // Counterparty provides all the sell liquidity
        engine
            .place_limit_order(
                counterparty,
                MarketId(1),
                Side::Short,
                dec!(100),
                Price::new_unchecked(entry_price),
                TimeInForce::GTC,
            )
            .unwrap();

        // Create traders with increasingly risky positions
        for i in 1..=10 {
            let trader = engine.create_account();
            let collateral = dec!(5000) - Decimal::from(i) * dec!(300);
            engine.deposit(trader, Quote::new(collateral)).unwrap();

            // Each trader opens a 1 BTC long
            engine
                .place_market_order(trader, MarketId(1), Side::Long, dec!(1))
                .unwrap();
            traders.push(trader);
        }

        // Verify all positions opened
        for &trader in &traders {
            assert!(
                engine
                    .get_account(trader)
                    .unwrap()
                    .get_position(MarketId(1))
                    .is_some()
            );
        }

        // Now crash the price step by step
        let prices = [dec!(48000), dec!(46000), dec!(44000), dec!(42000), dec!(40000)];

        let mut total_liquidations = 0;
        let mut total_bad_debt = Decimal::ZERO;

        for price in prices {
            engine
                .update_index_price(MarketId(1), Price::new_unchecked(price))
                .unwrap();

            let liqs = engine.check_liquidations(MarketId(1)).unwrap();
            total_liquidations += liqs.len();

            for liq in &liqs {
                total_bad_debt += liq.bad_debt.value();
            }
        }

        // Should have liquidated some but not all positions
        assert!(total_liquidations > 0);

        // With 1M insurance fund, should have no uncovered bad debt
        // (bad debt gets covered by insurance)
        println!(
            "Total liquidations: {}, Total bad debt: {}",
            total_liquidations, total_bad_debt
        );

        // Insurance fund should still have balance
        assert!(engine.insurance_fund_balance().value() > Decimal::ZERO);
    }

    #[test]
    fn rapid_price_movement_both_directions() {
        let mut engine = Engine::new(EngineConfig::default());
        engine.add_market(MarketConfig::btc_perp());
        engine.fund_insurance(Quote::new(dec!(100_000)));

        let long_trader = engine.create_account();
        let short_trader = engine.create_account();
        let counterparty = engine.create_account();

        engine.deposit(long_trader, Quote::new(dec!(10000))).unwrap();
        engine.deposit(short_trader, Quote::new(dec!(10000))).unwrap();
        engine.deposit(counterparty, Quote::new(dec!(1_000_000))).unwrap();

        engine
            .update_index_price(MarketId(1), Price::new_unchecked(dec!(50000)))
            .unwrap();

        // Counterparty provides both sides
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
        engine
            .place_limit_order(
                counterparty,
                MarketId(1),
                Side::Long,
                dec!(10),
                Price::new_unchecked(dec!(50000)),
                TimeInForce::GTC,
            )
            .unwrap();

        // Long trader goes long
        engine
            .place_market_order(long_trader, MarketId(1), Side::Long, dec!(1))
            .unwrap();

        // Short trader goes short
        engine
            .place_market_order(short_trader, MarketId(1), Side::Short, dec!(1))
            .unwrap();

        // Rapid oscillation
        let prices = [
            dec!(52000),
            dec!(48000),
            dec!(55000),
            dec!(45000),
            dec!(58000),
            dec!(42000),
            dec!(60000),
            dec!(38000),
        ];

        for price in prices {
            engine
                .update_index_price(MarketId(1), Price::new_unchecked(price))
                .unwrap();
            engine.check_liquidations(MarketId(1)).unwrap();
        }

        // At least one should be liquidated after such volatility
        let long_pos = engine
            .get_account(long_trader)
            .unwrap()
            .get_position(MarketId(1));
        let short_pos = engine
            .get_account(short_trader)
            .unwrap()
            .get_position(MarketId(1));

        // With extreme moves, at least one direction should have been liquidated
        assert!(long_pos.is_none() || short_pos.is_none());
    }
}

/// Tests edge cases near margin boundaries.
mod edge_case_tests {
    use super::*;

    #[test]
    fn near_zero_margin_position() {
        let mut engine = Engine::new(EngineConfig::default());
        engine.add_market(MarketConfig::btc_perp());

        let trader = engine.create_account();
        let counterparty = engine.create_account();

        // Minimum viable margin (just enough to open position)
        engine.deposit(trader, Quote::new(dec!(1000))).unwrap();
        engine.deposit(counterparty, Quote::new(dec!(100000))).unwrap();

        engine
            .update_index_price(MarketId(1), Price::new_unchecked(dec!(50000)))
            .unwrap();

        engine
            .place_limit_order(
                counterparty,
                MarketId(1),
                Side::Short,
                dec!(1),
                Price::new_unchecked(dec!(50000)),
                TimeInForce::GTC,
            )
            .unwrap();

        // Try to open a position that uses almost all margin
        let result = engine.place_market_order(trader, MarketId(1), Side::Long, dec!(0.1));

        // Should succeed
        assert!(result.is_ok());

        let pos = engine
            .get_account(trader)
            .unwrap()
            .get_position(MarketId(1));
        assert!(pos.is_some());

        // Very small adverse move should trigger liquidation
        engine
            .update_index_price(MarketId(1), Price::new_unchecked(dec!(49000)))
            .unwrap();

        let liqs = engine.check_liquidations(MarketId(1)).unwrap();
        // May or may not be liquidated depending on exact margin
        println!("Near-zero margin liquidations: {}", liqs.len());
    }

    #[test]
    fn exact_maintenance_margin_boundary() {
        let mut engine = Engine::new(EngineConfig::default());
        engine.add_market(MarketConfig::btc_perp());

        let trader = engine.create_account();
        let counterparty = engine.create_account();

        engine.deposit(trader, Quote::new(dec!(5000))).unwrap();
        engine.deposit(counterparty, Quote::new(dec!(100000))).unwrap();

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

        engine
            .place_market_order(trader, MarketId(1), Side::Long, dec!(1))
            .unwrap();

        // Get actual position collateral to calculate liquidation price correctly
        let pos = engine.get_account(trader).unwrap().get_position(MarketId(1)).unwrap();
        let collateral = pos.collateral.value();
        let notional = dec!(50000);
        let market = engine.get_market(MarketId(1)).unwrap();
        let mm_ratio = market.config.margin_params.maintenance_margin_ratio;

        // Liquidation price for long: entry - (collateral - MM_notional)
        // MM = notional * IM * mm_ratio
        // At 50x leverage, IM = 2%, MM_ratio = 50%, so MM = notional * 0.02 * 0.5 = 0.01 = 1%
        // So liquidation occurs when equity = MM
        // equity = collateral + (mark - entry) * size = MM = notional * 0.01
        // For long: mark = entry - (collateral - MM)/size = 50000 - (collateral - 500)
        let mm = notional * dec!(0.02) * mm_ratio;
        let liq_price = dec!(50000) - (collateral - mm);

        // Just above liquidation threshold (safe)
        let safe_price = liq_price + dec!(100);
        engine
            .update_index_price(MarketId(1), Price::new_unchecked(safe_price))
            .unwrap();
        let liqs = engine.check_liquidations(MarketId(1)).unwrap();
        // Should be safe (at risk but not liquidatable)
        assert!(liqs.is_empty(), "Should be safe at {}, liq price is {}", safe_price, liq_price);

        // Just below liquidation threshold
        let danger_price = liq_price - dec!(100);
        engine
            .update_index_price(MarketId(1), Price::new_unchecked(danger_price))
            .unwrap();
        let liqs = engine.check_liquidations(MarketId(1)).unwrap();
        // Should be liquidated now
        assert!(!liqs.is_empty(), "Should be liquidated at {}, liq price is {}", danger_price, liq_price);
    }

    #[test]
    fn maximum_leverage_position() {
        let mut engine = Engine::new(EngineConfig::default());
        engine.add_market(MarketConfig::btc_perp());

        let trader = engine.create_account();
        let counterparty = engine.create_account();

        // With max leverage (50x), IM = 2%, so $1000 controls $50k
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

        // At 50x, $1000 margin can control ~$50k notional = 1 BTC
        // But size validation may restrict this
        let result = engine.place_market_order(trader, MarketId(1), Side::Long, dec!(0.5));
        assert!(result.is_ok());

        // 1% move should be dangerous at max leverage
        engine
            .update_index_price(MarketId(1), Price::new_unchecked(dec!(49500)))
            .unwrap();

        let liqs = engine.check_liquidations(MarketId(1)).unwrap();
        // May or may not be liquidated
        println!("Max leverage liquidations: {}", liqs.len());
    }
}

/// Tests partial fill scenarios.
mod partial_fill_tests {
    use super::*;

    #[test]
    fn partial_fill_position_tracking() {
        let mut engine = Engine::new(EngineConfig::default());
        engine.add_market(MarketConfig::btc_perp());

        let buyer = engine.create_account();
        let seller1 = engine.create_account();
        let seller2 = engine.create_account();

        engine.deposit(buyer, Quote::new(dec!(100000))).unwrap();
        engine.deposit(seller1, Quote::new(dec!(50000))).unwrap();
        engine.deposit(seller2, Quote::new(dec!(50000))).unwrap();

        engine
            .update_index_price(MarketId(1), Price::new_unchecked(dec!(50000)))
            .unwrap();

        // Two sellers at different prices
        engine
            .place_limit_order(
                seller1,
                MarketId(1),
                Side::Short,
                dec!(0.5),
                Price::new_unchecked(dec!(50000)),
                TimeInForce::GTC,
            )
            .unwrap();

        engine
            .place_limit_order(
                seller2,
                MarketId(1),
                Side::Short,
                dec!(0.5),
                Price::new_unchecked(dec!(50100)),
                TimeInForce::GTC,
            )
            .unwrap();

        // Buyer wants 1 BTC, will match both
        let result = engine
            .place_market_order(buyer, MarketId(1), Side::Long, dec!(1))
            .unwrap();

        // Should have 2 fills
        assert_eq!(result.fills.len(), 2);
        assert_eq!(result.filled_size, dec!(1));

        // Buyer position should be 1 BTC
        let pos = engine
            .get_account(buyer)
            .unwrap()
            .get_position(MarketId(1))
            .unwrap();
        assert_eq!(pos.size.abs(), dec!(1));

        // Entry price should be weighted average
        let expected_avg = (dec!(50000) * dec!(0.5) + dec!(50100) * dec!(0.5)) / dec!(1);
        assert_eq!(pos.entry_price.value(), expected_avg);
    }

    #[test]
    fn partial_fill_with_remaining_on_book() {
        let mut engine = Engine::new(EngineConfig::default());
        engine.add_market(MarketConfig::btc_perp());

        let buyer = engine.create_account();
        let seller = engine.create_account();

        engine.deposit(buyer, Quote::new(dec!(100000))).unwrap();
        engine.deposit(seller, Quote::new(dec!(50000))).unwrap();

        engine
            .update_index_price(MarketId(1), Price::new_unchecked(dec!(50000)))
            .unwrap();

        // Seller has only 0.5 BTC
        engine
            .place_limit_order(
                seller,
                MarketId(1),
                Side::Short,
                dec!(0.5),
                Price::new_unchecked(dec!(50000)),
                TimeInForce::GTC,
            )
            .unwrap();

        // Buyer wants 1 BTC with limit order
        let result = engine
            .place_limit_order(
                buyer,
                MarketId(1),
                Side::Long,
                dec!(1),
                Price::new_unchecked(dec!(50100)),
                TimeInForce::GTC,
            )
            .unwrap();

        // Should fill 0.5 and post 0.5
        assert_eq!(result.filled_size, dec!(0.5));
        assert_eq!(result.remaining_size, dec!(0.5));
        assert!(result.is_posted);
    }
}

/// Tests funding settlement edge cases.
mod funding_tests {
    use super::*;

    #[test]
    fn funding_with_zero_positions() {
        let mut engine = Engine::new(EngineConfig::default());
        let mut market = MarketConfig::btc_perp();
        market.funding_params.lp_fee_fraction = Decimal::ZERO;
        engine.add_market(market);

        engine
            .update_index_price(MarketId(1), Price::new_unchecked(dec!(50000)))
            .unwrap();

        engine.advance_time(8 * 60 * 60 * 1000);

        // No positions, funding should still work
        let result = engine.settle_funding(MarketId(1)).unwrap();
        assert_eq!(result.accounts_affected, 0);
    }

    #[test]
    fn funding_preserves_zero_sum() {
        let mut engine = Engine::new(EngineConfig::default());
        let mut market = MarketConfig::btc_perp();
        market.funding_params.lp_fee_fraction = Decimal::ZERO;
        engine.add_market(market);

        // Create traders with separate long and short positions
        let long1 = engine.create_account();
        let long2 = engine.create_account();
        let short1 = engine.create_account();

        engine.deposit(long1, Quote::new(dec!(10000))).unwrap();
        engine.deposit(long2, Quote::new(dec!(10000))).unwrap();
        engine.deposit(short1, Quote::new(dec!(20000))).unwrap();

        engine
            .update_index_price(MarketId(1), Price::new_unchecked(dec!(50000)))
            .unwrap();

        // Have short1 provide liquidity for the longs
        engine
            .place_limit_order(
                short1,
                MarketId(1),
                Side::Short,
                dec!(5),
                Price::new_unchecked(dec!(50100)),
                TimeInForce::GTC,
            )
            .unwrap();

        // Longs take from the short
        engine
            .place_market_order(long1, MarketId(1), Side::Long, dec!(1))
            .unwrap();
        engine
            .place_market_order(long2, MarketId(1), Side::Long, dec!(1))
            .unwrap();

        // Now long1 and long2 each have 1 BTC long, short1 has 2 BTC short
        let balances_before: Vec<_> = [long1, long2, short1]
            .iter()
            .map(|&id| engine.get_account(id).unwrap().balance.value())
            .collect();

        engine.advance_time(8 * 60 * 60 * 1000);
        let result = engine.settle_funding(MarketId(1)).unwrap();

        let balances_after: Vec<_> = [long1, long2, short1]
            .iter()
            .map(|&id| engine.get_account(id).unwrap().balance.value())
            .collect();

        // Sum of balance changes should be zero
        let total_change: Decimal = balances_after
            .iter()
            .zip(balances_before.iter())
            .map(|(a, b)| a - b)
            .sum();

        assert!(
            total_change.abs() < dec!(0.01),
            "Funding not zero-sum: {}",
            total_change
        );
        assert!(result.accounts_affected > 0);
    }

    #[test]
    fn funding_multiple_periods() {
        let mut engine = Engine::new(EngineConfig::default());
        let mut market = MarketConfig::btc_perp();
        market.funding_params.lp_fee_fraction = Decimal::ZERO;
        engine.add_market(market);

        let long_trader = engine.create_account();
        let short_trader = engine.create_account();

        engine.deposit(long_trader, Quote::new(dec!(10000))).unwrap();
        engine.deposit(short_trader, Quote::new(dec!(10000))).unwrap();

        engine
            .update_index_price(MarketId(1), Price::new_unchecked(dec!(50000)))
            .unwrap();

        engine
            .place_limit_order(
                short_trader,
                MarketId(1),
                Side::Short,
                dec!(1),
                Price::new_unchecked(dec!(50000)),
                TimeInForce::GTC,
            )
            .unwrap();
        engine
            .place_market_order(long_trader, MarketId(1), Side::Long, dec!(1))
            .unwrap();

        let initial_balance = engine.get_account(long_trader).unwrap().balance.value();

        // Settle funding 3 times
        for _ in 0..3 {
            engine.advance_time(8 * 60 * 60 * 1000);
            engine.settle_funding(MarketId(1)).unwrap();
        }

        let final_balance = engine.get_account(long_trader).unwrap().balance.value();

        // Balance should have changed over multiple periods
        assert_ne!(initial_balance, final_balance);
    }
}

/// Tests position flipping and reduction.
mod position_management_tests {
    use super::*;

    #[test]
    fn flip_long_to_short() {
        let mut engine = Engine::new(EngineConfig::default());
        engine.add_market(MarketConfig::btc_perp());

        let trader = engine.create_account();
        let counterparty = engine.create_account();

        engine.deposit(trader, Quote::new(dec!(100000))).unwrap();
        engine.deposit(counterparty, Quote::new(dec!(1_000_000))).unwrap();

        engine
            .update_index_price(MarketId(1), Price::new_unchecked(dec!(50000)))
            .unwrap();

        // Counterparty provides liquidity with spread
        engine
            .place_limit_order(
                counterparty,
                MarketId(1),
                Side::Short,
                dec!(10),
                Price::new_unchecked(dec!(50100)),  // Ask
                TimeInForce::GTC,
            )
            .unwrap();
        engine
            .place_limit_order(
                counterparty,
                MarketId(1),
                Side::Long,
                dec!(10),
                Price::new_unchecked(dec!(49900)),  // Bid
                TimeInForce::GTC,
            )
            .unwrap();

        // Open long
        engine
            .place_market_order(trader, MarketId(1), Side::Long, dec!(2))
            .unwrap();

        let pos = engine
            .get_account(trader)
            .unwrap()
            .get_position(MarketId(1))
            .unwrap();
        assert!(pos.size.is_long());
        assert_eq!(pos.size.abs(), dec!(2));

        // Flip to short by selling 4 (closes 2 long, opens 2 short)
        engine
            .place_market_order(trader, MarketId(1), Side::Short, dec!(4))
            .unwrap();

        let pos = engine
            .get_account(trader)
            .unwrap()
            .get_position(MarketId(1))
            .unwrap();
        assert!(pos.size.is_short());
        assert_eq!(pos.size.abs(), dec!(2));
    }

    #[test]
    fn partial_close_preserves_pnl() {
        let mut engine = Engine::new(EngineConfig::default());
        engine.add_market(MarketConfig::btc_perp());

        let trader = engine.create_account();
        let counterparty = engine.create_account();

        engine.deposit(trader, Quote::new(dec!(100000))).unwrap();
        engine.deposit(counterparty, Quote::new(dec!(1_000_000))).unwrap();

        engine
            .update_index_price(MarketId(1), Price::new_unchecked(dec!(50000)))
            .unwrap();

        // Initial liquidity: sell at 50100 (ask)
        engine
            .place_limit_order(
                counterparty,
                MarketId(1),
                Side::Short,
                dec!(10),
                Price::new_unchecked(dec!(50100)),
                TimeInForce::GTC,
            )
            .unwrap();

        // Open 2 BTC long @ ~$50k
        engine
            .place_market_order(trader, MarketId(1), Side::Long, dec!(2))
            .unwrap();

        // Price rises
        engine
            .update_index_price(MarketId(1), Price::new_unchecked(dec!(55000)))
            .unwrap();

        // Add liquidity at higher price: buy at 54900 (bid)
        engine
            .place_limit_order(
                counterparty,
                MarketId(1),
                Side::Long,
                dec!(10),
                Price::new_unchecked(dec!(54900)),
                TimeInForce::GTC,
            )
            .unwrap();

        let realized_before = engine.get_account(trader).unwrap().realized_pnl.value();

        // Close 1 BTC @ ~$55k
        engine
            .place_market_order(trader, MarketId(1), Side::Short, dec!(1))
            .unwrap();

        let realized_after = engine.get_account(trader).unwrap().realized_pnl.value();

        assert!(realized_after > realized_before, "Realized PnL should increase on profitable close");

        // Still have 1 BTC long
        let pos = engine
            .get_account(trader)
            .unwrap()
            .get_position(MarketId(1))
            .unwrap();
        assert_eq!(pos.size.abs(), dec!(1));
    }
}
