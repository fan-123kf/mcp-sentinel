mod diagnostics;
mod tracker;
mod types;

pub use diagnostics::{generate_cleanup_suggestions, generate_health_report};
pub use tracker::HealthManager;
#[allow(unused_imports)]
pub use types::{HealthScore, ToolHealth};
