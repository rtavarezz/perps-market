// 9.3 liquidity.rs: pool code exists but not connected. we use order book not AMM.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::types::{AccountId, Side};

// Unique identifier for a liquidity pool
pub type PoolId = u32;

// Configuration for a liquidity pool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolConfig {
    pub pool_id: PoolId,
    pub name: String,
    // Total value locked in the pool
    pub tvl: Decimal,
    // Maximum utilization ratio (e.g. 0.8 = 80% can be used for positions)
    pub max_utilization: Decimal,
    // Fee tier in basis points
    pub fee_bps: u32,
    // Whether this pool is currently active
    pub active: bool,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            pool_id: 0,
            name: "Default Pool".to_string(),
            tvl: Decimal::ZERO,
            max_utilization: Decimal::new(8, 1), // 80%
            fee_bps: 10, // 0.1%
            active: true,
        }
    }
}

// Represents a quote from a liquidity source
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiquidityQuote {
    pub source_id: PoolId,
    pub side: Side,
    pub size: Decimal,
    pub price: Decimal,
    // Fee to execute this quote
    pub fee: Decimal,
    // Price impact of this trade
    pub price_impact: Decimal,
    // How long this quote is valid (in seconds)
    pub valid_for_seconds: u64,
    pub timestamp: u64,
}

impl LiquidityQuote {
    pub fn total_cost(&self) -> Decimal {
        self.size * self.price + self.fee
    }

    pub fn is_expired(&self, current_time: u64) -> bool {
        current_time > self.timestamp + self.valid_for_seconds
    }
}

// Errors from liquidity operations
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiquidityError {
    InsufficientLiquidity { requested: Decimal, available: Decimal },
    PoolNotFound { pool_id: PoolId },
    PoolInactive { pool_id: PoolId },
    QuoteExpired,
    ExcessivePriceImpact { impact: Decimal, max_allowed: Decimal },
    UtilizationExceeded { current: Decimal, max: Decimal },
}

// Trait for liquidity providers. Different implementations can model:
// - GLP style shared pool (like GMX)
// - HLP style vault (like Hyperliquid)
// - External market maker connections
// - On chain AMM integration
pub trait LiquidityProvider {
    fn pool_id(&self) -> PoolId;
    fn name(&self) -> &str;

    // Get a quote for a given trade
    fn get_quote(&self, side: Side, size: Decimal, current_price: Decimal) -> Result<LiquidityQuote, LiquidityError>;

    // Execute against a quote (would mutate state in real implementation)
    fn execute_quote(&mut self, quote: &LiquidityQuote) -> Result<(), LiquidityError>;

    // Current available liquidity on each side
    fn available_liquidity(&self) -> (Decimal, Decimal); // (long, short)

    // Current utilization ratio
    fn utilization(&self) -> Decimal;
}

// A simple shared liquidity pool similar to GMX's GLP model.
// LPs deposit collateral, traders trade against the pool.
#[derive(Debug, Clone)]
pub struct SharedPool {
    config: PoolConfig,
    // Total long open interest against this pool
    long_oi: Decimal,
    // Total short open interest against this pool
    short_oi: Decimal,
    // Accumulated fees
    accumulated_fees: Decimal,
}

impl SharedPool {
    pub fn new(config: PoolConfig) -> Self {
        Self {
            config,
            long_oi: Decimal::ZERO,
            short_oi: Decimal::ZERO,
            accumulated_fees: Decimal::ZERO,
        }
    }

    pub fn deposit(&mut self, amount: Decimal) {
        self.config.tvl += amount;
    }

    pub fn withdraw(&mut self, amount: Decimal) -> Result<(), LiquidityError> {
        let available = self.config.tvl - self.reserved_liquidity();
        if amount > available {
            return Err(LiquidityError::InsufficientLiquidity {
                requested: amount,
                available,
            });
        }
        self.config.tvl -= amount;
        Ok(())
    }

    fn reserved_liquidity(&self) -> Decimal {
        // liquidity reserved for existing positions
        // simplified: max of long/short OI represents net exposure
        self.long_oi.max(self.short_oi)
    }

    fn calculate_price_impact(&self, _side: Side, size: Decimal) -> Decimal {
        // simple linear impact model based on pool utilization
        // real implementation would use curves like x*y=k or bonding curves
        let total_oi = self.long_oi + self.short_oi + size;
        let utilization = total_oi / self.config.tvl;

        // impact scales quadratically with utilization
        let impact_factor = Decimal::new(1, 4); // 0.01% base
        impact_factor * utilization * utilization * Decimal::from(100)
    }

    pub fn tvl(&self) -> Decimal {
        self.config.tvl
    }

    pub fn fees_collected(&self) -> Decimal {
        self.accumulated_fees
    }
}

impl LiquidityProvider for SharedPool {
    fn pool_id(&self) -> PoolId {
        self.config.pool_id
    }

    fn name(&self) -> &str {
        &self.config.name
    }

    fn get_quote(&self, side: Side, size: Decimal, current_price: Decimal) -> Result<LiquidityQuote, LiquidityError> {
        if !self.config.active {
            return Err(LiquidityError::PoolInactive { pool_id: self.config.pool_id });
        }

        // check available liquidity
        let max_available = self.config.tvl * self.config.max_utilization - self.reserved_liquidity();
        if size * current_price > max_available {
            return Err(LiquidityError::InsufficientLiquidity {
                requested: size * current_price,
                available: max_available,
            });
        }

        let price_impact = self.calculate_price_impact(side, size);
        let fee = size * current_price * Decimal::from(self.config.fee_bps) / Decimal::from(10000);

        // adjust price based on impact
        let execution_price = match side {
            Side::Long => current_price * (Decimal::ONE + price_impact),
            Side::Short => current_price * (Decimal::ONE - price_impact),
        };

        Ok(LiquidityQuote {
            source_id: self.config.pool_id,
            side,
            size,
            price: execution_price,
            fee,
            price_impact,
            valid_for_seconds: 30,
            timestamp: 0, // caller should set
        })
    }

    fn execute_quote(&mut self, quote: &LiquidityQuote) -> Result<(), LiquidityError> {
        match quote.side {
            Side::Long => self.long_oi += quote.size,
            Side::Short => self.short_oi += quote.size,
        }
        self.accumulated_fees += quote.fee;
        Ok(())
    }

    fn available_liquidity(&self) -> (Decimal, Decimal) {
        let max = self.config.tvl * self.config.max_utilization;
        let reserved = self.reserved_liquidity();
        let available = max - reserved;
        (available, available) // symmetric for shared pool
    }

    fn utilization(&self) -> Decimal {
        if self.config.tvl == Decimal::ZERO {
            return Decimal::ZERO;
        }
        self.reserved_liquidity() / self.config.tvl
    }
}

// Aggregates multiple liquidity sources and routes orders optimally
#[derive(Debug)]
pub struct LiquidityRouter {
    pools: Vec<Box<dyn LiquidityProvider + Send + Sync>>,
    // Maximum allowed price impact
    max_price_impact: Decimal,
}

// need to implement Debug manually since trait objects don't auto derive
impl std::fmt::Debug for Box<dyn LiquidityProvider + Send + Sync> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LiquidityProvider({})", self.name())
    }
}

impl LiquidityRouter {
    pub fn new(max_price_impact: Decimal) -> Self {
        Self {
            pools: Vec::new(),
            max_price_impact,
        }
    }

    pub fn add_pool(&mut self, pool: Box<dyn LiquidityProvider + Send + Sync>) {
        self.pools.push(pool);
    }

    // Get the best quote across all pools
    pub fn get_best_quote(&self, side: Side, size: Decimal, current_price: Decimal) -> Result<LiquidityQuote, LiquidityError> {
        let mut best_quote: Option<LiquidityQuote> = None;

        for pool in &self.pools {
            if let Ok(quote) = pool.get_quote(side, size, current_price) {
                if quote.price_impact > self.max_price_impact {
                    continue;
                }

                let is_better = match &best_quote {
                    None => true,
                    Some(current_best) => {
                        // for buys, lower price is better; for sells, higher is better
                        match side {
                            Side::Long => quote.total_cost() < current_best.total_cost(),
                            Side::Short => quote.total_cost() > current_best.total_cost(),
                        }
                    }
                };

                if is_better {
                    best_quote = Some(quote);
                }
            }
        }

        best_quote.ok_or(LiquidityError::InsufficientLiquidity {
            requested: size,
            available: Decimal::ZERO,
        })
    }

    // Get aggregate liquidity across all pools
    pub fn total_liquidity(&self) -> (Decimal, Decimal) {
        let mut total_long = Decimal::ZERO;
        let mut total_short = Decimal::ZERO;

        for pool in &self.pools {
            let (long, short) = pool.available_liquidity();
            total_long += long;
            total_short += short;
        }

        (total_long, total_short)
    }

    pub fn pool_count(&self) -> usize {
        self.pools.len()
    }
}

// LP position in a shared pool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LpPosition {
    pub account_id: AccountId,
    pub pool_id: PoolId,
    // Share of the pool (like LP tokens)
    pub shares: Decimal,
    // Value at time of deposit
    pub deposited_value: Decimal,
    // Timestamp of deposit
    pub deposit_time: u64,
}

impl LpPosition {
    pub fn new(account_id: AccountId, pool_id: PoolId, shares: Decimal, value: Decimal, time: u64) -> Self {
        Self {
            account_id,
            pool_id,
            shares,
            deposited_value: value,
            deposit_time: time,
        }
    }

    // Calculate current value based on pool's share price
    pub fn current_value(&self, pool_share_price: Decimal) -> Decimal {
        self.shares * pool_share_price
    }

    // Calculate PnL
    pub fn pnl(&self, pool_share_price: Decimal) -> Decimal {
        self.current_value(pool_share_price) - self.deposited_value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dec(val: i64) -> Decimal {
        Decimal::from(val)
    }

    fn create_test_pool() -> SharedPool {
        let config = PoolConfig {
            pool_id: 1,
            name: "Test Pool".to_string(),
            tvl: dec(1_000_000),
            max_utilization: Decimal::new(8, 1),
            fee_bps: 10,
            active: true,
        };
        SharedPool::new(config)
    }

    #[test]
    fn test_pool_deposit_withdraw() {
        let mut pool = create_test_pool();
        assert_eq!(pool.tvl(), dec(1_000_000));

        pool.deposit(dec(500_000));
        assert_eq!(pool.tvl(), dec(1_500_000));

        pool.withdraw(dec(200_000)).unwrap();
        assert_eq!(pool.tvl(), dec(1_300_000));
    }

    #[test]
    fn test_pool_quote() {
        let pool = create_test_pool();
        let quote = pool.get_quote(Side::Long, dec(10), dec(50000)).unwrap();

        assert_eq!(quote.size, dec(10));
        assert!(quote.price > dec(50000)); // buy has positive impact
        assert!(quote.fee > Decimal::ZERO);
        assert!(quote.price_impact >= Decimal::ZERO);
    }

    #[test]
    fn test_pool_inactive() {
        let mut config = PoolConfig::default();
        config.active = false;
        let pool = SharedPool::new(config);

        let result = pool.get_quote(Side::Long, dec(1), dec(50000));
        assert!(matches!(result, Err(LiquidityError::PoolInactive { .. })));
    }

    #[test]
    fn test_pool_utilization() {
        let mut pool = create_test_pool();
        assert_eq!(pool.utilization(), Decimal::ZERO);

        let quote = pool.get_quote(Side::Long, dec(10), dec(50000)).unwrap();
        pool.execute_quote(&quote).unwrap();

        assert!(pool.utilization() > Decimal::ZERO);
    }

    #[test]
    fn test_insufficient_liquidity() {
        let pool = create_test_pool();

        let result = pool.get_quote(Side::Long, dec(20), dec(50000));
        assert!(matches!(result, Err(LiquidityError::InsufficientLiquidity { .. })));
    }

    #[test]
    fn test_lp_position() {
        let pos = LpPosition::new(AccountId(1), 1, dec(100), dec(10000), 0);

        let current_value = pos.current_value(dec(110));
        assert_eq!(current_value, dec(11000));

        let pnl = pos.pnl(dec(110));
        assert_eq!(pnl, dec(1000));
    }

    #[test]
    fn test_quote_expiry() {
        let quote = LiquidityQuote {
            source_id: 1,
            side: Side::Long,
            size: dec(1),
            price: dec(50000),
            fee: dec(5),
            price_impact: Decimal::ZERO,
            valid_for_seconds: 30,
            timestamp: 1000,
        };

        assert!(!quote.is_expired(1020));
        assert!(!quote.is_expired(1030));
        assert!(quote.is_expired(1031));
    }
}
