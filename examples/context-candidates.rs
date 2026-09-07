use innen_core::context_candidates::{extract, TextEvent};
use std::io::Read;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let events: Vec<TextEvent> = serde_json::from_str(&input)?;
    let candidates = extract(&events).map_err(std::io::Error::other)?;
    println!("{}", serde_json::to_string(&candidates)?);
    Ok(())
}
