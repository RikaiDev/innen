//! Experimental directed dependency selection, independent of any transcript.
//!
//! A requires edge A -> B means selecting A also selects B. Related edges do
//! not trigger expansion. Scores and costs come from the caller: this module
//! does not infer semantics, tokenize text, or guarantee answer completeness.
//! Greedy marginal relevance/cost is a heuristic, not an optimality claim.
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize)]
pub struct Node {
    pub id: u64,
    pub cost: u64,
    pub relevance: f64,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    Requires,
    Related,
}

#[derive(Debug, Deserialize)]
pub struct Edge {
    pub from: u64,
    pub to: u64,
    pub relation: Relation,
}

/// AND across facts; OR across alternatives; AND across IDs in one alternative.
/// Evidence sufficiency is supplied by the caller, never inferred by this solver.
#[derive(Debug, Deserialize)]
pub struct Fact {
    pub id: String,
    pub alternatives: Vec<BTreeSet<u64>>,
}

#[derive(Debug, thiserror::Error)]
pub enum SelectionError {
    #[error("invalid graph: {0}")]
    Invalid(String),
    #[error("required context costs {required}, exceeding budget {budget}")]
    BudgetInsufficient { required: u64, budget: u64 },
    #[error("no evidence combination fits budget {budget}")]
    FactsInfeasible { budget: u64 },
    #[error("evidence search exceeded {limit} states; feasibility is unknown")]
    SearchLimit { limit: usize },
}

#[derive(Debug, Serialize)]
pub struct Selection {
    pub selected: BTreeSet<u64>,
    pub cost: u64,
    pub budget: u64,
}

pub struct ContextGraph {
    nodes: BTreeMap<u64, Node>,
    requires: BTreeMap<u64, BTreeSet<u64>>,
}

impl ContextGraph {
    /// Exact minimum cost for the supplied finite fact alternatives and graph.
    /// Explicit search exhaustion is not reported as infeasibility or success.
    pub fn select_facts(
        &self,
        budget: u64,
        facts: &[Fact],
        state_limit: usize,
    ) -> Result<Selection, SelectionError> {
        let mut ids = BTreeSet::new();
        let mut closed = Vec::new();
        for fact in facts {
            if fact.id.is_empty()
                || !ids.insert(&fact.id)
                || fact.alternatives.is_empty()
                || fact.alternatives.iter().any(BTreeSet::is_empty)
            {
                return Err(SelectionError::Invalid(
                    "facts require unique IDs and nonempty evidence alternatives".into(),
                ));
            }
            closed.push(
                fact.alternatives
                    .iter()
                    .map(|a| self.closure(a))
                    .collect::<Result<Vec<_>, _>>()?,
            );
        }
        let mut search = FactSearch {
            graph: self,
            alternatives: &closed,
            budget,
            state_limit,
            states: 0,
            best: None,
        };
        search.visit(0, &BTreeSet::new())?;
        search
            .best
            .ok_or(SelectionError::FactsInfeasible { budget })
    }

    pub fn new(nodes: Vec<Node>, edges: Vec<Edge>) -> Result<Self, SelectionError> {
        let mut graph = Self {
            nodes: BTreeMap::new(),
            requires: BTreeMap::new(),
        };
        for node in nodes {
            if node.cost == 0 || !node.relevance.is_finite() || node.relevance < 0.0 {
                return Err(SelectionError::Invalid(
                    "cost must be positive and relevance finite/nonnegative".into(),
                ));
            }
            if graph.nodes.insert(node.id, node).is_some() {
                return Err(SelectionError::Invalid("duplicate node".into()));
            }
        }
        if !graph
            .nodes
            .values()
            .map(|n| n.relevance)
            .sum::<f64>()
            .is_finite()
        {
            return Err(SelectionError::Invalid("total relevance overflow".into()));
        }
        for edge in edges {
            if !graph.nodes.contains_key(&edge.from) || !graph.nodes.contains_key(&edge.to) {
                return Err(SelectionError::Invalid("dangling edge".into()));
            }
            if matches!(edge.relation, Relation::Requires) {
                graph.requires.entry(edge.from).or_default().insert(edge.to);
            }
        }
        Ok(graph)
    }

    pub fn closure(&self, seeds: &BTreeSet<u64>) -> Result<BTreeSet<u64>, SelectionError> {
        if seeds.iter().any(|s| !self.nodes.contains_key(s)) {
            return Err(SelectionError::Invalid("unknown seed".into()));
        }
        let mut result = seeds.clone();
        let mut pending: Vec<_> = seeds.iter().copied().collect();
        while let Some(id) = pending.pop() {
            if let Some(neighbors) = self.requires.get(&id) {
                for &neighbor in neighbors {
                    if result.insert(neighbor) {
                        pending.push(neighbor);
                    }
                }
            }
        }
        Ok(result)
    }

    fn cost(&self, ids: &BTreeSet<u64>) -> Result<u64, SelectionError> {
        ids.iter().try_fold(0u64, |total, id| {
            total
                .checked_add(self.nodes[id].cost)
                .ok_or_else(|| SelectionError::Invalid("cost overflow".into()))
        })
    }

    pub fn select(&self, budget: u64, seeds: &BTreeSet<u64>) -> Result<Selection, SelectionError> {
        let mut selected = self.closure(seeds)?;
        let mut cost = self.cost(&selected)?;
        if cost > budget {
            return Err(SelectionError::BudgetInsufficient {
                required: cost,
                budget,
            });
        }
        let closures: Vec<_> = self
            .nodes
            .keys()
            .map(|&id| self.closure(&BTreeSet::from([id])))
            .collect::<Result<_, _>>()?;
        loop {
            let mut best: Option<(f64, u64, BTreeSet<u64>)> = None;
            for closure in &closures {
                let extra: BTreeSet<_> = closure.difference(&selected).copied().collect();
                if extra.is_empty() {
                    continue;
                }
                let extra_cost = self.cost(&extra)?;
                if extra_cost > budget - cost {
                    continue;
                }
                let gain: f64 = extra.iter().map(|id| self.nodes[id].relevance).sum();
                if gain <= 0.0 {
                    continue;
                }
                let density = gain / extra_cost as f64;
                if best
                    .as_ref()
                    .is_none_or(|(previous, _, _)| density > *previous)
                {
                    best = Some((density, extra_cost, extra));
                }
            }
            let Some((_, extra_cost, extra)) = best else {
                break;
            };
            selected.extend(extra);
            cost += extra_cost;
        }
        Ok(Selection {
            selected,
            cost,
            budget,
        })
    }
}

struct FactSearch<'a> {
    graph: &'a ContextGraph,
    alternatives: &'a [Vec<BTreeSet<u64>>],
    budget: u64,
    state_limit: usize,
    states: usize,
    best: Option<Selection>,
}

impl FactSearch<'_> {
    fn visit(&mut self, index: usize, selected: &BTreeSet<u64>) -> Result<(), SelectionError> {
        if self.states >= self.state_limit {
            return Err(SelectionError::SearchLimit {
                limit: self.state_limit,
            });
        }
        self.states += 1;
        let cost = self.graph.cost(selected)?;
        if cost > self.budget || self.best.as_ref().is_some_and(|b| cost >= b.cost) {
            return Ok(());
        }
        if index == self.alternatives.len() {
            self.best = Some(Selection {
                selected: selected.clone(),
                cost,
                budget: self.budget,
            });
            return Ok(());
        }
        for alternative in &self.alternatives[index] {
            let next = selected.union(alternative).copied().collect();
            self.visit(index + 1, &next)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn node(id: u64, cost: u64, relevance: f64) -> Node {
        Node {
            id,
            cost,
            relevance,
        }
    }
    fn edge(from: u64, to: u64, relation: Relation) -> Edge {
        Edge { from, to, relation }
    }

    #[test]
    fn fact_alternatives_share_evidence_and_do_not_choose_each_cheapest_independently() {
        let g = ContextGraph::new(vec![node(1, 5, 0.), node(2, 5, 0.), node(3, 7, 0.)], vec![])
            .unwrap();
        let facts = vec![
            Fact {
                id: "a".into(),
                alternatives: vec![BTreeSet::from([1]), BTreeSet::from([3])],
            },
            Fact {
                id: "b".into(),
                alternatives: vec![BTreeSet::from([2]), BTreeSet::from([3])],
            },
        ];
        assert_eq!(
            g.select_facts(7, &facts, 100).unwrap().selected,
            BTreeSet::from([3])
        );
        assert!(matches!(
            g.select_facts(6, &facts, 100),
            Err(SelectionError::FactsInfeasible { .. })
        ));
    }

    #[test]
    fn fact_alternatives_keep_and_members_and_required_dependencies() {
        let g = ContextGraph::new(
            vec![node(1, 2, 0.), node(2, 2, 0.), node(3, 2, 0.)],
            vec![edge(2, 3, Relation::Requires)],
        )
        .unwrap();
        let facts = vec![Fact {
            id: "a".into(),
            alternatives: vec![BTreeSet::from([1, 2])],
        }];
        assert_eq!(
            g.select_facts(6, &facts, 100).unwrap().selected,
            BTreeSet::from([1, 2, 3])
        );
        assert!(matches!(
            g.select_facts(6, &facts, 1),
            Err(SelectionError::SearchLimit { .. })
        ));
        assert!(g
            .select_facts(
                6,
                &[Fact {
                    id: "bad".into(),
                    alternatives: vec![BTreeSet::new()]
                }],
                100
            )
            .is_err());
    }

    #[test]
    fn direction_and_related_edges_do_not_pull_entire_history() {
        let g = ContextGraph::new(
            vec![node(1, 10, 0.), node(2, 10, 1.), node(3, 100, 0.)],
            vec![
                edge(2, 1, Relation::Requires),
                edge(2, 3, Relation::Related),
            ],
        )
        .unwrap();
        assert_eq!(
            g.closure(&BTreeSet::from([1])).unwrap(),
            BTreeSet::from([1])
        );
        assert_eq!(
            g.select(20, &BTreeSet::from([2])).unwrap().selected,
            BTreeSet::from([1, 2])
        );
    }

    #[test]
    fn required_context_cannot_be_silently_cut_to_fit() {
        let g = ContextGraph::new(
            vec![node(1, 10, 0.), node(2, 10, 1.)],
            vec![edge(2, 1, Relation::Requires)],
        )
        .unwrap();
        assert!(matches!(
            g.select(19, &BTreeSet::from([2])),
            Err(SelectionError::BudgetInsufficient { required: 20, .. })
        ));
    }

    #[test]
    fn cycles_terminate_and_shared_dependencies_are_charged_once() {
        let g = ContextGraph::new(
            vec![node(1, 10, 1.), node(2, 10, 1.), node(3, 10, 1.)],
            vec![
                edge(1, 2, Relation::Requires),
                edge(2, 1, Relation::Requires),
                edge(3, 2, Relation::Requires),
            ],
        )
        .unwrap();
        assert_eq!(g.select(30, &BTreeSet::new()).unwrap().cost, 30);
        assert_eq!(g.closure(&BTreeSet::from([3])).unwrap().len(), 3);
    }

    #[test]
    fn malformed_graph_and_cost_overflow_are_explicit_errors() {
        assert!(ContextGraph::new(vec![node(1, 1, f64::NAN)], vec![]).is_err());
        assert!(
            ContextGraph::new(vec![node(1, 1, 0.)], vec![edge(1, 2, Relation::Related)]).is_err()
        );
        let g = ContextGraph::new(vec![node(1, u64::MAX, 0.), node(2, 1, 0.)], vec![]).unwrap();
        assert!(g.select(u64::MAX, &BTreeSet::from([1, 2])).is_err());
    }
}
