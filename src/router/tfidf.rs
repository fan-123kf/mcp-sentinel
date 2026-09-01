use crate::backend::Tool;
use crate::router::types::RankedTool;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::RwLock;
use unicode_segmentation::UnicodeSegmentation;

pub struct TfIdfIndex {
    documents: RwLock<Vec<Document>>,
    idf: RwLock<HashMap<String, f64>>,
}

struct Document {
    tool_id: String,
    tool_name: String,
    server_name: String,
    description: String,
    terms: Vec<String>,
    tf: HashMap<String, f64>,
}

impl TfIdfIndex {
    pub fn new() -> Self {
        Self {
            documents: RwLock::new(Vec::new()),
            idf: RwLock::new(HashMap::new()),
        }
    }

    pub fn build_index(&self, tools: Vec<Tool>) {
        let mut documents = Vec::new();
        let mut term_doc_count: HashMap<String, usize> = HashMap::new();

        for tool in tools {
            // Enriched index text: server + title + name + description +
            // parameter descriptions. Previously only name + description were
            // indexed, which caused (a) cross-server confusion ("list files"
            // matching github tools) and (b) parameter-level semantics being
            // invisible (github ships 79 param descriptions we threw away).
            let title = tool.title.as_deref().unwrap_or("");
            let param_descs: Vec<String> = tool
                .input_schema
                .get("properties")
                .and_then(|p| p.as_object())
                .map(|props| {
                    props
                        .iter()
                        .filter_map(|(pname, pv)| {
                            let desc = pv.get("description").and_then(Value::as_str)?;
                            Some(format!("{} {}", pname, desc))
                        })
                        .collect()
                })
                .unwrap_or_default();
            let required: Vec<&str> = tool
                .input_schema
                .get("required")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();

            let mut text = format!(
                "{} {} {} {}",
                tool.server_name.as_deref().unwrap_or(""),
                title,
                tool.name,
                tool.description
            );
            if !required.is_empty() {
                text.push_str(&format!(" required: {}", required.join(" ")));
            }
            if !param_descs.is_empty() {
                text.push_str(&format!(" params: {}", param_descs.join("; ")));
            }

            let terms = Self::tokenize(&text);
            let tf = Self::compute_tf(&terms);

            for term in terms.iter() {
                *term_doc_count.entry(term.clone()).or_insert(0) += 1;
            }

            documents.push(Document {
                tool_id: tool.tool_id.clone(),
                tool_name: tool.name.clone(),
                server_name: tool.server_name.unwrap_or_default(),
                description: tool.description.clone(),
                terms,
                tf,
            });
        }

        let total_docs = documents.len() as f64;
        let idf: HashMap<String, f64> = term_doc_count
            .into_iter()
            .map(|(term, count)| {
                let idf_score = (total_docs / count as f64).ln();
                (term, idf_score)
            })
            .collect();

        *self.documents.write().unwrap() = documents;
        *self.idf.write().unwrap() = idf;
    }

    pub fn search(&self, query: &str, top_k: usize) -> Vec<RankedTool> {
        let query_terms = Self::tokenize(query);
        let query_tf = Self::compute_tf(&query_terms);
        let documents = self.documents.read().unwrap();
        let idf = self.idf.read().unwrap();

        let mut scores: Vec<(usize, f64)> = documents
            .iter()
            .enumerate()
            .map(|(idx, doc_item)| {
                let score = Self::cosine_similarity(&query_tf, &doc_item.tf, &idf);
                (idx, score)
            })
            .collect();

        scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        scores
            .into_iter()
            .take(top_k)
            .filter(|(_, score)| *score > 0.0)
            .map(|(idx, score)| {
                let doc_item = &documents[idx];
                RankedTool {
                    tool_id: doc_item.tool_id.clone(),
                    tool_name: doc_item.tool_name.clone(),
                    server_name: doc_item.server_name.clone(),
                    description: doc_item.description.clone(),
                    semantic_score: score,
                    health_score: 1.0,
                    final_score: score,
                    degraded: false,
                }
            })
            .collect()
    }

    pub fn tokenize(text: &str) -> Vec<String> {
        text.to_lowercase()
            .unicode_words()
            .filter(|w| w.len() > 2)
            .map(|w| w.to_string())
            .collect()
    }

    pub fn compute_tf(terms: &[String]) -> HashMap<String, f64> {
        let mut tf = HashMap::new();
        let total = terms.len() as f64;

        for term in terms {
            *tf.entry(term.clone()).or_insert(0.0) += 1.0 / total;
        }

        tf
    }

    pub fn cosine_similarity(
        tf1: &HashMap<String, f64>,
        tf2: &HashMap<String, f64>,
        idf: &HashMap<String, f64>,
    ) -> f64 {
        let mut dot_product = 0.0;
        let mut norm1 = 0.0;
        let mut norm2 = 0.0;

        for (term, &tf1_val) in tf1 {
            let idf_val = idf.get(term).unwrap_or(&1.0);
            let tfidf1 = tf1_val * idf_val;
            norm1 += tfidf1 * tfidf1;

            if let Some(&tf2_val) = tf2.get(term) {
                let tfidf2 = tf2_val * idf_val;
                dot_product += tfidf1 * tfidf2;
            }
        }

        for (term, &tf2_val) in tf2 {
            let idf_val = idf.get(term).unwrap_or(&1.0);
            let tfidf2 = tf2_val * idf_val;
            norm2 += tfidf2 * tfidf2;
        }

        if norm1 == 0.0 || norm2 == 0.0 {
            0.0
        } else {
            dot_product / (norm1.sqrt() * norm2.sqrt())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Tool;

    #[test]
    fn test_tokenize() {
        let text = "Create a GitHub issue";
        let tokens = TfIdfIndex::tokenize(text);

        assert!(tokens.contains(&"create".to_string()));
        assert!(tokens.contains(&"github".to_string()));
        assert!(tokens.contains(&"issue".to_string()));
        assert!(!tokens.contains(&"a".to_string()));
    }

    #[test]
    fn test_compute_tf() {
        let terms = vec!["test".to_string(), "test".to_string(), "word".to_string()];

        let tf = TfIdfIndex::compute_tf(&terms);

        assert!((tf.get("test").unwrap() - 2.0 / 3.0).abs() < 0.001);
        assert!((tf.get("word").unwrap() - 1.0 / 3.0).abs() < 0.001);
    }

    #[test]
    fn test_index_and_search() {
        let index = TfIdfIndex::new();

        let tools = vec![
            Tool {
                name: "create_issue".to_string(),
                description: "Create a new issue in GitHub repository".to_string(),
                input_schema: serde_json::json!({}),
                tool_id: "github::create_issue".to_string(),
                server_name: Some("github".to_string()),
                title: None,
                annotations: None,
            },
            Tool {
                name: "search_code".to_string(),
                description: "Search for code in GitHub repositories".to_string(),
                input_schema: serde_json::json!({}),
                tool_id: "github::search_code".to_string(),
                server_name: Some("github".to_string()),
                title: None,
                annotations: None,
            },
            Tool {
                name: "read_file".to_string(),
                description: "Read contents of a file from filesystem".to_string(),
                input_schema: serde_json::json!({}),
                tool_id: "filesystem::read_file".to_string(),
                server_name: Some("filesystem".to_string()),
                title: None,
                annotations: None,
            },
        ];

        index.build_index(tools);

        let results = index.search("create github issue", 3);
        assert!(!results.is_empty());
        assert_eq!(results[0].tool_id, "github::create_issue");
        assert!(results[0].semantic_score > 0.0);

        let results = index.search("search code", 3);
        assert_eq!(results[0].tool_id, "github::search_code");

        let results = index.search("read file", 3);
        assert_eq!(results[0].tool_id, "filesystem::read_file");
    }

    #[test]
    fn test_cosine_similarity() {
        let mut tf1 = HashMap::new();
        tf1.insert("hello".to_string(), 0.5);
        tf1.insert("world".to_string(), 0.5);

        let mut tf2 = HashMap::new();
        tf2.insert("hello".to_string(), 0.5);
        tf2.insert("world".to_string(), 0.5);

        let mut idf = HashMap::new();
        idf.insert("hello".to_string(), 1.0);
        idf.insert("world".to_string(), 1.0);

        let similarity = TfIdfIndex::cosine_similarity(&tf1, &tf2, &idf);
        assert!((similarity - 1.0).abs() < 0.001);

        let mut tf3 = HashMap::new();
        tf3.insert("different".to_string(), 1.0);
        idf.insert("different".to_string(), 1.0);

        let similarity = TfIdfIndex::cosine_similarity(&tf1, &tf3, &idf);
        assert_eq!(similarity, 0.0);
    }
}
