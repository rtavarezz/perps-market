//! Market configuration and state.
//!
//! A market represents a single trading pair with its own order book,
//! funding state, and risk parameters.

use crate::funding::{FundingParams, FundingState};
use crate::liquidation::LiquidationParams;
use crate::margin::MarginParams;
use crate::mark_price::MarkPriceParams;
use crate::order::OrderBook;
use crate::types::{MarketId, Price, Timestamp};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Market status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarketStatus {
    /// Market is open for trading
    Active,
    /// Trading paused (e.g., during settlement or emergency)
    Paused,
    /// Market is closed permanently
    Closed,
}

impl Default for MarketStatus {
    fn default() -> Self {
        Self::Active
    }
}

/// Static market configuration (immutable after creation)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketConfig {
    pub id: MarketId,
    /// Human-readable name (e.g., "BTC-PERP")
    pub name: String,
    /// Base asset symbol (e.g., "BTC")
    pub base_asset: String,
    /// Quote asset symbol (e.g., "USD")
    pub quote_asset: String,
    /// Minimum order size
    pub min_order_size: Decimal,
    /// Tick size (minimum price increment)
    pub tick_size: Decimal,
    /// Lot size (minimum size increment)
    pub lot_size: Decimal,
    /// Margin parameters
    pub margin_params: MarginParams,
    /// Mark price parameters
    pub mark_price_params: MarkPriceParams,
    /// Funding parameters
    pub funding_params: FundingParams,
    /// Liquidation parameters
    pub liquidation_params: LiquidationParams,
}

impl MarketConfig {
    /// Create a default BTC-PERP market configuration
    pub fn btc_perp() -> Self {
        Self {
            id: MarketId(1),
            name: "BTC-PERP".to_string(),
            base_asset: "BTC".to_string(),
            quote_asset: "USD".to_string(),
            min_order_size: Decimal::new(1, 4), // 0.0001 BTC
            tick_size: Decimal::new(1, 1),      // $0.1
            lot_size: Decimal::new(1, 4),       // 0.0001 BTC
            margin_params: MarginParams::default(),
            mark_price_params: MarkPriceParams::default(),
            funding_params: FundingParams::default(),
            liquidation_params: LiquidationParams::default(),
        }
    }

    /// Validate an order size
    pub fn validate_size(&self, size: Decimal) -> Result<(), MarketError> {
        if size < self.min_order_size {
            return Err(MarketError::OrderTooSmall {
                size,
                minimum: self.min_order_size,
            });
        }
        // Check lot size alignment
        let remainder = size % self.lot_size;
        if !remainder.is_zero() {
            return Err(MarketError::InvalidLotSize {
                size,
                lot_size: self.lot_size,
            });
        }
        Ok(())
    }

    /// Validate and round a price to tick size
    pub fn validate_price(&self, price: Price) -> Result<Price, MarketError> {
        let value = price.value();
        if value <= Decimal::ZERO {
            return Err(MarketError::InvalidPrice(price));
        }
        // Round to nearest tick
        let ticks = (value / self.tick_size).round();
        let rounded = ticks * self.tick_size;
        Ok(Price::new_unchecked(rounded))
    }
}

/// Dynamic market state (changes during trading)
#[derive(Debug, Clone)]
pub struct MarketState {
    pub config: MarketConfig,
    pub status: MarketStatus,
    pub order_book: OrderBook,
    pub funding_state: FundingState,
    /// Current index price from oracle
    pub index_price: Option<Price>,
    /// Current mark price
    pub mark_price: Option<Price>,
    /// Smoothed premium for mark price calculation
    pub smoothed_premium: Decimal,
    /// Total open interest (sum of all position sizes)
    pub open_interest_long: Decimal,
    pub open_interest_short: Decimal,
    /// Last trade price
    pub last_trade_price: Option<Price>,
    /// 24h volume
    pub volume_24h: Decimal,
    /// Last update timestamp
    pub last_updated: Timestamp,
}

impl MarketState {
    pub fn new(config: MarketConfig, timestamp: Timestamp) -> Self {
        let order_book = OrderBook::new(config.id);
        let funding_state = FundingState::new(timestamp);

        Self {
            config,
            status: MarketStatus::Active,
            order_book,
            funding_state,
            index_price: None,
            mark_price: None,
            smoothed_premium: Decimal::ZERO,
            open_interest_long: Decimal::ZERO,
            open_interest_short: Decimal::ZERO,
            last_trade_price: None,
            volume_24h: Decimal::ZERO,
            last_updated: timestamp,
        }
    }

    pub fn is_active(&self) -> bool {
        self.status == MarketStatus::Active
    }

    /// Get effective mark price (falls back to index if no mark)
    pub fn effective_mark_price(&self) -> Option<Price> {
        self.mark_price.or(self.index_price)
    }

    /// Update open interest when positions change
    pub fn update_open_interest(&mut self, long_delta: Decimal, short_delta: Decimal) {
        self.open_interest_long += long_delta;
        self.open_interest_short += short_delta;

        // Sanity checks
        if self.open_interest_long < Decimal::ZERO {
            self.open_interest_long = Decimal::ZERO;
        }
        if self.open_interest_short < Decimal::ZERO {
            self.open_interest_short = Decimal::ZERO;
        }
    }

    /// Record a trade
    pub fn record_trade(&mut self, price: Price, size: Decimal) {
        self.last_trade_price = Some(price);
        self.volume_24h += size * price.value();
    }

    /// Get net open interest (longs - shorts, should balance to zero in practice)
    pub fn net_open_interest(&self) -> Decimal {
        self.open_interest_long - self.open_interest_short
    }

    /// Get total open interest
    pub fn total_open_interest(&self) -> Decimal {
        // In a futures market, long OI should equal short OI
        // We report the maximum to catch any imbalance
        self.open_interest_long.max(self.open_interest_short)
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum MarketError {
    #[error("Order size {size} below minimum {minimum}")]
    OrderTooSmall { size: Decimal, minimum: Decimal },

    #[error("Size {size} not aligned to lot size {lot_size}")]
    InvalidLotSize { size: Decimal, lot_size: Decimal },

    #[error("Invalid price: {0}")]
    InvalidPrice(Price),

    #[error("Market {0:?} is not active")]
    MarketNotActive(MarketId),

    #[error("Market {0:?} not found")]
    MarketNotFound(MarketId),

    #[error("No liquidity available")]
    NoLiquidity,

    #[error("Oracle price not available")]
    NoOraclePrice,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn btc_perp_defaults() {
        let config = MarketConfig::btc_perp();
        assert_eq!(config.name, "BTC-PERP");
        assert_eq!(config.base_asset, "BTC");
        assert_eq!(config.quote_asset, "USD");
    }

    #[test]
    fn validate_size_ok() {
        let config = MarketConfig::btc_perp();
        assert!(config.validate_size(dec!(0.001)).is_ok());
        assert!(config.validate_size(dec!(1.0)).is_ok());
    }

    #[test]
    fn validate_size_too_small() {
        let config = MarketConfig::btc_perp();
        let result = config.validate_size(dec!(0.00001));
        assert!(matches!(result, Err(MarketError::OrderTooSmall { .. })));
    }

    #[test]
    fn validate_price_rounds_to_tick() {
        let config = MarketConfig::btc_perp();
        let price = Price::new_unchecked(dec!(50000.123));
        let rounded = config.validate_price(price).unwrap();
        assert_eq!(rounded.value(), dec!(50000.1));
    }

    #[test]
    fn market_state_initialization() {
        let config = MarketConfig::btc_perp();
        let state = MarketState::new(config, Timestamp::from_millis(0));

        assert!(state.is_active());
        assert!(state.index_price.is_none());
        assert!(state.mark_price.is_none());
        assert_eq!(state.open_interest_long, Decimal::ZERO);
        assert_eq!(state.open_interest_short, Decimal::ZERO);
    }

    #[test]
    fn open_interest_tracking() {
        let config = MarketConfig::btc_perp();
        let mut state = MarketState::new(config, Timestamp::from_millis(0));

        state.update_open_interest(dec!(1.0), dec!(0));
        state.update_open_interest(dec!(0), dec!(1.0));

        assert_eq!(state.open_interest_long, dec!(1.0));
        assert_eq!(state.open_interest_short, dec!(1.0));
        assert_eq!(state.total_open_interest(), dec!(1.0));
    }

    #[test]
    fn trade_recording() {
        let config = MarketConfig::btc_perp();
        let mut state = MarketState::new(config, Timestamp::from_millis(0));

        let price = Price::new_unchecked(dec!(50000));
        state.record_trade(price, dec!(1.0));

        assert_eq!(state.last_trade_price, Some(price));
        assert_eq!(state.volume_24h, dec!(50000));
    }
}
