//! Conservative rule scanner for untrusted MCP metadata and results.
//! Findings contain paths and rule IDs, never the original content.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanLayer {
    Description,
    Schema,
    Output,
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub layer: ScanLayer,
    pub path: String,
    pub rule_id: String,
}

#[derive(Debug, Serialize)]
pub struct ScanReport {
    pub blocked: bool,
    pub findings: Vec<Finding>,
}

pub fn scan(value: &Value, layer: ScanLayer) -> ScanReport {
    let mut findings = Vec::new();
    visit(value, layer, "$", &mut findings, 0);
    ScanReport {
        blocked: !findings.is_empty(),
        findings,
    }
}

fn visit(value: &Value, layer: ScanLayer, path: &str, findings: &mut Vec<Finding>, depth: usize) {
    if depth > 64 {
        findings.push(Finding {
            layer,
            path: path.into(),
            rule_id: "nesting_limit".into(),
        });
        return;
    }
    match value {
        Value::String(s) => {
            let normalized = s
                .chars()
                .filter(|c| !matches!(c, '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{feff}'))
                .collect::<String>()
                .to_lowercase();
            let normalized = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
            let rules: &[(&str, &[&str])] = &[
                (
                    "instruction_override",
                    &[
                        "ignore previous instructions",
                        "ignore all previous",
                        "disregard previous",
                        "override system",
                        "忽略之前的指令",
                        "忽略所有指令",
                        "无视系统指令",
                    ],
                ),
                (
                    "role_spoofing",
                    &[
                        "<|im_start|>system",
                        "[system]",
                        "<system>",
                        "you are now the system",
                        "你现在是系统",
                    ],
                ),
                (
                    "secret_exfiltration",
                    &[
                        "send your api key",
                        "upload your token",
                        "reveal your password",
                        "exfiltrate",
                        "发送你的密钥",
                        "泄露密码",
                    ],
                ),
                (
                    "policy_bypass",
                    &[
                        "bypass confirmation",
                        "disable safety",
                        "skip authorization",
                        "跳过确认",
                        "绕过权限",
                        "关闭安全检查",
                    ],
                ),
            ];
            for (id, patterns) in rules {
                if patterns.iter().any(|p| normalized.contains(p)) {
                    findings.push(Finding {
                        layer,
                        path: path.into(),
                        rule_id: (*id).into(),
                    });
                }
            }
        }
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                visit(item, layer, &format!("{path}[{i}]"), findings, depth + 1);
            }
        }
        Value::Object(items) => {
            for (i, (key, item)) in items.iter().enumerate() {
                // Don't expose attacker-controlled keys in the diagnostic path.
                visit(
                    &Value::String(key.clone()),
                    layer,
                    &format!("{path}.key[{i}]"),
                    findings,
                    depth + 1,
                );
                visit(
                    item,
                    layer,
                    &format!("{path}.value[{i}]"),
                    findings,
                    depth + 1,
                );
            }
        }
        _ => {}
    }
}

pub fn scan_tool(tool: &crate::backend::Tool) -> ScanReport {
    let metadata = serde_json::json!([tool.name, tool.title, tool.description, tool.annotations]);
    let mut report = scan(&metadata, ScanLayer::Description);
    let schema = scan(&tool.input_schema, ScanLayer::Schema);
    report.findings.extend(schema.findings);
    report.blocked = !report.findings.is_empty();
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn blocks_all_three_layers_and_nested_schema() {
        for layer in [ScanLayer::Description, ScanLayer::Schema, ScanLayer::Output] {
            assert!(scan(&json!({"nested": ["IGNORE previous instructions"]}), layer).blocked);
        }
        assert!(scan(&json!("忽略之前的指令"), ScanLayer::Output).blocked);
        assert!(
            scan(
                &json!("ignore\u{200b} previous instructions"),
                ScanLayer::Output
            )
            .blocked
        );
    }
    #[test]
    fn ordinary_content_passes_and_findings_do_not_echo_payload() {
        assert!(
            !scan(
                &json!({"description": "Read a file", "type": "string"}),
                ScanLayer::Schema
            )
            .blocked
        );
        let report = scan(
            &json!("ignore previous instructions TOP_SECRET"),
            ScanLayer::Output,
        );
        assert!(!serde_json::to_string(&report)
            .unwrap()
            .contains("TOP_SECRET"));
    }
}
