// 13.0: mark price derivation. blends index price with order book mid price.
// uses clamped premium (±5% max) and EMA smoothing to resist manipulation.

use crate::types::{Price, Quote};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkPriceParams {
    pub max_premium: Decimal,
    pub ema_alpha: Decimal,
    pub index_weight: Decimal,
}

impl Default for MarkPriceParams {
    fn default() -> Self {
        Self {
            max_premium: dec!(0.05),
            ema_alpha: dec!(0.1),
            index_weight: dec!(0.75),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkPriceState {
    pub mark_price: Price,
    pub premium_index: Decimal,
    pub last_index_price: Price,
    pub last_mid_price: Option<Price>,
}

impl MarkPriceState {
    pub fn new(initial_index_price: Price) -> Self {
        Self {
            mark_price: initial_index_price,
            premium_index: Decimal::ZERO,
            last_index_price: initial_index_price,
            last_mid_price: None,
        }
    }
}

pub fn calculate_raw_premium(mid_price: Price, index_price: Price) -> Decimal {
    (mid_price.value() - index_price.value()) / index_price.value()
}

pub fn clamp_premium(premium: Decimal, max_premium: Decimal) -> Decimal {
    premium.max(-max_premium).min(max_premium)
}

pub fn smooth_premium(current: Decimal, previous: Decimal, alpha: Decimal) -> Decimal {
    alpha * current + (Decimal::ONE - alpha) * previous
}

pub fn mark_price_from_premium(index_price: Price, premium: Decimal) -> Price {
    let value = index_price.value() * (Decimal::ONE + premium);
    Price::new_unchecked(value)
}

pub fn blend_prices(index: Price, mid: Price, index_weight: Decimal) -> Price {
    let value =
        index.value() * index_weight + mid.value() * (Decimal::ONE - index_weight);
    Price::new_unchecked(value)
}

pub fn update_mark_price(
    state: &MarkPriceState,
    new_index_price: Price,
    new_mid_price: Option<Price>,
    params: &MarkPriceParams,
) -> MarkPriceState {
    // If no mid price, mark = index
    let effective_mid = new_mid_price.unwrap_or(new_index_price);

    // Calculate raw premium
    let raw_premium = calculate_raw_premium(effective_mid, new_index_price);

    // Clamp to prevent manipulation
    let clamped_premium = clamp_premium(raw_premium, params.max_premium);

    // Smooth with EMA
    let smoothed_premium = smooth_premium(clamped_premium, state.premium_index, params.ema_alpha);

    // Calculate mark price from index + premium
    let mark_price = mark_price_from_premium(new_index_price, smoothed_premium);

    MarkPriceState {
        mark_price,
        premium_index: smoothed_premium,
        last_index_price: new_index_price,
        last_mid_price: new_mid_price,
    }
}

// 13.1: estimate execution price for a large order, including slippage
pub fn estimate_impact_price(
    mark_price: Price,
    size: Decimal,
    liquidity_depth: Quote,
    is_buy: bool,
) -> Price {
    if liquidity_depth.value().is_zero() {
        return mark_price;
    }

    // Simple linear impact model: price moves proportionally to size/depth
    // More sophisticated models would use order book shape
    let notional = size.abs() * mark_price.value();
    let impact_fraction = notional / liquidity_depth.value();
    let impact = impact_fraction * dec!(0.001); // 0.1% per unit depth

    let adjustment = if is_buy {
        Decimal::ONE + impact
    } else {
        Decimal::ONE - impact
    };

    Price::new_unchecked(mark_price.value() * adjustment)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn test_params() -> MarkPriceParams {
        MarkPriceParams::default()
    }

    #[test]
    fn raw_premium_positive() {
        let mid = Price::new_unchecked(dec!(50500)); // Mid above index
        let index = Price::new_unchecked(dec!(50000));

        let premium = calculate_raw_premium(mid, index);
        assert_eq!(premium, dec!(0.01)); // 1% premium
    }

    #[test]
    fn raw_premium_negative() {
        let mid = Price::new_unchecked(dec!(49500)); // Mid below index
        let index = Price::new_unchecked(dec!(50000));

        let premium = calculate_raw_premium(mid, index);
        assert_eq!(premium, dec!(-0.01)); // -1% discount
    }

    #[test]
    fn premium_clamped() {
        let extreme_premium = dec!(0.10); // 10%
        let max = dec!(0.05);

        let clamped = clamp_premium(extreme_premium, max);
        assert_eq!(clamped, dec!(0.05));

        let clamped_neg = clamp_premium(dec!(-0.10), max);
        assert_eq!(clamped_neg, dec!(-0.05));
    }

    #[test]
    fn ema_smoothing() {
        let current = dec!(0.02);
        let previous = dec!(0.01);
        let alpha = dec!(0.5);

        let smoothed = smooth_premium(current, previous, alpha);
        assert_eq!(smoothed, dec!(0.015)); // Weighted average
    }

    #[test]
    fn mark_from_premium() {
        let index = Price::new_unchecked(dec!(50000));
        let premium = dec!(0.01); // 1%

        let mark = mark_price_from_premium(index, premium);
        assert_eq!(mark.value(), dec!(50500));
    }

    #[test]
    fn full_mark_price_update() {
        let params = test_params();
        let initial_state = MarkPriceState::new(Price::new_unchecked(dec!(50000)));

        let new_index = Price::new_unchecked(dec!(50000));
        let new_mid = Some(Price::new_unchecked(dec!(50250))); // 0.5% premium

        let new_state = update_mark_price(&initial_state, new_index, new_mid, &params);

        // Premium should be smoothed: 0.005 * 0.1 + 0 * 0.9 = 0.0005
        assert_eq!(new_state.premium_index, dec!(0.0005));
        // Mark = 50000 * 1.0005 = 50025
        assert_eq!(new_state.mark_price.value(), dec!(50025.0000));
    }

    #[test]
    fn mark_price_no_mid() {
        let params = test_params();
        let initial_state = MarkPriceState::new(Price::new_unchecked(dec!(50000)));

        let new_index = Price::new_unchecked(dec!(51000));
        let new_mid = None;

        let new_state = update_mark_price(&initial_state, new_index, new_mid, &params);

        // Without mid, premium stays near zero (just EMA decay)
        // Mark should track index closely
        assert!(new_state.mark_price.value() > dec!(50990));
        assert!(new_state.mark_price.value() < dec!(51010));
    }

    #[test]
    fn extreme_premium_clamped() {
        let params = test_params();
        let initial_state = MarkPriceState::new(Price::new_unchecked(dec!(50000)));

        // Mid price 20% above index (extreme manipulation attempt)
        let new_index = Price::new_unchecked(dec!(50000));
        let new_mid = Some(Price::new_unchecked(dec!(60000)));

        let new_state = update_mark_price(&initial_state, new_index, new_mid, &params);

        // Premium should be clamped to 5%
        assert!(new_state.premium_index <= dec!(0.05));
        // Mark should not exceed index + 5%
        assert!(new_state.mark_price.value() <= dec!(52500));
    }
}
