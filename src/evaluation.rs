//! Reproducible metrics computed from the same router used by the gateway.
use crate::{backend::Tool, health::HealthManager, router::SemanticRouter};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path;

#[derive(Deserialize)]
struct Query {
    query: String,
    expected_tools: Vec<String>,
}

pub async fn run(
    corpus_path: &Path,
    query_path: &Path,
    output: &Path,
    require_models: bool,
) -> anyhow::Result<()> {
    let corpus: Value = serde_json::from_slice(&std::fs::read(corpus_path)?)?;
    let mut tools = Vec::new();
    for (server, group) in corpus
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("Expected server map"))?
    {
        for value in group["tools"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Expected tools array"))?
        {
            let mut tool: Tool = serde_json::from_value(value.clone())?;
            tool.tool_id = format!("{server}::{}", tool.name);
            tool.server_name = Some(server.clone());
            tools.push(tool);
        }
    }
    let queries: Vec<Query> = std::fs::read_to_string(query_path)?
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    for q in &queries {
        for expected in &q.expected_tools {
            anyhow::ensure!(
                tools.iter().any(|t| &t.tool_id == expected),
                "Missing labeled tool {expected}"
            );
        }
    }
    let tool_count = tools.len();
    let backend_definitions = json!({"tools": tools});
    let router = SemanticRouter::new(HealthManager::new()).with_embedding(
        require_models || std::env::var("SENTINEL_EMBEDDING").is_ok_and(|v| v == "1"),
    );
    router.index_tools(tools).await;
    if require_models {
        router.require_models()?;
    }
    let mut rows = Vec::new();
    let (mut recall, mut mrr, mut hit1, mut relevant) = (0.0, 0.0, 0, 0);
    for q in &queries {
        let (hits, pipeline) = router.search_with_metadata(&q.query, 20, 0.0, None).await;
        if require_models {
            anyhow::ensure!(
                pipeline == "bge_m3_dense_sparse_rrf_cross_encoder",
                "Model pipeline fell back during evaluation"
            );
        }
        let ids: Vec<_> = hits.iter().map(|t| t.tool_id.clone()).collect();
        if !q.expected_tools.is_empty() {
            relevant += 1;
            recall += q
                .expected_tools
                .iter()
                .filter(|e| ids.iter().take(5).any(|id| id == *e))
                .count() as f64
                / q.expected_tools.len() as f64;
            if let Some(rank) = ids.iter().position(|id| q.expected_tools.contains(id)) {
                mrr += 1.0 / (rank + 1) as f64;
            }
            if ids.first().is_some_and(|id| q.expected_tools.contains(id)) {
                hit1 += 1;
            }
        }
        rows.push(json!({"query": q.query, "expected": q.expected_tools, "returned": ids, "pipeline": pipeline}));
    }
    anyhow::ensure!(relevant > 0, "No relevant queries");
    let report = json!({"tool_count": tool_count, "query_count": queries.len(), "relevant_queries": relevant,
        "recall_at_5": recall / relevant as f64, "mrr_at_20": mrr / relevant as f64, "top1_accuracy": hit1 as f64 / relevant as f64,
        "pipeline": router.strategy(), "rows": rows,
        "token_measurement": "Use scripts/token_account.py with the selected tokenizer; bytes are not tokens"});
    std::fs::write(output, serde_json::to_vec_pretty(&report)?)?;
    std::fs::write(
        output.with_extension("tools.json"),
        serde_json::to_vec(&backend_definitions)?,
    )?;
    std::fs::write(
        output.with_extension("meta.json"),
        serde_json::to_vec(&json!({"tools": crate::gateway::meta_tools::meta_tool_definitions()}))?,
    )?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
