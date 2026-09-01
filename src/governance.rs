use crate::backend::ToolCallResult;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    /// Authoritative classification from MCP tool annotations
    /// (readOnlyHint / destructiveHint / idempotentHint). Servers that provide
    /// annotations know their own side effects better than any name heuristic.
    pub fn from_annotations(annotations: &Value, tool_id: &str) -> Option<Self> {
        let obj = annotations.as_object()?;
        let read_only = obj.get("readOnlyHint").and_then(Value::as_bool);
        let destructive = obj.get("destructiveHint").and_then(Value::as_bool);

        match (read_only, destructive) {
            // Server explicitly claims read-only: trust it (Read, retry-safe).
            (Some(true), _) => Some(Self {
                side_effect: SideEffect::Read,
                confirmation_required: false,
                retry_safe: true,
                max_attempts: 2,
            }),
            // Server explicitly claims destructive.
            (_, Some(true)) => Some(Self {
                side_effect: SideEffect::Destructive,
                confirmation_required: true,
                retry_safe: false,
                max_attempts: 1,
            }),
            // Explicitly not read-only and not destructive => mutating (Write).
            (Some(false), Some(false) | None) => Some(Self {
                side_effect: SideEffect::Write,
                confirmation_required: true,
                retry_safe: false,
                max_attempts: 1,
            }),
            _ => None,
        }
    }

    pub fn infer(tool_id: &str) -> Self {
        let name = tool_id
            .rsplit("::")
            .next()
            .unwrap_or(tool_id)
            .to_ascii_lowercase();
        let destructive = [
            "delete", "remove", "destroy", "drop", "purge", "unlink", "truncate", "wipe", "erase",
        ];
        // Mutating verbs: anything that changes existing state or moves data across
        // namespaces. "move"/"rename"/"replace"/"sync"/"import"/"restore" were missing
        // previously (filesystem::move_file slipped through as Read).
        let writes = [
            "create", "update", "write", "send", "post", "put", "set", "merge", "move", "rename",
            "replace", "sync", "import", "restore", "apply", "edit", "append", "assign", "copy",
            "upload", "commit", "push", "close", "reopen",
        ];

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
    } else if message.contains("unauthorized")
        || message.contains("authentication")
        || message.contains("token")
    {
        Some(ErrorCategory::Authentication)
    } else if message.contains("forbidden") || message.contains("permission") {
        Some(ErrorCategory::Permission)
    } else if message.contains("429") || message.contains("rate limit") {
        Some(ErrorCategory::RateLimited)
    } else if message.contains("timeout") || message.contains("timed out") {
        Some(ErrorCategory::Timeout)
    } else if message.contains("unavailable")
        || message.contains("connection")
        || message.contains("503")
    {
        Some(ErrorCategory::Unavailable)
    } else {
        Some(ErrorCategory::Execution)
    }
}

pub fn is_transient(category: ErrorCategory) -> bool {
    matches!(
        category,
        ErrorCategory::RateLimited | ErrorCategory::Timeout | ErrorCategory::Unavailable
    )
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
    fn move_rename_and_other_mutators_require_confirmation() {
        // Regression: filesystem::move_file was classified Read before the
        // keyword table was extended (found in E2E eval round 2).
        for tool_id in [
            "filesystem::move_file",
            "filesystem::rename_file",
            "db::replace_record",
            "files::sync_dir",
            "etl::import_csv",
            "backup::restore_snapshot",
            "fs::copy_file",
            "storage::upload_object",
        ] {
            let policy = ToolPolicy::infer(tool_id);
            assert_eq!(policy.side_effect, SideEffect::Write, "tool: {tool_id}");
            assert!(policy.authorize(false).is_err(), "tool: {tool_id}");
        }
    }

    #[test]
    fn destructive_table_covers_more_verbs() {
        for tool_id in [
            "fs::unlink_path",
            "db::truncate_table",
            "store::wipe_bucket",
        ] {
            let policy = ToolPolicy::infer(tool_id);
            assert_eq!(
                policy.side_effect,
                SideEffect::Destructive,
                "tool: {tool_id}"
            );
            assert!(policy.authorize(false).is_err(), "tool: {tool_id}");
        }
    }

    #[test]
    fn reads_stay_read() {
        for tool_id in [
            "filesystem::read_file",
            "filesystem::list_directory",
            "everything::echo",
            "everything::get-sum",
            "github::get_issue",
            "github::list_pull_requests",
            "github::search_code",
        ] {
            let policy = ToolPolicy::infer(tool_id);
            assert_eq!(policy.side_effect, SideEffect::Read, "tool: {tool_id}");
            assert!(policy.authorize(false).is_ok(), "tool: {tool_id}");
        }
    }

    #[test]
    fn annotations_override_name_heuristic() {
        use serde_json::json;

        // Server says read-only, but the name contains "delete": annotations win.
        let p = ToolPolicy::from_annotations(
            &json!({"readOnlyHint": true, "destructiveHint": false}),
            "fs::delete_cache_file",
        )
        .expect("annotations present");
        assert_eq!(p.side_effect, SideEffect::Read);
        assert!(p.authorize(false).is_ok());

        // Server says destructive even though the name looks innocent.
        let p = ToolPolicy::from_annotations(
            &json!({"readOnlyHint": false, "destructiveHint": true}),
            "fs::sync_data",
        )
        .expect("annotations present");
        assert_eq!(p.side_effect, SideEffect::Destructive);
        assert!(p.authorize(false).is_err());

        // Server says mutating but not destructive => Write.
        let p = ToolPolicy::from_annotations(
            &json!({"readOnlyHint": false, "destructiveHint": false}),
            "fs::gzip_file_as_resource",
        )
        .expect("annotations present");
        assert_eq!(p.side_effect, SideEffect::Write);
        assert!(p.authorize(false).is_err());

        // No hints at all => fall back to heuristic.
        assert!(ToolPolicy::from_annotations(&json!({}), "anything").is_none());
        assert!(ToolPolicy::from_annotations(&json!(null), "anything").is_none());
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
