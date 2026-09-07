//! Development harness: one JSON request per stdin line; no Python runtime needed.
use innen_core::context_selection::{ContextGraph, Edge, Fact, Node};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::io::BufRead;

#[derive(Deserialize)]
struct Request {
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    budget: u64,
    #[serde(default)]
    seeds: BTreeSet<u64>,
    facts: Option<Vec<Fact>>,
    state_limit: Option<usize>,
}

fn main() {
    for line in std::io::stdin().lock().lines() {
        let result = (|| -> Result<_, Box<dyn std::error::Error>> {
            let input: Request = serde_json::from_str(&line?)?;
            let graph = ContextGraph::new(input.nodes, input.edges)?;
            Ok(if let Some(facts) = input.facts {
                graph.select_facts(input.budget, &facts, input.state_limit.unwrap_or(100_000))?
            } else {
                graph.select(input.budget, &input.seeds)?
            })
        })();
        match result {
            Ok(selection) => println!("{}", serde_json::to_string(&selection).unwrap()),
            Err(error) => println!("{}", serde_json::json!({"error":error.to_string()})),
        }
    }
}
