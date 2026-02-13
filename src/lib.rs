// perps-core: perpetual futures trading engine.
// risk-first architecture: margin math and liquidation take priority.
// all computation is deterministic with no external I/O.
//
// file map (search X.0 for structs, X.1+ for logic):
//   1.x  types.rs: primitives: MarketId, Side, Price, Quote, Leverage
//   2.x  order.rs: CLOB order book and matching engine
//   2.1x conditional.rs: stop loss, take profit, trailing stops, OCO
//   3.x  margin.rs: IM/MM calculation, leverage tiers
//   4.x  position.rs: position struct, PnL, increase/reduce/flip
//   5.x  funding.rs: 8-hour funding cycle, premium index
//   6.x  liquidation.rs: liquidation detection, penalty, insurance
//   6.2  adl.rs: auto-deleveraging when insurance empty
//   6.3  risk.rs: circuit breakers, position/OI limits
//   7.x  config.rs: fees, margins, risk params, env presets
//   8.x  engine/: core engine: orders, positions, funding, liquidations
//   9.x  price_feed.rs: oracle aggregation (mocked)
//   9.1  settlement.rs: settlement batching (mocked)
//   9.2  custody.rs: deposit/withdraw flows (mocked)
//   9.3  liquidity.rs: LP pool abstraction (mocked)
//   10.x account.rs: account + collateral management
//   11.x events.rs: state transition events for audit
//   12.x market.rs: market config + runtime state
//   13.x mark_price.rs: blended mark price derivation

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
