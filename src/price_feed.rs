// Price Feed Integration
//
// This module abstracts how the engine receives price updates. The core engine
// is agnostic to whether prices come from Pyth, Chainlink, a CEX aggregator,
// or a custom oracle. We define traits and types that any price source can implement.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// Unique identifier for a price source
pub type PriceSourceId = u32;

/// A single price update from an oracle or feed
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceUpdate {
    pub price: Decimal,
    pub timestamp: u64,
    pub source_id: PriceSourceId,
    /// Confidence interval (if provided by source like Pyth)
    pub confidence: Option<Decimal>,
    /// Time to live in seconds before this price is considered stale
    pub ttl_seconds: u64,
}

impl PriceUpdate {
    pub fn new(price: Decimal, timestamp: u64, source_id: PriceSourceId) -> Self {
        Self {
            price,
            timestamp,
            source_id,
            confidence: None,
            ttl_seconds: 60, // default 1 minute TTL
        }
    }

    pub fn with_confidence(mut self, confidence: Decimal) -> Self {
        self.confidence = Some(confidence);
        self
    }

    pub fn with_ttl(mut self, ttl: u64) -> Self {
        self.ttl_seconds = ttl;
        self
    }

    pub fn is_stale(&self, current_time: u64) -> bool {
        current_time > self.timestamp + self.ttl_seconds
    }
}

/// Configuration for price feed aggregation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceFeedConfig {
    /// Minimum number of sources required for a valid price
    pub min_sources: usize,
    /// Maximum age in seconds before a price is considered stale
    pub max_staleness_seconds: u64,
    /// Maximum deviation between sources before rejecting (as ratio, e.g. 0.01 = 1%)
    pub max_source_deviation: Decimal,
    /// Whether to use median (true) or weighted average (false)
    pub use_median: bool,
    /// Weight assigned to each source (source_id -> weight)
    pub source_weights: Vec<(PriceSourceId, Decimal)>,
}

impl Default for PriceFeedConfig {
    fn default() -> Self {
        Self {
            min_sources: 1,
            max_staleness_seconds: 60,
            max_source_deviation: Decimal::new(2, 2), // 2%
            use_median: true,
            source_weights: Vec::new(),
        }
    }
}

/// Errors that can occur during price aggregation
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PriceFeedError {
    InsufficientSources { required: usize, available: usize },
    AllSourcesStale,
    ExcessiveDeviation { max_deviation: Decimal },
    NoPriceAvailable,
}

/// Aggregates prices from multiple sources into a single reliable price.
/// This handles the complexity of combining Pyth, Chainlink, CEX feeds, etc.
#[derive(Debug, Clone)]
pub struct PriceAggregator {
    config: PriceFeedConfig,
    /// Recent prices from each source (source_id -> price history)
    source_prices: Vec<(PriceSourceId, VecDeque<PriceUpdate>)>,
    /// Maximum history per source
    max_history: usize,
}

impl PriceAggregator {
    pub fn new(config: PriceFeedConfig) -> Self {
        Self {
            config,
            source_prices: Vec::new(),
            max_history: 100,
        }
    }

    /// Register a new price source
    pub fn add_source(&mut self, source_id: PriceSourceId) {
        if !self.source_prices.iter().any(|(id, _)| *id == source_id) {
            self.source_prices.push((source_id, VecDeque::new()));
        }
    }

    /// Submit a price update from a source
    pub fn submit_price(&mut self, update: PriceUpdate) {
        // find or create the source entry
        let entry = self.source_prices
            .iter_mut()
            .find(|(id, _)| *id == update.source_id);

        if let Some((_, history)) = entry {
            history.push_back(update);
            while history.len() > self.max_history {
                history.pop_front();
            }
        } else {
            let mut history = VecDeque::new();
            history.push_back(update.clone());
            self.source_prices.push((update.source_id, history));
        }
    }

    /// Get the aggregated price at the given timestamp
    pub fn get_price(&self, current_time: u64) -> Result<PriceUpdate, PriceFeedError> {
        // collect fresh prices from each source
        let mut fresh_prices: Vec<(PriceSourceId, Decimal)> = Vec::new();

        for (source_id, history) in &self.source_prices {
            // get the most recent non stale price
            if let Some(update) = history.back() {
                if !update.is_stale(current_time) {
                    fresh_prices.push((*source_id, update.price));
                }
            }
        }

        if fresh_prices.is_empty() {
            return Err(PriceFeedError::AllSourcesStale);
        }

        if fresh_prices.len() < self.config.min_sources {
            return Err(PriceFeedError::InsufficientSources {
                required: self.config.min_sources,
                available: fresh_prices.len(),
            });
        }

        // check deviation between sources
        if fresh_prices.len() > 1 {
            let prices: Vec<Decimal> = fresh_prices.iter().map(|(_, p)| *p).collect();
            let min_price = prices.iter().min().unwrap();
            let max_price = prices.iter().max().unwrap();

            if *min_price > Decimal::ZERO {
                let deviation = (*max_price - *min_price) / *min_price;
                if deviation > self.config.max_source_deviation {
                    return Err(PriceFeedError::ExcessiveDeviation {
                        max_deviation: deviation,
                    });
                }
            }
        }

        // aggregate the prices
        let final_price = if self.config.use_median {
            self.calculate_median(&fresh_prices)
        } else {
            self.calculate_weighted_average(&fresh_prices)
        };

        Ok(PriceUpdate {
            price: final_price,
            timestamp: current_time,
            source_id: 0, // aggregated source
            confidence: None,
            ttl_seconds: self.config.max_staleness_seconds,
        })
    }

    fn calculate_median(&self, prices: &[(PriceSourceId, Decimal)]) -> Decimal {
        let mut sorted: Vec<Decimal> = prices.iter().map(|(_, p)| *p).collect();
        sorted.sort();

        let len = sorted.len();
        if len == 0 {
            return Decimal::ZERO;
        }

        if len % 2 == 0 {
            (sorted[len / 2 - 1] + sorted[len / 2]) / Decimal::new(2, 0)
        } else {
            sorted[len / 2]
        }
    }

    fn calculate_weighted_average(&self, prices: &[(PriceSourceId, Decimal)]) -> Decimal {
        let mut total_weight = Decimal::ZERO;
        let mut weighted_sum = Decimal::ZERO;

        for (source_id, price) in prices {
            let weight = self.config.source_weights
                .iter()
                .find(|(id, _)| id == source_id)
                .map(|(_, w)| *w)
                .unwrap_or(Decimal::ONE);

            weighted_sum += *price * weight;
            total_weight += weight;
        }

        if total_weight > Decimal::ZERO {
            weighted_sum / total_weight
        } else {
            Decimal::ZERO
        }
    }

    /// Get the latest price from a specific source
    pub fn get_source_price(&self, source_id: PriceSourceId) -> Option<&PriceUpdate> {
        self.source_prices
            .iter()
            .find(|(id, _)| *id == source_id)
            .and_then(|(_, history)| history.back())
    }

    /// Get all source IDs
    pub fn sources(&self) -> Vec<PriceSourceId> {
        self.source_prices.iter().map(|(id, _)| *id).collect()
    }
}

/// Trait for price feed adapters. Implement this to integrate with specific
/// oracle networks or data sources.
pub trait PriceFeedAdapter {
    /// Unique identifier for this adapter
    fn source_id(&self) -> PriceSourceId;

    /// Human readable name
    fn name(&self) -> &str;

    /// Fetch the latest price (could be async in real implementation)
    fn fetch_price(&self) -> Option<PriceUpdate>;

    /// Check if the adapter is healthy/connected
    fn is_healthy(&self) -> bool;
}

/// Mock adapter for testing
pub struct MockPriceFeed {
    source_id: PriceSourceId,
    name: String,
    current_price: Decimal,
    healthy: bool,
}

impl MockPriceFeed {
    pub fn new(source_id: PriceSourceId, name: &str, price: Decimal) -> Self {
        Self {
            source_id,
            name: name.to_string(),
            current_price: price,
            healthy: true,
        }
    }

    pub fn set_price(&mut self, price: Decimal) {
        self.current_price = price;
    }

    pub fn set_healthy(&mut self, healthy: bool) {
        self.healthy = healthy;
    }
}

impl PriceFeedAdapter for MockPriceFeed {
    fn source_id(&self) -> PriceSourceId {
        self.source_id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn fetch_price(&self) -> Option<PriceUpdate> {
        if self.healthy {
            Some(PriceUpdate::new(self.current_price, 0, self.source_id))
        } else {
            None
        }
    }

    fn is_healthy(&self) -> bool {
        self.healthy
    }
}

/// TWAP (Time Weighted Average Price) calculator for mark price
#[derive(Debug, Clone)]
pub struct TwapCalculator {
    /// Price samples with timestamps
    samples: VecDeque<(u64, Decimal)>,
    /// Window size in seconds
    window_seconds: u64,
    /// Max samples to keep
    max_samples: usize,
}

impl TwapCalculator {
    pub fn new(window_seconds: u64) -> Self {
        Self {
            samples: VecDeque::new(),
            window_seconds,
            max_samples: 1000,
        }
    }

    pub fn add_sample(&mut self, timestamp: u64, price: Decimal) {
        self.samples.push_back((timestamp, price));

        // remove old samples outside the window
        while let Some((ts, _)) = self.samples.front() {
            if timestamp.saturating_sub(*ts) > self.window_seconds {
                self.samples.pop_front();
            } else {
                break;
            }
        }

        // cap total samples
        while self.samples.len() > self.max_samples {
            self.samples.pop_front();
        }
    }

    pub fn get_twap(&self) -> Option<Decimal> {
        if self.samples.is_empty() {
            return None;
        }

        if self.samples.len() == 1 {
            return Some(self.samples[0].1);
        }

        // time weighted calculation
        let mut weighted_sum = Decimal::ZERO;
        let mut total_time = Decimal::ZERO;

        for i in 1..self.samples.len() {
            let (prev_ts, prev_price) = self.samples[i - 1];
            let (curr_ts, _) = self.samples[i];
            let duration = Decimal::from(curr_ts - prev_ts);

            weighted_sum += prev_price * duration;
            total_time += duration;
        }

        // add the last sample's contribution up to "now"
        // (in practice, caller would pass current timestamp)

        if total_time > Decimal::ZERO {
            Some(weighted_sum / total_time)
        } else {
            self.samples.back().map(|(_, p)| *p)
        }
    }

    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_price_update_staleness() {
        let update = PriceUpdate::new(dec!(50000), 1000, 1).with_ttl(60);

        assert!(!update.is_stale(1030)); // 30 seconds later, still fresh
        assert!(!update.is_stale(1060)); // exactly at TTL
        assert!(update.is_stale(1061));  // 1 second past TTL
    }

    #[test]
    fn test_aggregator_single_source() {
        let config = PriceFeedConfig::default();
        let mut agg = PriceAggregator::new(config);

        agg.add_source(1);
        agg.submit_price(PriceUpdate::new(dec!(50000), 1000, 1).with_ttl(100));

        let result = agg.get_price(1050).unwrap();
        assert_eq!(result.price, dec!(50000));
    }

    #[test]
    fn test_aggregator_median() {
        let config = PriceFeedConfig {
            min_sources: 3,
            use_median: true,
            ..Default::default()
        };
        let mut agg = PriceAggregator::new(config);

        agg.submit_price(PriceUpdate::new(dec!(49900), 1000, 1).with_ttl(100));
        agg.submit_price(PriceUpdate::new(dec!(50000), 1000, 2).with_ttl(100));
        agg.submit_price(PriceUpdate::new(dec!(50100), 1000, 3).with_ttl(100));

        let result = agg.get_price(1050).unwrap();
        assert_eq!(result.price, dec!(50000)); // median
    }

    #[test]
    fn test_aggregator_insufficient_sources() {
        let config = PriceFeedConfig {
            min_sources: 3,
            ..Default::default()
        };
        let mut agg = PriceAggregator::new(config);

        agg.submit_price(PriceUpdate::new(dec!(50000), 1000, 1).with_ttl(100));
        agg.submit_price(PriceUpdate::new(dec!(50000), 1000, 2).with_ttl(100));

        let result = agg.get_price(1050);
        assert!(matches!(result, Err(PriceFeedError::InsufficientSources { .. })));
    }

    #[test]
    fn test_aggregator_stale_prices() {
        let config = PriceFeedConfig::default();
        let mut agg = PriceAggregator::new(config);

        agg.submit_price(PriceUpdate::new(dec!(50000), 1000, 1).with_ttl(60));

        // price is fresh at t=1050
        assert!(agg.get_price(1050).is_ok());

        // price is stale at t=1100
        let result = agg.get_price(1100);
        assert!(matches!(result, Err(PriceFeedError::AllSourcesStale)));
    }

    #[test]
    fn test_aggregator_excessive_deviation() {
        let config = PriceFeedConfig {
            min_sources: 2,
            max_source_deviation: Decimal::new(1, 2), // 1%
            ..Default::default()
        };
        let mut agg = PriceAggregator::new(config);

        // 5% deviation between sources
        agg.submit_price(PriceUpdate::new(dec!(50000), 1000, 1).with_ttl(100));
        agg.submit_price(PriceUpdate::new(dec!(52500), 1000, 2).with_ttl(100));

        let result = agg.get_price(1050);
        assert!(matches!(result, Err(PriceFeedError::ExcessiveDeviation { .. })));
    }

    #[test]
    fn test_twap_calculator() {
        let mut twap = TwapCalculator::new(3600); // 1 hour window

        twap.add_sample(0, dec!(50000));
        twap.add_sample(1000, dec!(51000));
        twap.add_sample(2000, dec!(52000));

        let result = twap.get_twap().unwrap();
        // TWAP should be weighted by time intervals
        // 50000 for 1000s, 51000 for 1000s = (50000*1000 + 51000*1000) / 2000 = 50500
        assert_eq!(result, dec!(50500));
    }

    #[test]
    fn test_mock_price_feed() {
        let mut feed = MockPriceFeed::new(1, "Pyth", dec!(50000));

        assert!(feed.is_healthy());
        assert_eq!(feed.fetch_price().unwrap().price, dec!(50000));

        feed.set_healthy(false);
        assert!(!feed.is_healthy());
        assert!(feed.fetch_price().is_none());
    }

    // helper macro for decimal literals in tests
    macro_rules! dec {
        ($val:expr) => {
            Decimal::from($val)
        };
    }
    use dec;
}
