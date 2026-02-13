// 4.0: open position tracking. pnl = size * (mark - entry).
// 4.1 has increase/reduce/flip logic at the bottom.

use crate::types::{Leverage, MarketId, Price, Quote, Side, SignedSize, Timestamp};
use rust_decimal::Decimal;
use rust_decimal::prelude::Signed;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub market_id: MarketId,
    pub size: SignedSize,
    pub entry_price: Price,
    pub collateral: Quote,
    pub leverage: Leverage,
    pub entry_funding_index: Decimal,
    pub opened_at: Timestamp,
    pub updated_at: Timestamp,
    pub realized_pnl: Quote,
}

impl Position {
    pub fn new(
        market_id: MarketId,
        size: SignedSize,
        entry_price: Price,
        collateral: Quote,
        leverage: Leverage,
        funding_index: Decimal,
        timestamp: Timestamp,
    ) -> Self {
        Self {
            market_id,
            size,
            entry_price,
            collateral,
            leverage,
            entry_funding_index: funding_index,
            opened_at: timestamp,
            updated_at: timestamp,
            realized_pnl: Quote::zero(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.size.is_zero()
    }

    pub fn side(&self) -> Option<Side> {
        self.size.side()
    }

    // 4.1: paper gains/losses based on current price
    pub fn unrealized_pnl(&self, mark_price: Price) -> Quote {
        calculate_unrealized_pnl(self.size, self.entry_price, mark_price)
    }

    pub fn pending_funding(&self, current_funding_index: Decimal) -> Quote {
        let funding_delta = current_funding_index - self.entry_funding_index;
        Quote::new(self.size.value() * funding_delta)
    }

    // 4.2: collateral + pnl - funding. this vs MM determines liquidation
    pub fn equity(&self, mark_price: Price, current_funding_index: Decimal) -> Quote {
        let pnl = self.unrealized_pnl(mark_price);
        let funding = self.pending_funding(current_funding_index);
        Quote::new(self.collateral.value() + pnl.value() - funding.value())
    }

    pub fn notional_value(&self, mark_price: Price) -> Quote {
        Quote::new(self.size.abs() * mark_price.value())
    }

    pub fn entry_value(&self) -> Quote {
        Quote::new(self.size.abs() * self.entry_price.value())
    }
}

// 4.3: the pnl formula. size * (mark - entry)
pub fn calculate_unrealized_pnl(
    size: SignedSize,
    entry_price: Price,
    mark_price: Price,
) -> Quote {
    let pnl = size.value() * (mark_price.value() - entry_price.value());
    Quote::new(pnl)
}

pub fn calculate_realized_pnl(
    close_size: SignedSize,
    entry_price: Price,
    exit_price: Price,
) -> Quote {
    let pnl = close_size.value() * (exit_price.value() - entry_price.value());
    Quote::new(pnl)
}

#[derive(Debug, Clone)]
pub struct PositionUpdate {
    pub new_position: Option<Position>,
    pub realized_pnl: Quote,
    pub collateral_returned: Quote,
    pub collateral_required: Quote,
}

// 4.4: adds to existing position. averages the entry price
pub fn increase_position(
    position: &Position,
    delta_size: Decimal,
    fill_price: Price,
    additional_collateral: Quote,
    new_funding_index: Decimal,
    timestamp: Timestamp,
) -> Position {
    debug_assert!(
        (delta_size > Decimal::ZERO) == position.size.is_long() || position.is_empty(),
        "increase must be same direction as existing position"
    );

    let old_size = position.size.value();
    let new_size_value = old_size + delta_size;
    let new_size = SignedSize::new(new_size_value);

    // Weighted average entry price
    let new_entry = if new_size_value.abs() > Decimal::ZERO {
        let weighted_sum =
            old_size.abs() * position.entry_price.value() + delta_size.abs() * fill_price.value();
        Price::new_unchecked(weighted_sum / new_size_value.abs())
    } else {
        position.entry_price
    };

    // Average the funding index too
    let new_funding_index = if position.is_empty() {
        new_funding_index
    } else {
        let old_weight = old_size.abs() / new_size_value.abs();
        let new_weight = delta_size.abs() / new_size_value.abs();
        old_weight * position.entry_funding_index + new_weight * new_funding_index
    };

    Position {
        market_id: position.market_id,
        size: new_size,
        entry_price: new_entry,
        collateral: position.collateral.add(additional_collateral),
        leverage: position.leverage,
        entry_funding_index: new_funding_index,
        opened_at: position.opened_at,
        updated_at: timestamp,
        realized_pnl: position.realized_pnl,
    }
}

pub fn reduce_position(
    position: &Position,
    reduce_amount: Decimal,
    fill_price: Price,
    current_funding_index: Decimal,
    timestamp: Timestamp,
) -> PositionUpdate {
    debug_assert!(reduce_amount > Decimal::ZERO, "reduce amount must be positive");

    let position_abs_size = position.size.abs();
    let reduce_amount = reduce_amount.min(position_abs_size);

    // Calculate PnL for the reduced portion
    // We're closing part of the position, so PnL is based on original direction
    let close_size_for_pnl = SignedSize::new(position.size.value().signum() * reduce_amount);
    let realized = calculate_realized_pnl(close_size_for_pnl, position.entry_price, fill_price);

    // Calculate funding for the reduced portion
    let reduce_fraction = reduce_amount / position_abs_size;
    let position_funding = position.pending_funding(current_funding_index);
    let funding_for_reduced = Quote::new(position_funding.value() * reduce_fraction);

    // Collateral returned proportionally
    let collateral_returned = Quote::new(position.collateral.value() * reduce_fraction);

    // Remaining position size: reduce the absolute size, keeping the sign
    let remaining_abs = position_abs_size - reduce_amount;
    let remaining_size = if remaining_abs.is_zero() {
        SignedSize::zero()
    } else {
        SignedSize::new(position.size.value().signum() * remaining_abs)
    };

    if remaining_size.is_zero() {
        // Fully closed
        return PositionUpdate {
            new_position: None,
            realized_pnl: Quote::new(
                realized.value() + position.realized_pnl.value() - funding_for_reduced.value(),
            ),
            collateral_returned: position.collateral,
            collateral_required: Quote::zero(),
        };
    }

    // Partial close
    let new_position = Position {
        market_id: position.market_id,
        size: remaining_size,
        entry_price: position.entry_price, // Entry price unchanged on reduction
        collateral: position.collateral.sub(collateral_returned),
        leverage: position.leverage,
        entry_funding_index: position.entry_funding_index,
        opened_at: position.opened_at,
        updated_at: timestamp,
        realized_pnl: position.realized_pnl.add(realized),
    };

    PositionUpdate {
        new_position: Some(new_position),
        realized_pnl: Quote::new(realized.value() - funding_for_reduced.value()),
        collateral_returned,
        collateral_required: Quote::zero(),
    }
}

pub fn flip_position(
    position: &Position,
    new_side_size: Decimal,
    fill_price: Price,
    collateral: Quote,
    leverage: Leverage,
    current_funding_index: Decimal,
    timestamp: Timestamp,
) -> PositionUpdate {
    // First close existing position
    let close_result = reduce_position(
        position,
        position.size.abs(),
        fill_price,
        current_funding_index,
        timestamp,
    );

    // Then open new position
    let new_position = Position::new(
        position.market_id,
        SignedSize::new(new_side_size),
        fill_price,
        collateral,
        leverage,
        current_funding_index,
        timestamp,
    );

    PositionUpdate {
        new_position: Some(new_position),
        realized_pnl: close_result.realized_pnl,
        collateral_returned: close_result.collateral_returned,
        collateral_required: collateral,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn test_position() -> Position {
        Position::new(
            MarketId(1),
            SignedSize::new(dec!(1)), // 1 BTC long
            Price::new_unchecked(dec!(50000)),
            Quote::new(dec!(5000)), // 10x leverage
            Leverage::new(dec!(10)).unwrap(),
            Decimal::ZERO,
            Timestamp::from_millis(0),
        )
    }

    #[test]
    fn unrealized_pnl_long_profit() {
        let pos = test_position();
        let mark = Price::new_unchecked(dec!(52000)); // Price up $2000

        let pnl = pos.unrealized_pnl(mark);
        assert_eq!(pnl.value(), dec!(2000)); // 1 BTC * $2000 = $2000 profit
    }

    #[test]
    fn unrealized_pnl_long_loss() {
        let pos = test_position();
        let mark = Price::new_unchecked(dec!(48000)); // Price down $2000

        let pnl = pos.unrealized_pnl(mark);
        assert_eq!(pnl.value(), dec!(-2000)); // $2000 loss
    }

    #[test]
    fn unrealized_pnl_short_profit() {
        let pos = Position::new(
            MarketId(1),
            SignedSize::new(dec!(-1)), // 1 BTC short
            Price::new_unchecked(dec!(50000)),
            Quote::new(dec!(5000)),
            Leverage::new(dec!(10)).unwrap(),
            Decimal::ZERO,
            Timestamp::from_millis(0),
        );
        let mark = Price::new_unchecked(dec!(48000)); // Price down

        let pnl = pos.unrealized_pnl(mark);
        assert_eq!(pnl.value(), dec!(2000)); // Short profits when price drops
    }

    #[test]
    fn position_equity() {
        let pos = test_position();
        let mark = Price::new_unchecked(dec!(52000)); // $2000 profit

        let equity = pos.equity(mark, Decimal::ZERO);
        // Collateral ($5000) + PnL ($2000) - Funding ($0)
        assert_eq!(equity.value(), dec!(7000));
    }

    #[test]
    fn position_equity_with_funding() {
        let pos = test_position();
        let mark = Price::new_unchecked(dec!(52000));

        // Funding index increased by 100 (long pays)
        let equity = pos.equity(mark, dec!(100));
        // $5000 + $2000 - (1 * 100) = $6900
        assert_eq!(equity.value(), dec!(6900));
    }

    #[test]
    fn increase_position_averaging() {
        let pos = test_position(); // 1 BTC @ $50000
        let fill_price = Price::new_unchecked(dec!(52000));

        let new_pos = increase_position(
            &pos,
            dec!(1),                   // Add 1 BTC
            fill_price,
            Quote::new(dec!(5200)),    // More collateral
            Decimal::ZERO,
            Timestamp::from_millis(1000),
        );

        assert_eq!(new_pos.size.value(), dec!(2)); // 2 BTC total
        // Average: (1 * 50000 + 1 * 52000) / 2 = 51000
        assert_eq!(new_pos.entry_price.value(), dec!(51000));
        assert_eq!(new_pos.collateral.value(), dec!(10200)); // Combined
    }

    #[test]
    fn reduce_position_partial() {
        let pos = Position::new(
            MarketId(1),
            SignedSize::new(dec!(2)), // 2 BTC long
            Price::new_unchecked(dec!(50000)),
            Quote::new(dec!(10000)), // $10k collateral
            Leverage::new(dec!(10)).unwrap(),
            Decimal::ZERO,
            Timestamp::from_millis(0),
        );
        let fill_price = Price::new_unchecked(dec!(52000)); // Exit at profit

        let update = reduce_position(
            &pos,
            dec!(1), // Close 1 BTC
            fill_price,
            Decimal::ZERO,
            Timestamp::from_millis(1000),
        );

        let new_pos = update.new_position.unwrap();
        assert_eq!(new_pos.size.value(), dec!(1)); // 1 BTC remaining
        assert_eq!(new_pos.entry_price.value(), dec!(50000)); // Entry unchanged
        assert_eq!(new_pos.collateral.value(), dec!(5000)); // Half returned

        // Realized: 1 * (52000 - 50000) = 2000
        assert_eq!(update.realized_pnl.value(), dec!(2000));
        assert_eq!(update.collateral_returned.value(), dec!(5000));
    }

    #[test]
    fn reduce_position_full_close() {
        let pos = test_position();
        let fill_price = Price::new_unchecked(dec!(51000));

        let update = reduce_position(
            &pos,
            dec!(1),
            fill_price,
            Decimal::ZERO,
            Timestamp::from_millis(1000),
        );

        assert!(update.new_position.is_none());
        assert_eq!(update.realized_pnl.value(), dec!(1000)); // 1 * (51000 - 50000)
        assert_eq!(update.collateral_returned.value(), dec!(5000));
    }

    #[test]
    fn notional_value() {
        let pos = test_position();
        let mark = Price::new_unchecked(dec!(55000));

        let notional = pos.notional_value(mark);
        assert_eq!(notional.value(), dec!(55000)); // 1 BTC * $55000
    }

    #[test]
    fn flip_position_long_to_short() {
        let pos = test_position(); // Long 1 BTC
        let fill_price = Price::new_unchecked(dec!(51000));

        let update = flip_position(
            &pos,
            dec!(-2), // Flip to short 2 BTC
            fill_price,
            Quote::new(dec!(10200)),
            Leverage::new(dec!(10)).unwrap(),
            Decimal::ZERO,
            Timestamp::from_millis(1000),
        );

        let new_pos = update.new_position.unwrap();
        assert!(new_pos.size.is_short());
        assert_eq!(new_pos.size.value(), dec!(-2));
        assert_eq!(new_pos.entry_price.value(), dec!(51000));
    }
}
