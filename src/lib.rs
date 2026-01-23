//! Perpetual DEX Core Engine.
//!
//! Risk first architecture for perpetual futures trading. Margin math and
//! liquidation logic take priority over liquidity concerns. All computation
//! is deterministic and pure with no external I/O dependencies.

pub mod types;
pub mod margin;
pub mod mark_price;
pub mod funding;
pub mod liquidation;
pub mod position;
pub mod account;
pub mod events;

pub use types::*;
pub use margin::*;
pub use mark_price::*;
pub use funding::*;
pub use liquidation::*;
pub use position::*;
pub use account::*;
pub use events::*;
