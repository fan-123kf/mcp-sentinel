use crate::health::HealthManager;
use crate::storage::StorageManager;
use anyhow::Result;
use chrono::Utc;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

pub async fn generate_health_report(
    health_manager: &HealthManager,
    storage: Option<&Arc<StorageManager>>,
    time_window_days: u64,
) -> Result<String> {
    let mut report = String::new();
    
    report.push_str("# MCP Sentinel Health Report\n\n");
    report.push_str(&format!("**Generated**: {}\n\n", Utc::now().format("%Y-%m-%d %H:%M:%S UTC")));
    
    // Get all health scores
    let all_scores = health_manager.get_all_scores().await;
    
    let healthy_count = all_scores.iter().filter(|s| !s.degraded && !s.zombie).count();
    let degraded_count = all_scores.iter().filter(|s| s.degraded).count();
    let zombie_count = all_scores.iter().filter(|s| s.zombie).count();
    
    // Summary section
    report.push_str("## Summary\n\n");
    report.push_str(&format!("- **Total tools**: {} across all servers\n", all_scores.len()));
    report.push_str(&format!("- **Healthy**: {} ({:.1}%)\n", healthy_count, healthy_count as f64 / all_scores.len().max(1) as f64 * 100.0));
    report.push_str(&format!("- **Degraded**: {} ({:.1}%)\n", degraded_count, degraded_count as f64 / all_scores.len().max(1) as f64 * 100.0));
    report.push_str(&format!("- **Zombie**: {} ({:.1}%)\n", zombie_count, zombie_count as f64 / all_scores.len().max(1) as f64 * 100.0));
    report.push_str(&format!("- **Estimated wasted tokens/turn from zombies**: ~{}\n\n", zombie_count * 150));
    
    // Most used tools section (if storage available)
    if let Some(storage) = storage {
        report.push_str(&format!("## Top 10 Most-Used Tools ({}d)\n\n", time_window_days));
        
        let mut tool_stats: Vec<(String, u32, f64, f64)> = Vec::new();
        
        for score in &all_scores {
            if let Ok(count) = storage.get_call_count_window(&score.tool_id, time_window_days).await {
                if count > 0 {
                    if let Ok(p95) = storage.get_p95_latency(&score.tool_id, 1000).await {
                        if let Some(health) = health_manager.get_detailed_health(&score.tool_id).await {
                            let success_rate = if health.success_count + health.failure_count > 0 {
                                health.success_count as f64 / (health.success_count + health.failure_count) as f64 * 100.0
                            } else {
                                100.0
                            };
                            tool_stats.push((score.tool_id.clone(), count, success_rate, p95));
                        }
                    }
                }
            }
        }
        
        tool_stats.sort_by(|a, b| b.1.cmp(&a.1));
        
        report.push_str("| Tool | Calls | Success Rate | p95 Latency |\n");
        report.push_str("|------|-------|-------------|-------------|\n");
        
        for (tool_id, count, success_rate, p95) in tool_stats.iter().take(10) {
            report.push_str(&format!("| `{}` | {} | {:.1}% | {:.0}ms |\n", tool_id, count, success_rate, p95));
        }
        
        report.push_str("\n");
    }
    
    // Zombie tools section
    let zombies: Vec<_> = all_scores.iter().filter(|s| s.zombie).collect();
    
    if !zombies.is_empty() {
        report.push_str(&format!("## Zombie Tools (0 calls in {}+ days)\n\n", time_window_days));
        
        report.push_str("| Tool | Days Inactive | Estimated Token Waste |\n");
        report.push_str("|------|---------------|----------------------|\n");
        
        for zombie in &zombies {
            if let Some(health) = health_manager.get_detailed_health(&zombie.tool_id).await {
                let days_inactive = if let Some(last_call) = health.last_call {
                    if let Ok(elapsed) = last_call.elapsed() {
                        elapsed.as_secs() / 86400
                    } else {
                        7
                    }
                } else {
                    7
                };
                
                report.push_str(&format!("| `{}` | {} | ~150 tokens/turn |\n", zombie.tool_id, days_inactive));
            }
        }
        
        report.push_str("\n");
    }
    
    // Degraded tools section
    let degraded: Vec<_> = all_scores.iter().filter(|s| s.degraded && !s.zombie).collect();
    
    if !degraded.is_empty() {
        report.push_str("## Degraded Tools (Consecutive Failures)\n\n");
        
        report.push_str("| Tool | Health Score | Consecutive Failures | Last Success |\n");
        report.push_str("|------|-------------|---------------------|-------------|\n");
        
        for deg in &degraded {
            if let Some(health) = health_manager.get_detailed_health(&deg.tool_id).await {
                let last_success_str = if let Some(last_success) = health.last_success {
                    if let Ok(elapsed) = last_success.elapsed() {
                        let mins = elapsed.as_secs() / 60;
                        if mins < 60 {
                            format!("{}m ago", mins)
                        } else {
                            format!("{}h ago", mins / 60)
                        }
                    } else {
                        "unknown".to_string()
                    }
                } else {
                    "never".to_string()
                };
                
                report.push_str(&format!(
                    "| `{}` | {:.3} | {} | {} |\n",
                    deg.tool_id,
                    deg.health_score,
                    health.consecutive_failures,
                    last_success_str
                ));
            }
        }
        
        report.push_str("\n");
    }
    
    // Recommendations section
    report.push_str("## Recommendations\n\n");
    
    if !zombies.is_empty() {
        let zombie_servers: HashMap<String, usize> = zombies
            .iter()
            .filter_map(|z| z.tool_id.split("::").next().map(|s| s.to_string()))
            .fold(HashMap::new(), |mut acc, server| {
                *acc.entry(server).or_insert(0) += 1;
                acc
            });
        
        for (server, count) in zombie_servers.iter() {
            report.push_str(&format!(
                "1. **Remove `{}` server** — {} zombie tools wasting ~{} tokens/turn, no usage in {}+ days\n",
                server,
                count,
                count * 150,
                time_window_days
            ));
        }
    }
    
    if !degraded.is_empty() {
        for deg in degraded.iter().take(3) {
            report.push_str(&format!(
                "1. **Investigate `{}`** — {}% health score, likely auth token expired or service unreachable\n",
                deg.tool_id,
                (deg.health_score * 100.0) as i32
            ));
        }
    }
    
    if zombies.is_empty() && degraded.is_empty() {
        report.push_str("✅ All tools are healthy! No action needed.\n");
    }
    
    Ok(report)
}

pub async fn generate_cleanup_suggestions(
    health_manager: &HealthManager,
    storage: Option<&Arc<StorageManager>>,
    aggressive: bool,
) -> Result<serde_json::Value> {
    let threshold_days = if aggressive { 3 } else { 7 };
    let all_scores = health_manager.get_all_scores().await;
    
    let mut zombie_tools = Vec::new();
    let mut degraded_tools = Vec::new();
    
    for score in &all_scores {
        // Check if zombie based on call count
        let is_zombie = if let Some(storage) = storage {
            if let Ok(count) = storage.get_call_count_window(&score.tool_id, threshold_days).await {
                count == 0
            } else {
                score.zombie
            }
        } else {
            score.zombie
        };
        
        if is_zombie {
            zombie_tools.push(json!({
                "tool_id": score.tool_id,
                "reason": format!("No calls in {}+ days", threshold_days),
                "estimated_token_waste": 150
            }));
        } else if score.degraded {
            degraded_tools.push(json!({
                "tool_id": score.tool_id,
                "health_score": format!("{:.3}", score.health_score),
                "recommendation": "Check server connectivity and auth tokens"
            }));
        }
    }
    
    let total_token_savings = zombie_tools.len() * 150;
    
    // Group zombies by server
    let mut zombie_servers: HashMap<String, Vec<String>> = HashMap::new();
    for tool in &zombie_tools {
        if let Some(tool_id) = tool.get("tool_id").and_then(|v| v.as_str()) {
            if let Some(server) = tool_id.split("::").next() {
                zombie_servers
                    .entry(server.to_string())
                    .or_insert_with(Vec::new)
                    .push(tool_id.to_string());
            }
        }
    }
    
    let zombie_server_summary: Vec<_> = zombie_servers
        .iter()
        .map(|(server, tools)| {
            json!({
                "server": server,
                "zombie_count": tools.len(),
                "reason": format!("{} tools unused for {}+ days, wasting ~{} tokens/turn",
                    tools.len(),
                    threshold_days,
                    tools.len() * 150)
            })
        })
        .collect();
    
    Ok(json!({
        "zombie_tools": zombie_tools,
        "zombie_servers": zombie_server_summary,
        "degraded_tools": degraded_tools,
        "estimated_token_savings": total_token_savings,
        "recommendations": [
            format!("Remove {} zombie tools to reduce context bloat", zombie_tools.len()),
            format!("Investigate {} degraded tools for connectivity issues", degraded_tools.len())
        ]
    }))
}
