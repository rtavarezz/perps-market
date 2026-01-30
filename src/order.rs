//! Order types and order book implementation.
//!
//! Minimal order matching for perpetual futures. Supports limit orders with
//! price-time priority and market orders that execute immediately.

use crate::types::{AccountId, MarketId, OrderId, Price, Side, Timestamp};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Order time in force options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimeInForce {
    /// Good till canceled. Remains on book until filled or canceled.
    GTC,
    /// Immediate or cancel. Fill what is possible, cancel the rest.
    IOC,
    /// Fill or kill. Fill entirely or cancel entirely.
    FOK,
    /// Post only. Reject if would take liquidity.
    PostOnly,
}

impl Default for TimeInForce {
    fn default() -> Self {
        Self::GTC
    }
}

/// Order type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderType {
    /// Limit order with specified price.
    Limit,
    /// Market order. Executes at best available price.
    Market,
}

/// A trading order
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Order {
    pub id: OrderId,
    pub account_id: AccountId,
    pub market_id: MarketId,
    pub side: Side,
    pub order_type: OrderType,
    pub size: Decimal,
    pub remaining_size: Decimal,
    pub price: Option<Price>,
    pub time_in_force: TimeInForce,
    pub reduce_only: bool,
    pub post_only: bool,
    pub created_at: Timestamp,
}

impl Order {
    pub fn new_limit(
        id: OrderId,
        account_id: AccountId,
        market_id: MarketId,
        side: Side,
        size: Decimal,
        price: Price,
        time_in_force: TimeInForce,
        timestamp: Timestamp,
    ) -> Self {
        Self {
            id,
            account_id,
            market_id,
            side,
            order_type: OrderType::Limit,
            size,
            remaining_size: size,
            price: Some(price),
            time_in_force,
            reduce_only: false,
            post_only: time_in_force == TimeInForce::PostOnly,
            created_at: timestamp,
        }
    }

    pub fn new_market(
        id: OrderId,
        account_id: AccountId,
        market_id: MarketId,
        side: Side,
        size: Decimal,
        timestamp: Timestamp,
    ) -> Self {
        Self {
            id,
            account_id,
            market_id,
            side,
            order_type: OrderType::Market,
            size,
            remaining_size: size,
            price: None,
            time_in_force: TimeInForce::IOC,
            reduce_only: false,
            post_only: false,
            created_at: timestamp,
        }
    }

    pub fn is_filled(&self) -> bool {
        self.remaining_size.is_zero()
    }

    pub fn is_bid(&self) -> bool {
        self.side == Side::Long
    }

    pub fn is_ask(&self) -> bool {
        self.side == Side::Short
    }

    pub fn fill(&mut self, size: Decimal) {
        debug_assert!(size <= self.remaining_size, "cannot fill more than remaining");
        self.remaining_size -= size;
    }
}

/// Order priority key for price-time ordering
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OrderKey {
    price: Price,
    timestamp: Timestamp,
    order_id: OrderId,
}

impl OrderKey {
    fn new(price: Price, timestamp: Timestamp, order_id: OrderId) -> Self {
        Self {
            price,
            timestamp,
            order_id,
        }
    }
}

impl PartialOrd for OrderKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OrderKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // First compare by price, then by timestamp (earlier is better), then by order_id
        self.price
            .cmp(&other.price)
            .then(self.timestamp.cmp(&other.timestamp))
            .then(self.order_id.0.cmp(&other.order_id.0))
    }
}

/// A single price level in the order book
#[derive(Debug, Clone)]
pub struct PriceLevel {
    pub price: Price,
    pub total_size: Decimal,
    pub order_count: usize,
}

/// Central Limit Order Book (CLOB)
#[derive(Debug, Clone)]
pub struct OrderBook {
    pub market_id: MarketId,
    /// Bids sorted by price descending (highest first)
    bids: BTreeMap<OrderKey, Order>,
    /// Asks sorted by price ascending (lowest first)
    asks: BTreeMap<OrderKey, Order>,
    /// Quick lookup by order ID
    order_index: std::collections::HashMap<OrderId, (Side, OrderKey)>,
}

impl OrderBook {
    pub fn new(market_id: MarketId) -> Self {
        Self {
            market_id,
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            order_index: std::collections::HashMap::new(),
        }
    }

    /// Get the best bid price (highest buy order)
    pub fn best_bid(&self) -> Option<Price> {
        self.bids.iter().next_back().map(|(k, _)| k.price)
    }

    /// Get the best ask price (lowest sell order)
    pub fn best_ask(&self) -> Option<Price> {
        self.asks.iter().next().map(|(k, _)| k.price)
    }

    /// Get the mid price (average of best bid and ask)
    pub fn mid_price(&self) -> Option<Price> {
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) => {
                let mid = (bid.value() + ask.value()) / Decimal::TWO;
                Some(Price::new_unchecked(mid))
            }
            _ => None,
        }
    }

    /// Get the spread between best bid and ask
    pub fn spread(&self) -> Option<Decimal> {
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) => Some(ask.value() - bid.value()),
            _ => None,
        }
    }

    /// Insert an order into the book
    pub fn insert(&mut self, order: Order) {
        let price = order.price.expect("limit order must have price");
        let key = OrderKey::new(price, order.created_at, order.id);
        let side = order.side;

        self.order_index.insert(order.id, (side, key));

        match side {
            Side::Long => {
                self.bids.insert(key, order);
            }
            Side::Short => {
                self.asks.insert(key, order);
            }
        }
    }

    /// Remove an order from the book by ID
    pub fn remove(&mut self, order_id: OrderId) -> Option<Order> {
        if let Some((side, key)) = self.order_index.remove(&order_id) {
            match side {
                Side::Long => self.bids.remove(&key),
                Side::Short => self.asks.remove(&key),
            }
        } else {
            None
        }
    }

    /// Get an order by ID
    pub fn get(&self, order_id: OrderId) -> Option<&Order> {
        if let Some((side, key)) = self.order_index.get(&order_id) {
            match side {
                Side::Long => self.bids.get(key),
                Side::Short => self.asks.get(key),
            }
        } else {
            None
        }
    }

    /// Get mutable reference to an order by ID
    pub fn get_mut(&mut self, order_id: OrderId) -> Option<&mut Order> {
        if let Some((side, key)) = self.order_index.get(&order_id).cloned() {
            match side {
                Side::Long => self.bids.get_mut(&key),
                Side::Short => self.asks.get_mut(&key),
            }
        } else {
            None
        }
    }

    /// Get the best bid orders up to a certain depth
    pub fn top_bids(&self, depth: usize) -> Vec<&Order> {
        // For bids: highest price is best, but at same price, earlier time is better
        // BTreeMap orders by OrderKey (price asc, then time asc)
        // We need to reverse by price but not by time at the same price
        // Collect all, group by price, then return in descending price order
        let mut bids: Vec<&Order> = self.bids.values().collect();
        bids.sort_by(|a, b| {
            b.price.unwrap().cmp(&a.price.unwrap()) // Price descending
                .then(a.created_at.cmp(&b.created_at)) // Time ascending
                .then(a.id.0.cmp(&b.id.0)) // Order ID ascending
        });
        bids.into_iter().take(depth).collect()
    }

    /// Get the best ask orders up to a certain depth
    pub fn top_asks(&self, depth: usize) -> Vec<&Order> {
        self.asks.values().take(depth).collect()
    }

    /// Get bid depth at each price level
    pub fn bid_levels(&self, max_levels: usize) -> Vec<PriceLevel> {
        let mut levels: Vec<PriceLevel> = Vec::new();
        let mut current_price: Option<Price> = None;

        for (key, order) in self.bids.iter().rev() {
            if Some(key.price) != current_price {
                if levels.len() >= max_levels {
                    break;
                }
                current_price = Some(key.price);
                levels.push(PriceLevel {
                    price: key.price,
                    total_size: Decimal::ZERO,
                    order_count: 0,
                });
            }
            if let Some(level) = levels.last_mut() {
                level.total_size += order.remaining_size;
                level.order_count += 1;
            }
        }

        levels
    }

    /// Get ask depth at each price level
    pub fn ask_levels(&self, max_levels: usize) -> Vec<PriceLevel> {
        let mut levels: Vec<PriceLevel> = Vec::new();
        let mut current_price: Option<Price> = None;

        for (key, order) in self.asks.iter() {
            if Some(key.price) != current_price {
                if levels.len() >= max_levels {
                    break;
                }
                current_price = Some(key.price);
                levels.push(PriceLevel {
                    price: key.price,
                    total_size: Decimal::ZERO,
                    order_count: 0,
                });
            }
            if let Some(level) = levels.last_mut() {
                level.total_size += order.remaining_size;
                level.order_count += 1;
            }
        }

        levels
    }

    /// Check if the book is crossed (best bid >= best ask)
    pub fn is_crossed(&self) -> bool {
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) => bid.value() >= ask.value(),
            _ => false,
        }
    }

    /// Total number of orders in the book
    pub fn order_count(&self) -> usize {
        self.bids.len() + self.asks.len()
    }

    /// Is the book empty?
    pub fn is_empty(&self) -> bool {
        self.bids.is_empty() && self.asks.is_empty()
    }
}

/// Result of matching an incoming order against the book
#[derive(Debug, Clone)]
pub struct MatchResult {
    pub fills: Vec<Fill>,
    pub remaining_size: Decimal,
    pub fully_filled: bool,
}

/// A fill (execution) between two orders
#[derive(Debug, Clone)]
pub struct Fill {
    pub maker_order_id: OrderId,
    pub maker_account_id: AccountId,
    pub taker_order_id: OrderId,
    pub taker_account_id: AccountId,
    pub price: Price,
    pub size: Decimal,
    pub taker_side: Side,
}

/// Match an incoming order against the order book
/// Returns fills and any remaining unfilled size
pub fn match_order(book: &mut OrderBook, mut order: Order) -> MatchResult {
    let mut fills = Vec::new();

    // Market orders must match immediately or fail
    // Limit orders match if they cross the spread
    let is_buy = order.is_bid();
    let limit_price = order.price;

    while !order.is_filled() {
        // Get the best opposing order
        let best_opposing = if is_buy {
            // Buying: match against asks (lowest first)
            book.asks.iter().next().map(|(k, _)| *k)
        } else {
            // Selling: match against bids (highest first)
            book.bids.iter().next_back().map(|(k, _)| *k)
        };

        let Some(opposing_key) = best_opposing else {
            break; // No liquidity
        };

        // Check if price matches
        let can_match = if is_buy {
            // Buy order matches if bid price >= ask price
            limit_price.map_or(true, |p| p.value() >= opposing_key.price.value())
        } else {
            // Sell order matches if ask price <= bid price
            limit_price.map_or(true, |p| p.value() <= opposing_key.price.value())
        };

        if !can_match {
            break; // Price doesn't cross
        }

        // Get the opposing order
        let opposing = if is_buy {
            book.asks.get_mut(&opposing_key).unwrap()
        } else {
            book.bids.get_mut(&opposing_key).unwrap()
        };

        // Calculate fill size
        let fill_size = order.remaining_size.min(opposing.remaining_size);

        // Create fill at maker's price (price improvement for taker)
        let fill = Fill {
            maker_order_id: opposing.id,
            maker_account_id: opposing.account_id,
            taker_order_id: order.id,
            taker_account_id: order.account_id,
            price: opposing_key.price,
            size: fill_size,
            taker_side: order.side,
        };

        // Update both orders
        order.fill(fill_size);
        opposing.fill(fill_size);

        let opposing_filled = opposing.is_filled();
        let opposing_id = opposing.id;

        fills.push(fill);

        // Remove filled maker order
        if opposing_filled {
            book.remove(opposing_id);
        }
    }

    let remaining_size = order.remaining_size;
    let fully_filled = order.is_filled();

    MatchResult {
        fills,
        remaining_size,
        fully_filled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn create_bid(id: u64, price: Decimal, size: Decimal, ts: i64) -> Order {
        Order::new_limit(
            OrderId(id),
            AccountId(1),
            MarketId(1),
            Side::Long,
            size,
            Price::new_unchecked(price),
            TimeInForce::GTC,
            Timestamp::from_millis(ts),
        )
    }

    fn create_ask(id: u64, price: Decimal, size: Decimal, ts: i64) -> Order {
        Order::new_limit(
            OrderId(id),
            AccountId(2),
            MarketId(1),
            Side::Short,
            size,
            Price::new_unchecked(price),
            TimeInForce::GTC,
            Timestamp::from_millis(ts),
        )
    }

    #[test]
    fn empty_book() {
        let book = OrderBook::new(MarketId(1));
        assert!(book.is_empty());
        assert!(book.best_bid().is_none());
        assert!(book.best_ask().is_none());
        assert!(book.mid_price().is_none());
    }

    #[test]
    fn insert_and_retrieve() {
        let mut book = OrderBook::new(MarketId(1));

        let bid = create_bid(1, dec!(50000), dec!(1), 0);
        book.insert(bid);

        let ask = create_ask(2, dec!(50100), dec!(1), 0);
        book.insert(ask);

        assert_eq!(book.best_bid().unwrap().value(), dec!(50000));
        assert_eq!(book.best_ask().unwrap().value(), dec!(50100));
        assert_eq!(book.spread().unwrap(), dec!(100));
    }

    #[test]
    fn price_time_priority() {
        let mut book = OrderBook::new(MarketId(1));

        // Same price, different times
        book.insert(create_bid(1, dec!(50000), dec!(1), 100));
        book.insert(create_bid(2, dec!(50000), dec!(1), 50)); // Earlier

        // Higher price bid
        book.insert(create_bid(3, dec!(50100), dec!(1), 200));

        let top_bids = book.top_bids(3);
        // Should be: highest price first, then by time
        assert_eq!(top_bids[0].id.0, 3); // Highest price
        assert_eq!(top_bids[1].id.0, 2); // Same price, earlier time
        assert_eq!(top_bids[2].id.0, 1); // Same price, later time
    }

    #[test]
    fn match_market_buy() {
        let mut book = OrderBook::new(MarketId(1));

        // Add asks
        book.insert(create_ask(1, dec!(50000), dec!(1), 0));
        book.insert(create_ask(2, dec!(50100), dec!(2), 0));

        // Market buy for 1.5 BTC
        let market_buy = Order::new_market(
            OrderId(3),
            AccountId(3),
            MarketId(1),
            Side::Long,
            dec!(1.5),
            Timestamp::from_millis(100),
        );

        let result = match_order(&mut book, market_buy);

        assert_eq!(result.fills.len(), 2);
        assert!(result.fully_filled);

        // First fill at best ask
        assert_eq!(result.fills[0].price.value(), dec!(50000));
        assert_eq!(result.fills[0].size, dec!(1));

        // Second fill at next ask
        assert_eq!(result.fills[1].price.value(), dec!(50100));
        assert_eq!(result.fills[1].size, dec!(0.5));

        // Order 1 should be removed, order 2 partially filled
        assert!(book.get(OrderId(1)).is_none());
        assert_eq!(book.get(OrderId(2)).unwrap().remaining_size, dec!(1.5));
    }

    #[test]
    fn match_limit_buy_crossing() {
        let mut book = OrderBook::new(MarketId(1));

        book.insert(create_ask(1, dec!(50000), dec!(1), 0));

        // Limit buy above best ask, should match.
        let limit_buy = Order::new_limit(
            OrderId(2),
            AccountId(2),
            MarketId(1),
            Side::Long,
            dec!(0.5),
            Price::new_unchecked(dec!(50100)),
            TimeInForce::GTC,
            Timestamp::from_millis(100),
        );

        let result = match_order(&mut book, limit_buy);

        assert_eq!(result.fills.len(), 1);
        assert!(result.fully_filled);
        // Price improvement: filled at maker's price (50000) not taker's (50100)
        assert_eq!(result.fills[0].price.value(), dec!(50000));
    }

    #[test]
    fn limit_order_no_cross() {
        let mut book = OrderBook::new(MarketId(1));

        book.insert(create_ask(1, dec!(50000), dec!(1), 0));

        // Limit buy below best ask, should not match.
        let limit_buy = Order::new_limit(
            OrderId(2),
            AccountId(2),
            MarketId(1),
            Side::Long,
            dec!(1),
            Price::new_unchecked(dec!(49900)),
            TimeInForce::GTC,
            Timestamp::from_millis(100),
        );

        let result = match_order(&mut book, limit_buy);

        assert!(result.fills.is_empty());
        assert!(!result.fully_filled);
        assert_eq!(result.remaining_size, dec!(1));
    }

    #[test]
    fn remove_order() {
        let mut book = OrderBook::new(MarketId(1));

        book.insert(create_bid(1, dec!(50000), dec!(1), 0));
        assert_eq!(book.order_count(), 1);

        let removed = book.remove(OrderId(1));
        assert!(removed.is_some());
        assert!(book.is_empty());
    }

    #[test]
    fn bid_ask_levels() {
        let mut book = OrderBook::new(MarketId(1));

        // Multiple orders at same price
        book.insert(create_bid(1, dec!(50000), dec!(1), 0));
        book.insert(create_bid(2, dec!(50000), dec!(2), 10));
        book.insert(create_bid(3, dec!(49900), dec!(1), 20));

        let levels = book.bid_levels(10);
        assert_eq!(levels.len(), 2);
        assert_eq!(levels[0].price.value(), dec!(50000));
        assert_eq!(levels[0].total_size, dec!(3));
        assert_eq!(levels[0].order_count, 2);
        assert_eq!(levels[1].price.value(), dec!(49900));
    }
}
