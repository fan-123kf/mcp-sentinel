use serde::{Deserialize, Serialize};
use std::time::SystemTime;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolHealth {
    pub tool_id: String,
    pub success_count: u64,
    pub failure_count: u64,
    pub total_latency_ms: u64,
    pub latency_p95: f64,
    pub consecutive_failures: u32,
    pub last_success: Option<SystemTime>,
    pub last_failure: Option<SystemTime>,
    pub last_call: Option<SystemTime>,
    pub call_count_7d: u32,
    pub zombie_score: f64,
    pub health_score: f64,
}

impl ToolHealth {
    pub fn new(tool_id: String) -> Self {
        Self {
            tool_id,
            success_count: 0,
            failure_count: 0,
            total_latency_ms: 0,
            latency_p95: 0.0,
            consecutive_failures: 0,
            last_success: None,
            last_failure: None,
            last_call: None,
            call_count_7d: 0,
            zombie_score: 0.0,
            health_score: 1.0,
        }
    }

    pub fn compute_health_score(&mut self) {
        let success_rate = if self.success_count + self.failure_count > 0 {
            self.success_count as f64 / (self.success_count + self.failure_count) as f64
        } else {
            1.0
        };

        let latency_penalty = 1.0 / (1.0 + self.latency_p95 / 2000.0);

        let staleness = match self.last_call {
            Some(t) => {
                if let Ok(elapsed) = t.elapsed() {
                    let days = elapsed.as_secs() / 86400;
                    1.0 / (1.0 + days as f64 * 0.1)
                } else {
                    0.5
                }
            }
            None => 0.5,
        };

        self.health_score = success_rate * latency_penalty * staleness;
        self.zombie_score = if self.call_count_7d == 0 { 1.0 } else { 0.0 };
    }

    pub fn record_success(&mut self, latency_ms: u64) {
        self.success_count += 1;
        self.total_latency_ms += latency_ms;
        self.consecutive_failures = 0;
        self.last_success = Some(SystemTime::now());
        self.last_call = Some(SystemTime::now());
        self.call_count_7d += 1;
        self.compute_health_score();
    }

    pub fn record_failure(&mut self) {
        self.failure_count += 1;
        self.consecutive_failures += 1;
        self.last_failure = Some(SystemTime::now());
        self.last_call = Some(SystemTime::now());
        self.call_count_7d += 1;
        self.compute_health_score();
    }

    pub fn is_degraded(&self, threshold: u32) -> bool {
        self.consecutive_failures >= threshold
    }

    pub fn is_zombie(&self) -> bool {
        self.zombie_score >= 0.9
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthScore {
    pub tool_id: String,
    pub health_score: f64,
    pub degraded: bool,
    pub zombie: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_health_new() {
        let health = ToolHealth::new("test::tool".to_string());
        assert_eq!(health.tool_id, "test::tool");
        assert_eq!(health.success_count, 0);
        assert_eq!(health.failure_count, 0);
        assert_eq!(health.consecutive_failures, 0);
        assert_eq!(health.health_score, 1.0);
    }

    #[test]
    fn test_record_success() {
        let mut health = ToolHealth::new("test::tool".to_string());
        health.record_success(100);

        assert_eq!(health.success_count, 1);
        assert_eq!(health.consecutive_failures, 0);
        assert!(health.last_success.is_some());
        assert!(health.health_score > 0.0);
    }

    #[test]
    fn test_record_failure() {
        let mut health = ToolHealth::new("test::tool".to_string());
        health.record_failure();

        assert_eq!(health.failure_count, 1);
        assert_eq!(health.consecutive_failures, 1);
        assert!(health.last_failure.is_some());
    }

    #[test]
    fn test_is_degraded() {
        let mut health = ToolHealth::new("test::tool".to_string());
        assert!(!health.is_degraded(5));

        for _ in 0..5 {
            health.record_failure();
        }
        assert!(health.is_degraded(5));

        health.record_success(100);
        assert!(!health.is_degraded(5));
    }

    #[test]
    fn test_health_score_computation() {
        let mut health = ToolHealth::new("test::tool".to_string());

        for _ in 0..10 {
            health.record_success(100);
        }
        assert!(health.health_score > 0.8);

        for _ in 0..5 {
            health.record_failure();
        }
        health.compute_health_score();
        assert!(health.health_score < 0.8);
    }

    #[test]
    fn test_zombie_detection() {
        let mut health = ToolHealth::new("test::tool".to_string());

        health.call_count_7d = 0;
        health.compute_health_score();
        assert!(health.is_zombie());

        health.call_count_7d = 5;
        health.compute_health_score();
        assert!(!health.is_zombie());
    }
}
