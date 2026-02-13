// 8.0: core trading engine. coordinates order execution, position management,
// price updates, funding settlements, and liquidation checks.
// deterministic and event-driven with no external I/O.

mod config;
mod core;
mod orders;
mod positions;
mod pricing;
mod funding;
mod liquidations;
mod results;

pub use config::EngineConfig;
pub use core::Engine;
pub use results::{EngineError, FundingResult, LiquidationResult, OrderResult};
