// 9.2 custody.rs: MOCKED. just balance changes, no real token transfers.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::types::AccountId;

// Unique identifier for a deposit/withdrawal transaction
pub type TxId = String;

// Supported collateral types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CollateralType {
    Usd,
    Usdc,
    Usdt,
    Btc,
    Eth,
}

impl CollateralType {
    pub fn decimals(&self) -> u32 {
        match self {
            CollateralType::Usd => 2,
            CollateralType::Usdc => 6,
            CollateralType::Usdt => 6,
            CollateralType::Btc => 8,
            CollateralType::Eth => 18,
        }
    }

    pub fn symbol(&self) -> &'static str {
        match self {
            CollateralType::Usd => "USD",
            CollateralType::Usdc => "USDC",
            CollateralType::Usdt => "USDT",
            CollateralType::Btc => "BTC",
            CollateralType::Eth => "ETH",
        }
    }
}

// Status of a deposit or withdrawal
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferStatus {
    Pending,
    Confirming,
    Confirmed,
    Failed,
    Cancelled,
}

// A deposit request from external source to the engine
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepositRequest {
    pub tx_id: TxId,
    pub account_id: AccountId,
    pub collateral_type: CollateralType,
    pub amount: Decimal,
    pub source_address: Option<String>,
    pub status: TransferStatus,
    pub created_at: u64,
    pub confirmed_at: Option<u64>,
    // Number of blockchain confirmations (for on chain deposits)
    pub confirmations: u32,
    pub required_confirmations: u32,
}

impl DepositRequest {
    pub fn new(
        tx_id: TxId,
        account_id: AccountId,
        collateral_type: CollateralType,
        amount: Decimal,
        created_at: u64,
    ) -> Self {
        Self {
            tx_id,
            account_id,
            collateral_type,
            amount,
            source_address: None,
            status: TransferStatus::Pending,
            created_at,
            confirmed_at: None,
            confirmations: 0,
            required_confirmations: 1,
        }
    }

    pub fn with_source(mut self, address: String) -> Self {
        self.source_address = Some(address);
        self
    }

    pub fn with_confirmations(mut self, required: u32) -> Self {
        self.required_confirmations = required;
        self
    }

    pub fn is_confirmed(&self) -> bool {
        self.status == TransferStatus::Confirmed
    }

    pub fn add_confirmation(&mut self) {
        self.confirmations += 1;
        if self.confirmations >= self.required_confirmations {
            self.status = TransferStatus::Confirmed;
        } else {
            self.status = TransferStatus::Confirming;
        }
    }
}

// A withdrawal request from the engine to external destination
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WithdrawalRequest {
    pub tx_id: TxId,
    pub account_id: AccountId,
    pub collateral_type: CollateralType,
    pub amount: Decimal,
    pub destination_address: String,
    pub status: TransferStatus,
    pub created_at: u64,
    pub processed_at: Option<u64>,
    // Fee charged for the withdrawal
    pub fee: Decimal,
}

impl WithdrawalRequest {
    pub fn new(
        tx_id: TxId,
        account_id: AccountId,
        collateral_type: CollateralType,
        amount: Decimal,
        destination: String,
        created_at: u64,
    ) -> Self {
        Self {
            tx_id,
            account_id,
            collateral_type,
            amount,
            destination_address: destination,
            status: TransferStatus::Pending,
            created_at,
            processed_at: None,
            fee: Decimal::ZERO,
        }
    }

    pub fn with_fee(mut self, fee: Decimal) -> Self {
        self.fee = fee;
        self
    }

    pub fn net_amount(&self) -> Decimal {
        self.amount - self.fee
    }
}

// Errors from custody operations
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CustodyError {
    InsufficientBalance { available: Decimal, requested: Decimal },
    AccountNotFound { account_id: AccountId },
    TransferNotFound { tx_id: TxId },
    TransferAlreadyProcessed { tx_id: TxId },
    WithdrawalLocked { reason: String },
    InvalidAmount,
    UnsupportedCollateral { collateral_type: CollateralType },
}

// Configuration for custody operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustodyConfig {
    // Minimum deposit amount per collateral type
    pub min_deposits: HashMap<CollateralType, Decimal>,
    // Maximum withdrawal per request
    pub max_withdrawal: Decimal,
    // Withdrawal fee (flat)
    pub withdrawal_fee: Decimal,
    // Required confirmations per collateral type
    pub confirmation_requirements: HashMap<CollateralType, u32>,
    // Cooldown period after deposit before withdrawal (seconds)
    pub withdrawal_cooldown: u64,
}

impl Default for CustodyConfig {
    fn default() -> Self {
        let mut min_deposits = HashMap::new();
        min_deposits.insert(CollateralType::Usd, Decimal::new(10, 0));
        min_deposits.insert(CollateralType::Usdc, Decimal::new(10, 0));

        let mut confirmations = HashMap::new();
        confirmations.insert(CollateralType::Btc, 3);
        confirmations.insert(CollateralType::Eth, 12);
        confirmations.insert(CollateralType::Usdc, 12);

        Self {
            min_deposits,
            max_withdrawal: Decimal::new(1_000_000, 0),
            withdrawal_fee: Decimal::new(1, 0), // $1
            confirmation_requirements: confirmations,
            withdrawal_cooldown: 0,
        }
    }
}

// Manages custody operations for the engine.
// This is the bridge between external funds and internal engine balances.
#[derive(Debug)]
pub struct CustodyManager {
    config: CustodyConfig,
    // Pending deposits by tx_id
    pending_deposits: HashMap<TxId, DepositRequest>,
    // Pending withdrawals by tx_id
    pending_withdrawals: HashMap<TxId, WithdrawalRequest>,
    // Last deposit time per account (for cooldown)
    last_deposit: HashMap<AccountId, u64>,
    // Total deposits processed
    total_deposited: Decimal,
    // Total withdrawals processed
    total_withdrawn: Decimal,
}

impl CustodyManager {
    pub fn new(config: CustodyConfig) -> Self {
        Self {
            config,
            pending_deposits: HashMap::new(),
            pending_withdrawals: HashMap::new(),
            last_deposit: HashMap::new(),
            total_deposited: Decimal::ZERO,
            total_withdrawn: Decimal::ZERO,
        }
    }

    // Initiate a deposit (called when funds arrive)
    pub fn initiate_deposit(&mut self, request: DepositRequest) -> Result<(), CustodyError> {
        // check minimum deposit
        if let Some(min) = self.config.min_deposits.get(&request.collateral_type) {
            if request.amount < *min {
                return Err(CustodyError::InvalidAmount);
            }
        }

        self.pending_deposits.insert(request.tx_id.clone(), request);
        Ok(())
    }

    // Confirm a deposit (called after sufficient confirmations)
    pub fn confirm_deposit(&mut self, tx_id: &TxId, timestamp: u64) -> Result<DepositRequest, CustodyError> {
        let deposit = self.pending_deposits.remove(tx_id)
            .ok_or_else(|| CustodyError::TransferNotFound { tx_id: tx_id.clone() })?;

        if deposit.is_confirmed() {
            return Err(CustodyError::TransferAlreadyProcessed { tx_id: tx_id.clone() });
        }

        let mut confirmed = deposit;
        confirmed.status = TransferStatus::Confirmed;
        confirmed.confirmed_at = Some(timestamp);

        self.last_deposit.insert(confirmed.account_id, timestamp);
        self.total_deposited += confirmed.amount;

        Ok(confirmed)
    }

    // Request a withdrawal
    pub fn request_withdrawal(
        &mut self,
        account_id: AccountId,
        collateral_type: CollateralType,
        amount: Decimal,
        destination: String,
        current_time: u64,
        available_balance: Decimal,
    ) -> Result<WithdrawalRequest, CustodyError> {
        // check balance
        if amount > available_balance {
            return Err(CustodyError::InsufficientBalance {
                available: available_balance,
                requested: amount,
            });
        }

        // check max withdrawal
        if amount > self.config.max_withdrawal {
            return Err(CustodyError::InvalidAmount);
        }

        // check cooldown
        if let Some(last) = self.last_deposit.get(&account_id) {
            if current_time < last + self.config.withdrawal_cooldown {
                return Err(CustodyError::WithdrawalLocked {
                    reason: "Withdrawal cooldown active".to_string(),
                });
            }
        }

        let tx_id = format!("wd-{}-{}", account_id.0, current_time);
        let mut request = WithdrawalRequest::new(
            tx_id.clone(),
            account_id,
            collateral_type,
            amount,
            destination,
            current_time,
        );
        request.fee = self.config.withdrawal_fee;

        self.pending_withdrawals.insert(tx_id, request.clone());
        Ok(request)
    }

    // Process a pending withdrawal (called by settlement layer)
    pub fn process_withdrawal(&mut self, tx_id: &TxId, timestamp: u64) -> Result<WithdrawalRequest, CustodyError> {
        let withdrawal = self.pending_withdrawals.remove(tx_id)
            .ok_or_else(|| CustodyError::TransferNotFound { tx_id: tx_id.clone() })?;

        let mut processed = withdrawal;
        processed.status = TransferStatus::Confirmed;
        processed.processed_at = Some(timestamp);

        self.total_withdrawn += processed.net_amount();

        Ok(processed)
    }

    // Cancel a pending withdrawal
    pub fn cancel_withdrawal(&mut self, tx_id: &TxId) -> Result<WithdrawalRequest, CustodyError> {
        let mut withdrawal = self.pending_withdrawals.remove(tx_id)
            .ok_or_else(|| CustodyError::TransferNotFound { tx_id: tx_id.clone() })?;

        withdrawal.status = TransferStatus::Cancelled;
        Ok(withdrawal)
    }

    // Get pending deposits for an account
    pub fn pending_deposits_for(&self, account_id: AccountId) -> Vec<&DepositRequest> {
        self.pending_deposits.values()
            .filter(|d| d.account_id == account_id)
            .collect()
    }

    // Get pending withdrawals for an account
    pub fn pending_withdrawals_for(&self, account_id: AccountId) -> Vec<&WithdrawalRequest> {
        self.pending_withdrawals.values()
            .filter(|w| w.account_id == account_id)
            .collect()
    }

    pub fn total_deposited(&self) -> Decimal {
        self.total_deposited
    }

    pub fn total_withdrawn(&self) -> Decimal {
        self.total_withdrawn
    }

    pub fn pending_deposit_count(&self) -> usize {
        self.pending_deposits.len()
    }

    pub fn pending_withdrawal_count(&self) -> usize {
        self.pending_withdrawals.len()
    }
}

// Trait for settlement adapters. Implement this for different chains/systems.
pub trait SettlementAdapter {
    // Check if a deposit transaction exists and get its confirmation count
    fn check_deposit(&self, tx_id: &TxId) -> Option<(Decimal, u32)>;

    // Submit a withdrawal transaction
    fn submit_withdrawal(&mut self, request: &WithdrawalRequest) -> Result<TxId, CustodyError>;

    // Check withdrawal transaction status
    fn check_withdrawal(&self, tx_id: &TxId) -> Option<TransferStatus>;
}

// Mock settlement for testing
pub struct MockSettlement {
    deposits: HashMap<TxId, (Decimal, u32)>,
    withdrawals: HashMap<TxId, TransferStatus>,
}

impl MockSettlement {
    pub fn new() -> Self {
        Self {
            deposits: HashMap::new(),
            withdrawals: HashMap::new(),
        }
    }

    pub fn add_deposit(&mut self, tx_id: TxId, amount: Decimal, confirmations: u32) {
        self.deposits.insert(tx_id, (amount, confirmations));
    }

    pub fn confirm_withdrawal(&mut self, tx_id: &TxId) {
        self.withdrawals.insert(tx_id.clone(), TransferStatus::Confirmed);
    }
}

impl Default for MockSettlement {
    fn default() -> Self {
        Self::new()
    }
}

impl SettlementAdapter for MockSettlement {
    fn check_deposit(&self, tx_id: &TxId) -> Option<(Decimal, u32)> {
        self.deposits.get(tx_id).copied()
    }

    fn submit_withdrawal(&mut self, request: &WithdrawalRequest) -> Result<TxId, CustodyError> {
        self.withdrawals.insert(request.tx_id.clone(), TransferStatus::Pending);
        Ok(request.tx_id.clone())
    }

    fn check_withdrawal(&self, tx_id: &TxId) -> Option<TransferStatus> {
        self.withdrawals.get(tx_id).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dec(val: i64) -> Decimal {
        Decimal::from(val)
    }

    #[test]
    fn test_deposit_flow() {
        let config = CustodyConfig::default();
        let mut custody = CustodyManager::new(config);

        let deposit = DepositRequest::new(
            "tx123".to_string(),
            AccountId(1),
            CollateralType::Usdc,
            dec(1000),
            1000,
        );

        custody.initiate_deposit(deposit).unwrap();
        assert_eq!(custody.pending_deposit_count(), 1);

        let confirmed = custody.confirm_deposit(&"tx123".to_string(), 1001).unwrap();
        assert!(confirmed.is_confirmed());
        assert_eq!(custody.total_deposited(), dec(1000));
        assert_eq!(custody.pending_deposit_count(), 0);
    }

    #[test]
    fn test_withdrawal_flow() {
        let config = CustodyConfig::default();
        let mut custody = CustodyManager::new(config);

        let request = custody.request_withdrawal(
            AccountId(1),
            CollateralType::Usdc,
            dec(500),
            "0xabc123".to_string(),
            1000,
            dec(1000),
        ).unwrap();

        assert_eq!(request.amount, dec(500));
        assert_eq!(request.fee, dec(1)); // default fee
        assert_eq!(request.net_amount(), dec(499));
        assert_eq!(custody.pending_withdrawal_count(), 1);

        let processed = custody.process_withdrawal(&request.tx_id, 1001).unwrap();
        assert_eq!(processed.status, TransferStatus::Confirmed);
        assert_eq!(custody.total_withdrawn(), dec(499));
    }

    #[test]
    fn test_insufficient_balance() {
        let config = CustodyConfig::default();
        let mut custody = CustodyManager::new(config);

        let result = custody.request_withdrawal(
            AccountId(1),
            CollateralType::Usdc,
            dec(500),
            "0xabc".to_string(),
            1000,
            dec(100),
        );

        assert!(matches!(result, Err(CustodyError::InsufficientBalance { .. })));
    }

    #[test]
    fn test_min_deposit() {
        let mut config = CustodyConfig::default();
        config.min_deposits.insert(CollateralType::Usdc, dec(100));
        let mut custody = CustodyManager::new(config);

        let small = DepositRequest::new(
            "small".to_string(),
            AccountId(1),
            CollateralType::Usdc,
            dec(50),
            1000,
        );

        let result = custody.initiate_deposit(small);
        assert!(matches!(result, Err(CustodyError::InvalidAmount)));
    }

    #[test]
    fn test_withdrawal_cooldown() {
        let mut config = CustodyConfig::default();
        config.withdrawal_cooldown = 3600;
        let mut custody = CustodyManager::new(config);

        let deposit = DepositRequest::new(
            "dep1".to_string(),
            AccountId(1),
            CollateralType::Usdc,
            dec(1000),
            1000,
        );
        custody.initiate_deposit(deposit).unwrap();
        custody.confirm_deposit(&"dep1".to_string(), 1001).unwrap();

        let result = custody.request_withdrawal(
            AccountId(1),
            CollateralType::Usdc,
            dec(100),
            "0xabc".to_string(),
            1500,
            dec(1000),
        );

        assert!(matches!(result, Err(CustodyError::WithdrawalLocked { .. })));

        let result = custody.request_withdrawal(
            AccountId(1),
            CollateralType::Usdc,
            dec(100),
            "0xabc".to_string(),
            5000,
            dec(1000),
        );

        assert!(result.is_ok());
    }

    #[test]
    fn test_cancel_withdrawal() {
        let config = CustodyConfig::default();
        let mut custody = CustodyManager::new(config);

        let request = custody.request_withdrawal(
            AccountId(1),
            CollateralType::Usdc,
            dec(100),
            "0xabc".to_string(),
            1000,
            dec(500),
        ).unwrap();

        let cancelled = custody.cancel_withdrawal(&request.tx_id).unwrap();
        assert_eq!(cancelled.status, TransferStatus::Cancelled);
        assert_eq!(custody.pending_withdrawal_count(), 0);
    }

    #[test]
    fn test_collateral_type_properties() {
        assert_eq!(CollateralType::Usdc.decimals(), 6);
        assert_eq!(CollateralType::Btc.symbol(), "BTC");
        assert_eq!(CollateralType::Eth.decimals(), 18);
    }

    #[test]
    fn test_mock_settlement() {
        let mut settlement = MockSettlement::new();

        settlement.add_deposit("tx1".to_string(), dec(1000), 12);
        let (amount, confs) = settlement.check_deposit(&"tx1".to_string()).unwrap();
        assert_eq!(amount, dec(1000));
        assert_eq!(confs, 12);

        let request = WithdrawalRequest::new(
            "wd1".to_string(),
            AccountId(1),
            CollateralType::Usdc,
            dec(500),
            "0xabc".to_string(),
            1000,
        );
        settlement.submit_withdrawal(&request).unwrap();
        assert_eq!(settlement.check_withdrawal(&"wd1".to_string()), Some(TransferStatus::Pending));

        settlement.confirm_withdrawal(&"wd1".to_string());
        assert_eq!(settlement.check_withdrawal(&"wd1".to_string()), Some(TransferStatus::Confirmed));
    }
}
