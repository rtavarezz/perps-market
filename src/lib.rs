//! Perpetual DEX Core Engine.
//!
//! Risk first architecture for perpetual futures trading. Margin math and
//! liquidation logic take priority over liquidity concerns. All computation
//! is deterministic and pure with no external I/O dependencies.
//!
//! Core components include order book matching, market state management,
//! position tracking with isolated margin, funding rate settlement,
//! and liquidation detection.
//!
//! Additions: circuit breakers, auto deleveraging, stop loss and take
//! profit orders, comprehensive stress testing, and solvency invariants.
//! API service layer, price feed integration, liquidity
//! abstractions, custody and deposit/withdrawal flows, settlement layer,
//! and unified configuration management.

// core trading modules
pub mod account;
pub mod engine;
pub mod events;
pub mod funding;
pub mod liquidation;
pub mod margin;
pub mod mark_price;
pub mod market;
pub mod order;
pub mod position;
pub mod types;

// risk and safety modules
pub mod adl;
pub mod conditional;
pub mod risk;

// integration modules
pub mod api;
pub mod config;
pub mod custody;
pub mod liquidity;
pub mod price_feed;
pub mod settlement;

// re exports for convenience
pub use account::*;
pub use adl::*;
pub use conditional::*;
pub use engine::*;
pub use events::*;
pub use funding::*;
pub use liquidation::*;
pub use margin::*;
pub use mark_price::*;
pub use market::*;
pub use order::*;
pub use position::*;
pub use risk::*;
pub use types::*;
pub use api::{EngineCommand, EngineQuery, ApiResponse, ApiError, ErrorCode};
pub use config::{IntegrationConfig, MarketConfig as IntegrationMarketConfig, FeeConfig, Environment};
pub use custody::{CustodyManager, DepositRequest, WithdrawalRequest, CollateralType};
pub use liquidity::{LiquidityProvider, SharedPool, LiquidityRouter, LiquidityQuote};
pub use price_feed::{PriceUpdate, PriceAggregator, TwapCalculator};
pub use settlement::{SettlementManager, SettlementBatch, SettlementInstruction};
