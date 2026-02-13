// 9.1 settlement.rs: MOCKED. in-memory, would be blockchain txs in prod.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

use crate::types::AccountId;

// Unique identifier for a settlement batch
pub type BatchId = u64;

// Types of settlements the engine can produce
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SettlementInstruction {
    // Transfer collateral between accounts (internal)
    Transfer {
        from: AccountId,
        to: AccountId,
        amount: Decimal,
        reason: TransferReason,
    },

    // Credit an account (deposit finalized)
    Credit {
        account_id: AccountId,
        amount: Decimal,
        source: String,
    },

    // Debit an account (withdrawal initiated)
    Debit {
        account_id: AccountId,
        amount: Decimal,
        destination: String,
    },

    // Realize PnL from a closed position
    RealizePnl {
        account_id: AccountId,
        pnl: Decimal,
        counterparty: AccountId,
    },

    // Settle funding payment
    FundingPayment {
        payer: AccountId,
        receiver: AccountId,
        amount: Decimal,
    },

    // Liquidation settlement
    Liquidation {
        liquidated: AccountId,
        liquidator: AccountId,
        position_value: Decimal,
        penalty: Decimal,
    },

    // Insurance fund contribution
    InsuranceContribution {
        from: AccountId,
        amount: Decimal,
    },

    // Insurance fund payout (to cover bad debt)
    InsurancePayout {
        to: AccountId,
        amount: Decimal,
    },
}

// Why a transfer is happening
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferReason {
    TradeFee,
    FundingPayment,
    PnlRealization,
    Liquidation,
    Insurance,
    Referral,
}

// Status of a settlement batch
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchStatus {
    Pending,
    Processing,
    Committed,
    Failed,
    Reverted,
}

// A batch of settlement instructions to be executed atomically
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettlementBatch {
    pub batch_id: BatchId,
    pub instructions: Vec<SettlementInstruction>,
    pub status: BatchStatus,
    pub created_at: u64,
    pub processed_at: Option<u64>,
    // Hash or signature for verification
    pub commitment: Option<String>,
}

impl SettlementBatch {
    pub fn new(batch_id: BatchId, created_at: u64) -> Self {
        Self {
            batch_id,
            instructions: Vec::new(),
            status: BatchStatus::Pending,
            created_at,
            processed_at: None,
            commitment: None,
        }
    }

    pub fn add(&mut self, instruction: SettlementInstruction) {
        self.instructions.push(instruction);
    }

    pub fn is_empty(&self) -> bool {
        self.instructions.is_empty()
    }

    pub fn instruction_count(&self) -> usize {
        self.instructions.len()
    }

    // Calculate net flows per account for validation
    pub fn net_flows(&self) -> std::collections::HashMap<AccountId, Decimal> {
        let mut flows = std::collections::HashMap::new();

        for instruction in &self.instructions {
            match instruction {
                SettlementInstruction::Transfer { from, to, amount, .. } => {
                    *flows.entry(*from).or_insert(Decimal::ZERO) -= *amount;
                    *flows.entry(*to).or_insert(Decimal::ZERO) += *amount;
                }
                SettlementInstruction::Credit { account_id, amount, .. } => {
                    *flows.entry(*account_id).or_insert(Decimal::ZERO) += *amount;
                }
                SettlementInstruction::Debit { account_id, amount, .. } => {
                    *flows.entry(*account_id).or_insert(Decimal::ZERO) -= *amount;
                }
                SettlementInstruction::RealizePnl { account_id, pnl, counterparty } => {
                    *flows.entry(*account_id).or_insert(Decimal::ZERO) += *pnl;
                    *flows.entry(*counterparty).or_insert(Decimal::ZERO) -= *pnl;
                }
                SettlementInstruction::FundingPayment { payer, receiver, amount } => {
                    *flows.entry(*payer).or_insert(Decimal::ZERO) -= *amount;
                    *flows.entry(*receiver).or_insert(Decimal::ZERO) += *amount;
                }
                SettlementInstruction::Liquidation { liquidated, liquidator, penalty, .. } => {
                    *flows.entry(*liquidated).or_insert(Decimal::ZERO) -= *penalty;
                    *flows.entry(*liquidator).or_insert(Decimal::ZERO) += *penalty;
                }
                SettlementInstruction::InsuranceContribution { from, amount } => {
                    *flows.entry(*from).or_insert(Decimal::ZERO) -= *amount;
                }
                SettlementInstruction::InsurancePayout { to, amount } => {
                    *flows.entry(*to).or_insert(Decimal::ZERO) += *amount;
                }
            }
        }

        flows
    }
}

// Errors from settlement operations
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettlementError {
    BatchNotFound { batch_id: BatchId },
    BatchAlreadyProcessed { batch_id: BatchId },
    InsufficientFunds { account_id: AccountId, required: Decimal },
    InvalidInstruction { reason: String },
    CommitmentMismatch,
    NetworkError { message: String },
}

// Manages settlement batching and execution
#[derive(Debug)]
pub struct SettlementManager {
    next_batch_id: BatchId,
    // Current batch being built
    current_batch: Option<SettlementBatch>,
    // Pending batches awaiting execution
    pending_batches: VecDeque<SettlementBatch>,
    // Completed batches (for audit trail)
    completed_batches: Vec<SettlementBatch>,
    // Maximum instructions per batch
    max_batch_size: usize,
}

impl SettlementManager {
    pub fn new(max_batch_size: usize) -> Self {
        Self {
            next_batch_id: 1,
            current_batch: None,
            pending_batches: VecDeque::new(),
            completed_batches: Vec::new(),
            max_batch_size,
        }
    }

    // Start a new settlement batch
    pub fn begin_batch(&mut self, timestamp: u64) -> BatchId {
        let batch_id = self.next_batch_id;
        self.next_batch_id += 1;
        self.current_batch = Some(SettlementBatch::new(batch_id, timestamp));
        batch_id
    }

    // Add an instruction to the current batch
    pub fn add_instruction(&mut self, instruction: SettlementInstruction) -> Result<(), SettlementError> {
        let batch = self.current_batch.as_mut()
            .ok_or_else(|| SettlementError::InvalidInstruction {
                reason: "No active batch".to_string(),
            })?;

        if batch.instruction_count() >= self.max_batch_size {
            return Err(SettlementError::InvalidInstruction {
                reason: "Batch is full".to_string(),
            });
        }

        batch.add(instruction);
        Ok(())
    }

    // Finalize the current batch and queue for execution
    pub fn commit_batch(&mut self) -> Result<BatchId, SettlementError> {
        let batch = self.current_batch.take()
            .ok_or_else(|| SettlementError::InvalidInstruction {
                reason: "No active batch".to_string(),
            })?;

        if batch.is_empty() {
            return Err(SettlementError::InvalidInstruction {
                reason: "Cannot commit empty batch".to_string(),
            });
        }

        let batch_id = batch.batch_id;
        self.pending_batches.push_back(batch);
        Ok(batch_id)
    }

    // Abort the current batch without committing
    pub fn abort_batch(&mut self) {
        self.current_batch = None;
    }

    // Get the next pending batch for execution
    pub fn next_pending(&mut self) -> Option<SettlementBatch> {
        self.pending_batches.pop_front()
    }

    // Mark a batch as completed
    pub fn mark_completed(&mut self, mut batch: SettlementBatch, timestamp: u64) {
        batch.status = BatchStatus::Committed;
        batch.processed_at = Some(timestamp);
        self.completed_batches.push(batch);
    }

    // Mark a batch as failed
    pub fn mark_failed(&mut self, mut batch: SettlementBatch) {
        batch.status = BatchStatus::Failed;
        self.completed_batches.push(batch);
    }

    pub fn pending_count(&self) -> usize {
        self.pending_batches.len()
    }

    pub fn completed_count(&self) -> usize {
        self.completed_batches.len()
    }

    pub fn current_batch_size(&self) -> usize {
        self.current_batch.as_ref().map(|b| b.instruction_count()).unwrap_or(0)
    }
}

// Trait for settlement execution backends
pub trait SettlementBackend {
    // Execute a settlement batch
    fn execute(&mut self, batch: &SettlementBatch) -> Result<String, SettlementError>;

    // Check the status of a previously submitted batch
    fn check_status(&self, commitment: &str) -> BatchStatus;

    // Get the backend type identifier
    fn backend_type(&self) -> &str;
}

// In memory settlement backend for testing and simulation
pub struct InMemorySettlement {
    balances: std::collections::HashMap<AccountId, Decimal>,
    executed_batches: Vec<String>,
}

impl InMemorySettlement {
    pub fn new() -> Self {
        Self {
            balances: std::collections::HashMap::new(),
            executed_batches: Vec::new(),
        }
    }

    pub fn set_balance(&mut self, account_id: AccountId, balance: Decimal) {
        self.balances.insert(account_id, balance);
    }

    pub fn get_balance(&self, account_id: AccountId) -> Decimal {
        self.balances.get(&account_id).copied().unwrap_or(Decimal::ZERO)
    }
}

impl Default for InMemorySettlement {
    fn default() -> Self {
        Self::new()
    }
}

impl SettlementBackend for InMemorySettlement {
    fn execute(&mut self, batch: &SettlementBatch) -> Result<String, SettlementError> {
        // validate all instructions first
        let flows = batch.net_flows();
        for (account_id, flow) in &flows {
            if *flow < Decimal::ZERO {
                let current = self.get_balance(*account_id);
                if current + flow < Decimal::ZERO {
                    return Err(SettlementError::InsufficientFunds {
                        account_id: *account_id,
                        required: flow.abs(),
                    });
                }
            }
        }

        // apply all flows
        for (account_id, flow) in flows {
            *self.balances.entry(account_id).or_insert(Decimal::ZERO) += flow;
        }

        let commitment = format!("batch-{}", batch.batch_id);
        self.executed_batches.push(commitment.clone());
        Ok(commitment)
    }

    fn check_status(&self, commitment: &str) -> BatchStatus {
        if self.executed_batches.contains(&commitment.to_string()) {
            BatchStatus::Committed
        } else {
            BatchStatus::Pending
        }
    }

    fn backend_type(&self) -> &str {
        "in_memory"
    }
}

// Settlement configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettlementConfig {
    // Maximum instructions per batch
    pub max_batch_size: usize,
    // How often to flush batches (in seconds)
    pub flush_interval: u64,
    // Minimum batch size before auto flush
    pub min_batch_size: usize,
    // Whether to validate balances before settlement
    pub pre_validate: bool,
}

impl Default for SettlementConfig {
    fn default() -> Self {
        Self {
            max_batch_size: 1000,
            flush_interval: 1,
            min_batch_size: 1,
            pre_validate: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dec(val: i64) -> Decimal {
        Decimal::from(val)
    }

    #[test]
    fn test_batch_creation() {
        let mut manager = SettlementManager::new(100);

        let batch_id = manager.begin_batch(1000);
        assert_eq!(batch_id, 1);
        assert_eq!(manager.current_batch_size(), 0);

        manager.add_instruction(SettlementInstruction::Transfer {
            from: AccountId(1),
            to: AccountId(2),
            amount: dec(100),
            reason: TransferReason::TradeFee,
        }).unwrap();

        assert_eq!(manager.current_batch_size(), 1);
    }

    #[test]
    fn test_batch_commit() {
        let mut manager = SettlementManager::new(100);

        manager.begin_batch(1000);
        manager.add_instruction(SettlementInstruction::Credit {
            account_id: AccountId(1),
            amount: dec(1000),
            source: "deposit".to_string(),
        }).unwrap();

        let batch_id = manager.commit_batch().unwrap();
        assert_eq!(manager.pending_count(), 1);
        assert_eq!(batch_id, 1);
    }

    #[test]
    fn test_empty_batch_commit_fails() {
        let mut manager = SettlementManager::new(100);
        manager.begin_batch(1000);

        let result = manager.commit_batch();
        assert!(matches!(result, Err(SettlementError::InvalidInstruction { .. })));
    }

    #[test]
    fn test_batch_net_flows() {
        let mut batch = SettlementBatch::new(1, 1000);

        batch.add(SettlementInstruction::Transfer {
            from: AccountId(1),
            to: AccountId(2),
            amount: dec(100),
            reason: TransferReason::TradeFee,
        });
        batch.add(SettlementInstruction::Transfer {
            from: AccountId(2),
            to: AccountId(3),
            amount: dec(50),
            reason: TransferReason::TradeFee,
        });

        let flows = batch.net_flows();
        assert_eq!(flows.get(&AccountId(1)), Some(&dec(-100)));
        assert_eq!(flows.get(&AccountId(2)), Some(&dec(50)));
        assert_eq!(flows.get(&AccountId(3)), Some(&dec(50)));
    }

    #[test]
    fn test_in_memory_settlement() {
        let mut backend = InMemorySettlement::new();
        backend.set_balance(AccountId(1), dec(1000));
        backend.set_balance(AccountId(2), dec(500));

        let mut batch = SettlementBatch::new(1, 1000);
        batch.add(SettlementInstruction::Transfer {
            from: AccountId(1),
            to: AccountId(2),
            amount: dec(200),
            reason: TransferReason::PnlRealization,
        });

        let commitment = backend.execute(&batch).unwrap();
        assert_eq!(backend.get_balance(AccountId(1)), dec(800));
        assert_eq!(backend.get_balance(AccountId(2)), dec(700));
        assert_eq!(backend.check_status(&commitment), BatchStatus::Committed);
    }

    #[test]
    fn test_insufficient_funds() {
        let mut backend = InMemorySettlement::new();
        backend.set_balance(AccountId(1), dec(100));

        let mut batch = SettlementBatch::new(1, 1000);
        batch.add(SettlementInstruction::Debit {
            account_id: AccountId(1),
            amount: dec(200), // more than balance
            destination: "external".to_string(),
        });

        let result = backend.execute(&batch);
        assert!(matches!(result, Err(SettlementError::InsufficientFunds { .. })));
    }

    #[test]
    fn test_funding_settlement() {
        let mut backend = InMemorySettlement::new();
        backend.set_balance(AccountId(1), dec(1000)); // long
        backend.set_balance(AccountId(2), dec(1000)); // short

        let mut batch = SettlementBatch::new(1, 1000);
        batch.add(SettlementInstruction::FundingPayment {
            payer: AccountId(1),
            receiver: AccountId(2),
            amount: dec(10),
        });

        backend.execute(&batch).unwrap();
        assert_eq!(backend.get_balance(AccountId(1)), dec(990));
        assert_eq!(backend.get_balance(AccountId(2)), dec(1010));
    }

    #[test]
    fn test_liquidation_settlement() {
        let mut backend = InMemorySettlement::new();
        backend.set_balance(AccountId(1), dec(100)); // liquidated
        backend.set_balance(AccountId(2), dec(5000)); // liquidator

        let mut batch = SettlementBatch::new(1, 1000);
        batch.add(SettlementInstruction::Liquidation {
            liquidated: AccountId(1),
            liquidator: AccountId(2),
            position_value: dec(1000),
            penalty: dec(50),
        });

        backend.execute(&batch).unwrap();
        assert_eq!(backend.get_balance(AccountId(1)), dec(50)); // lost penalty
        assert_eq!(backend.get_balance(AccountId(2)), dec(5050)); // gained penalty
    }

    #[test]
    fn test_manager_workflow() {
        let mut manager = SettlementManager::new(100);
        let mut backend = InMemorySettlement::new();
        backend.set_balance(AccountId(1), dec(1000));

        // build batch
        manager.begin_batch(1000);
        manager.add_instruction(SettlementInstruction::Debit {
            account_id: AccountId(1),
            amount: dec(100),
            destination: "0xabc".to_string(),
        }).unwrap();
        manager.commit_batch().unwrap();

        // execute
        let batch = manager.next_pending().unwrap();
        let commitment = backend.execute(&batch).unwrap();
        assert!(!commitment.is_empty());

        manager.mark_completed(batch, 1001);
        assert_eq!(manager.completed_count(), 1);
        assert_eq!(manager.pending_count(), 0);
    }
}
