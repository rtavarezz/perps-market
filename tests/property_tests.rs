//! Property-based tests for stress testing core math.
//!
//! These tests verify invariants hold under random inputs.

use perps_core::*;
use proptest::prelude::*;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

// Strategies for generating test data
fn price_strategy() -> impl Strategy<Value = Decimal> {
    (1i64..1_000_000i64).prop_map(|x| Decimal::new(x, 2)) // $0.01 to $10,000
}

fn size_strategy() -> impl Strategy<Value = Decimal> {
    (1i64..10_000i64).prop_map(|x| Decimal::new(x, 4)) // 0.0001 to 1.0
}

fn leverage_strategy() -> impl Strategy<Value = Decimal> {
    (1u32..=50u32).prop_map(Decimal::from) // 1x to 50x
}

fn premium_strategy() -> impl Strategy<Value = Decimal> {
    (-100i64..=100i64).prop_map(|x| Decimal::new(x, 4)) // -1% to +1%
}

proptest! {
    /// Unrealized PnL is zero when mark = entry
    #[test]
    fn pnl_zero_at_entry(
        size in size_strategy(),
        entry in price_strategy(),
    ) {
        let signed_size = SignedSize::new(size);
        let entry_price = Price::new_unchecked(entry);
        
        let pnl = calculate_unrealized_pnl(signed_size, entry_price, entry_price);
        prop_assert_eq!(pnl.value(), Decimal::ZERO);
    }

    /// PnL sign is correct for longs: profit when mark > entry
    #[test]
    fn pnl_sign_long(
        size in size_strategy(),
        entry in price_strategy(),
        delta in -500i64..=500i64,
    ) {
        let signed_size = SignedSize::new(size); // Long
        let entry_price = Price::new_unchecked(entry);
        let mark_val = entry + Decimal::new(delta, 2);
        
        if mark_val > Decimal::ZERO {
            let mark_price = Price::new_unchecked(mark_val);
            let pnl = calculate_unrealized_pnl(signed_size, entry_price, mark_price);
            
            if mark_val > entry {
                prop_assert!(pnl.value() > Decimal::ZERO, "Long should profit when mark > entry");
            } else if mark_val < entry {
                prop_assert!(pnl.value() < Decimal::ZERO, "Long should lose when mark < entry");
            }
        }
    }

    /// PnL sign is correct for shorts: profit when mark < entry
    #[test]
    fn pnl_sign_short(
        size in size_strategy(),
        entry in price_strategy(),
        delta in -500i64..=500i64,
    ) {
        let signed_size = SignedSize::new(-size); // Short
        let entry_price = Price::new_unchecked(entry);
        let mark_val = entry + Decimal::new(delta, 2);
        
        if mark_val > Decimal::ZERO {
            let mark_price = Price::new_unchecked(mark_val);
            let pnl = calculate_unrealized_pnl(signed_size, entry_price, mark_price);
            
            if mark_val < entry {
                prop_assert!(pnl.value() > Decimal::ZERO, "Short should profit when mark < entry");
            } else if mark_val > entry {
                prop_assert!(pnl.value() < Decimal::ZERO, "Short should lose when mark > entry");
            }
        }
    }

    /// Initial margin is always positive for non-zero positions
    #[test]
    fn initial_margin_positive(
        size in size_strategy(),
        price in price_strategy(),
        leverage in leverage_strategy(),
    ) {
        let signed_size = SignedSize::new(size);
        let mark_price = Price::new_unchecked(price);
        let lev = Leverage::new(leverage).unwrap();
        let params = MarginParams::default();
        
        let req = calculate_margin_requirement(signed_size, mark_price, lev, &params);
        
        prop_assert!(req.initial.value() > Decimal::ZERO);
        prop_assert!(req.maintenance.value() > Decimal::ZERO);
        prop_assert!(req.maintenance.value() < req.initial.value());
    }

    /// Maintenance margin is always less than initial margin
    #[test]
    fn maintenance_less_than_initial(
        size in size_strategy(),
        price in price_strategy(),
        leverage in leverage_strategy(),
    ) {
        let signed_size = SignedSize::new(size);
        let mark_price = Price::new_unchecked(price);
        let lev = Leverage::new(leverage).unwrap();
        let params = MarginParams::default();
        
        let req = calculate_margin_requirement(signed_size, mark_price, lev, &params);
        
        prop_assert!(
            req.maintenance.value() <= req.initial.value(),
            "MM {} should be <= IM {}",
            req.maintenance,
            req.initial
        );
    }

    /// Mark price premium is clamped within bounds
    #[test]
    fn premium_clamped(
        raw_premium in premium_strategy(),
    ) {
        let max_premium = dec!(0.05);
        let clamped = clamp_premium(raw_premium, max_premium);
        
        prop_assert!(clamped >= -max_premium);
        prop_assert!(clamped <= max_premium);
    }

    /// Funding rate is clamped within bounds
    #[test]
    fn funding_rate_bounded(
        premium in premium_strategy(),
    ) {
        let params = FundingParams::default();
        let rate = calculate_funding_rate(premium, &params);
        
        prop_assert!(rate >= -params.max_rate);
        prop_assert!(rate <= params.max_rate);
    }

    /// Funding payments are zero-sum between long and short
    #[test]
    fn funding_zero_sum(
        size in size_strategy(),
        price in price_strategy(),
        rate in -100i64..=100i64,
    ) {
        let mark_price = Price::new_unchecked(price);
        let funding_rate = Decimal::new(rate, 6); // Small rate
        
        let long_size = SignedSize::new(size);
        let short_size = SignedSize::new(-size);
        
        let long_payment = calculate_funding_payment(long_size, mark_price, funding_rate);
        let short_payment = calculate_funding_payment(short_size, mark_price, funding_rate);
        
        // Long pays what short receives (and vice versa)
        let sum = long_payment.value() + short_payment.value();
        prop_assert_eq!(sum, Decimal::ZERO, "Funding should be zero-sum");
    }

    /// Position equity = collateral + PnL - funding
    #[test]
    fn position_equity_formula(
        size in size_strategy(),
        entry in price_strategy(),
        mark in price_strategy(),
        collateral in (1000i64..100_000i64).prop_map(|x| Decimal::new(x, 2)),
        funding_delta in (-100i64..100i64).prop_map(|x| Decimal::new(x, 2)),
    ) {
        let signed_size = SignedSize::new(size);
        let entry_price = Price::new_unchecked(entry);
        let mark_price = Price::new_unchecked(mark);
        
        let position = Position::new(
            MarketId(1),
            signed_size,
            entry_price,
            Quote::new(collateral),
            Leverage::new(dec!(10)).unwrap(),
            Decimal::ZERO,
            Timestamp::from_millis(0),
        );
        
        let pnl = position.unrealized_pnl(mark_price);
        let funding = position.pending_funding(funding_delta);
        let equity = position.equity(mark_price, funding_delta);
        
        let expected = collateral + pnl.value() - funding.value();
        prop_assert_eq!(equity.value(), expected);
    }

    /// Liquidation price is below entry for longs
    #[test]
    fn liquidation_price_long_below_entry(
        entry in price_strategy(),
        leverage in leverage_strategy(),
    ) {
        let entry_price = Price::new_unchecked(entry);
        let lev = Leverage::new(leverage).unwrap();
        let mmf = lev.initial_margin_fraction() * dec!(0.5);
        
        let liq_price = calculate_liquidation_price(entry_price, lev, Side::Long, mmf);
        
        prop_assert!(
            liq_price.value() < entry_price.value(),
            "Long liq price {} should be < entry {}",
            liq_price,
            entry_price
        );
    }

    /// Liquidation price is above entry for shorts
    #[test]
    fn liquidation_price_short_above_entry(
        entry in price_strategy(),
        leverage in leverage_strategy(),
    ) {
        let entry_price = Price::new_unchecked(entry);
        let lev = Leverage::new(leverage).unwrap();
        let mmf = lev.initial_margin_fraction() * dec!(0.5);
        
        let liq_price = calculate_liquidation_price(entry_price, lev, Side::Short, mmf);
        
        prop_assert!(
            liq_price.value() > entry_price.value(),
            "Short liq price {} should be > entry {}",
            liq_price,
            entry_price
        );
    }

    /// Higher leverage = tighter liquidation price
    #[test]
    fn higher_leverage_tighter_liquidation(
        entry in (100i64..1_000_000i64).prop_map(|x| Decimal::new(x, 2)), // More realistic range
    ) {
        let entry_price = Price::new_unchecked(entry);
        
        let low_lev = Leverage::new(dec!(5)).unwrap();
        let high_lev = Leverage::new(dec!(20)).unwrap();
        
        let low_mmf = low_lev.initial_margin_fraction() * dec!(0.5);
        let high_mmf = high_lev.initial_margin_fraction() * dec!(0.5);
        
        let low_liq = calculate_liquidation_price(entry_price, low_lev, Side::Long, low_mmf);
        let high_liq = calculate_liquidation_price(entry_price, high_lev, Side::Long, high_mmf);
        
        // Higher leverage = less room before liquidation = higher liq price for long
        prop_assert!(
            high_liq.value() > low_liq.value(),
            "20x liq {} should be closer to entry {} than 5x liq {}",
            high_liq,
            entry_price,
            low_liq
        );
    }
}

/// Non-proptest stress scenarios
#[cfg(test)]
mod stress_tests {
    use super::*;

    #[test]
    fn extreme_price_movements() {
        let entry = Price::new_unchecked(dec!(50000));
        let size = SignedSize::new(dec!(1));
        let _leverage = Leverage::new(dec!(10)).unwrap();
        let _params = MarginParams::default();

        // 50% crash
        let crash_price = Price::new_unchecked(dec!(25000));
        let pnl = calculate_unrealized_pnl(size, entry, crash_price);
        assert_eq!(pnl.value(), dec!(-25000));

        // 100% pump
        let pump_price = Price::new_unchecked(dec!(100000));
        let pnl_pump = calculate_unrealized_pnl(size, entry, pump_price);
        assert_eq!(pnl_pump.value(), dec!(50000));
    }

    #[test]
    fn max_leverage_margin_calculation() {
        let size = SignedSize::new(dec!(1));
        let price = Price::new_unchecked(dec!(50000));
        let max_lev = Leverage::new(dec!(50)).unwrap();
        let params = MarginParams::default();

        let req = calculate_margin_requirement(size, price, max_lev, &params);

        // At 50x: IM = 2%, MM = 1%
        assert_eq!(req.initial.value(), dec!(1000)); // 50000 * 0.02
        assert_eq!(req.maintenance.value(), dec!(500)); // 1000 * 0.5
    }

    #[test]
    fn insurance_fund_depletion() {
        let mut fund = InsuranceFund::new(Quote::new(dec!(10000)));

        // Multiple bad debts
        for _ in 0..5 {
            let covered = fund.cover_bad_debt(Quote::new(dec!(3000)));
            if fund.balance.value().is_zero() {
                // Fund depleted, can only partially cover
                assert!(covered.value() < dec!(3000));
            }
        }

        assert!(fund.balance.value().is_zero() || fund.balance.value() < dec!(10000));
    }

    #[test]
    fn cascading_liquidations_dont_overflow() {
        // Simulate many positions being liquidated
        let params = MarginParams::default();
        let liq_params = LiquidationParams::default();
        let entry = Price::new_unchecked(dec!(50000));

        for i in 1..100 {
            let size = SignedSize::new(Decimal::from(i));
            let leverage = Leverage::new(dec!(10)).unwrap();
            let margin_req = calculate_margin_requirement(size, entry, leverage, &params);

            // Position at margin call
            let equity = Quote::new(margin_req.maintenance.value() - dec!(1));
            let notional = notional_value(size, entry);

            let status = evaluate_liquidation(
                equity,
                &margin_req,
                notional,
                entry,
                entry,
                Side::Long,
            );

            assert!(matches!(status, LiquidationStatus::Liquidatable { .. }));

            let penalty = calculate_liquidation_penalty(notional, &liq_params);
            assert!(penalty.total.value() > Decimal::ZERO);
        }
    }

    #[test]
    fn funding_accumulation_over_time() {
        let params = FundingParams::default();
        let mut state = FundingState::new(Timestamp::from_millis(0));

        let mark = Price::new_unchecked(dec!(50500)); // 1% premium
        let index = Price::new_unchecked(dec!(50000));

        // Simulate 24 hours (3 funding periods)
        for hour in 1..=24 {
            let timestamp = Timestamp::from_millis(hour * 3600 * 1000);
            state = update_funding_state(&state, mark, index, timestamp, &params);
        }

        // Cumulative funding should have accumulated
        assert!(state.cumulative_funding > Decimal::ZERO);
        assert!(state.current_rate > Decimal::ZERO); // Premium means positive rate
    }
}
