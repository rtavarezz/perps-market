//! Result types and errors for engine operations.

use crate::order::Fill;
use crate::types::{AccountId, MarketId, OrderId, Price, Quote, SignedSize};
use crate::account::AccountError;
use crate::market::MarketError;
use rust_decimal::Decimal;

/// Result of placing an order.
#[derive(Debug, Clone)]
pub struct OrderResult {
    pub order_id: OrderId,
    pub filled_size: Decimal,
    pub remaining_size: Decimal,
    pub average_price: Option<Price>,
    pub is_posted: bool,
    pub fills: Vec<Fill>,
}

/// Result of funding settlement.
#[derive(Debug, Clone)]
pub struct FundingResult {
    pub funding_rate: Decimal,
    pub total_long_payments: Quote,
    pub total_short_payments: Quote,
    pub accounts_affected: usize,
}

/// Result of a liquidation.
#[derive(Debug, Clone)]
pub struct LiquidationResult {
    pub account_id: AccountId,
    pub market_id: MarketId,
    pub position_size: SignedSize,
    pub liquidation_price: Price,
    pub penalty: Quote,
    pub bad_debt: Quote,
    pub realized_pnl: Quote,
}

/// Engine error types.
#[derive(Debug, Clone, thiserror::Error)]
pub enum EngineError {
    #[error("Market {0:?} not found")]
    MarketNotFound(MarketId),

    #[error("Market {0:?} is not active")]
    MarketNotActive(MarketId),

    #[error("Account {0:?} not found")]
    AccountNotFound(AccountId),

    #[error("Order {0:?} not found")]
    OrderNotFound(OrderId),

    #[error("No mark price available for market {0:?}")]
    NoMarkPrice(MarketId),

    #[error("No index price available for market {0:?}")]
    NoIndexPrice(MarketId),

    #[error("Account error: {0}")]
    Account(#[from] AccountError),

    #[error("Market error: {0}")]
    Market(#[from] MarketError),
}
