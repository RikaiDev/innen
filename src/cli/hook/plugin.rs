//! The opencode adapter template written by `hook install`.

pub(super) const OPENCODE_PLUGIN: &str = r#"/**
 * innen stop hook (opencode adapter).
 *
 * Event-driven capture only: on every model dispatch, snapshot the worktree
 * via `innen hook run` (digest-deduped, silent). First turn of each session
 * additionally reports the pending inbox count so the agent drains it.
 * Never throws; never blocks the agent.
 */
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const KB_ROOT = "__KB_ROOT__";
const SEEN = path.join(os.homedir(), ".agents", "state", "innen-stop-hook-seen.json");

function loadSeen() {
  try {
    return JSON.parse(fs.readFileSync(SEEN, "utf8"));
  } catch {
    return {};
  }
}

function pendingCount() {
  try {
    return fs
      .readdirSync(path.join(KB_ROOT, "00-inbox", "harvest"))
      .filter((f) => f.startsWith("pending-") && f.endsWith(".md")).length;
  } catch {
    return 0;
  }
}

function capture(cwd, sessionID) {
  try {
    const argv = ["hook", "run", "--event", "session-idle", "--kb-root", KB_ROOT, "--cwd", cwd];
    if (sessionID) argv.push("--session-id", sessionID);
    spawnSync("innen", argv, {
      timeout: 8000,
      // Forward this event as hook stdin. `hook run` reads cwd, session id,
      // and transcript path from stdin, and an adapter that discards stdin
      // leaves every snapshot without an identity.
      stdio: ["pipe", "ignore", "ignore"],
      input: JSON.stringify({ cwd, session_id: sessionID || undefined }),
    });
  } catch {
    /* fail-open */
  }
}

export default {
  id: "innen-stop-hook",
  async setup(ctx) {
    const controller = new AbortController();
    void (async () => {
      try {
        for await (const event of ctx.event.subscribe({ signal: controller.signal })) {
          // One snapshot per real idle. Subscribing to "context" instead would
          // fire on every model request and mint a receipt per dispatch.
          if (event.type !== "session.idle") continue;
          try {
            capture(process.cwd(), event.sessionID);
          } catch {
            /* fail-open */
          }
        }
      } catch {
        /* fail-open */
      }
    })();

    await ctx.session.hook("context", (event) => {
      const seen = loadSeen();
      if (seen[event.sessionID]) return;
      seen[event.sessionID] = Date.now();
      try {
        fs.mkdirSync(path.dirname(SEEN), { recursive: true });
        fs.writeFileSync(SEEN, JSON.stringify(seen));
      } catch {
        /* fail-open */
      }
      const n = pendingCount();
      event.system.push({
        type: "text",
        text: "[innen-stop-hook] Harvest inbox pending files: " + n + ". If >0, drain before new work (verify, graph/wiki, ingest, delete).",
      });
    });

    return () => controller.abort();
  },
};
"#;
