use innen_core::context_evidence::{describe_sequence, EvidenceInput};
use std::io::Read;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text)?;
    let inputs: Vec<EvidenceInput> = serde_json::from_str(&text)?;
    let output = describe_sequence(inputs);
    println!("{}", serde_json::to_string(&output)?);
    Ok(())
}
