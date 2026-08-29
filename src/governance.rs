use crate::backend::ToolCallResult;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SideEffect {
    Read,
    Write,
    Destructive,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolPolicy {
    pub side_effect: SideEffect,
    pub confirmation_required: bool,
    pub retry_safe: bool,
    pub max_attempts: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    Validation,
    Authentication,
    Permission,
    RateLimited,
    Timeout,
    Unavailable,
    Execution,
}

impl ToolPolicy {
    pub fn infer(tool_id: &str) -> Self {
        let name = tool_id.rsplit("::").next().unwrap_or(tool_id).to_ascii_lowercase();
        let destructive = ["delete", "remove", "destroy", "drop", "purge"];
        let writes = ["create", "update", "write", "send", "post", "put", "set", "merge"];

        if destructive.iter().any(|verb| name.contains(verb)) {
            return Self {
                side_effect: SideEffect::Destructive,
                confirmation_required: true,
                retry_safe: false,
                max_attempts: 1,
            };
        }

        if writes.iter().any(|verb| name.contains(verb)) {
            return Self {
                side_effect: SideEffect::Write,
                confirmation_required: true,
                retry_safe: false,
                max_attempts: 1,
            };
        }

        Self {
            side_effect: SideEffect::Read,
            confirmation_required: false,
            retry_safe: true,
            max_attempts: 2,
        }
    }

    pub fn authorize(&self, confirmed: bool) -> Result<(), String> {
        if self.confirmation_required && !confirmed {
            return Err("Tool has side effects and requires explicit confirmation".to_string());
        }
        Ok(())
    }
}

pub fn classify_error(result: &ToolCallResult) -> Option<ErrorCategory> {
    let ToolCallResult::Error { error, .. } = result else {
        return None;
    };
    let message = error.to_ascii_lowercase();

    if message.contains("invalid") || message.contains("validation") || message.contains("schema") {
        Some(ErrorCategory::Validation)
    } else if message.contains("unauthorized") || message.contains("authentication") || message.contains("token") {
        Some(ErrorCategory::Authentication)
    } else if message.contains("forbidden") || message.contains("permission") {
        Some(ErrorCategory::Permission)
    } else if message.contains("429") || message.contains("rate limit") {
        Some(ErrorCategory::RateLimited)
    } else if message.contains("timeout") || message.contains("timed out") {
        Some(ErrorCategory::Timeout)
    } else if message.contains("unavailable") || message.contains("connection") || message.contains("503") {
        Some(ErrorCategory::Unavailable)
    } else {
        Some(ErrorCategory::Execution)
    }
}

pub fn is_transient(category: ErrorCategory) -> bool {
    matches!(category, ErrorCategory::RateLimited | ErrorCategory::Timeout | ErrorCategory::Unavailable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_require_confirmation_and_are_not_retried() {
        let policy = ToolPolicy::infer("github::create_issue");
        assert_eq!(policy.side_effect, SideEffect::Write);
        assert!(policy.authorize(false).is_err());
        assert!(!policy.retry_safe);
    }

    #[test]
    fn reads_can_retry_transient_errors() {
        let policy = ToolPolicy::infer("filesystem::read_file");
        assert_eq!(policy.side_effect, SideEffect::Read);
        assert!(policy.retry_safe);
        assert!(is_transient(ErrorCategory::Timeout));
        assert!(!is_transient(ErrorCategory::Permission));
    }
}
