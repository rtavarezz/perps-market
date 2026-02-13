// 6.2: auto-deleveraging. when insurance fund is empty, profitable traders get force-closed.
// ranked by pnl * leverage score: highest score gets deleveraged first.

use crate::position::Position;
use crate::types::{AccountId, MarketId, Price, Quote, Side};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdlParams {
    pub min_trigger_amount: Quote,      // min bad debt to trigger ADL
    pub max_accounts_per_round: usize,  // cap per ADL round
}

impl Default for AdlParams {
    fn default() -> Self {
        Self {
            min_trigger_amount: Quote::new(dec!(100)),
            max_accounts_per_round: 50,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AdlCandidate {
    pub account_id: AccountId,
    pub position: Position,
    pub score: Decimal,          // higher = deleveraged first
    pub unrealized_pnl: Quote,
}

impl AdlCandidate {
    pub fn new(account_id: AccountId, position: Position, mark_price: Price) -> Self {
        let unrealized_pnl = position.unrealized_pnl(mark_price);
        let score = calculate_adl_score(&position, unrealized_pnl);

        Self {
            account_id,
            position,
            score,
            unrealized_pnl,
        }
    }
}

impl PartialEq for AdlCandidate {
    fn eq(&self, other: &Self) -> bool {
        self.score == other.score && self.account_id == other.account_id
    }
}

impl Eq for AdlCandidate {}

impl PartialOrd for AdlCandidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for AdlCandidate {
    fn cmp(&self, other: &Self) -> Ordering {
        // Higher score = higher priority for deleveraging
        // Descending order by score, then by account ID for stability
        other
            .score
            .cmp(&self.score)
            .then(self.account_id.0.cmp(&other.account_id.0))
    }
}

// score = pnl_ratio * leverage. profitable high-leverage positions get deleveraged first.
fn calculate_adl_score(position: &Position, unrealized_pnl: Quote) -> Decimal {
    let pnl_ratio = if position.collateral.value().is_zero() {
        Decimal::ZERO
    } else {
        unrealized_pnl.value() / position.collateral.value()
    };

    let leverage = position.leverage.value();

    // Score = PnL ratio * leverage. Profitable high leverage positions score highest.
    pnl_ratio * leverage
}

#[derive(Debug, Clone)]
pub struct AdlResult {
    pub market_id: MarketId,
    pub bankrupt_account: AccountId,
    pub bad_debt: Quote,
    pub deleveraged: Vec<AdlExecution>,
    pub remaining_debt: Quote,  // should be zero if successful
}

#[derive(Debug, Clone)]
pub struct AdlExecution {
    pub account_id: AccountId,
    pub size_reduced: Decimal,
    pub price: Price,
    pub realized_pnl: Quote,
}

// builds ranked list of ADL candidates. sorted by priority, highest first.
pub fn rank_adl_candidates(
    positions: Vec<(AccountId, Position)>,
    target_side: Side,
    mark_price: Price,
) -> Vec<AdlCandidate> {
    let mut candidates: Vec<AdlCandidate> = positions
        .into_iter()
        .filter(|(_, pos)| pos.side() == Some(target_side))
        .filter(|(_, pos)| !pos.size.is_zero())
        .map(|(id, pos)| AdlCandidate::new(id, pos, mark_price))
        .filter(|c| c.unrealized_pnl.value() > Decimal::ZERO)
        .collect();

    candidates.sort();
    candidates
}

// determines how much to close from each candidate to cover the bad debt
pub fn calculate_adl_sizes(
    candidates: &[AdlCandidate],
    bad_debt: Quote,
    mark_price: Price,
    params: &AdlParams,
) -> Vec<(AccountId, Decimal)> {
    let mut remaining_debt = bad_debt.value();
    let mut results = Vec::new();

    for candidate in candidates.iter().take(params.max_accounts_per_round) {
        if remaining_debt <= Decimal::ZERO {
            break;
        }

        // Only deleverage up to the candidate's profit
        let max_coverage = candidate.unrealized_pnl.value().min(remaining_debt);

        if max_coverage <= Decimal::ZERO {
            continue;
        }

        // Calculate size needed to realize this amount of PnL
        let size_to_close = if mark_price.value().is_zero() {
            Decimal::ZERO
        } else {
            max_coverage / mark_price.value()
        };

        let actual_size = size_to_close.min(candidate.position.size.abs());

        if actual_size > Decimal::ZERO {
            results.push((candidate.account_id, actual_size));
            remaining_debt -= max_coverage;
        }
    }

    results
}

pub fn should_trigger_adl(uncovered_debt: Quote, params: &AdlParams) -> bool {
    uncovered_debt.value() >= params.min_trigger_amount.value()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Leverage, MarketId, SignedSize, Timestamp};
    use rust_decimal_macros::dec;

    fn create_position(
        size: Decimal,
        entry: Decimal,
        collateral: Decimal,
        leverage: Decimal,
    ) -> Position {
        Position::new(
            MarketId(1),
            SignedSize::new(size),
            Price::new_unchecked(entry),
            Quote::new(collateral),
            Leverage::new(leverage).unwrap(),
            Decimal::ZERO,
            Timestamp::from_millis(0),
        )
    }

    #[test]
    fn adl_score_calculation() {
        // Profitable position at 10x: high score
        let pos = create_position(dec!(1), dec!(50000), dec!(5000), dec!(10));
        let mark = Price::new_unchecked(dec!(55000));
        let pnl = pos.unrealized_pnl(mark);
        let score = calculate_adl_score(&pos, pnl);

        assert!(score > Decimal::ZERO);

        // Losing position: negative or zero score
        let mark_down = Price::new_unchecked(dec!(45000));
        let pnl_loss = pos.unrealized_pnl(mark_down);
        let score_loss = calculate_adl_score(&pos, pnl_loss);

        assert!(score_loss < Decimal::ZERO);
    }

    #[test]
    fn adl_ranking_prioritizes_profitable_high_leverage() {
        let pos_low_lev = create_position(dec!(1), dec!(50000), dec!(25000), dec!(2));
        let pos_high_lev = create_position(dec!(1), dec!(50000), dec!(5000), dec!(10));

        let mark = Price::new_unchecked(dec!(55000)); // 10% profit

        let positions = vec![
            (AccountId(1), pos_low_lev),
            (AccountId(2), pos_high_lev),
        ];

        let ranked = rank_adl_candidates(positions, Side::Long, mark);

        assert_eq!(ranked.len(), 2);
        // High leverage profitable position should be first
        assert_eq!(ranked[0].account_id, AccountId(2));
    }

    #[test]
    fn adl_excludes_losing_positions() {
        let pos = create_position(dec!(1), dec!(50000), dec!(5000), dec!(10));
        let mark = Price::new_unchecked(dec!(45000)); // Losing

        let positions = vec![(AccountId(1), pos)];
        let ranked = rank_adl_candidates(positions, Side::Long, mark);

        assert!(ranked.is_empty());
    }

    #[test]
    fn adl_size_calculation() {
        let pos = create_position(dec!(1), dec!(50000), dec!(5000), dec!(10));
        let mark = Price::new_unchecked(dec!(55000)); // $5000 profit

        let candidate = AdlCandidate::new(AccountId(1), pos, mark);
        let candidates = vec![candidate];

        let params = AdlParams::default();

        // Need to cover $2000 bad debt
        let bad_debt = Quote::new(dec!(2000));
        let sizes = calculate_adl_sizes(&candidates, bad_debt, mark, &params);

        assert_eq!(sizes.len(), 1);
        // Should only reduce enough to cover debt
        let (account, size) = sizes[0];
        assert_eq!(account, AccountId(1));
        assert!(size < dec!(1)); // Less than full position
    }

    #[test]
    fn adl_trigger_threshold() {
        let params = AdlParams::default();

        assert!(!should_trigger_adl(Quote::new(dec!(50)), &params));
        assert!(should_trigger_adl(Quote::new(dec!(100)), &params));
        assert!(should_trigger_adl(Quote::new(dec!(500)), &params));
    }

    #[test]
    fn adl_respects_max_accounts() {
        let mut positions = Vec::new();
        for i in 1..=100 {
            let pos = create_position(dec!(1), dec!(50000), dec!(5000), dec!(10));
            positions.push((AccountId(i), pos));
        }

        let mark = Price::new_unchecked(dec!(55000));
        let ranked = rank_adl_candidates(positions, Side::Long, mark);

        let params = AdlParams {
            max_accounts_per_round: 10,
            ..Default::default()
        };

        // Very large debt to force hitting all candidates
        let bad_debt = Quote::new(dec!(1_000_000));
        let sizes = calculate_adl_sizes(&ranked, bad_debt, mark, &params);

        // Should stop at max_accounts_per_round
        assert!(sizes.len() <= params.max_accounts_per_round);
    }
}
