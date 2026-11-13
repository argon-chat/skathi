/** Windows native inspector memory/profile regression.
 * bun scripts/aot/test-memory.ts --host <prod+native-aot bare host> --manifest <inspector.native.json>
 *   --payload <synthetic app.vnpk> --out <fresh output directory> [--cycles 300]
 * The supplied payload is executed; use a synthetic fixture, never a real profile.
 */
import { spawn, type ChildProcess } from "node:child_process";
import { mkdirSync, existsSync, openSync, closeSync, readFileSync, writeFileSync } from "node:fs";
import { resolve, join } from "node:path";
import { Wire, readHandshake } from "../../packages/vn-test/src/wire";

if (process.platform !== "win32") throw Error("This probe measures the Windows host");
const values = new Map<string, string>();
const allowed = new Set(["--host", "--manifest", "--payload", "--out", "--cycles", "--media-selector", "--profiles"]);
for (let i = 2; i < process.argv.length; i += 2) {
  const key = process.argv[i]!,
    value = process.argv[i + 1];
  if (!allowed.has(key) || !value || value.startsWith("--") || values.has(key)) throw Error(`Invalid option ${key}`);
  values.set(key, value);
}
const required = (key: string) => {
  const value = values.get(key);
  if (!value) throw Error(`${key} is required`);
  return resolve(value);
};
const host = required("--host"),
  manifest = required("--manifest"),
  payload = required("--payload"),
  out = required("--out");
const cycles = Number(values.get("--cycles") ?? "300");
if (!Number.isInteger(cycles) || cycles < 10 || cycles > 10000) throw Error("--cycles must be 10..10000");
const profiles = (values.get("--profiles") ?? "default,compact").split(",");
if (
  !profiles.length ||
  profiles.some((p) => !["default", "compact", "auto"].includes(p)) ||
  new Set(profiles).size !== profiles.length
)
  throw Error("--profiles accepts unique default,compact,auto values");
if (existsSync(out)) throw Error("Choose a fresh --out directory; stale handshakes are refused");
mkdirSync(out, { recursive: true });
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const save = (name: string, value: unknown) => writeFileSync(join(out, name), JSON.stringify(value, null, 2) + "\n");
const children: ChildProcess[] = [],
  wires: Wire[] = [];
async function start(name: string, profile?: "default" | "compact" | "auto", target?: string) {
  const dist = join(out, name, "dist");
  mkdirSync(dist, { recursive: true });
  const env: Record<string, string | undefined> = {
    ...process.env,
    VN_DIST: dist,
    VN_DATA_DIR: join(out, name, "profile"),
    APPDATA: join(out, name, "roaming"),
    LOCALAPPDATA: join(out, name, "local"),
    VN_APP_NAME: `NativeMemoryProbe-${process.pid}-${name}`,
    VN_SINGLE_INSTANCE: "0",
    VN_WINDOW_FOCUS: "never",
    VN_DEVTOOLS: "1",
    VN_DEVTOOLS_RAISE: "off",
    VN_WINDOW: "900x640",
    VN_OWNER_PID: String(process.pid),
    VN_MEDIA_CAPTURE: "deny",
  };
  for (const key of [
    "VN_PAYLOAD",
    "VN_DEVTOOLS_NATIVE_IMAGE",
    "VN_AOT_MEMORY_PROFILE",
    "JSC_structureHeapSizeInKB",
    "JSC_markedBlockCageSizeInMB",
  ])
    delete env[key];
  if (profile) {
    env.VN_DEVTOOLS_NATIVE_IMAGE = manifest;
    if (profile !== "auto") env.VN_AOT_MEMORY_PROFILE = profile;
  } else env.VN_PAYLOAD = payload;
  const log = join(out, name, "host.log"),
    fd = openSync(log, "w");
  const child = spawn(host, profile ? ["--devtools", "--target", target!] : [], {
    cwd: out,
    env,
    stdio: ["ignore", fd, fd],
  });
  closeSync(fd);
  children.push(child);
  let spawnError: Error | undefined;
  child.on("error", (error) => {
    spawnError = error;
  });
  let wire: Wire | undefined;
  const deadline = Date.now() + 60000;
  while (Date.now() < deadline) {
    if (spawnError) throw Error(`${name} failed to spawn: ${spawnError.message}`);
    if (child.exitCode !== null || child.signalCode !== null) throw Error(`${name} exited; see ${log}`);
    if (existsSync(join(dist, "devtools.json"))) {
      const h = readHandshake(dist);
      if (h.pid === child.pid) {
        const candidate = new Wire(h, 20000);
        try {
          await candidate.send("ping");
          wire = candidate;
          break;
        } catch {
          candidate.close();
        }
      }
    }
    await sleep(50);
  }
  if (!wire) throw Error(`${name} did not publish its own handshake; see ${log}`);
  wires.push(wire);
  for (let i = 0; i < 200; i++) {
    const perf = await wire.send("perf");
    if (perf.nodes > 1) break;
    await sleep(50);
  }
  return { child, wire, dist, log };
}
async function evaluate(wire: Wire, js: string) {
  const result = await wire.send("eval", { js });
  if (result.exception) throw Error(result.exception);
  return result.value;
}
async function until(read: () => Promise<string>, text: string) {
  const deadline = Date.now() + 15000;
  let last = "";
  do {
    last = await read();
    if (last.includes(text)) return last;
    await sleep(50);
  } while (Date.now() < deadline);
  throw Error(`Did not observe ${text}: ${last.slice(0, 100)}`);
}
interface MediaState {
  ready: number;
  width: number;
  height: number;
  duration: number;
  time: number;
  error: unknown;
}
const summary: Record<string, unknown>[] = [];
try {
  const target = await start("target");
  const mediaSelector = values.get("--media-selector");
  if (mediaSelector) {
    const selector = JSON.stringify(mediaSelector);
    const mediaState = async (): Promise<MediaState> =>
      JSON.parse(
        String(
          await evaluate(
            target.wire,
            `JSON.stringify((()=>{
      const v=document.querySelector(${selector}); if(!v)throw Error('Missing synthetic media');
      return {ready:v.readyState,width:v.videoWidth,height:v.videoHeight,duration:v.duration,time:v.currentTime,error:v.error&&v.error.message};
    })())`,
          ),
        ),
      );
    const poll = async (predicate: (state: MediaState) => boolean) => {
      const end = Date.now() + 20000;
      let state: MediaState;
      do {
        state = await mediaState();
        if (predicate(state)) return state;
        await sleep(50);
      } while (Date.now() < end);
      throw Error(`Media did not reach expected state: ${JSON.stringify(state)}`);
    };
    const metadata = await poll((s) => s.ready >= 1 && s.width > 0 && s.duration > 0);
    await evaluate(
      target.wire,
      `(()=>{const v=document.querySelector(${selector});v.muted=true;v.play().catch(e=>console.error('synthetic media play',e));return true})()`,
    );
    const played = await poll((s) => s.time > 0 && s.ready >= 2);
    const seek = Math.min(metadata.duration / 2, 0.5);
    await evaluate(
      target.wire,
      `(()=>{const v=document.querySelector(${selector});v.pause();v.currentTime=${seek};return true})()`,
    );
    const sought = await poll((s) => Math.abs(s.time - seek) < 0.15);
    const memory = await target.wire.send("mem", { gc: true, regions: true });
    const shot = await target.wire.send("screenshot", { path: join(out, "packaged-video.png") });
    save("packaged-video.json", { metadata, played, sought, seek, memory, screenshot: shot });
  }
  await evaluate(
    target.wire,
    `(() => {
    const root=document.createElement('div');root.id='memory-probe';
    for(let i=0;i<1200;i++){const el=document.createElement('span');el.textContent='synthetic node '+i;root.appendChild(el);}
    document.body.appendChild(root);return true;
  })()`,
  );
  for (const selected of profiles) {
    const profile = selected as "default" | "compact" | "auto";
    const app = await start(profile, profile, target.dist);
    const text = async () => String((await app.wire.send("text", { selector: ".app" })).text);
    await until(text, String(target.child.pid));
    await until(text, "live");
    const active = async () => String((await app.wire.send("text", { selector: ".tabs .tab.on" })).text);
    const switchPanel = async (index: number, label: string) => {
      await app.wire.send("click", { selector: ".tabs .tab", index });
      await until(active, label);
    };
    for (let i = 0; i < 20; i++) {
      await switchPanel(1, "Network");
      await switchPanel(0, "Elements");
    }
    for (let i = 0; i < 100; i++) {
      if ((await app.wire.send("query", { selector: ".list .row" })).count > 0) break;
      if (i === 99) throw Error("The target tree was never rendered");
      await sleep(50);
    }
    const beforePerf = await app.wire.send("perf");
    const baseline = await app.wire.send("mem", { gc: true });
    save(`${profile}-before.json`, baseline);
    const samples: { cycle: number; process: unknown; jsc: unknown }[] = [];
    for (let i = 0; i < cycles; i++) {
      await switchPanel(1, "Network");
      if (i % 50 === 0) await until(text, "Nothing yet.");
      await switchPanel(0, "Elements");
      if ((i + 1) % 100 === 0) {
        const memory = await app.wire.send("mem", { gc: true });
        samples.push({ cycle: i + 1, process: memory.process, jsc: memory.jsc });
      }
    }
    const marker = `AOT_MEMORY_PUSH_${profile}_${cycles}`;
    await evaluate(target.wire, `console.log(${JSON.stringify(marker)});true`);
    await until(text, marker);
    const after = await app.wire.send("mem", { gc: true, regions: true });
    save(`${profile}-after.json`, after);
    const screenshot = await app.wire.send("screenshot", { path: join(out, `${profile}.png`) });
    save(`${profile}-screenshot.json`, screenshot);
    const log = readFileSync(app.log, "utf8");
    if (!log.includes("inspector native entry completed") || !log.includes("source view read-only true"))
      throw Error(`${profile}: native/read-only proof missing`);
    if (profile !== "default" && !log.includes("structure heap 262144 KiB; cell cage 1024 MiB"))
      throw Error("Compact limits were not applied");
    const afterPerf = await app.wire.send("perf");
    const growth = {
      nodes: afterPerf.nodes - beforePerf.nodes,
      objects: after.jsc.objects - baseline.jsc.objects,
      protected: after.jsc.protected - baseline.jsc.protected,
      privateMiB: after.process.privateBytesMB - baseline.process.privateBytesMB,
    };
    const limits = {
      nodes: 64,
      objects: Math.max(8192, Math.ceil(baseline.jsc.objects * 0.25)),
      protected: 64,
      privateMiB: 64,
    };
    const plateau = Object.entries(growth).every(
      ([key, value]) => Number.isFinite(value) && value <= limits[key as keyof typeof limits],
    );
    const row = {
      profile,
      cycles,
      transitions: cycles * 2,
      targetNodes: 1200,
      sourceReadOnly: true,
      before: baseline.process,
      after: after.process,
      jscAfter: after.jsc,
      samples,
      push: true,
      growth,
      limits,
      plateau,
    };
    summary.push(row);
    save("summary.json", summary);
    console.log(JSON.stringify(row));
    if (!plateau) throw Error(`${profile}: post-GC retained state exceeded the recorded stress budget`);
    app.wire.close();
    app.child.kill();
    await sleep(250);
  }
} finally {
  for (const wire of wires) wire.close();
  for (const child of [...children].reverse())
    if (child.pid && child.exitCode === null && child.signalCode === null) child.kill();
  await sleep(500);
  const cleanup = children.map((c) => ({
    pid: c.pid,
    terminated: !c.pid || c.exitCode !== null || c.signalCode !== null,
  }));
  save("cleanup.json", cleanup);
  if (cleanup.some((c) => !c.terminated)) throw Error("An owned test process did not terminate");
}
