// 8.8: funding rate settlement. LP fee split: payers pay full, receivers get (1 - lp_fee_fraction).

use super::core::Engine;
use super::results::{EngineError, FundingResult};
use crate::events::{EventPayload, FundingFeeCollectedEvent, FundingSettledEvent, OiUpdatedEvent};
use crate::funding::{calculate_funding_payment, calculate_funding_rate, calculate_premium_index};
use crate::types::{AccountId, MarketId, Quote, SignedSize};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

impl Engine {
    // settle funding for a market. paying side pays full rate,
    // receiving side gets (1 - lp_fee_fraction). remainder accrues to pool.
    pub fn settle_funding(&mut self, market_id: MarketId) -> Result<FundingResult, EngineError> {
        let market = self
            .markets
            .get(&market_id)
            .ok_or(EngineError::MarketNotFound(market_id))?;

        let Some(mark_price) = market.mark_price else {
            return Err(EngineError::NoMarkPrice(market_id));
        };

        let Some(index_price) = market.index_price else {
            return Err(EngineError::NoIndexPrice(market_id));
        };

        let premium = calculate_premium_index(mark_price, index_price);
        let funding_rate = calculate_funding_rate(premium, &market.config.funding_params);

        let elapsed_ms = self.current_time.as_millis() - market.funding_state.last_update.as_millis();
        let funding_period_ms = (market.config.funding_params.period_hours * rust_decimal_macros::dec!(3600000))
            .to_i64()
            .unwrap_or(8 * 3600000);
        let time_fraction = Decimal::from(elapsed_ms) / Decimal::from(funding_period_ms);
        let prorated_rate = funding_rate * time_fraction;
        let lp_fee_fraction = market.config.funding_params.lp_fee_fraction;

        let mut total_long_payments = Decimal::ZERO;
        let mut total_short_payments = Decimal::ZERO;
        let mut account_payments: Vec<(AccountId, Quote, SignedSize)> = Vec::new();

        // First pass: compute gross payments per account
        for (account_id, account) in &self.accounts {
            if let Some(position) = account.get_position(market_id) {
                let payment = calculate_funding_payment(position.size, mark_price, prorated_rate);

                if position.size.is_long() {
                    total_long_payments += payment.value();
                } else {
                    total_short_payments += payment.value();
                }

                account_payments.push((*account_id, payment, position.size));
            }
        }

        // Determine the LP fee pool cut from the gross receiving side.
        // Whichever side has negative total (receiving money), we skim lp_fee_fraction.
        let gross_receiver_total = if total_long_payments > Decimal::ZERO {
            // Longs are paying, shorts are receiving
            total_short_payments.abs()
        } else {
            total_long_payments.abs()
        };
        let lp_fee_amount = gross_receiver_total * lp_fee_fraction;

        // Second pass: apply payments. Payers pay full. Receivers get (1 - lp_fee_fraction).
        for (account_id, payment, position_size) in &account_payments {
            let is_receiver = payment.value() < Decimal::ZERO; // negative payment = receiving
            let adjusted_payment = if is_receiver {
                // Receiver gets less: they receive (1 - lp_fee_fraction) of gross
                Quote::new(payment.value() * (Decimal::ONE - lp_fee_fraction))
            } else {
                *payment // payer pays full amount
            };

            let account = self.accounts.get_mut(account_id).unwrap();
            let new_balance = account.balance.value() - adjusted_payment.value();
            account.balance = Quote::new(new_balance.max(Decimal::ZERO));

            if let Some(position) = account.get_position_mut(market_id) {
                let market = self.markets.get(&market_id).unwrap();
                position.entry_funding_index = market.funding_state.cumulative_funding;
            }

            self.emit_event(EventPayload::FundingSettled(FundingSettledEvent {
                market_id,
                account_id: *account_id,
                payment: adjusted_payment,
                funding_rate: prorated_rate,
                position_size: *position_size,
            }));
        }

        // Accrue LP fee to market state
        let market = self.markets.get_mut(&market_id).unwrap();
        market.pool_funding_fees += lp_fee_amount;
        market.funding_state.last_update = self.current_time;
        market.funding_state.cumulative_funding += prorated_rate;
        market.funding_state.current_rate = prorated_rate;

        if lp_fee_amount > Decimal::ZERO {
            self.emit_event(EventPayload::FundingFeeCollected(FundingFeeCollectedEvent {
                market_id,
                lp_fee_amount: Quote::new(lp_fee_amount),
                funding_rate: prorated_rate,
            }));
        }

        // Emit OI snapshot after funding
        let market = self.markets.get(&market_id).unwrap();
        let long_oi = market.open_interest_long;
        let short_oi = market.open_interest_short;
        self.emit_event(EventPayload::OiUpdated(OiUpdatedEvent {
            market_id,
            long_oi,
            short_oi,
            total_oi: long_oi.max(short_oi),
        }));

        Ok(FundingResult {
            funding_rate: prorated_rate,
            total_long_payments: Quote::new(total_long_payments),
            total_short_payments: Quote::new(total_short_payments),
            lp_fee_collected: Quote::new(lp_fee_amount),
            accounts_affected: account_payments.len(),
        })
    }
}
