use innen_core::semantic_proposals::{validate_source, Proposal};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("usage: semantic-proposals <source.jsonl> <proposal.json>".into());
    }
    let raw = std::fs::read(&args[0])?;
    let proposal: Proposal = serde_json::from_slice(&std::fs::read(&args[1])?)?;
    let packet = validate_source(&raw, proposal).map_err(std::io::Error::other)?;
    println!("{}", serde_json::to_string(&packet)?);
    Ok(())
}
