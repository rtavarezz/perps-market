//! Perpetual DEX Core Engine.
//!
//! Risk first architecture for perpetual futures trading. Margin math and
//! liquidation logic take priority over liquidity concerns. All computation
//! is deterministic and pure with no external I/O dependencies.
//!
//! Core components include order book matching, market state management,
//! position tracking with isolated margin, funding rate settlement,
//! and liquidation detection.

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

pub use account::*;
pub use engine::*;
pub use events::*;
pub use funding::*;
pub use liquidation::*;
pub use margin::*;
pub use mark_price::*;
pub use market::*;
pub use order::*;
pub use position::*;
pub use types::*;
