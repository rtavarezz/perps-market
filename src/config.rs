// 7.0 config.rs: all settings in one place. fees, margins, risk params.
// 7.1 FeeConfig has maker/taker fees. no builder fees or referrals yet.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::custody::CollateralType;
use crate::margin::MarginParams;
use crate::funding::FundingParams;
use crate::risk::RiskParams;

// Complete configuration for a perps market
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketConfig {
    // Human readable market name
    pub name: String,
    // Market symbol (e.g. "BTC-PERP")
    pub symbol: String,
    // Base asset (what you're trading)
    pub base_asset: String,
    // Quote asset (what you're settling in)
    pub quote_asset: String,
    // Minimum order size
    pub min_order_size: Decimal,
    // Maximum order size
    pub max_order_size: Decimal,
    // Price tick size (minimum price increment)
    pub tick_size: Decimal,
    // Size step (minimum size increment)
    pub lot_size: Decimal,
    // Whether the market is currently active
    pub active: bool,
}

impl Default for MarketConfig {
    fn default() -> Self {
        Self {
            name: "Bitcoin Perpetual".to_string(),
            symbol: "BTC-PERP".to_string(),
            base_asset: "BTC".to_string(),
            quote_asset: "USD".to_string(),
            min_order_size: Decimal::new(1, 4), // 0.0001 BTC
            max_order_size: Decimal::new(1000, 0), // 1000 BTC
            tick_size: Decimal::new(1, 1), // $0.10
            lot_size: Decimal::new(1, 4), // 0.0001 BTC
            active: true,
        }
    }
}

/** 7.2: fee settings. maker/taker in bps. 100 bps = 1% */
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeeConfig {
    // Maker fee in basis points (negative = rebate)
    pub maker_fee_bps: i32,
    // Taker fee in basis points
    pub taker_fee_bps: u32,
    // Liquidation fee in basis points
    pub liquidation_fee_bps: u32,
    // Percentage of trading fees routed to the referrer (e.g. 0.10 = 10%)
    pub referral_fee_pct: Decimal,
    // Fee discount tiers based on volume
    pub volume_discounts: Vec<VolumeDiscount>,
}

impl Default for FeeConfig {
    fn default() -> Self {
        Self {
            maker_fee_bps: 2,  // 0.02%
            taker_fee_bps: 5,  // 0.05%
            liquidation_fee_bps: 50, // 0.5%
            referral_fee_pct: Decimal::new(10, 2), // 10%
            volume_discounts: vec![
                VolumeDiscount { min_volume: Decimal::new(1_000_000, 0), discount_pct: 10 },
                VolumeDiscount { min_volume: Decimal::new(10_000_000, 0), discount_pct: 20 },
                VolumeDiscount { min_volume: Decimal::new(100_000_000, 0), discount_pct: 30 },
            ],
        }
    }
}

// Volume based fee discount tier
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeDiscount {
    // Minimum 30 day volume to qualify
    pub min_volume: Decimal,
    // Discount percentage on fees
    pub discount_pct: u32,
}

// Price feed configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceFeedConfig {
    // Minimum number of sources for valid price
    pub min_sources: usize,
    // Maximum staleness in seconds
    pub max_staleness_secs: u64,
    // Maximum deviation between sources
    pub max_deviation_pct: Decimal,
    // Use median (true) or weighted average
    pub use_median: bool,
    // TWAP window in seconds
    pub twap_window_secs: u64,
}

impl Default for PriceFeedConfig {
    fn default() -> Self {
        Self {
            min_sources: 1,
            max_staleness_secs: 60,
            max_deviation_pct: Decimal::new(2, 0), // 2%
            use_median: true,
            twap_window_secs: 300, // 5 minutes
        }
    }
}

// Liquidity pool configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiquidityConfig {
    // Maximum utilization ratio
    pub max_utilization: Decimal,
    // Maximum price impact allowed
    pub max_price_impact: Decimal,
    // Pool fee in basis points
    pub pool_fee_bps: u32,
}

impl Default for LiquidityConfig {
    fn default() -> Self {
        Self {
            max_utilization: Decimal::new(80, 2), // 80%
            max_price_impact: Decimal::new(1, 2), // 1%
            pool_fee_bps: 10, // 0.1%
        }
    }
}

// The complete integration configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrationConfig {
    pub market: MarketConfig,
    pub margin: MarginParams,
    pub funding: FundingParams,
    pub risk: RiskParams,
    pub fees: FeeConfig,
    pub price_feed: PriceFeedConfig,
    pub liquidity: LiquidityConfig,
    // Accepted collateral types and their weights
    pub collateral_weights: HashMap<CollateralType, Decimal>,
    // Insurance fund target (as ratio of OI)
    pub insurance_target_ratio: Decimal,
}

impl Default for IntegrationConfig {
    fn default() -> Self {
        let mut collateral_weights = HashMap::new();
        collateral_weights.insert(CollateralType::Usd, Decimal::ONE);
        collateral_weights.insert(CollateralType::Usdc, Decimal::new(999, 3)); // 0.999
        collateral_weights.insert(CollateralType::Usdt, Decimal::new(995, 3)); // 0.995

        Self {
            market: MarketConfig::default(),
            margin: MarginParams::default(),
            funding: FundingParams::default(),
            risk: RiskParams::default(),
            fees: FeeConfig::default(),
            price_feed: PriceFeedConfig::default(),
            liquidity: LiquidityConfig::default(),
            collateral_weights,
            insurance_target_ratio: Decimal::new(1, 2), // 1% of OI
        }
    }
}

impl IntegrationConfig {
    // Create a configuration preset for testnet
    pub fn testnet() -> Self {
        let mut config = Self::default();
        config.market.name = "BTC-PERP Testnet".to_string();
        config.margin.max_leverage = crate::types::Leverage::new(rust_decimal_macros::dec!(20)).unwrap(); // 20x max
        config.margin.maintenance_margin_ratio = Decimal::new(25, 3); // 2.5%
        config.fees.maker_fee_bps = 0; // free makers on testnet
        config.fees.taker_fee_bps = 1; // minimal taker fee
        config
    }

    // Create a configuration preset for mainnet with conservative settings
    pub fn mainnet_conservative() -> Self {
        let mut config = Self::default();
        config.margin.max_leverage = crate::types::Leverage::new(rust_decimal_macros::dec!(10)).unwrap(); // 10x max
        config.margin.maintenance_margin_ratio = Decimal::new(5, 2); // 5%
        config.risk.max_price_deviation = Decimal::new(5, 2); // 5%
        config.risk.circuit_breaker_cooldown_ms = 300_000; // 5 minute cooldown
        config.price_feed.min_sources = 3;
        config
    }

    // Create a configuration for high frequency trading
    pub fn hft_optimized() -> Self {
        let mut config = Self::default();
        config.fees.maker_fee_bps = -1; // maker rebate
        config.fees.taker_fee_bps = 3;
        config.market.tick_size = Decimal::new(1, 2); // $0.01 ticks
        config.price_feed.max_staleness_secs = 5; // very fresh prices
        config
    }

    // Validate the configuration for internal consistency
    pub fn validate(&self) -> Result<(), ConfigError> {
        // margin checks
        // maintenance_margin_ratio is multiplied by initial margin (1/leverage)
        // so it must be in range (0, 1) meaning MM < IM
        if self.margin.maintenance_margin_ratio <= Decimal::ZERO 
            || self.margin.maintenance_margin_ratio >= Decimal::ONE {
            return Err(ConfigError::InvalidMargin {
                reason: "MM ratio must be between 0 and 1".to_string(),
            });
        }

        // market checks
        if self.market.min_order_size >= self.market.max_order_size {
            return Err(ConfigError::InvalidMarket {
                reason: "Min order must be less than max".to_string(),
            });
        }

        if self.market.tick_size <= Decimal::ZERO {
            return Err(ConfigError::InvalidMarket {
                reason: "Tick size must be positive".to_string(),
            });
        }

        // fee checks
        if self.fees.taker_fee_bps > 100 {
            return Err(ConfigError::InvalidFees {
                reason: "Taker fee too high (>1%)".to_string(),
            });
        }

        // price feed checks
        if self.price_feed.min_sources == 0 {
            return Err(ConfigError::InvalidPriceFeed {
                reason: "Need at least 1 price source".to_string(),
            });
        }

        Ok(())
    }

    // Calculate max leverage based on margin params
    pub fn max_leverage(&self) -> Decimal {
        self.margin.max_leverage.value()
    }

    // Get collateral weight for a given type
    pub fn collateral_weight(&self, collateral_type: CollateralType) -> Decimal {
        self.collateral_weights
            .get(&collateral_type)
            .copied()
            .unwrap_or(Decimal::ZERO)
    }
}

// Configuration validation errors
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    InvalidMargin { reason: String },
    InvalidMarket { reason: String },
    InvalidFees { reason: String },
    InvalidPriceFeed { reason: String },
    InvalidRisk { reason: String },
}

// Environment presets
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Environment {
    Development,
    Testnet,
    Mainnet,
}

impl Environment {
    pub fn config(&self) -> IntegrationConfig {
        match self {
            Environment::Development => IntegrationConfig::default(),
            Environment::Testnet => IntegrationConfig::testnet(),
            Environment::Mainnet => IntegrationConfig::mainnet_conservative(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_valid() {
        let config = IntegrationConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_testnet_config_valid() {
        let config = IntegrationConfig::testnet();
        assert!(config.validate().is_ok());
        assert_eq!(config.fees.maker_fee_bps, 0);
    }

    #[test]
    fn test_mainnet_config_valid() {
        let config = IntegrationConfig::mainnet_conservative();
        assert!(config.validate().is_ok());
        assert_eq!(config.price_feed.min_sources, 3);
    }

    #[test]
    fn test_max_leverage() {
        let config = IntegrationConfig::default();
        // default max leverage is 50x
        assert_eq!(config.max_leverage(), Decimal::new(50, 0));

        let testnet = IntegrationConfig::testnet();
        // testnet max leverage is 20x
        assert_eq!(testnet.max_leverage(), Decimal::new(20, 0));
    }

    #[test]
    fn test_invalid_margin() {
        let mut config = IntegrationConfig::default();
        // set MM ratio >= 1 which is invalid (would make MM >= IM)
        config.margin.maintenance_margin_ratio = Decimal::new(110, 2); // 1.1

        let result = config.validate();
        assert!(matches!(result, Err(ConfigError::InvalidMargin { .. })));
    }

    #[test]
    fn test_invalid_market() {
        let mut config = IntegrationConfig::default();
        config.market.min_order_size = Decimal::new(100, 0);
        config.market.max_order_size = Decimal::new(10, 0);

        let result = config.validate();
        assert!(matches!(result, Err(ConfigError::InvalidMarket { .. })));
    }

    #[test]
    fn test_collateral_weights() {
        let config = IntegrationConfig::default();
        assert_eq!(config.collateral_weight(CollateralType::Usd), Decimal::ONE);
        assert!(config.collateral_weight(CollateralType::Usdc) < Decimal::ONE);
        assert_eq!(config.collateral_weight(CollateralType::Btc), Decimal::ZERO); // not configured
    }

    #[test]
    fn test_environment_presets() {
        assert!(Environment::Development.config().validate().is_ok());
        assert!(Environment::Testnet.config().validate().is_ok());
        assert!(Environment::Mainnet.config().validate().is_ok());
    }

    #[test]
    fn test_config_serialization() {
        let config = IntegrationConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        let back: IntegrationConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.market.symbol, config.market.symbol);
    }
}
