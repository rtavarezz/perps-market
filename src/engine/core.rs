//! Core engine struct and basic operations.

use super::config::EngineConfig;
use super::results::EngineError;
use crate::account::Account;
use crate::events::{DepositEvent, Event, EventId, EventPayload, WithdrawalEvent};
use crate::liquidation::InsuranceFund;
use crate::market::{MarketConfig, MarketState, MarketStatus};
use crate::types::{AccountId, MarketId, Quote, Timestamp};
use std::collections::HashMap;

/// The core perpetual trading engine.
#[derive(Debug)]
pub struct Engine {
    pub(super) config: EngineConfig,
    pub(super) markets: HashMap<MarketId, MarketState>,
    pub(super) accounts: HashMap<AccountId, Account>,
    pub(super) insurance_fund: InsuranceFund,
    pub(super) events: Vec<Event>,
    pub(super) next_event_id: u64,
    pub(super) next_order_id: u64,
    pub(super) current_time: Timestamp,
}

impl Engine {
    /// Create a new engine with the given configuration.
    pub fn new(config: EngineConfig) -> Self {
        Self {
            config,
            markets: HashMap::new(),
            accounts: HashMap::new(),
            insurance_fund: InsuranceFund::new(Quote::zero()),
            events: Vec::new(),
            next_event_id: 1,
            next_order_id: 1,
            current_time: Timestamp::from_millis(0),
        }
    }

    /// Set the current engine time.
    pub fn set_time(&mut self, timestamp: Timestamp) {
        self.current_time = timestamp;
    }

    /// Get the current engine time.
    pub fn time(&self) -> Timestamp {
        self.current_time
    }

    /// Advance time by a duration in milliseconds.
    pub fn advance_time(&mut self, millis: i64) {
        self.current_time = Timestamp::from_millis(self.current_time.as_millis() + millis);
    }

    /// Add a new market.
    pub fn add_market(&mut self, config: MarketConfig) -> MarketId {
        let market_id = config.id;
        let state = MarketState::new(config, self.current_time);
        self.markets.insert(market_id, state);
        market_id
    }

    /// Get a market by ID.
    pub fn get_market(&self, market_id: MarketId) -> Option<&MarketState> {
        self.markets.get(&market_id)
    }

    /// Get a mutable market by ID.
    pub fn get_market_mut(&mut self, market_id: MarketId) -> Option<&mut MarketState> {
        self.markets.get_mut(&market_id)
    }

    /// Pause a market.
    pub fn pause_market(&mut self, market_id: MarketId) -> Result<(), EngineError> {
        let market = self
            .markets
            .get_mut(&market_id)
            .ok_or(EngineError::MarketNotFound(market_id))?;
        market.status = MarketStatus::Paused;
        Ok(())
    }

    /// Resume a market.
    pub fn resume_market(&mut self, market_id: MarketId) -> Result<(), EngineError> {
        let market = self
            .markets
            .get_mut(&market_id)
            .ok_or(EngineError::MarketNotFound(market_id))?;
        market.status = MarketStatus::Active;
        Ok(())
    }

    /// Create a new account.
    pub fn create_account(&mut self) -> AccountId {
        let id = AccountId(self.accounts.len() as u64 + 1);
        let account = Account::new(id, self.current_time);
        self.accounts.insert(id, account);
        id
    }

    /// Get an account by ID.
    pub fn get_account(&self, account_id: AccountId) -> Option<&Account> {
        self.accounts.get(&account_id)
    }

    /// Get a mutable account by ID.
    pub fn get_account_mut(&mut self, account_id: AccountId) -> Option<&mut Account> {
        self.accounts.get_mut(&account_id)
    }

    /// Iterate over all accounts.
    pub fn accounts_iter(&self) -> impl Iterator<Item = (&AccountId, &Account)> {
        self.accounts.iter()
    }

    /// Deposit funds into an account.
    pub fn deposit(&mut self, account_id: AccountId, amount: Quote) -> Result<(), EngineError> {
        let account = self
            .accounts
            .get_mut(&account_id)
            .ok_or(EngineError::AccountNotFound(account_id))?;

        account.deposit(amount);
        let new_balance = account.balance;

        self.emit_event(EventPayload::Deposit(DepositEvent {
            account_id,
            amount,
            new_balance,
        }));

        Ok(())
    }

    /// Withdraw funds from an account.
    pub fn withdraw(&mut self, account_id: AccountId, amount: Quote) -> Result<(), EngineError> {
        let account = self
            .accounts
            .get_mut(&account_id)
            .ok_or(EngineError::AccountNotFound(account_id))?;

        account.withdraw(amount).map_err(EngineError::Account)?;
        let new_balance = account.balance;

        self.emit_event(EventPayload::Withdrawal(WithdrawalEvent {
            account_id,
            amount,
            new_balance,
        }));

        Ok(())
    }

    /// Get recent events.
    pub fn recent_events(&self, count: usize) -> &[Event] {
        let start = self.events.len().saturating_sub(count);
        &self.events[start..]
    }

    /// Get all events.
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// Get insurance fund balance.
    pub fn insurance_fund_balance(&self) -> Quote {
        self.insurance_fund.balance
    }

    /// Add funds to insurance fund.
    pub fn fund_insurance(&mut self, amount: Quote) {
        self.insurance_fund.deposit(amount);
    }

    /// Emit an event and add it to the event log.
    pub(super) fn emit_event(&mut self, payload: EventPayload) {
        let event = Event::new(EventId(self.next_event_id), self.current_time, payload);
        self.next_event_id += 1;

        if self.config.verbose {
            println!("[Event {}] {:?}", event.id.0, event.payload);
        }

        self.events.push(event);

        if self.events.len() > self.config.max_events {
            let drain_count = self.events.len() - self.config.max_events;
            self.events.drain(0..drain_count);
        }
    }
}
