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

import type { Target, TargetList } from "./protocol";

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
export function listTargets(): TargetList {
  const empty: TargetList = { preferred: null, self: null, targets: [] };
  // `__vn_devtools_targets` is registered by every DEV host and by no prod one
  // (`devtools::install`), so the probe is the honest question rather than a
  // formality — which is why protocol.d.ts declares it a plain function and
  // this is the guard.
  if (typeof globalThis.__vn_devtools_targets !== "function") {
    return {
      ...empty,
      error:
        "__vn_devtools_targets is not registered — this looks like a `--features prod` host, " +
        "which has no devtools at all",
    };
  }
  try {
    return { ...empty, ...(JSON.parse(globalThis.__vn_devtools_targets()) as Partial<TargetList>) };
  } catch (e) {
    return { ...empty, error: `the target list did not parse: ${e}` };
  }
}

// One command at a time, in order. Every command costs the TARGET a frame
// (docs/DEVTOOLS.md: the queue is drained one per `pump_frame`), so firing a
// poll while another is in flight buys nothing and makes the failure mode
// ("which of these three timed out?") much harder to read.
let tail: Promise<unknown> = Promise.resolve();

function enqueue<T>(run: () => Promise<T>): Promise<T> {
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
 *
 * `T` is the caller's claim about the `data` shape, checked against nothing:
 * this is a wire, and the reply is whatever the host on the other end sent.
 * `./protocol.ts` is where those shapes are written down from the Rust that
 * builds them, so the claim has one place to be right or wrong.
 */
export function send<T>(
  target: Target | null,
  cmd: string,
  args: Record<string, unknown> = {},
  timeoutMs = TIMEOUT_MS,
): Promise<T> {
  if (!target) return Promise.reject(new Error("not attached to a target"));
  return enqueue(() => request<T>(target, cmd, args, timeoutMs));
}

async function request<T>(
  target: Target,
  cmd: string,
  args: Record<string, unknown>,
  timeoutMs: number,
): Promise<T> {
  const body = JSON.stringify({ token: target.token, cmd, ...args });
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  // `engine.d.ts` types `fetch` as possibly absent, and it means it: an
  // embedder can build redjs without it. For THIS app that is not a degraded
  // mode — HTTP on the devtools port is the entire transport (§3) — so it is
  // one guard with the reason attached rather than a silent `undefined` call.
  const http = fetch;
  if (!http) {
    throw new Error(
      `${cmd}: this host has no \`fetch\`, and the inspector's only transport is HTTP on the ` +
        `target's devtools port (docs/DEVTOOLS-UI.md §3)`,
    );
  }
  let res;
  try {
    res = await http(`http://127.0.0.1:${target.port}/`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body,
      signal: controller.signal,
    });
  } catch (e) {
    throw new Error(
      `${cmd}: cannot reach ${target.slot} on port ${target.port} — the host exited, or its ` +
        `frame pump is wedged (${message(e)})`,
    );
  } finally {
    clearTimeout(timer);
  }
  // Same story as `fetch` above: `VnFetchResponse.text` is declared optional
  // because engine.d.ts types only what the voice shim reads. redjs's response
  // has it (`webapi::fetch`), and this app cannot work without it.
  if (!res.text) throw new Error(`${cmd}: this host's fetch response has no \`text()\``);
  const text = await res.text();
  let reply: { ok?: boolean; error?: string; data?: T };
  try {
    reply = JSON.parse(text);
  } catch {
    throw new Error(
      `${cmd}: the host answered with something that is not JSON: ${text.slice(0, 200)}`,
    );
  }
  if (!reply.ok) throw new Error(reply.error || `${cmd}: the host refused the command`);
  return reply.data as T;
}

/**
 * A thrown value's message.
 *
 * `catch (e)` is `unknown` under `strict`, and it genuinely is: `fetch` rejects
 * with an `Error`, but an app being inspected can `throw` anything at all and
 * the console has to print SOMETHING either way.
 */
export function message(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (e && typeof e === "object" && typeof (e as { message?: unknown }).message === "string") {
    return (e as { message: string }).message;
  }
  return String(e);
}
