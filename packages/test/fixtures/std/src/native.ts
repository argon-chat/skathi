import { Encoding } from "skathi:encoding";
import { dlopen, NativeCallback, NativePointer } from "skathi:ffi";
import { os } from "skathi:os";

const attempt = (run: () => unknown): string => {
  try {
    run();
    return "ok";
  } catch (e) {
    const error = e as Error;
    return `${error.name}: ${error.message}`;
  }
};

const PRINT = {
  args: ["buffer", "usize", "cstring", "i32", "f64", "cstring"],
  returns: "i32",
  variadic: 3,
} as const;

function windows() {
  const kernel = dlopen("kernel32", {
    SetLastError: { args: ["u32"], errno: true },
    GetCurrentProcessId: { returns: "u32" },
    lstrlenW: { args: ["wstring"], returns: "i32" },
    GetModuleHandleW: { args: ["wstring"], returns: "pointer" },
    GetEnvironmentVariableW: { args: ["wstring", "buffer", "u32"], returns: "u32" },
  });
  const crt = dlopen("crt", {
    strtoul: { args: ["cstring", "pointer", "i32"], returns: "u32" },
    _atoi64: { args: ["cstring"], returns: "i64" },
  });
  const out = new Uint16Array(64);
  const length = kernel.symbols.GetEnvironmentVariableW("SystemRoot", out, out.length);
  return {
    lastError: kernel.symbols.SetLastError(1234),
    pid: kernel.symbols.GetCurrentProcessId() > 0,
    wide: kernel.symbols.lstrlenW("héllo, мир"),
    module: kernel.symbols.GetModuleHandleW(null).isNull(),
    systemRoot: String.fromCharCode(...out.subarray(0, length)).toLowerCase(),
    unsigned: [crt.symbols.strtoul("2147483647", null, 10), crt.symbols.strtoul("2147483648", null, 10)],
    top: crt.symbols.strtoul("4294967295", null, 10),
    big: String(crt.symbols._atoi64("-9007199254740993")),
    bigType: typeof crt.symbols._atoi64("1"),
  };
}

const int32 = (pointer: NativePointer) => new Int32Array(pointer.read(4).buffer)[0]!;

export function callbacks() {
  const crt = dlopen("crt", { qsort: { args: ["buffer", "usize", "usize", "pointer"] } });
  const before = NativeCallback.live();
  let calls = 0;
  const descending = new NativeCallback(
    (a, b) => {
      calls++;
      return int32(b) - int32(a);
    },
    { args: ["pointer", "pointer"], returns: "i32" },
  );
  const numbers = new Int32Array([3, -1, 40, 7, 0, 12, -8, 5]);
  crt.symbols.qsort(numbers, numbers.length, 4, descending.pointer);

  const throwing = new NativeCallback(
    function compare() {
      throw new RangeError("not comparable");
    },
    { args: ["pointer", "pointer"], returns: "i32" },
  );
  const untyped = new NativeCallback((() => "1") as unknown as () => number, {
    args: ["pointer", "pointer"],
    returns: "i32",
  });
  const sort = (callback: NativeCallback) =>
    attempt(() => crt.symbols.qsort(new Int32Array([2, 1, 3]), 3, 4, callback.pointer));
  const thrown = sort(throwing);
  const wrongAnswer = sort(untyped);
  const afterThrow = sort(descending);

  const open = NativeCallback.live() - before;
  for (const callback of [descending, throwing, untyped]) callback.close();
  descending.close();
  return {
    sorted: [...numbers],
    called: calls > 0,
    thrown,
    wrongAnswer,
    afterThrow,
    open,
    left: NativeCallback.live() - before,
    closed: [descending.closed, attempt(() => descending.pointer)],
    notFunction: attempt(() => new NativeCallback(1 as unknown as () => void, {})),
    buffer: attempt(() => new NativeCallback(() => {}, { args: ["buffer" as "pointer"] })),
    text: attempt(() => new NativeCallback(() => null, { returns: "cstring" as "pointer" })),
  };
}

/** A callback the system's thread pool calls: it runs here, on the JS thread, a frame later. */
export async function threads() {
  if (os.platform !== "win32") return null;
  const kernel = dlopen("kernel32", {
    TrySubmitThreadpoolCallback: { args: ["pointer", "pointer", "pointer"], returns: "i32" },
    GetCurrentThreadId: { returns: "u32" },
    CreateThread: { args: ["pointer", "usize", "pointer", "pointer", "u32", "pointer"], returns: "pointer" },
    WaitForSingleObject: { args: ["pointer", "u32"], returns: "u32" },
    CloseHandle: { args: ["pointer"], returns: "i32" },
  });
  const here = kernel.symbols.GetCurrentThreadId();
  const context = NativePointer.of(new Uint8Array(8));
  const seen: { thread: number; same: boolean }[] = [];
  let done = () => {};
  const all = new Promise<void>((resolve) => (done = resolve));
  const work = new NativeCallback(
    (_instance, given) => {
      seen.push({ thread: kernel.symbols.GetCurrentThreadId(), same: given.equals(context) });
      if (seen.length === 3) done();
    },
    { args: ["pointer", "pointer"] },
  );
  const accepted = [1, 2, 3].map(() => kernel.symbols.TrySubmitThreadpoolCallback(work.pointer, context, null));
  const inline = seen.length;
  await all;
  work.close();

  const answering = new NativeCallback(() => 7, { args: ["pointer"], returns: "u32" });
  const thread = kernel.symbols.CreateThread(null, 0, answering.pointer, null, 0, null);
  const waited = kernel.symbols.WaitForSingleObject(thread, 5000);
  kernel.symbols.CloseHandle(thread);
  answering.close();
  return {
    accepted,
    inline,
    onJsThread: seen.every((call) => call.thread === here),
    context: seen.every((call) => call.same),
    waited,
  };
}

export function ffi() {
  const crt = dlopen("crt", {
    strlen: { args: ["cstring"], returns: "usize" },
    abs: { args: ["i32"], returns: "i32" },
    pow: { args: ["f64", "f64"], returns: "f64" },
    memcpy: { args: ["buffer", "buffer", "usize"], returns: "pointer" },
    strchr: { args: ["pointer", "i32"], returns: "pointer" },
    snprintf: { ...PRINT, optional: true },
    _snprintf: { ...PRINT, optional: true },
    sk_no_such_function: { optional: true },
  });
  const byPointer = dlopen("crt", { strlen: { args: ["pointer"], returns: "usize" } });

  const from = new Uint8Array([1, 2, 3, 4]);
  const to = new Uint8Array(4);
  crt.symbols.memcpy(to, from, 4);

  const text = Encoding.encode("skathi/ffi\0", "utf-8");
  const start = NativePointer.of(text);
  const slash = crt.symbols.strchr(start, 47);

  const line = new Uint8Array(64);
  const print = crt.symbols.snprintf ?? crt.symbols._snprintf;
  const written = print ? print(line, line.length, "%d %.2f %s", -7, 2.5, "ok") : 0;

  const small = new Uint8Array(8);
  let stable = true;
  for (let i = 0; i < 2000; i++) {
    crt.symbols.memcpy(small, new Uint8Array([i & 255, 1, 2, 3, 4, 5, 6, 7]), 8);
    stable &&= small[0] === (i & 255) && small[7] === 7;
  }

  return {
    strlen: String(crt.symbols.strlen("héllo")),
    strlenType: typeof crt.symbols.strlen(""),
    abs: [crt.symbols.abs(-5), crt.symbols.abs(-5n), crt.symbols.abs(new Float64Array([-7])[0]!)],
    pow: crt.symbols.pow(2, 10),
    copied: [...to],
    slash: slash.readCString(),
    slashAt: Number(slash.address - start.address),
    offset: start.offset(7).readCString(2),
    bytes: [...start.read(3)],
    sameStart: start.equals(NativePointer.of(text)),
    nullFound: crt.symbols.strchr(start, 33).isNull(),
    byPointer: String(byPointer.symbols.strlen(start)),
    line: Encoding.decode(line.subarray(0, written), "utf-8"),
    stable,
    missing: crt.symbols.sk_no_such_function,
    names: [crt.symbols.strlen.name, crt.symbols.strlen.signature.returns, crt.name],
    path: crt.path.length > 0,
    errors: {
      notListed: attempt(() => dlopen("gdi32", {})),
      noSymbol: attempt(() => dlopen("crt", { sk_no_such_function: {} })),
      badType: attempt(() => dlopen("crt", { strlen: { args: ["struct" as "pointer"] } })),
      count: attempt(() => (crt.symbols.abs as unknown as () => number)()),
      text: attempt(() => crt.symbols.abs("5" as unknown as number)),
      fraction: attempt(() => crt.symbols.abs(1.5)),
      range: attempt(() => crt.symbols.abs(2 ** 31)),
      bufferAsPointer: attempt(() => crt.symbols.strchr(text as unknown as NativePointer, 47)),
      nul: attempt(() => crt.symbols.strlen("a\0b")),
      address: attempt(() => NativePointer.fromAddress(8n)),
      nullRead: attempt(() => NativePointer.NULL.readCString()),
      construct: attempt(() => new (NativePointer as unknown as new () => unknown)()),
      closed: attempt(() => {
        const again = dlopen("crt", { abs: { args: ["i32"], returns: "i32" } });
        again.close();
        again.symbols.abs(1);
      }),
    },
    windows: os.platform === "win32" ? windows() : null,
  };
}
