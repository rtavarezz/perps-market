// 5.0: funding rates. every 8hrs longs pay shorts or vice versa to keep perp price near spot.
// 5.0 has the params/state structs. 5.1 has the rate calculation logic.

use crate::types::{Price, Quote, SignedSize, Timestamp};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FundingParams {
    pub max_rate: Decimal,
    pub interest_rate: Decimal,
    pub period_hours: Decimal,
    pub dampening_factor: Decimal,
    // fraction of gross funding routed to the LP pool (0.10 = 10%).
    // payers pay full amount, receivers get (1 - this), remainder goes to pool.
    pub lp_fee_fraction: Decimal,
}

impl Default for FundingParams {
    fn default() -> Self {
        Self {
            max_rate: dec!(0.01),
            interest_rate: dec!(0.0001),
            period_hours: dec!(8),
            dampening_factor: dec!(0.5),
            lp_fee_fraction: dec!(0.10),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FundingState {
    pub current_rate: Decimal,
    pub cumulative_funding: Decimal,
    pub last_update: Timestamp,
    pub twap_premium: Decimal,
}

impl FundingState {
    pub fn new(timestamp: Timestamp) -> Self {
        Self {
            current_rate: Decimal::ZERO,
            cumulative_funding: Decimal::ZERO,
            last_update: timestamp,
            twap_premium: Decimal::ZERO,
        }
    }
}

// 5.1: how far perp is from spot. positive = perp above spot
pub fn calculate_premium_index(mark_price: Price, index_price: Price) -> Decimal {
    (mark_price.value() - index_price.value()) / index_price.value()
}

// 5.2: dampens and clamps the rate to prevent wild swings
pub fn calculate_funding_rate(premium_index: Decimal, params: &FundingParams) -> Decimal {
    let dampened_premium = premium_index * params.dampening_factor;
    let rate = dampened_premium + params.interest_rate;
    rate.max(-params.max_rate).min(params.max_rate)
}

pub fn calculate_accrued_funding(
    funding_rate: Decimal,
    hours_elapsed: Decimal,
    period_hours: Decimal,
) -> Decimal {
    funding_rate * hours_elapsed / period_hours
}

// 5.3: how much you pay/receive. size * price * rate
pub fn calculate_funding_payment(
    position_size: SignedSize,
    mark_price: Price,
    funding_rate: Decimal,
) -> Quote {
    let payment = position_size.value() * mark_price.value() * funding_rate;
    Quote::new(payment)
}

pub fn calculate_funding_from_cumulative(
    position_size: SignedSize,
    entry_cumulative: Decimal,
    current_cumulative: Decimal,
) -> Quote {
    let funding_delta = current_cumulative - entry_cumulative;
    Quote::new(position_size.value() * funding_delta)
}

// 5.4: updates funding state with new rate and cumulative amount
pub fn update_funding_state(
    state: &FundingState,
    mark_price: Price,
    index_price: Price,
    current_time: Timestamp,
    params: &FundingParams,
) -> FundingState {
    let hours_elapsed = state.last_update.elapsed_hours(&current_time);
    let premium = calculate_premium_index(mark_price, index_price);

    // EMA smoothing for TWAP
    let alpha = dec!(0.1);
    let new_twap = alpha * premium + (Decimal::ONE - alpha) * state.twap_premium;
    let new_rate = calculate_funding_rate(new_twap, params);
    let accrued = calculate_accrued_funding(new_rate, hours_elapsed, params.period_hours);
    let new_cumulative = state.cumulative_funding + accrued * mark_price.value();

    FundingState {
        current_rate: new_rate,
        cumulative_funding: new_cumulative,
        last_update: current_time,
        twap_premium: new_twap,
    }
}

pub fn annual_to_period_rate(annual_rate: Decimal, periods_per_year: u32) -> Decimal {
    annual_rate / Decimal::from(periods_per_year)
}

pub fn period_to_annual_rate(period_rate: Decimal, periods_per_year: u32) -> Decimal {
    period_rate * Decimal::from(periods_per_year)
}

pub fn annualized_funding_rate(period_rate: Decimal) -> Decimal {
    period_rate * dec!(1095)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn test_params() -> FundingParams {
        FundingParams::default()
    }

    #[test]
    fn premium_index_positive() {
        let mark = Price::new_unchecked(dec!(50500));
        let index = Price::new_unchecked(dec!(50000));

        let premium = calculate_premium_index(mark, index);
        assert_eq!(premium, dec!(0.01)); // 1% premium
    }

    #[test]
    fn premium_index_negative() {
        let mark = Price::new_unchecked(dec!(49500));
        let index = Price::new_unchecked(dec!(50000));

        let premium = calculate_premium_index(mark, index);
        assert_eq!(premium, dec!(-0.01)); // 1% discount
    }

    #[test]
    fn funding_rate_with_dampening() {
        let params = test_params();
        let premium = dec!(0.02); // 2% premium

        let rate = calculate_funding_rate(premium, &params);

        // Dampened: 0.02 * 0.5 = 0.01, + 0.0001 interest = 0.0101
        // Clamped to max 0.01
        assert_eq!(rate, dec!(0.01));
    }

    #[test]
    fn funding_rate_small_premium() {
        let params = test_params();
        let premium = dec!(0.001); // 0.1% premium

        let rate = calculate_funding_rate(premium, &params);

        // 0.001 * 0.5 = 0.0005 + 0.0001 = 0.0006
        assert_eq!(rate, dec!(0.0006));
    }

    #[test]
    fn funding_payment_long() {
        let size = SignedSize::new(dec!(1)); // 1 BTC long
        let price = Price::new_unchecked(dec!(50000));
        let rate = dec!(0.001); // 0.1%

        let payment = calculate_funding_payment(size, price, rate);

        // 1 * 50000 * 0.001 = 50 (long pays 50)
        assert_eq!(payment.value(), dec!(50));
    }

    #[test]
    fn funding_payment_short() {
        let size = SignedSize::new(dec!(-1)); // 1 BTC short
        let price = Price::new_unchecked(dec!(50000));
        let rate = dec!(0.001); // 0.1%

        let payment = calculate_funding_payment(size, price, rate);

        // -1 * 50000 * 0.001 = -50 (short receives 50)
        assert_eq!(payment.value(), dec!(-50));
    }

    #[test]
    fn accrued_funding_full_period() {
        let rate = dec!(0.001);
        let hours = dec!(8);
        let period = dec!(8);

        let accrued = calculate_accrued_funding(rate, hours, period);
        assert_eq!(accrued, dec!(0.001)); // Full period = full rate
    }

    #[test]
    fn accrued_funding_half_period() {
        let rate = dec!(0.001);
        let hours = dec!(4);
        let period = dec!(8);

        let accrued = calculate_accrued_funding(rate, hours, period);
        assert_eq!(accrued, dec!(0.0005)); // Half period = half rate
    }

    #[test]
    fn cumulative_funding_tracking() {
        let size = SignedSize::new(dec!(1));
        let entry_cumulative = dec!(100);
        let current_cumulative = dec!(150);

        let payment = calculate_funding_from_cumulative(size, entry_cumulative, current_cumulative);

        // 1 * (150 - 100) = 50 paid
        assert_eq!(payment.value(), dec!(50));
    }

    #[test]
    fn annualized_rate() {
        let period_rate = dec!(0.001); // 0.1% per 8h

        let annual = annualized_funding_rate(period_rate);

        // 0.001 * 3 * 365 = 1.095 = 109.5% APR
        assert_eq!(annual, dec!(1.095));
    }

    #[test]
    fn funding_state_update() {
        let params = test_params();
        let t0 = Timestamp::from_millis(0);
        let t1 = Timestamp::from_millis(8 * 3600 * 1000); // 8 hours later

        let initial_state = FundingState::new(t0);
        let mark = Price::new_unchecked(dec!(50500)); // 1% premium
        let index = Price::new_unchecked(dec!(50000));

        let new_state = update_funding_state(&initial_state, mark, index, t1, &params);

        // Funding rate should be positive (longs pay)
        assert!(new_state.current_rate > Decimal::ZERO);
        // Cumulative should have increased
        assert!(new_state.cumulative_funding > Decimal::ZERO);
    }
}
