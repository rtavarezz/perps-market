//! Margin calculation for initial and maintenance requirements.
//!
//! Initial margin (IM) is required to open a position, calculated as
//! notional value divided by leverage. Maintenance margin (MM) is the
//! minimum to keep a position open, typically 50% of IM.
//!
//! Leverage tiers reduce max leverage as position size grows to limit
//! protocol risk from large positions.

use crate::types::{Leverage, Price, Quote, SignedSize};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarginParams {
    pub max_leverage: Leverage,
    pub maintenance_margin_ratio: Decimal,
    pub leverage_tiers: Vec<LeverageTier>,
}

impl Default for MarginParams {
    fn default() -> Self {
        Self {
            max_leverage: Leverage::new(dec!(50)).unwrap(),
            maintenance_margin_ratio: dec!(0.5),
            leverage_tiers: vec![
                LeverageTier {
                    max_notional: Quote::new(dec!(100_000)),
                    max_leverage: Leverage::new(dec!(50)).unwrap(),
                },
                LeverageTier {
                    max_notional: Quote::new(dec!(500_000)),
                    max_leverage: Leverage::new(dec!(20)).unwrap(),
                },
                LeverageTier {
                    max_notional: Quote::new(dec!(2_000_000)),
                    max_leverage: Leverage::new(dec!(10)).unwrap(),
                },
                LeverageTier {
                    max_notional: Quote::new(dec!(10_000_000)),
                    max_leverage: Leverage::new(dec!(5)).unwrap(),
                },
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeverageTier {
    pub max_notional: Quote,
    pub max_leverage: Leverage,
}

#[derive(Debug, Clone)]
pub struct MarginRequirement {
    pub initial: Quote,
    pub maintenance: Quote,
    pub effective_leverage: Leverage,
}

pub fn notional_value(size: SignedSize, price: Price) -> Quote {
    Quote::new(size.abs() * price.value())
}

pub fn effective_max_leverage(notional: Quote, params: &MarginParams) -> Leverage {
    for tier in &params.leverage_tiers {
        if notional.value() <= tier.max_notional.value() {
            return tier.max_leverage;
        }
    }
    params
        .leverage_tiers
        .last()
        .map(|t| t.max_leverage)
        .unwrap_or(params.max_leverage)
}

pub fn calculate_margin_requirement(
    size: SignedSize,
    mark_price: Price,
    requested_leverage: Leverage,
    params: &MarginParams,
) -> MarginRequirement {
    let notional = notional_value(size, mark_price);
    let max_lev = effective_max_leverage(notional, params);
    let effective_leverage = if requested_leverage.value() > max_lev.value() {
        max_lev
    } else {
        requested_leverage
    };

    let im_fraction = effective_leverage.initial_margin_fraction();
    let initial = Quote::new(notional.value() * im_fraction);
    let maintenance = Quote::new(initial.value() * params.maintenance_margin_ratio);

    MarginRequirement {
        initial,
        maintenance,
        effective_leverage,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarginStatus {
    Healthy,
    Warning,
    Liquidatable,
}

pub fn evaluate_margin_status(
    account_equity: Quote,
    margin_req: &MarginRequirement,
) -> MarginStatus {
    if account_equity.value() >= margin_req.initial.value() {
        MarginStatus::Healthy
    } else if account_equity.value() >= margin_req.maintenance.value() {
        MarginStatus::Warning
    } else {
        MarginStatus::Liquidatable
    }
}

pub fn margin_ratio(equity: Quote, notional: Quote) -> Decimal {
    if notional.value().is_zero() {
        return Decimal::MAX;
    }
    equity.value() / notional.value()
}

/// Calculate free margin (equity available for new positions)
pub fn free_margin(account_equity: Quote, margin_used: Quote) -> Quote {
    Quote::new(account_equity.value() - margin_used.value())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn test_params() -> MarginParams {
        MarginParams::default()
    }

    #[test]
    fn notional_calculation() {
        let size = SignedSize::new(dec!(1)); // 1 BTC long
        let price = Price::new_unchecked(dec!(50000)); // $50k
        let notional = notional_value(size, price);
        assert_eq!(notional.value(), dec!(50000));
    }

    #[test]
    fn margin_at_10x_leverage() {
        let size = SignedSize::new(dec!(1));
        let price = Price::new_unchecked(dec!(50000));
        let leverage = Leverage::new(dec!(10)).unwrap();

        let req = calculate_margin_requirement(size, price, leverage, &test_params());

        // 50k notional / 10x = 5k initial margin
        assert_eq!(req.initial.value(), dec!(5000));
        // 5k * 0.5 = 2.5k maintenance
        assert_eq!(req.maintenance.value(), dec!(2500));
        assert_eq!(req.effective_leverage.value(), dec!(10));
    }

    #[test]
    fn leverage_capped_by_tier() {
        // Large position should have reduced leverage
        let size = SignedSize::new(dec!(100)); // 100 BTC
        let price = Price::new_unchecked(dec!(50000)); // $5M notional
        let requested = Leverage::new(dec!(50)).unwrap();

        let req = calculate_margin_requirement(size, price, requested, &test_params());

        // $5M falls in tier 4 (2M-10M), max 5x
        assert_eq!(req.effective_leverage.value(), dec!(5));
        // IM = 5M / 5 = 1M
        assert_eq!(req.initial.value(), dec!(1_000_000));
    }

    #[test]
    fn margin_status_healthy() {
        let size = SignedSize::new(dec!(1));
        let price = Price::new_unchecked(dec!(50000));
        let leverage = Leverage::new(dec!(10)).unwrap();

        let req = calculate_margin_requirement(size, price, leverage, &test_params());
        let equity = Quote::new(dec!(10000)); // 10k, above 5k IM

        assert_eq!(evaluate_margin_status(equity, &req), MarginStatus::Healthy);
    }

    #[test]
    fn margin_status_warning() {
        let size = SignedSize::new(dec!(1));
        let price = Price::new_unchecked(dec!(50000));
        let leverage = Leverage::new(dec!(10)).unwrap();

        let req = calculate_margin_requirement(size, price, leverage, &test_params());
        let equity = Quote::new(dec!(4000)); // 4k, between 2.5k MM and 5k IM

        assert_eq!(evaluate_margin_status(equity, &req), MarginStatus::Warning);
    }

    #[test]
    fn margin_status_liquidatable() {
        let size = SignedSize::new(dec!(1));
        let price = Price::new_unchecked(dec!(50000));
        let leverage = Leverage::new(dec!(10)).unwrap();

        let req = calculate_margin_requirement(size, price, leverage, &test_params());
        let equity = Quote::new(dec!(2000)); // 2k, below 2.5k MM

        assert_eq!(
            evaluate_margin_status(equity, &req),
            MarginStatus::Liquidatable
        );
    }

    #[test]
    fn margin_ratio_calculation() {
        let equity = Quote::new(dec!(5000));
        let notional = Quote::new(dec!(50000));

        let ratio = margin_ratio(equity, notional);
        assert_eq!(ratio, dec!(0.1)); // 10% margin ratio = 10x leverage
    }

    #[test]
    fn free_margin_calculation() {
        let equity = Quote::new(dec!(10000));
        let margin_used = Quote::new(dec!(5000));

        let free = free_margin(equity, margin_used);
        assert_eq!(free.value(), dec!(5000));
    }
}
