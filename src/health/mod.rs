mod diagnostics;
mod types;
mod tracker;

pub use diagnostics::{generate_health_report, generate_cleanup_suggestions};
pub use types::{ToolHealth, HealthScore};
pub use tracker::HealthManager;
