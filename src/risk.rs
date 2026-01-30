//! Risk management and circuit breakers.
//!
//! Protects the exchange from extreme market conditions. Circuit breakers pause
//! trading when prices move too rapidly or when systemic risk thresholds are breached.
//! These safeguards help prevent cascading liquidations and bad debt accumulation.

use crate::types::{MarketId, Price, Quote, Timestamp};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};

/// Risk parameters that control circuit breakers and trading limits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskParams {
    /// Maximum price move allowed per time window (e.g., 0.15 for 15%).
    pub max_price_deviation: Decimal,
    /// Time window for price deviation check in milliseconds.
    pub price_window_ms: i64,
    /// Minimum time between price updates in milliseconds.
    pub min_price_update_interval_ms: i64,
    /// Maximum open interest allowed in quote currency.
    pub max_open_interest: Quote,
    /// Maximum single position size as fraction of open interest.
    pub max_position_ratio: Decimal,
    /// Insurance fund minimum before ADL triggers (as ratio of OI).
    pub adl_trigger_ratio: Decimal,
    /// Maximum funding rate per period (absolute value).
    pub max_funding_rate: Decimal,
    /// Cooldown period after circuit breaker in milliseconds.
    pub circuit_breaker_cooldown_ms: i64,
}

impl Default for RiskParams {
    fn default() -> Self {
        Self {
            max_price_deviation: dec!(0.15),
            price_window_ms: 60_000,
            min_price_update_interval_ms: 100,
            max_open_interest: Quote::new(dec!(100_000_000)),
            max_position_ratio: dec!(0.1),
            adl_trigger_ratio: dec!(0.01),
            max_funding_rate: dec!(0.01),
            circuit_breaker_cooldown_ms: 300_000,
        }
    }
}

/// Current state of risk monitoring for a market.
#[derive(Debug, Clone)]
pub struct RiskState {
    pub market_id: MarketId,
    /// Recent price history for deviation checks.
    pub price_history: Vec<(Timestamp, Price)>,
    /// Whether circuit breaker is active.
    pub circuit_breaker_active: bool,
    /// When circuit breaker was triggered.
    pub circuit_breaker_triggered_at: Option<Timestamp>,
    /// Reason for current circuit breaker.
    pub circuit_breaker_reason: Option<CircuitBreakerReason>,
    /// Cumulative bad debt this session.
    pub cumulative_bad_debt: Quote,
    /// Number of liquidations this session.
    pub liquidation_count: u64,
    /// High water mark for open interest.
    pub peak_open_interest: Quote,
}

impl RiskState {
    pub fn new(market_id: MarketId) -> Self {
        Self {
            market_id,
            price_history: Vec::with_capacity(1000),
            circuit_breaker_active: false,
            circuit_breaker_triggered_at: None,
            circuit_breaker_reason: None,
            cumulative_bad_debt: Quote::zero(),
            liquidation_count: 0,
            peak_open_interest: Quote::zero(),
        }
    }

    /// Record a price update and check for violations.
    pub fn record_price(
        &mut self,
        price: Price,
        timestamp: Timestamp,
        params: &RiskParams,
    ) -> Option<CircuitBreakerReason> {
        self.price_history.push((timestamp, price));
        self.prune_old_prices(timestamp, params.price_window_ms);

        if let Some(reason) = self.check_price_deviation(price, params) {
            return Some(reason);
        }

        None
    }

    /// Remove prices outside the time window.
    fn prune_old_prices(&mut self, current: Timestamp, window_ms: i64) {
        let cutoff = current.as_millis() - window_ms;
        self.price_history
            .retain(|(ts, _)| ts.as_millis() >= cutoff);
    }

    /// Check if current price deviates too much from recent history.
    fn check_price_deviation(
        &self,
        current_price: Price,
        params: &RiskParams,
    ) -> Option<CircuitBreakerReason> {
        if self.price_history.len() < 2 {
            return None;
        }

        let oldest = self.price_history.first().map(|(_, p)| p.value())?;
        let current = current_price.value();

        if oldest.is_zero() {
            return None;
        }

        let deviation = ((current - oldest) / oldest).abs();

        if deviation > params.max_price_deviation {
            return Some(CircuitBreakerReason::PriceDeviation {
                deviation,
                threshold: params.max_price_deviation,
            });
        }

        None
    }

    /// Record a liquidation event.
    pub fn record_liquidation(&mut self, bad_debt: Quote) {
        self.liquidation_count += 1;
        self.cumulative_bad_debt = self.cumulative_bad_debt.add(bad_debt);
    }

    /// Update peak open interest.
    pub fn update_peak_oi(&mut self, current_oi: Quote) {
        if current_oi.value() > self.peak_open_interest.value() {
            self.peak_open_interest = current_oi;
        }
    }

    /// Trigger circuit breaker.
    pub fn trigger_circuit_breaker(&mut self, reason: CircuitBreakerReason, timestamp: Timestamp) {
        self.circuit_breaker_active = true;
        self.circuit_breaker_triggered_at = Some(timestamp);
        self.circuit_breaker_reason = Some(reason);
    }

    /// Check if circuit breaker can be reset.
    pub fn can_reset_circuit_breaker(&self, current: Timestamp, cooldown_ms: i64) -> bool {
        if let Some(triggered_at) = self.circuit_breaker_triggered_at {
            current.as_millis() - triggered_at.as_millis() >= cooldown_ms
        } else {
            true
        }
    }

    /// Reset circuit breaker after cooldown.
    pub fn reset_circuit_breaker(&mut self) {
        self.circuit_breaker_active = false;
        self.circuit_breaker_triggered_at = None;
        self.circuit_breaker_reason = None;
    }
}

/// Reasons why a circuit breaker might be triggered.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CircuitBreakerReason {
    PriceDeviation {
        deviation: Decimal,
        threshold: Decimal,
    },
    ExcessiveOpenInterest {
        current: Quote,
        maximum: Quote,
    },
    InsuranceFundDepleted {
        balance: Quote,
        threshold: Quote,
    },
    OracleStale {
        last_update: Timestamp,
        max_staleness_ms: i64,
    },
    ManualHalt {
        reason: String,
    },
}

/// Result of a risk check operation.
#[derive(Debug, Clone)]
pub enum RiskCheckResult {
    /// Operation is allowed.
    Allowed,
    /// Operation is blocked due to risk limits.
    Blocked(RiskViolation),
    /// Operation triggers circuit breaker.
    CircuitBreaker(CircuitBreakerReason),
}

/// Details about why an operation was blocked.
#[derive(Debug, Clone)]
pub enum RiskViolation {
    CircuitBreakerActive,
    PositionTooLarge {
        requested: Quote,
        maximum: Quote,
    },
    OpenInterestExceeded {
        current: Quote,
        delta: Quote,
        maximum: Quote,
    },
    PriceStale {
        age_ms: i64,
        max_age_ms: i64,
    },
}

/// Check if a new position is within risk limits.
pub fn check_position_risk(
    position_value: Quote,
    current_oi: Quote,
    risk_state: &RiskState,
    params: &RiskParams,
) -> RiskCheckResult {
    if risk_state.circuit_breaker_active {
        return RiskCheckResult::Blocked(RiskViolation::CircuitBreakerActive);
    }

    let max_position = Quote::new(params.max_open_interest.value() * params.max_position_ratio);
    if position_value.value() > max_position.value() {
        return RiskCheckResult::Blocked(RiskViolation::PositionTooLarge {
            requested: position_value,
            maximum: max_position,
        });
    }

    let new_oi = Quote::new(current_oi.value() + position_value.value());
    if new_oi.value() > params.max_open_interest.value() {
        return RiskCheckResult::Blocked(RiskViolation::OpenInterestExceeded {
            current: current_oi,
            delta: position_value,
            maximum: params.max_open_interest,
        });
    }

    RiskCheckResult::Allowed
}

/// Check if insurance fund is healthy enough to continue trading.
pub fn check_insurance_health(
    insurance_balance: Quote,
    total_oi: Quote,
    params: &RiskParams,
) -> Option<CircuitBreakerReason> {
    if total_oi.value().is_zero() {
        return None;
    }

    let threshold = Quote::new(total_oi.value() * params.adl_trigger_ratio);

    if insurance_balance.value() < threshold.value() {
        return Some(CircuitBreakerReason::InsuranceFundDepleted {
            balance: insurance_balance,
            threshold,
        });
    }

    None
}

/// Check if oracle price is fresh enough.
pub fn check_price_freshness(
    last_update: Timestamp,
    current_time: Timestamp,
    max_staleness_ms: i64,
) -> Option<RiskViolation> {
    let age = current_time.as_millis() - last_update.as_millis();
    if age > max_staleness_ms {
        return Some(RiskViolation::PriceStale {
            age_ms: age,
            max_age_ms: max_staleness_ms,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn risk_state_creation() {
        let state = RiskState::new(MarketId(1));
        assert!(!state.circuit_breaker_active);
        assert_eq!(state.liquidation_count, 0);
    }

    #[test]
    fn price_deviation_triggers_circuit_breaker() {
        let mut state = RiskState::new(MarketId(1));
        let params = RiskParams::default();

        let base_price = Price::new_unchecked(dec!(50000));
        state.record_price(base_price, Timestamp::from_millis(0), &params);

        // 20% move should trigger (threshold is 15%)
        let spike_price = Price::new_unchecked(dec!(60000));
        let result = state.record_price(spike_price, Timestamp::from_millis(1000), &params);

        assert!(matches!(
            result,
            Some(CircuitBreakerReason::PriceDeviation { .. })
        ));
    }

    #[test]
    fn normal_price_movement_allowed() {
        let mut state = RiskState::new(MarketId(1));
        let params = RiskParams::default();

        let p1 = Price::new_unchecked(dec!(50000));
        state.record_price(p1, Timestamp::from_millis(0), &params);

        // 5% move should be fine
        let p2 = Price::new_unchecked(dec!(52500));
        let result = state.record_price(p2, Timestamp::from_millis(1000), &params);

        assert!(result.is_none());
    }

    #[test]
    fn circuit_breaker_cooldown() {
        let mut state = RiskState::new(MarketId(1));
        let params = RiskParams::default();

        state.trigger_circuit_breaker(
            CircuitBreakerReason::ManualHalt {
                reason: "test".to_string(),
            },
            Timestamp::from_millis(0),
        );

        assert!(state.circuit_breaker_active);

        // Should not reset before cooldown
        assert!(!state.can_reset_circuit_breaker(
            Timestamp::from_millis(100_000),
            params.circuit_breaker_cooldown_ms
        ));

        // Should reset after cooldown
        assert!(state.can_reset_circuit_breaker(
            Timestamp::from_millis(400_000),
            params.circuit_breaker_cooldown_ms
        ));
    }

    #[test]
    fn position_size_limit() {
        let state = RiskState::new(MarketId(1));
        let params = RiskParams::default();

        let current_oi = Quote::new(dec!(10_000_000));

        // Small position should pass (max is 10% of 100M = 10M)
        let small = Quote::new(dec!(1_000_000));
        let result = check_position_risk(small, current_oi, &state, &params);
        assert!(matches!(result, RiskCheckResult::Allowed));

        // Large position should fail
        let large = Quote::new(dec!(15_000_000));
        let result = check_position_risk(large, current_oi, &state, &params);
        assert!(matches!(
            result,
            RiskCheckResult::Blocked(RiskViolation::PositionTooLarge { .. })
        ));
    }

    #[test]
    fn open_interest_limit() {
        let state = RiskState::new(MarketId(1));
        let params = RiskParams::default();

        // At 99M OI, trying to add 2M should fail (max is 100M)
        let current_oi = Quote::new(dec!(99_000_000));
        let new_position = Quote::new(dec!(2_000_000));

        let result = check_position_risk(new_position, current_oi, &state, &params);
        assert!(matches!(
            result,
            RiskCheckResult::Blocked(RiskViolation::OpenInterestExceeded { .. })
        ));
    }

    #[test]
    fn insurance_fund_health_check() {
        let params = RiskParams::default();

        // Healthy fund (1% of 10M = 100k threshold)
        let healthy = Quote::new(dec!(500_000));
        let oi = Quote::new(dec!(10_000_000));
        assert!(check_insurance_health(healthy, oi, &params).is_none());

        // Depleted fund
        let depleted = Quote::new(dec!(50_000));
        let result = check_insurance_health(depleted, oi, &params);
        assert!(matches!(
            result,
            Some(CircuitBreakerReason::InsuranceFundDepleted { .. })
        ));
    }

    #[test]
    fn price_staleness_check() {
        let last = Timestamp::from_millis(0);
        let current = Timestamp::from_millis(5000);
        let max_staleness = 3000i64;

        let result = check_price_freshness(last, current, max_staleness);
        assert!(matches!(
            result,
            Some(RiskViolation::PriceStale { age_ms: 5000, .. })
        ));
    }
}
