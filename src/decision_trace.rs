use crate::governance::{ErrorCategory, SideEffect};
use crate::router::RankedTool;
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::RwLock;

const MAX_TRACES: usize = 200;

#[derive(Debug, Clone, Serialize)]
pub struct SearchTrace {
    pub query: String,
    pub candidate_count: usize,
    pub selected_tools: Vec<String>,
    pub strategy: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct InvocationTrace {
    pub tool_id: String,
    pub side_effect: SideEffect,
    pub confirmation_required: bool,
    pub confirmed: bool,
    pub attempts: u8,
    pub outcome: String,
    pub error_category: Option<ErrorCategory>,
    pub latency_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DecisionTrace {
    pub trace_id: String,
    pub timestamp: DateTime<Utc>,
    pub search: Option<SearchTrace>,
    pub invocation: Option<InvocationTrace>,
}

#[derive(Clone, Default)]
pub struct DecisionTraceStore {
    traces: Arc<RwLock<VecDeque<DecisionTrace>>>,
}

impl DecisionTraceStore {
    pub async fn record_search(
        &self,
        query: &str,
        candidates: &[RankedTool],
        strategy: &str,
    ) -> String {
        let trace_id = new_trace_id();
        let trace = DecisionTrace {
            trace_id: trace_id.clone(),
            timestamp: Utc::now(),
            search: Some(SearchTrace {
                query: query.to_string(),
                candidate_count: candidates.len(),
                selected_tools: candidates.iter().map(|tool| tool.tool_id.clone()).collect(),
                strategy: strategy.to_string(),
            }),
            invocation: None,
        };
        self.push(trace).await;
        trace_id
    }

    pub async fn record_invocation(&self, invocation: InvocationTrace) -> String {
        let trace_id = new_trace_id();
        let trace = DecisionTrace {
            trace_id: trace_id.clone(),
            timestamp: Utc::now(),
            search: None,
            invocation: Some(invocation),
        };
        self.push(trace).await;
        trace_id
    }

    pub async fn get(&self, trace_id: &str) -> Option<DecisionTrace> {
        self.traces
            .read()
            .await
            .iter()
            .find(|trace| trace.trace_id == trace_id)
            .cloned()
    }

    async fn push(&self, trace: DecisionTrace) {
        let mut traces = self.traces.write().await;
        if traces.len() == MAX_TRACES {
            traces.pop_front();
        }
        traces.push_back(trace);
    }
}

fn new_trace_id() -> String {
    format!("trc-{}", Utc::now().format("%Y%m%d%H%M%S%6f"))
}
