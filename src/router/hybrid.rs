use std::collections::HashMap;

pub fn expand_query(query: &str) -> String {
    let expansions = [
        ("缺陷", "bug issue defect"),
        ("故障", "incident outage issue"),
        ("测试用例", "test case"),
        ("测试", "test testing"),
        ("需求", "requirement prd specification"),
        ("文档", "document documentation"),
        ("读取", "read file"),
        ("搜索", "search find query"),
        ("创建", "create new"),
        ("更新", "update edit"),
        ("删除", "delete remove"),
    ];

    let mut expanded = query.to_string();
    for (term, aliases) in expansions {
        if query.contains(term) {
            expanded.push(' ');
            expanded.push_str(aliases);
        }
    }
    expanded
}

pub fn reciprocal_rank_fusion<T: Clone>(rankings: Vec<Vec<(String, T)>>, k: f64) -> Vec<(T, f64)> {
    let mut fused: HashMap<String, (T, f64)> = HashMap::new();
    for ranking in rankings {
        for (index, (id, item)) in ranking.into_iter().enumerate() {
            let score = 1.0 / (k + index as f64 + 1.0);
            fused
                .entry(id)
                .and_modify(|(_, total)| *total += score)
                .or_insert((item, score));
        }
    }

    let mut fused: Vec<(T, f64)> = fused.into_values().collect();
    fused.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    fused
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_common_chinese_tool_intents() {
        let expanded = expand_query("帮我登记一个线上故障");
        assert!(expanded.contains("incident"));
        assert!(expanded.contains("issue"));
    }

    #[test]
    fn rrf_promotes_results_seen_by_both_retrievers() {
        let fused = reciprocal_rank_fusion(
            vec![
                vec![("a".to_string(), "a"), ("b".to_string(), "b")],
                vec![("b".to_string(), "b"), ("c".to_string(), "c")],
            ],
            60.0,
        );
        assert_eq!(fused[0].0, "b");
    }
}
