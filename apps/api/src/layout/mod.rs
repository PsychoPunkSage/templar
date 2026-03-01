// Phase 3: Layout Optimization System
// Implements: line-fill simulator, 2-line contract enforcement, page fill guarantees.
// CPU-bound simulation must run inside tokio::task::spawn_blocking.

pub mod contract;
pub mod font_metrics;
pub mod page_fill;
pub mod prompts;
pub mod simulator;

// Re-export the public API consumed by other modules (generator, handlers).
pub use font_metrics::{default_page_config, FontFamily, PageConfig};
pub use simulator::{run_simulation_loop, SimulatedBullet};
