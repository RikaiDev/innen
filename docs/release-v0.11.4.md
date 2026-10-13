# innen v0.11.4

One defect, found by using the tool on a machine that installs the hooks. The
installed `SessionStart` command was one the CLI itself rejected.

## Fixed

### `hook install` wrote a SessionStart command the CLI refuses to run

`pending_command` emitted

```
innen hook pending --kb-root <kb>
```

`HookOp::Pending` is a unit variant (`src/cli/hook/mod.rs:31`) and takes no
arguments at all. `--kb-root` belongs to `HookRunArgs` (same file, line 72), so
clap rejected it on every invocation:

```
error: unexpected argument '--kb-root' found
Usage: innen hook pending [OPTIONS]
```

Every session start therefore logged a non-blocking hook error, exited 2, and
returned no pending context. Agent transcripts record it from 2026-09-29
onward across four workspaces. `hook run --kb-root` was never affected — the
broken string was the only broken one.

The emitted flag is now `--root`.

### Why `--root` works, and why that is worth a test

`--root` is not an argument of `hook pending` either. It is a global flag on
the top-level `Cli` (`#[arg(long, global = true)]`, `src/cli/types.rs:47`),
which is why clap accepts it after the subcommand.

That distinction decides what the regression test has to assert. A test that
checked the installed string contained `--root` would keep passing if someone
later dropped `global = true` from that flag — while every installed hook died
on every session start again, with nothing failing.

`hook_install_session_start_command_is_accepted_by_the_cli` therefore hands the
installed command to `sh -c` and requires it to succeed and print
`{"pending":[]}`. `current_exe()` resolves to the `target/debug/innen` built
for that run, so the command is verified against the binary that produced it.
Against the unfixed source it fails with exit 2.

## Changed

| | before | after |
| --- | --- | --- |
| `hook install` SessionStart command | `hook pending --kb-root <kb>` | `hook pending --root <kb>` |

No behaviour change for `hook run`, `hook install`, or the pending list itself.
`hook install` remains remove-then-rewrite: it calls
`remove_hook_commands(…, "hook pending")` before merging, so re-running it after
upgrading replaces every pending handler rather than accumulating them.

## Measured

| | before | after |
| --- | --- | --- |
| `hook pending --kb-root` on the installed binary | exit 2, `unexpected argument` | not emitted |
| `hook pending --root` | exit 0, `{"pending":[…]}` | exit 0 |
| regression test on the unfixed source | fails, exit 2 | passes |
| `cargo test --test hook_cli` | — | 11 passed, 0 failed |
| `cargo test --workspace --all-features` | — | 408 passed, 0 failed |

`cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
clean. `cargo fmt --all --check` clean. `python3 scripts/check-structure.py` clean
(184 files).

## Note

Until this version reaches Homebrew, re-running `hook install` on 0.11.3 writes
the broken flag back into every agent config it manages. A hand-edit of the
config files is a stopgap, not a fix: `hook install` is the only writer, and
every pre-release invocation overwrites it.
