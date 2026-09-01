//! Debug helper: print cosine scores for a query against the live index.
//! Run: cargo test --release debug_cosine -- --nocapture

use super::*;
use crate::router::embedding::{DEFAULT_MODEL_DIR, MODEL_DIR_ENV};
use std::path::PathBuf;

#[test]
fn debug_cosine_scores() {
    let dir = std::env::var(MODEL_DIR_ENV).unwrap_or_else(|_| DEFAULT_MODEL_DIR.to_string());
    if !PathBuf::from(&dir).join("onnx/model.onnx").exists() {
        eprintln!("skip: no model");
        return;
    }
    let index = EmbeddingIndex::new();
    let tools = vec![
        tool(
            "github::create_pull_request",
            "github",
            "create_pull_request",
            "Create a new pull request in a GitHub repository",
        ),
        tool(
            "github::get_pull_request_files",
            "github",
            "get_pull_request_files",
            "Get the list of files modified in a pull request",
        ),
        tool(
            "github::push_files",
            "github",
            "push_files",
            "Push multiple files to a branch in one commit",
        ),
        tool(
            "github::search_code",
            "github",
            "search_code",
            "Search for code in GitHub repositories",
        ),
        tool(
            "filesystem::read_file",
            "filesystem",
            "read_file",
            "Read contents of a file from filesystem",
        ),
        tool(
            "everything::echo",
            "everything",
            "echo",
            "Echoes back the input string",
        ),
    ];
    index.build_index(&tools).expect("build");
    for q in [
        "我想让同事们看到我改的代码",
        "flibberty gibbet wobble xyzzy",
        "create a github issue",
    ] {
        let r = index.search_ranked(q, 3).expect("search");
        eprintln!("query: {q}");
        for (id, rt) in &r {
            eprintln!("  {} {:.4}", rt.tool_id, rt.semantic_score);
        }
    }
}

fn tool(id: &str, server: &str, name: &str, desc: &str) -> Tool {
    Tool {
        name: name.to_string(),
        description: desc.to_string(),
        input_schema: serde_json::json!({}),
        annotations: None,
        title: None,
        tool_id: id.to_string(),
        server_name: Some(server.to_string()),
    }
}
