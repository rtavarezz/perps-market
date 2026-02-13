// 8.0.1: engine config. max events, verbose logging, fee schedule.

use crate::config::FeeConfig;

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub max_events: usize,
    pub verbose: bool,
    pub fees: FeeConfig,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            max_events: 100_000,
            verbose: false,
            fees: FeeConfig::default(),
        }
    }
}
