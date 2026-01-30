//! Price update operations.

use super::core::Engine;
use super::results::EngineError;
use crate::events::{EventPayload, IndexPriceUpdateEvent, MarkPriceUpdateEvent};
use crate::mark_price::{update_mark_price, MarkPriceState};
use crate::types::{MarketId, Price};

impl Engine {
    /// Update the index price from oracle.
    pub fn update_index_price(
        &mut self,
        market_id: MarketId,
        index_price: Price,
    ) -> Result<(), EngineError> {
        let market = self
            .markets
            .get_mut(&market_id)
            .ok_or(EngineError::MarketNotFound(market_id))?;

        market.index_price = Some(index_price);

        self.emit_event(EventPayload::IndexPriceUpdate(IndexPriceUpdateEvent {
            market_id,
            price: index_price,
            source: "mock_oracle".to_string(),
        }));

        self.update_mark_price(market_id)?;

        Ok(())
    }

    /// Update the mark price based on current state.
    fn update_mark_price(&mut self, market_id: MarketId) -> Result<(), EngineError> {
        let market = self
            .markets
            .get_mut(&market_id)
            .ok_or(EngineError::MarketNotFound(market_id))?;

        let Some(index_price) = market.index_price else {
            return Ok(());
        };

        let mid_price = market.order_book.mid_price();

        let current_state = MarkPriceState {
            mark_price: market.mark_price.unwrap_or(index_price),
            premium_index: market.smoothed_premium,
            last_index_price: index_price,
            last_mid_price: mid_price,
        };

        let new_state = update_mark_price(
            &current_state,
            index_price,
            mid_price,
            &market.config.mark_price_params,
        );

        market.mark_price = Some(new_state.mark_price);
        market.smoothed_premium = new_state.premium_index;
        market.last_updated = self.current_time;

        self.emit_event(EventPayload::MarkPriceUpdate(MarkPriceUpdateEvent {
            market_id,
            mark_price: new_state.mark_price,
            index_price,
            premium_index: new_state.premium_index,
        }));

        Ok(())
    }
}
