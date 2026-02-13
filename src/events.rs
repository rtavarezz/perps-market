// 11.0: every state change produces an event. used for audit trails, state reconstruction,
// and notifying external systems. the EventPayload enum lists all event types.

use crate::types::{AccountId, MarketId, OrderId, Price, Quote, Side, SignedSize, Timestamp};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EventId(pub u64);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: EventId,
    pub timestamp: Timestamp,
    pub payload: EventPayload,
}

impl Event {
    pub fn new(id: EventId, timestamp: Timestamp, payload: EventPayload) -> Self {
        Self {
            id,
            timestamp,
            payload,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EventPayload {
    // Trade events
    Fill(FillEvent),
    OrderPlaced(OrderPlacedEvent),
    OrderCanceled(OrderCanceledEvent),

    // Price events
    IndexPriceUpdate(IndexPriceUpdateEvent),
    MarkPriceUpdate(MarkPriceUpdateEvent),

    // Account events
    Deposit(DepositEvent),
    Withdrawal(WithdrawalEvent),
    FundingSettled(FundingSettledEvent),

    // Risk events
    Liquidation(LiquidationEvent),
    MarginCall(MarginCallEvent),
    BadDebt(BadDebtEvent),

    // Position events
    PositionOpened(PositionOpenedEvent),
    PositionClosed(PositionClosedEvent),
    PositionUpdated(PositionUpdatedEvent),

    // Market data events
    OiUpdated(OiUpdatedEvent),
    FundingFeeCollected(FundingFeeCollectedEvent),

    // Custody events
    WithdrawalRejected(WithdrawalRejectedEvent),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FillEvent {
    pub market_id: MarketId,
    pub order_id: OrderId,
    pub account_id: AccountId,
    pub side: Side,
    pub size: Decimal,
    pub price: Price,
    pub fee: Quote,
    pub is_maker: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderPlacedEvent {
    pub market_id: MarketId,
    pub order_id: OrderId,
    pub account_id: AccountId,
    pub side: Side,
    pub size: Decimal,
    pub price: Option<Price>,
    pub reduce_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderCanceledEvent {
    pub market_id: MarketId,
    pub order_id: OrderId,
    pub account_id: AccountId,
    pub reason: CancelReason,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CancelReason {
    UserRequested,
    InsufficientMargin,
    Expired,
    PostOnlyWouldTake,
    ReduceOnlyInvalid,
    Liquidation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexPriceUpdateEvent {
    pub market_id: MarketId,
    pub price: Price,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkPriceUpdateEvent {
    pub market_id: MarketId,
    pub mark_price: Price,
    pub index_price: Price,
    pub premium_index: Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepositEvent {
    pub account_id: AccountId,
    pub amount: Quote,
    pub new_balance: Quote,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WithdrawalEvent {
    pub account_id: AccountId,
    pub amount: Quote,
    pub new_balance: Quote,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FundingSettledEvent {
    pub market_id: MarketId,
    pub account_id: AccountId,
    pub funding_rate: Decimal,
    pub payment: Quote,
    pub position_size: SignedSize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiquidationEvent {
    pub market_id: MarketId,
    pub account_id: AccountId,
    pub liquidated_size: SignedSize,
    pub liquidation_price: Price,
    pub penalty: Quote,
    pub liquidator_account: Option<AccountId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarginCallEvent {
    pub account_id: AccountId,
    pub market_id: MarketId,
    pub margin_ratio: Decimal,
    pub maintenance_margin: Quote,
    pub current_equity: Quote,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BadDebtEvent {
    pub market_id: MarketId,
    pub account_id: AccountId,
    pub debt_amount: Quote,
    pub covered_by_insurance: Quote,
    pub socialized_loss: Quote,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionOpenedEvent {
    pub market_id: MarketId,
    pub account_id: AccountId,
    pub side: Side,
    pub size: Decimal,
    pub entry_price: Price,
    pub collateral: Quote,
    pub leverage: Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionClosedEvent {
    pub market_id: MarketId,
    pub account_id: AccountId,
    pub exit_price: Price,
    pub realized_pnl: Quote,
    pub collateral_returned: Quote,
    pub close_reason: CloseReason,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CloseReason {
    UserClosed,
    StopLoss,
    TakeProfit,
    Liquidation,
    AutoDeleverage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionUpdatedEvent {
    pub market_id: MarketId,
    pub account_id: AccountId,
    pub old_size: SignedSize,
    pub new_size: SignedSize,
    pub old_entry_price: Price,
    pub new_entry_price: Price,
    pub realized_pnl: Quote,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OiUpdatedEvent {
    pub market_id: MarketId,
    pub long_oi: Decimal,
    pub short_oi: Decimal,
    pub total_oi: Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FundingFeeCollectedEvent {
    pub market_id: MarketId,
    pub lp_fee_amount: Quote, // portion routed to LP pool
    pub funding_rate: Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WithdrawalRejectedEvent {
    pub account_id: AccountId,
    pub amount: Quote,
    pub reason: String,
}

pub trait EventEmitter {
    fn emit(&mut self, event: Event);
}

#[derive(Debug, Default)]
pub struct EventCollector {
    events: Vec<Event>,
    next_id: u64,
}

impl EventCollector {
    pub fn new() -> Self {
        Self {
            events: Vec::new(),
            next_id: 1,
        }
    }

    pub fn events(&self) -> &[Event] {
        &self.events
    }

    pub fn clear(&mut self) {
        self.events.clear();
    }

    pub fn next_id(&mut self) -> EventId {
        let id = EventId(self.next_id);
        self.next_id += 1;
        id
    }
}

impl EventEmitter for EventCollector {
    fn emit(&mut self, event: Event) {
        self.events.push(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn event_collector() {
        let mut collector = EventCollector::new();

        let event = Event::new(
            collector.next_id(),
            Timestamp::from_millis(1000),
            EventPayload::Deposit(DepositEvent {
                account_id: AccountId(1),
                amount: Quote::new(dec!(10000)),
                new_balance: Quote::new(dec!(10000)),
            }),
        );

        collector.emit(event);
        assert_eq!(collector.events().len(), 1);

        collector.clear();
        assert!(collector.events().is_empty());
    }

    #[test]
    fn fill_event_creation() {
        let fill = FillEvent {
            market_id: MarketId(1),
            order_id: OrderId(123),
            account_id: AccountId(1),
            side: Side::Long,
            size: dec!(1),
            price: Price::new_unchecked(dec!(50000)),
            fee: Quote::new(dec!(25)),
            is_maker: false,
        };

        assert_eq!(fill.market_id.0, 1);
        assert_eq!(fill.fee.value(), dec!(25));
    }

    #[test]
    fn liquidation_event() {
        let liq = LiquidationEvent {
            market_id: MarketId(1),
            account_id: AccountId(42),
            liquidated_size: SignedSize::new(dec!(-1)), // Closing a long
            liquidation_price: Price::new_unchecked(dec!(47500)),
            penalty: Quote::new(dec!(475)),
            liquidator_account: Some(AccountId(99)),
        };

        assert!(liq.liquidated_size.is_short()); // Liquidation is opposite direction
    }
}
