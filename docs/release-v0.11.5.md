# innen v0.11.5

Found by running the v0.11.4 upgrade procedure on a real machine, not by
reading the code: `hook install --agent grok` left the old handler in place
beside the new one.

## Fixed

### `hook install --agent grok` accumulated stale handlers

`install_claude` and `install_codex` call `remove_hook_commands` before merging,
so re-running them replaces the handlers an earlier innen wrote. `install_grok`
called no removal at all — it merged, and nothing else.

Every emitted command embeds `std::env::current_exe()`. The string therefore
changes whenever the binary moves: a bare `innen` resolved through PATH becomes
`/opt/homebrew/bin/innen`, an old prefix becomes a new one. Each such change
appended a dead copy of the previous command, and both copies then ran on every
session.

Measured on the machine that motivated this, after upgrading to v0.11.4 and
re-running the install:

```json
"SessionEnd":   [ {"command": "innen hook run --event session-end --kb-root …"},
                  {"command": "/opt/homebrew/bin/innen hook run --event session-end --kb-root …"} ],
"SessionStart": [ {"command": "innen hook pending --root …"},
                  {"command": "/opt/homebrew/bin/innen hook pending --root …"} ]
```

Four handlers where two belong. `install_grok` now performs the same
remove-then-merge as its two siblings.

`install_antigravity` and `install_opencode` were checked and are unaffected:
the first replaces the whole `innen-progress` key with `root.insert`, the
second rewrites the plugin file outright. Neither merges.

## Changed

| | before | after |
| --- | --- | --- |
| `incremental` compilation setting | declared twice, in `~/.cargo/config.toml` (`profile.dev`, `profile.test`) and in `.cargo/config.toml` (`CARGO_INCREMENTAL`) — opposite values | declared once, in `Cargo.toml` (`[profile.dev]`, `[profile.test]`) |

The two files disagreed about whether this workspace compiles incrementally, and
the disagreement was not theoretical: `target/debug/incremental` measured
1.2 GiB on the machine above, while the repository config explicitly declared
the opposite ("sccache and incremental are mutually exclusive; cache wins").
Cargo reads profiles from the manifest, so the decision now lives there, and
`.cargo/config.toml` points at it instead of restating it. CI does not set
`CARGO_INCREMENTAL`, so the repository's `[env]` entry already applied there —
this is a relocation, not a behaviour change.

Cargo does not reclaim `target/`. The same directory held 20 copies of the
49 MiB `innen` binary and 298 test binaries totalling 2.6 GiB. Turning off
incremental stops one source of that growth; it is not a substitute for
occasional cleanup.

## Measured

| | before | after |
| --- | --- | --- |
| `hook_install_grok_replaces_stale_handlers_with_absolute_executable` | fails: `SessionEnd kept a stale handler beside the new one`, 2 ≠ 1 | passes |
| innen handlers in `~/.grok/hooks/innen-progress.json` after re-running install | 4 | 2 |
| `target/debug/incremental` after a build | 1.2 GiB | 0 B |

`cargo fmt --all --check` and `python3 scripts/check-structure.py` clean (184
files). The clippy and workspace test results for this tag are the CI run that
built it; the local machine could not run them — its APFS container had 11.6 GB
free, below the 8 GiB running floor the build coordinator enforces, and two
attempts were stopped for that reason rather than by a test failure.

## Note on verification

The claim that CI is green for this tag is evidence, not a formality: the
previous release in this series was verified both locally and on CI, and this
one only on CI. Both are stated rather than merged into one claim.
