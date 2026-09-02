//! Reciprocal Rank Fusion over heterogeneous retrieval rankings.
//!
//! Replaces the previous synonym-expansion + RRF module. The synonym
//! expansion has been retired -- BGE-M3's learned sparse vector captures
//! the same cross-language intent signal natively.

use std::collections::HashMap;

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

    #[test]
    fn rrf_handles_single_lane() {
        let fused = reciprocal_rank_fusion(
            vec![vec![
                ("a".to_string(), 1),
                ("b".to_string(), 2),
                ("c".to_string(), 3),
            ]],
            60.0,
        );
        assert_eq!(fused.len(), 3);
        // k=60: scores are 1/61, 1/62, 1/63 -> descending order preserved.
        assert_eq!(fused[0].0, 1);
    }

    #[test]
    fn rrf_empty_rankings() {
        let fused: Vec<(i32, f64)> = reciprocal_rank_fusion(vec![], 60.0);
        assert!(fused.is_empty());
    }
}
