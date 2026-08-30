// The inspector's transport: the devtools protocol over HTTP.
//
// docs/DEVTOOLS.md's wire is newline-delimited JSON over a localhost TCP port.
// JS in this runtime cannot open a TCP socket — but it CAN `fetch()`, and
// `fetch` here is a real, off-thread, promise-shaped HTTP client. So the host's
// devtools listener learned to answer HTTP on the SAME port (`POST /` with the
// request object as the body, one command per connection), and this file is the
// whole client: ~40 lines, no new protocol, no relay process, and the exact
// same commands `ctl.ts` and `vn-test` send.
//
// Discovery is the one thing fetch cannot do — a handshake file has to be read
// off disk — so it comes from `__vn_devtools_targets()`, a dev-only host fn
// that lists every `<dist>/devtools.json` it can find and probes each port.
// See docs/DEVTOOLS-UI.md §3.

/** How long a single command may take before the UI stops waiting for it. */
const TIMEOUT_MS = 8000;

/**
 * Every attachable instance, right now.
 *
 * Returns `{ preferred, self, targets, error }`. `preferred` is the slot
 * `--target` named (via `VN_DEVTOOLS_TARGET`); `targets[].self` marks THIS
 * process, which is attachable too — the inspector inspecting itself is a
 * legitimate (and useful) thing to do.
 */
export function listTargets() {
  const empty = { preferred: null, self: null, targets: [] };
  if (typeof globalThis.__vn_devtools_targets !== "function") {
    return {
      ...empty,
      error:
        "__vn_devtools_targets is not registered — this looks like a `--features prod` host, " +
        "which has no devtools at all",
    };
  }
  try {
    return { ...empty, ...JSON.parse(globalThis.__vn_devtools_targets()) };
  } catch (e) {
    return { ...empty, error: `the target list did not parse: ${e}` };
  }
}

// One command at a time, in order. Every command costs the TARGET a frame
// (docs/DEVTOOLS.md: the queue is drained one per `pump_frame`), so firing a
// poll while another is in flight buys nothing and makes the failure mode
// ("which of these three timed out?") much harder to read.
let tail = Promise.resolve();

function enqueue(run) {
  const result = tail.then(run, run);
  // The chain must survive a rejection: one failed poll cannot be allowed to
  // wedge every command after it.
  tail = result.then(
    () => {},
    () => {},
  );
  return result;
}

/**
 * One devtools command against `target`, unwrapped to its `data`.
 *
 * A `{ok:false}` reply becomes a thrown `Error` carrying the host's own words —
 * those messages are written to be read by a human (they list the candidates
 * when a selector is ambiguous, they name the unsupported selector syntax), so
 * the UI shows them verbatim rather than inventing its own.
 */
export function send(target, cmd, args = {}, timeoutMs = TIMEOUT_MS) {
  if (!target) return Promise.reject(new Error("not attached to a target"));
  return enqueue(() => request(target, cmd, args, timeoutMs));
}

async function request(target, cmd, args, timeoutMs) {
  const body = JSON.stringify({ token: target.token, cmd, ...args });
  const controller = typeof AbortController === "function" ? new AbortController() : null;
  const timer = controller
    ? setTimeout(() => controller.abort(), timeoutMs)
    : null;
  let res;
  try {
    res = await fetch(`http://127.0.0.1:${target.port}/`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body,
      ...(controller ? { signal: controller.signal } : {}),
    });
  } catch (e) {
    throw new Error(
      `${cmd}: cannot reach ${target.slot} on port ${target.port} — the host exited, or its ` +
        `frame pump is wedged (${e && e.message ? e.message : e})`,
    );
  } finally {
    if (timer !== null) clearTimeout(timer);
  }
  const text = await res.text();
  let reply;
  try {
    reply = JSON.parse(text);
  } catch {
    throw new Error(`${cmd}: the host answered with something that is not JSON: ${text.slice(0, 200)}`);
  }
  if (!reply.ok) throw new Error(reply.error || `${cmd}: the host refused the command`);
  return reply.data;
}
