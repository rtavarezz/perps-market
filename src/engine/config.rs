//! Engine configuration options.

/// Engine configuration.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Maximum number of events to retain in memory.
    pub max_events: usize,
    /// Enable verbose logging.
    pub verbose: bool,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            max_events: 100_000,
            verbose: false,
        }
    }
}
