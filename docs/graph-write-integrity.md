# Graph write integrity

`innen graph relate` resolves `--from` and node-valued `--to` as exact node
IDs in the current journal before appending an edge. A missing endpoint, an
unreadable journal, or any malformed non-empty journal line stops the command
without appending, repairing, rewriting, or quarantining journal data.

URI targets remain valid without a node when the graph adjacency contract
allows them. They must start with `/` or contain `://`. For built-in edge
types, URI targets are accepted only for legal `LOCATED_AT` or
`ORIGINATED_AT` relationships. Custom edges retain the existing custom-party
provenance and adjacency contract.

Use `--allow-dangling` only for an intentional migration or out-of-order graph
ingestion. The flag permits missing node endpoints; it does not bypass journal
integrity, URI legality, custom-party provenance, or adjacency validation when
both endpoint kinds exist. It never performs alias or project-name resolution.

Historical dangling edges remain readable. `innen doctor` continues to report
them under its existing health contract.
