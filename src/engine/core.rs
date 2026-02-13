// 8.0 engine/core.rs: main engine. holds all markets, accounts, insurance fund.

use super::config::EngineConfig;
use super::results::EngineError;
use crate::account::Account;
use crate::events::{DepositEvent, Event, EventId, EventPayload, WithdrawalEvent, WithdrawalRejectedEvent};
use crate::liquidation::InsuranceFund;
use crate::market::{MarketConfig, MarketState, MarketStatus};
use crate::types::{AccountId, MarketId, Quote, Timestamp};
use std::collections::HashMap;

/** 8.1: main engine struct. all state lives here */
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

    pub fn set_time(&mut self, timestamp: Timestamp) {
        self.current_time = timestamp;
    }

    pub fn time(&self) -> Timestamp {
        self.current_time
    }

    pub fn advance_time(&mut self, millis: i64) {
        self.current_time = Timestamp::from_millis(self.current_time.as_millis() + millis);
    }

    pub fn add_market(&mut self, config: MarketConfig) -> MarketId {
        let market_id = config.id;
        let state = MarketState::new(config, self.current_time);
        self.markets.insert(market_id, state);
        market_id
    }

    pub fn get_market(&self, market_id: MarketId) -> Option<&MarketState> {
        self.markets.get(&market_id)
    }

    pub fn get_market_mut(&mut self, market_id: MarketId) -> Option<&mut MarketState> {
        self.markets.get_mut(&market_id)
    }

    pub fn pause_market(&mut self, market_id: MarketId) -> Result<(), EngineError> {
        let market = self
            .markets
            .get_mut(&market_id)
            .ok_or(EngineError::MarketNotFound(market_id))?;
        market.status = MarketStatus::Paused;
        Ok(())
    }

    pub fn resume_market(&mut self, market_id: MarketId) -> Result<(), EngineError> {
        let market = self
            .markets
            .get_mut(&market_id)
            .ok_or(EngineError::MarketNotFound(market_id))?;
        market.status = MarketStatus::Active;
        Ok(())
    }

    pub fn create_account(&mut self) -> AccountId {
        let id = AccountId(self.accounts.len() as u64 + 1);
        let account = Account::new(id, self.current_time);
        self.accounts.insert(id, account);
        id
    }

    pub fn get_account(&self, account_id: AccountId) -> Option<&Account> {
        self.accounts.get(&account_id)
    }

    pub fn get_account_mut(&mut self, account_id: AccountId) -> Option<&mut Account> {
        self.accounts.get_mut(&account_id)
    }

    pub fn accounts_iter(&self) -> impl Iterator<Item = (&AccountId, &Account)> {
        self.accounts.iter()
    }

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

    // blocked if positions are open
    pub fn withdraw(&mut self, account_id: AccountId, amount: Quote) -> Result<(), EngineError> {
        let account = self
            .accounts
            .get_mut(&account_id)
            .ok_or(EngineError::AccountNotFound(account_id))?;

        if let Err(e) = account.withdraw(amount) {
            // Emit rejection event for audit
            self.emit_event(EventPayload::WithdrawalRejected(WithdrawalRejectedEvent {
                account_id,
                amount,
                reason: e.to_string(),
            }));
            return Err(EngineError::Account(e));
        }
        let new_balance = account.balance;

        self.emit_event(EventPayload::Withdrawal(WithdrawalEvent {
            account_id,
            amount,
            new_balance,
        }));

        Ok(())
    }

    // requires initial pool deposit above min_pool_tvl
    pub fn add_market_with_pool(
        &mut self,
        config: MarketConfig,
        initial_pool_deposit: Quote,
        min_pool_tvl: Quote,
    ) -> Result<MarketId, EngineError> {
        if initial_pool_deposit.value() < min_pool_tvl.value() {
            return Err(EngineError::InsufficientPoolLiquidity {
                provided: initial_pool_deposit,
                minimum: min_pool_tvl,
            });
        }
        let market_id = self.add_market(config);
        // Pool deposit is tracked on the market's pool_funding_fees for now.
        // In production this would create a SharedPool and deposit into it.
        let market = self.markets.get_mut(&market_id).unwrap();
        market.pool_funding_fees += initial_pool_deposit.value();
        Ok(market_id)
    }

    pub fn set_referrer(&mut self, account_id: AccountId, referrer_id: AccountId) -> Result<(), EngineError> {
        if !self.accounts.contains_key(&referrer_id) {
            return Err(EngineError::AccountNotFound(referrer_id));
        }
        let account = self
            .accounts
            .get_mut(&account_id)
            .ok_or(EngineError::AccountNotFound(account_id))?;
        account.set_referrer(referrer_id);
        Ok(())
    }

    pub fn recent_events(&self, count: usize) -> &[Event] {
        let start = self.events.len().saturating_sub(count);
        &self.events[start..]
    }

    pub fn events(&self) -> &[Event] {
        &self.events
    }

    pub fn insurance_fund_balance(&self) -> Quote {
        self.insurance_fund.balance
    }

    pub fn fund_insurance(&mut self, amount: Quote) {
        self.insurance_fund.deposit(amount);
    }

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
