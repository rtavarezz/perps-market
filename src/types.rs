//! Core types for the perpetual DEX engine.
//!
//! All monetary values use `Decimal` for 28 digit precision.
//! Sizes are signed, positive for long and negative for short.

use rust_decimal::Decimal;
use std::iter::Sum;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Market identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MarketId(pub u32);

/// Account identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AccountId(pub u64);

/// Unique order identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OrderId(pub u64);

/// Position side
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    Long,
    Short,
}

impl Side {
    pub fn sign(&self) -> Decimal {
        match self {
            Side::Long => dec!(1),
            Side::Short => dec!(-1),
        }
    }

    pub fn opposite(&self) -> Self {
        match self {
            Side::Long => Side::Short,
            Side::Short => Side::Long,
        }
    }
}

/// Signed size where positive is long and negative is short.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedSize(Decimal);

impl SignedSize {
    pub fn new(size: Decimal) -> Self {
        Self(size)
    }

    pub fn zero() -> Self {
        Self(Decimal::ZERO)
    }

    pub fn from_side(side: Side, abs_size: Decimal) -> Self {
        Self(side.sign() * abs_size.abs())
    }

    pub fn value(&self) -> Decimal {
        self.0
    }

    pub fn abs(&self) -> Decimal {
        self.0.abs()
    }

    pub fn is_zero(&self) -> bool {
        self.0.is_zero()
    }

    pub fn is_long(&self) -> bool {
        self.0 > Decimal::ZERO
    }

    pub fn is_short(&self) -> bool {
        self.0 < Decimal::ZERO
    }

    pub fn side(&self) -> Option<Side> {
        if self.is_long() {
            Some(Side::Long)
        } else if self.is_short() {
            Some(Side::Short)
        } else {
            None
        }
    }

    pub fn add(&self, delta: Decimal) -> Self {
        Self(self.0 + delta)
    }
}

impl fmt::Display for SignedSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Price in quote currency per unit of base.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Price(Decimal);

impl Price {
    #[must_use]
    pub fn new(value: Decimal) -> Option<Self> {
        if value > Decimal::ZERO {
            Some(Self(value))
        } else {
            None
        }
    }

    pub fn new_unchecked(value: Decimal) -> Self {
        debug_assert!(value > Decimal::ZERO);
        Self(value)
    }

    pub fn value(&self) -> Decimal {
        self.0
    }
}

impl fmt::Display for Price {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Quote currency amount for collateral, margin, and PnL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quote(Decimal);

impl Quote {
    pub fn new(value: Decimal) -> Self {
        Self(value)
    }

    pub fn zero() -> Self {
        Self(Decimal::ZERO)
    }

    pub fn value(&self) -> Decimal {
        self.0
    }

    pub fn is_negative(&self) -> bool {
        self.0 < Decimal::ZERO
    }

    pub fn abs(&self) -> Self {
        Self(self.0.abs())
    }

    pub fn add(&self, other: Quote) -> Self {
        Self(self.0 + other.0)
    }

    pub fn sub(&self, other: Quote) -> Self {
        Self(self.0 - other.0)
    }

    pub fn mul(&self, factor: Decimal) -> Self {
        Self(self.0 * factor)
    }

    pub fn negate(&self) -> Self {
        Self(-self.0)
    }
}

impl fmt::Display for Quote {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl PartialOrd for Quote {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Quote {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

impl Sum for Quote {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Self::zero(), |acc, q| acc.add(q))
    }
}

impl<'a> Sum<&'a Quote> for Quote {
    fn sum<I: Iterator<Item = &'a Self>>(iter: I) -> Self {
        iter.fold(Self::zero(), |acc, q| acc.add(*q))
    }
}

/// Leverage multiplier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Leverage(Decimal);

impl Leverage {
    #[must_use]
    pub fn new(value: Decimal) -> Option<Self> {
        if value >= Decimal::ONE {
            Some(Self(value))
        } else {
            None
        }
    }

    pub fn value(&self) -> Decimal {
        self.0
    }

    pub fn initial_margin_fraction(&self) -> Decimal {
        Decimal::ONE / self.0
    }
}

impl fmt::Display for Leverage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}x", self.0)
    }
}

/// Basis points where 1 bp equals 0.01 percent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bps(i32);

impl Bps {
    pub fn new(bps: i32) -> Self {
        Self(bps)
    }

    pub fn value(&self) -> i32 {
        self.0
    }

    pub fn as_fraction(&self) -> Decimal {
        Decimal::new(self.0 as i64, 4)
    }
}

/// Timestamp in milliseconds since epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Timestamp(pub i64);

impl Timestamp {
    pub fn now() -> Self {
        Self(chrono::Utc::now().timestamp_millis())
    }

    pub fn from_millis(ms: i64) -> Self {
        Self(ms)
    }

    pub fn as_millis(&self) -> i64 {
        self.0
    }

    pub fn elapsed_hours(&self, other: &Timestamp) -> Decimal {
        let diff_ms = (other.0 - self.0).abs();
        Decimal::new(diff_ms, 0) / dec!(3_600_000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn signed_size_operations() {
        let long = SignedSize::from_side(Side::Long, dec!(10));
        assert!(long.is_long());
        assert_eq!(long.abs(), dec!(10));

        let short = SignedSize::from_side(Side::Short, dec!(10));
        assert!(short.is_short());
        assert_eq!(short.abs(), dec!(10));
        assert_eq!(short.value(), dec!(-10));
    }

    #[test]
    fn leverage_margin_fraction() {
        let lev_10x = Leverage::new(dec!(10)).unwrap();
        assert_eq!(lev_10x.initial_margin_fraction(), dec!(0.1));

        let lev_20x = Leverage::new(dec!(20)).unwrap();
        assert_eq!(lev_20x.initial_margin_fraction(), dec!(0.05));
    }

    #[test]
    fn bps_conversion() {
        let hundred_bps = Bps::new(100);
        assert_eq!(hundred_bps.as_fraction(), dec!(0.01)); // 1%

        let fifty_bps = Bps::new(50);
        assert_eq!(fifty_bps.as_fraction(), dec!(0.005)); // 0.5%
    }
}
