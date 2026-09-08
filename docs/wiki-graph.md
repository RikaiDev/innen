# Wiki graph projection

`innen wiki sync` explicitly projects Markdown below `<root>/02-wiki/` into
the append-only journal. It does not run as a hidden side effect of query,
harvest, or graph reads.

Each page becomes a custom `Wiki` node with its title, tags, Markdown body,
absolute `path`, relative `wiki_path`, content `sha256`, and
`managed_by: innen:wiki-sync`. An explicit safe `node_id` (or legacy `id`) is
used on first projection. Otherwise the initial relative path deterministically
mints `wiki:<sha256(wiki_path)>`. The stored `wiki_path` registration remains
authoritative for later updates. A page that disappears is retained and marked
`missing: true`; restoring the registered path restores the same identity.

The sync is a two-pass projection, so a source or related wikilink can resolve
to another page regardless of file order. `sources` create `DERIVED_FROM`
edges. `related` creates custom `RELATED_TO` edges only when a wiki path or
basename resolves unambiguously. Missing and ambiguous references are warnings.

Source reference behavior:

- `graph:<node-id>` and an existing literal node ID target that node.
- A wiki path, basename, or wikilink targets the matching `Wiki` node.
- `conversation:<source>:<session-id>` creates a `Source` node with a
  conversation locator.
- `http://` and `https://` references create a `Source` node with a URL
  locator. Sync never fetches the URL.
- Other references create a `Source` node with an absolute file locator.
  `~/` expands to the current home directory. Sync records the locator without
  opening the target. Relative paths that escape the knowledge-base root are
  skipped with a warning.

Source IDs are `source:<sha256(source_ref)>`. Projection-owned edges carry
per-reference provenance. A later sync asserts only missing edges and retracts
only obsolete projection-owned relationships. Foreign edges are retained.

Before opening the journal, sync recursively enumerates regular `*.md` files,
without following symlinks, and validates every page. Frontmatter must be one
YAML mapping with `title`, `tags`, `sources`, and `related`; list fields accept
a scalar, a list of scalars, or YAML null as an empty list. Invalid YAML,
missing fields, unsafe explicit IDs, or identity collisions fail explicitly.
Because this preflight finishes before journal access, malformed frontmatter
cannot leave a partial graph projection.
