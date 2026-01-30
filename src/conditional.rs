//! Conditional orders: stop-loss, take-profit, and trailing stops.
//!
//! Conditional orders are stored separately from the order book and only become
//! active when their trigger conditions are met. This enables traders to manage
//! risk without constant monitoring.

use crate::types::{AccountId, MarketId, Price, Side, Timestamp};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Unique identifier for conditional orders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConditionalOrderId(pub u64);

/// Type of conditional order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConditionalType {
    /// Triggers when price falls below threshold (for longs) or rises above (for shorts).
    StopLoss,
    /// Triggers when price rises above threshold (for longs) or falls below (for shorts).
    TakeProfit,
    /// Stop that trails the price by a fixed amount or percentage.
    TrailingStop,
}

/// How the trigger price should be compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TriggerCondition {
    /// Triggers when price crosses above the trigger.
    Above,
    /// Triggers when price crosses below the trigger.
    Below,
}

/// A conditional order waiting to be triggered.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConditionalOrder {
    pub id: ConditionalOrderId,
    pub account_id: AccountId,
    pub market_id: MarketId,
    pub side: Side,
    pub size: Decimal,
    pub order_type: ConditionalType,
    pub trigger_price: Price,
    pub trigger_condition: TriggerCondition,
    /// Optional limit price for the resulting order (None means market order).
    pub limit_price: Option<Price>,
    /// Whether this order reduces an existing position only.
    pub reduce_only: bool,
    /// For trailing stops, the trail amount or percentage.
    pub trail_value: Option<Decimal>,
    /// For trailing stops, the highest/lowest seen price.
    pub trail_reference: Option<Price>,
    pub created_at: Timestamp,
}

impl ConditionalOrder {
    pub fn new_stop_loss(
        id: ConditionalOrderId,
        account_id: AccountId,
        market_id: MarketId,
        position_side: Side,
        size: Decimal,
        trigger_price: Price,
        timestamp: Timestamp,
    ) -> Self {
        // Stop loss triggers when price moves against the position
        let trigger_condition = match position_side {
            Side::Long => TriggerCondition::Below,
            Side::Short => TriggerCondition::Above,
        };

        // Close side is opposite of position
        let order_side = position_side.opposite();

        Self {
            id,
            account_id,
            market_id,
            side: order_side,
            size,
            order_type: ConditionalType::StopLoss,
            trigger_price,
            trigger_condition,
            limit_price: None,
            reduce_only: true,
            trail_value: None,
            trail_reference: None,
            created_at: timestamp,
        }
    }

    pub fn new_take_profit(
        id: ConditionalOrderId,
        account_id: AccountId,
        market_id: MarketId,
        position_side: Side,
        size: Decimal,
        trigger_price: Price,
        timestamp: Timestamp,
    ) -> Self {
        // Take profit triggers when price moves in favor of position
        let trigger_condition = match position_side {
            Side::Long => TriggerCondition::Above,
            Side::Short => TriggerCondition::Below,
        };

        let order_side = position_side.opposite();

        Self {
            id,
            account_id,
            market_id,
            side: order_side,
            size,
            order_type: ConditionalType::TakeProfit,
            trigger_price,
            trigger_condition,
            limit_price: None,
            reduce_only: true,
            trail_value: None,
            trail_reference: None,
            created_at: timestamp,
        }
    }

    pub fn new_trailing_stop(
        id: ConditionalOrderId,
        account_id: AccountId,
        market_id: MarketId,
        position_side: Side,
        size: Decimal,
        trail_amount: Decimal,
        current_price: Price,
        timestamp: Timestamp,
    ) -> Self {
        let order_side = position_side.opposite();

        // Calculate initial trigger based on current price
        let trigger_price = match position_side {
            Side::Long => Price::new_unchecked(current_price.value() - trail_amount),
            Side::Short => Price::new_unchecked(current_price.value() + trail_amount),
        };

        let trigger_condition = match position_side {
            Side::Long => TriggerCondition::Below,
            Side::Short => TriggerCondition::Above,
        };

        Self {
            id,
            account_id,
            market_id,
            side: order_side,
            size,
            order_type: ConditionalType::TrailingStop,
            trigger_price,
            trigger_condition,
            limit_price: None,
            reduce_only: true,
            trail_value: Some(trail_amount),
            trail_reference: Some(current_price),
            created_at: timestamp,
        }
    }

    /// Check if this order should trigger at the given price.
    pub fn should_trigger(&self, mark_price: Price) -> bool {
        match self.trigger_condition {
            TriggerCondition::Above => mark_price.value() >= self.trigger_price.value(),
            TriggerCondition::Below => mark_price.value() <= self.trigger_price.value(),
        }
    }

    /// Update trailing stop based on favorable price movement.
    pub fn update_trailing(&mut self, mark_price: Price) {
        if self.order_type != ConditionalType::TrailingStop {
            return;
        }

        let Some(trail_amount) = self.trail_value else {
            return;
        };

        // Determine the position side from the order side (order is opposite)
        let position_side = self.side.opposite();

        match position_side {
            Side::Long => {
                // For long positions, trail reference is the high water mark
                let current_ref = self.trail_reference.map(|p| p.value()).unwrap_or(Decimal::ZERO);
                if mark_price.value() > current_ref {
                    self.trail_reference = Some(mark_price);
                    self.trigger_price =
                        Price::new_unchecked(mark_price.value() - trail_amount);
                }
            }
            Side::Short => {
                // For short positions, trail reference is the low water mark
                let current_ref = self.trail_reference.map(|p| p.value()).unwrap_or(Decimal::MAX);
                if mark_price.value() < current_ref {
                    self.trail_reference = Some(mark_price);
                    self.trigger_price =
                        Price::new_unchecked(mark_price.value() + trail_amount);
                }
            }
        }
    }
}

/// Manages conditional orders for a market.
#[derive(Debug, Clone)]
pub struct ConditionalOrderBook {
    pub market_id: MarketId,
    orders: HashMap<ConditionalOrderId, ConditionalOrder>,
    by_account: HashMap<AccountId, Vec<ConditionalOrderId>>,
    next_id: u64,
}

impl ConditionalOrderBook {
    pub fn new(market_id: MarketId) -> Self {
        Self {
            market_id,
            orders: HashMap::new(),
            by_account: HashMap::new(),
            next_id: 1,
        }
    }

    /// Generate a new conditional order ID.
    pub fn next_id(&mut self) -> ConditionalOrderId {
        let id = ConditionalOrderId(self.next_id);
        self.next_id += 1;
        id
    }

    /// Insert a conditional order.
    pub fn insert(&mut self, order: ConditionalOrder) {
        let id = order.id;
        let account = order.account_id;

        self.orders.insert(id, order);
        self.by_account.entry(account).or_default().push(id);
    }

    /// Remove a conditional order.
    pub fn remove(&mut self, id: ConditionalOrderId) -> Option<ConditionalOrder> {
        if let Some(order) = self.orders.remove(&id) {
            if let Some(ids) = self.by_account.get_mut(&order.account_id) {
                ids.retain(|&oid| oid != id);
            }
            Some(order)
        } else {
            None
        }
    }

    /// Get a conditional order.
    pub fn get(&self, id: ConditionalOrderId) -> Option<&ConditionalOrder> {
        self.orders.get(&id)
    }

    /// Get all orders for an account.
    pub fn get_by_account(&self, account_id: AccountId) -> Vec<&ConditionalOrder> {
        self.by_account
            .get(&account_id)
            .map(|ids| ids.iter().filter_map(|id| self.orders.get(id)).collect())
            .unwrap_or_default()
    }

    /// Check all orders and return those that should trigger.
    pub fn check_triggers(&self, mark_price: Price) -> Vec<ConditionalOrderId> {
        self.orders
            .iter()
            .filter(|(_, order)| order.should_trigger(mark_price))
            .map(|(id, _)| *id)
            .collect()
    }

    /// Update all trailing stops based on price movement.
    pub fn update_trailing_stops(&mut self, mark_price: Price) {
        for order in self.orders.values_mut() {
            order.update_trailing(mark_price);
        }
    }

    /// Cancel all conditional orders for an account.
    pub fn cancel_all_for_account(&mut self, account_id: AccountId) -> Vec<ConditionalOrder> {
        let ids = self.by_account.remove(&account_id).unwrap_or_default();
        ids.into_iter()
            .filter_map(|id| self.orders.remove(&id))
            .collect()
    }

    /// Total number of conditional orders.
    pub fn len(&self) -> usize {
        self.orders.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.orders.is_empty()
    }
}

/// Result of checking conditional orders.
#[derive(Debug, Clone)]
pub struct TriggeredOrders {
    pub triggered: Vec<ConditionalOrder>,
    pub remaining: usize,
}

/// Check and collect triggered orders, removing them from the book.
pub fn process_triggers(
    book: &mut ConditionalOrderBook,
    mark_price: Price,
) -> TriggeredOrders {
    let triggered_ids = book.check_triggers(mark_price);

    let triggered: Vec<ConditionalOrder> = triggered_ids
        .into_iter()
        .filter_map(|id| book.remove(id))
        .collect();

    TriggeredOrders {
        triggered,
        remaining: book.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn stop_loss_for_long() {
        let order = ConditionalOrder::new_stop_loss(
            ConditionalOrderId(1),
            AccountId(1),
            MarketId(1),
            Side::Long,
            dec!(1),
            Price::new_unchecked(dec!(48000)),
            Timestamp::from_millis(0),
        );

        assert_eq!(order.side, Side::Short); // Closes long
        assert_eq!(order.trigger_condition, TriggerCondition::Below);
        assert!(order.reduce_only);

        // Should not trigger above stop
        assert!(!order.should_trigger(Price::new_unchecked(dec!(50000))));

        // Should trigger at or below stop
        assert!(order.should_trigger(Price::new_unchecked(dec!(48000))));
        assert!(order.should_trigger(Price::new_unchecked(dec!(47000))));
    }

    #[test]
    fn stop_loss_for_short() {
        let order = ConditionalOrder::new_stop_loss(
            ConditionalOrderId(1),
            AccountId(1),
            MarketId(1),
            Side::Short,
            dec!(1),
            Price::new_unchecked(dec!(52000)),
            Timestamp::from_millis(0),
        );

        assert_eq!(order.side, Side::Long); // Closes short
        assert_eq!(order.trigger_condition, TriggerCondition::Above);

        // Should not trigger below stop
        assert!(!order.should_trigger(Price::new_unchecked(dec!(50000))));

        // Should trigger at or above stop
        assert!(order.should_trigger(Price::new_unchecked(dec!(52000))));
    }

    #[test]
    fn take_profit_for_long() {
        let order = ConditionalOrder::new_take_profit(
            ConditionalOrderId(1),
            AccountId(1),
            MarketId(1),
            Side::Long,
            dec!(1),
            Price::new_unchecked(dec!(55000)),
            Timestamp::from_millis(0),
        );

        assert_eq!(order.trigger_condition, TriggerCondition::Above);

        assert!(!order.should_trigger(Price::new_unchecked(dec!(54000))));
        assert!(order.should_trigger(Price::new_unchecked(dec!(55000))));
    }

    #[test]
    fn trailing_stop_long() {
        let mut order = ConditionalOrder::new_trailing_stop(
            ConditionalOrderId(1),
            AccountId(1),
            MarketId(1),
            Side::Long,
            dec!(1),
            dec!(2000), // $2000 trail
            Price::new_unchecked(dec!(50000)),
            Timestamp::from_millis(0),
        );

        // Initial trigger is 50000 - 2000 = 48000
        assert_eq!(order.trigger_price.value(), dec!(48000));

        // Price goes up, trail reference and trigger should update
        order.update_trailing(Price::new_unchecked(dec!(52000)));
        assert_eq!(order.trigger_price.value(), dec!(50000)); // 52000 - 2000

        // Price goes down, trigger should not change
        order.update_trailing(Price::new_unchecked(dec!(51000)));
        assert_eq!(order.trigger_price.value(), dec!(50000));

        // Should trigger when price hits stop
        assert!(order.should_trigger(Price::new_unchecked(dec!(50000))));
        assert!(order.should_trigger(Price::new_unchecked(dec!(49000))));
        assert!(!order.should_trigger(Price::new_unchecked(dec!(51000))));
    }

    #[test]
    fn conditional_order_book_operations() {
        let mut book = ConditionalOrderBook::new(MarketId(1));

        let id1 = book.next_id();
        let order1 = ConditionalOrder::new_stop_loss(
            id1,
            AccountId(1),
            MarketId(1),
            Side::Long,
            dec!(1),
            Price::new_unchecked(dec!(48000)),
            Timestamp::from_millis(0),
        );

        let id2 = book.next_id();
        let order2 = ConditionalOrder::new_take_profit(
            id2,
            AccountId(1),
            MarketId(1),
            Side::Long,
            dec!(1),
            Price::new_unchecked(dec!(55000)),
            Timestamp::from_millis(0),
        );

        book.insert(order1);
        book.insert(order2);

        assert_eq!(book.len(), 2);
        assert_eq!(book.get_by_account(AccountId(1)).len(), 2);

        // Check triggers at low price
        let triggers = book.check_triggers(Price::new_unchecked(dec!(47000)));
        assert_eq!(triggers.len(), 1);
        assert_eq!(triggers[0], id1);

        // Remove triggered order
        let removed = book.remove(id1);
        assert!(removed.is_some());
        assert_eq!(book.len(), 1);
    }

    #[test]
    fn process_multiple_triggers() {
        let mut book = ConditionalOrderBook::new(MarketId(1));

        // Add stop loss and take profit
        let sl_id = book.next_id();
        let sl = ConditionalOrder::new_stop_loss(
            sl_id,
            AccountId(1),
            MarketId(1),
            Side::Long,
            dec!(1),
            Price::new_unchecked(dec!(48000)),
            Timestamp::from_millis(0),
        );

        let tp_id = book.next_id();
        let tp = ConditionalOrder::new_take_profit(
            tp_id,
            AccountId(1),
            MarketId(1),
            Side::Long,
            dec!(1),
            Price::new_unchecked(dec!(55000)),
            Timestamp::from_millis(0),
        );

        book.insert(sl);
        book.insert(tp);

        // Price crashes, only stop loss triggers
        let result = process_triggers(&mut book, Price::new_unchecked(dec!(47000)));
        assert_eq!(result.triggered.len(), 1);
        assert_eq!(result.triggered[0].order_type, ConditionalType::StopLoss);
        assert_eq!(result.remaining, 1);
    }

    #[test]
    fn cancel_all_for_account() {
        let mut book = ConditionalOrderBook::new(MarketId(1));

        for i in 1..=5 {
            let id = book.next_id();
            let order = ConditionalOrder::new_stop_loss(
                id,
                AccountId(1),
                MarketId(1),
                Side::Long,
                Decimal::from(i),
                Price::new_unchecked(dec!(48000)),
                Timestamp::from_millis(0),
            );
            book.insert(order);
        }

        assert_eq!(book.len(), 5);

        let canceled = book.cancel_all_for_account(AccountId(1));
        assert_eq!(canceled.len(), 5);
        assert!(book.is_empty());
    }
}
