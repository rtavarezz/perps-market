//! Account and collateral management.
//!
//! Accounts hold trading collateral with isolated margin where each position has
//! its own collateral and risk is isolated between positions.

use crate::margin::{calculate_margin_requirement, MarginParams};
use crate::position::Position;
use crate::types::{AccountId, MarketId, Price, Quote, Timestamp};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: AccountId,
    pub balance: Quote,
    pub positions: HashMap<MarketId, Position>,
    pub total_deposited: Quote,
    pub total_withdrawn: Quote,
    pub realized_pnl: Quote,
    pub created_at: Timestamp,
}

impl Account {
    pub fn new(id: AccountId, timestamp: Timestamp) -> Self {
        Self {
            id,
            balance: Quote::zero(),
            positions: HashMap::new(),
            total_deposited: Quote::zero(),
            total_withdrawn: Quote::zero(),
            realized_pnl: Quote::zero(),
            created_at: timestamp,
        }
    }

    pub fn deposit(&mut self, amount: Quote) {
        self.balance = self.balance.add(amount);
        self.total_deposited = self.total_deposited.add(amount);
    }

    pub fn withdraw(&mut self, amount: Quote) -> Result<(), AccountError> {
        if amount.value() > self.balance.value() {
            return Err(AccountError::InsufficientBalance {
                requested: amount,
                available: self.balance,
            });
        }
        self.balance = self.balance.sub(amount);
        self.total_withdrawn = self.total_withdrawn.add(amount);
        Ok(())
    }

    pub fn get_position(&self, market_id: MarketId) -> Option<&Position> {
        self.positions.get(&market_id)
    }

    pub fn get_position_mut(&mut self, market_id: MarketId) -> Option<&mut Position> {
        self.positions.get_mut(&market_id)
    }

    pub fn set_position(&mut self, position: Position) {
        self.positions.insert(position.market_id, position);
    }

    pub fn remove_position(&mut self, market_id: MarketId) -> Option<Position> {
        self.positions.remove(&market_id)
    }

    pub fn realize_pnl(&mut self, pnl: Quote) {
        self.balance = self.balance.add(pnl);
        self.realized_pnl = self.realized_pnl.add(pnl);
    }

    pub fn return_collateral(&mut self, amount: Quote) {
        self.balance = self.balance.add(amount);
    }

    pub fn reserve_collateral(&mut self, amount: Quote) -> Result<(), AccountError> {
        if amount.value() > self.balance.value() {
            return Err(AccountError::InsufficientBalance {
                requested: amount,
                available: self.balance,
            });
        }
        self.balance = self.balance.sub(amount);
        Ok(())
    }
}

pub struct AccountMetrics {
    pub total_equity: Quote,
    pub unrealized_pnl: Quote,
    pub pending_funding: Quote,
    pub margin_used: Quote,
    pub free_margin: Quote,
    pub margin_ratio: Decimal,
}

pub fn calculate_account_metrics(
    account: &Account,
    market_prices: &HashMap<MarketId, (Price, Decimal)>,
    margin_params: &MarginParams,
) -> AccountMetrics {
    let mut unrealized_pnl = Quote::zero();
    let mut pending_funding = Quote::zero();
    let mut margin_used = Quote::zero();
    let mut total_notional = Quote::zero();

    for (market_id, position) in &account.positions {
        if let Some((mark_price, funding_index)) = market_prices.get(market_id) {
            let pnl = position.unrealized_pnl(*mark_price);
            unrealized_pnl = unrealized_pnl.add(pnl);

            let funding = position.pending_funding(*funding_index);
            pending_funding = pending_funding.add(funding);

            let notional = position.notional_value(*mark_price);
            total_notional = total_notional.add(notional);

            let margin_req =
                calculate_margin_requirement(position.size, *mark_price, position.leverage, margin_params);
            margin_used = margin_used.add(margin_req.initial);
        }
    }

    let position_collateral: Quote = account.positions.values().map(|p| p.collateral).sum();

    let total_equity = Quote::new(
        account.balance.value() + position_collateral.value() + unrealized_pnl.value()
            - pending_funding.value(),
    );

    let free_margin = Quote::new(total_equity.value() - margin_used.value());

    let margin_ratio = if total_notional.value().is_zero() {
        Decimal::MAX
    } else {
        total_equity.value() / total_notional.value()
    };

    AccountMetrics {
        total_equity,
        unrealized_pnl,
        pending_funding,
        margin_used,
        free_margin,
        margin_ratio,
    }
}

pub fn can_open_position(
    account: &Account,
    collateral_required: Quote,
    market_prices: &HashMap<MarketId, (Price, Decimal)>,
    margin_params: &MarginParams,
) -> bool {
    let metrics = calculate_account_metrics(account, market_prices, margin_params);
    
    // Must have free margin to cover initial margin
    metrics.free_margin.value() >= collateral_required.value()
        && account.balance.value() >= collateral_required.value()
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum AccountError {
    #[error("Insufficient balance: requested {requested}, available {available}")]
    InsufficientBalance { requested: Quote, available: Quote },

    #[error("Insufficient margin: required {required}, available {available}")]
    InsufficientMargin { required: Quote, available: Quote },

    #[error("Position not found for market {0:?}")]
    PositionNotFound(MarketId),

    #[error("Account is liquidatable")]
    Liquidatable,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Leverage, SignedSize};
    use rust_decimal_macros::dec;

    fn test_account() -> Account {
        let mut account = Account::new(AccountId(1), Timestamp::from_millis(0));
        account.deposit(Quote::new(dec!(10000)));
        account
    }

    fn test_position(market_id: MarketId) -> Position {
        Position::new(
            market_id,
            SignedSize::new(dec!(1)),
            Price::new_unchecked(dec!(50000)),
            Quote::new(dec!(5000)),
            Leverage::new(dec!(10)).unwrap(),
            Decimal::ZERO,
            Timestamp::from_millis(0),
        )
    }

    #[test]
    fn account_deposit_withdraw() {
        let mut account = test_account();
        assert_eq!(account.balance.value(), dec!(10000));

        account.deposit(Quote::new(dec!(5000)));
        assert_eq!(account.balance.value(), dec!(15000));

        account.withdraw(Quote::new(dec!(3000))).unwrap();
        assert_eq!(account.balance.value(), dec!(12000));
    }

    #[test]
    fn withdraw_insufficient_balance() {
        let mut account = test_account();
        let result = account.withdraw(Quote::new(dec!(20000)));
        assert!(matches!(result, Err(AccountError::InsufficientBalance { .. })));
    }

    #[test]
    fn position_management() {
        let mut account = test_account();
        let pos = test_position(MarketId(1));

        account.set_position(pos.clone());
        assert!(account.get_position(MarketId(1)).is_some());

        let removed = account.remove_position(MarketId(1));
        assert!(removed.is_some());
        assert!(account.get_position(MarketId(1)).is_none());
    }

    #[test]
    fn account_metrics_no_positions() {
        let account = test_account();
        let market_prices = HashMap::new();
        let params = MarginParams::default();

        let metrics = calculate_account_metrics(&account, &market_prices, &params);

        assert_eq!(metrics.total_equity.value(), dec!(10000));
        assert_eq!(metrics.unrealized_pnl.value(), dec!(0));
        assert_eq!(metrics.margin_used.value(), dec!(0));
        assert_eq!(metrics.free_margin.value(), dec!(10000));
    }

    #[test]
    fn account_metrics_with_position() {
        let mut account = test_account();
        account.reserve_collateral(Quote::new(dec!(5000))).unwrap(); // Reserve for position
        
        let pos = test_position(MarketId(1));
        account.set_position(pos);

        let mut market_prices = HashMap::new();
        // Price went up to 52000, position has $2000 unrealized profit
        market_prices.insert(MarketId(1), (Price::new_unchecked(dec!(52000)), dec!(0)));

        let params = MarginParams::default();
        let metrics = calculate_account_metrics(&account, &market_prices, &params);

        // Balance: 5000, Position collateral: 5000, PnL: 2000
        assert_eq!(metrics.unrealized_pnl.value(), dec!(2000));
        assert_eq!(metrics.total_equity.value(), dec!(12000)); // 5000 + 5000 + 2000
    }

    #[test]
    fn account_metrics_with_funding() {
        let mut account = test_account();
        account.reserve_collateral(Quote::new(dec!(5000))).unwrap();
        
        let pos = test_position(MarketId(1));
        account.set_position(pos);

        let mut market_prices = HashMap::new();
        // Price unchanged, but funding accumulated (100 per unit)
        market_prices.insert(MarketId(1), (Price::new_unchecked(dec!(50000)), dec!(100)));

        let params = MarginParams::default();
        let metrics = calculate_account_metrics(&account, &market_prices, &params);

        // Long position pays funding: 1 * 100 = 100
        assert_eq!(metrics.pending_funding.value(), dec!(100));
        // Equity: 5000 + 5000 + 0 - 100 = 9900
        assert_eq!(metrics.total_equity.value(), dec!(9900));
    }

    #[test]
    fn can_open_new_position() {
        let account = test_account();
        let market_prices = HashMap::new();
        let params = MarginParams::default();

        // Can open with available balance
        assert!(can_open_position(
            &account,
            Quote::new(dec!(5000)),
            &market_prices,
            &params
        ));

        // Cannot open if requires more than balance
        assert!(!can_open_position(
            &account,
            Quote::new(dec!(15000)),
            &market_prices,
            &params
        ));
    }

    #[test]
    fn realize_pnl() {
        let mut account = test_account();

        account.realize_pnl(Quote::new(dec!(1000)));
        assert_eq!(account.balance.value(), dec!(11000));
        assert_eq!(account.realized_pnl.value(), dec!(1000));

        // Loss
        account.realize_pnl(Quote::new(dec!(-500)));
        assert_eq!(account.balance.value(), dec!(10500));
        assert_eq!(account.realized_pnl.value(), dec!(500));
    }

    #[test]
    fn reserve_and_return_collateral() {
        let mut account = test_account();

        account.reserve_collateral(Quote::new(dec!(3000))).unwrap();
        assert_eq!(account.balance.value(), dec!(7000));

        account.return_collateral(Quote::new(dec!(3000)));
        assert_eq!(account.balance.value(), dec!(10000));
    }
}
