//! Core trading engine.
//!
//! The engine coordinates order execution, position management, price updates,
//! funding settlements, and liquidation checks. Designed to be deterministic
//! and event driven with no external I/O dependencies.

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
