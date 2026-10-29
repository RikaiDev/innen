# Migration notes

Version-specific migration notes moved out of the README. The current
contract lives in the README sections referenced from the top.

## Output migration (project view)

**Output migration:** consumers of the previous `{id,render}` project response
must use `project <id> --view full`. That view preserves the legacy page contract,
including its unfiltered Tasks section. `--view evidence` includes terminal tasks,
merged node metadata and append-only history with event IDs and source line numbers.
The default brief is a navigation layer, not lossless compression of that history.

## v0.3.0 migration

Project lookup now accepts canonical IDs, namespace-free slugs, unique labels,
unique basenames, and absolute paths containing a `/Workspace/` suffix. An
ambiguous shorthand fails with candidate IDs instead of guessing.

`artifact add-tree` archives a content-addressed directory manifest, not the
source bytes. Keep or separately archive the source directory until another
verified byte copy exists. Accepted task-proposal reviews now require explicit
premises with supported, missing, or counterevidence spans; source-valid model
text alone remains insufficient.

Conversation grammar, evidence contracts, and task proposals remain opt-in;
existing conversation/query defaults are unchanged.

## v0.2.0 migration

Register the knowledge-base root once with an absolute path:

```bash
innen config set --global root /absolute/path/to/knowledge-base
```

An explicit `--root` takes precedence, followed by a non-empty `INNEN_ROOT`;
the registered global root is used otherwise. The root must be absolute.

The default `query` view is now `context`; use `--view hits` for the legacy
lexical-plus-graph hit shape and `--view evidence` for expanded evidence. The
default `project` output is the compact brief; use `project <id> --view full`
for the legacy full page. `resume <session-id>` emits the bounded brief by
default; use `--view context`, `--view dialogue`, or `--view events` when a
different page is needed.

Verify which executable is being used when Homebrew and a local wrapper are
both installed:

```bash
command -v innen
innen --version
```

`~/.local/bin/innen` (a dev build from `scripts/dev-install.sh`) shadows the
Homebrew installation until it is retired; see "Build from source" below.

The repository also contains local research harnesses for evaluating output
length and retrieval fidelity. They are excluded from the release package and
their private corpora are not release evidence; output size is not billed-token
savings or a comprehension guarantee.
