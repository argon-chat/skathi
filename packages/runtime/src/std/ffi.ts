// `skathi:ffi`: C functions of the libraries sk.config.json allows. The host loads, converts and
// calls; the functions of `symbols` are closures, which is why this part is JS.

type Small = "i8" | "u8" | "i16" | "u16" | "i32" | "u32";
type Wide = "i64" | "u64" | "isize" | "usize";
type Text = "cstring" | "wstring";

/**
 * A C type. `buffer` is the address of a typed array's bytes for the length of the call; `cstring`
 * and `wstring` are a JS string as NUL-terminated UTF-8 and UTF-16.
 */
export type FFIType = "void" | "bool" | Small | Wide | "f32" | "f64" | "pointer" | "buffer" | Text;

/** What a field of a struct holds: a scalar, another struct, or `[type, count]` for an array. */
export type FieldType = "bool" | Small | Wide | "f32" | "f64" | "pointer" | StructType | readonly [FieldType, number];
export type StructFields = Readonly<Record<string, FieldType>>;

type Given<T> =
  T extends StructType<infer F> ? Packed<F> : T extends readonly [infer E, number] ? ArrayLike<Given<E>> : ArgOf<T>;
type Read<T> =
  T extends StructType<infer F> ? Unpacked<F> : T extends readonly [infer E, number] ? Read<E>[] : ValueOf<T>;
/** What a struct is written from. A field left out is zero. */
export type Packed<F extends StructFields> = { readonly [K in keyof F]?: Given<F[K]> };
/** What a struct reads as. */
export type Unpacked<F extends StructFields> = { -readonly [K in keyof F]: Read<F[K]> };

/** A C struct: fields in order, each at a multiple of its size, as a C compiler lays them out. */
export interface StructType<F extends StructFields = StructFields> {
  readonly fields: F;
  readonly size: number;
  readonly align: number;
  readonly offsets: { readonly [K in keyof F]: number };
  /** The bytes of a struct, for a `buffer` parameter that takes a pointer to it. */
  pack(value: Packed<F>): Uint8Array;
  /** Reads a struct from bytes. A pointer field that is not null needs `"permissions": { "ffiAddress": true }`. */
  unpack(bytes: ArrayBuffer | ArrayBufferView, byteOffset?: number): Unpacked<F>;
}

export interface Signature {
  /** A struct type is a struct passed by value: an object of its fields, or bytes that hold it. */
  readonly args?: readonly (Exclude<FFIType, "void"> | StructType)[];
  /** `"void"` when left out. A `buffer` cannot be returned. A struct type is a struct returned by value. */
  readonly returns?: Exclude<FFIType, "buffer"> | StructType;
  /** The call answers `{ value, errno }`: `errno`, or `GetLastError()` on Windows, read right after it. */
  readonly errno?: boolean;
  /** A library without the symbol loads all the same, and the function is `null`. */
  readonly optional?: boolean;
  /** For a variadic function: how many of `args` are its named parameters. */
  readonly variadic?: number;
  /**
   * The call runs on a worker thread and answers with a promise, so the window keeps painting.
   * A `buffer` argument is copied for the call and written back when it returns.
   */
  readonly nonblocking?: boolean;
}

type ArgOf<T> =
  T extends StructType<infer F>
    ? Packed<F> | ArrayBuffer | ArrayBufferView
    : T extends "bool"
      ? boolean
      : T extends Small | Wide
        ? number | bigint
        : T extends "f32" | "f64"
          ? number
          : T extends "pointer"
            ? NativePointer | null
            : T extends "buffer"
              ? ArrayBuffer | ArrayBufferView | null
              : T extends Text
                ? string | null
                : never;

type ValueOf<T> =
  T extends StructType<infer F>
    ? Unpacked<F>
    : T extends "bool"
      ? boolean
      : T extends Small | "f32" | "f64"
        ? number
        : T extends Wide
          ? bigint
          : T extends "pointer"
            ? NativePointer
            : T extends Text
              ? string | null
              : void;

type Each<A extends readonly unknown[]> = { -readonly [I in keyof A]: ArgOf<A[I]> };
type ArgsOf<S extends Signature> = S["args"] extends readonly unknown[] ? Each<S["args"]> : [];
type Returned<S extends Signature> = ValueOf<S["returns"] extends FFIType | StructType ? S["returns"] : "void">;
type Answered<S extends Signature> = S["errno"] extends true ? { value: Returned<S>; errno: number } : Returned<S>;
type ReturnOf<S extends Signature> = S["nonblocking"] extends true ? Promise<Answered<S>> : Answered<S>;

export interface NativeFunction<S extends Signature = Signature> {
  (...args: ArgsOf<S>): ReturnOf<S>;
  readonly name: string;
  readonly signature: S;
}

export type SymbolsOf<S extends Record<string, Signature>> = {
  readonly [K in keyof S]: S[K]["optional"] extends true ? NativeFunction<S[K]> | null : NativeFunction<S[K]>;
};

/** An address in native memory. One taken from a buffer keeps the buffer alive. */
export interface NativePointer {
  readonly address: bigint;
  isNull(): boolean;
  /** The pointer `bytes` further on, or back for a negative number. */
  offset(bytes: number): NativePointer;
  equals(other: NativePointer): boolean;
  /** The UTF-8 text up to the first NUL, or to `maxBytes`. */
  readCString(maxBytes?: number): string;
  /** The UTF-16 text up to the first NUL, or to `maxUnits` code units. */
  readWString(maxUnits?: number): string;
  /** A copy of `length` bytes. */
  read(length: number): Uint8Array;
  /** Copies `bytes` to the address. */
  write(bytes: ArrayBuffer | ArrayBufferView): void;
}

export interface NativePointerStatics {
  readonly prototype: NativePointer;
  readonly NULL: NativePointer;
  /** The address of a buffer's bytes, `offset` bytes in. The pointer keeps the buffer alive. */
  of(source: ArrayBuffer | ArrayBufferView, offset?: number): NativePointer;
  /** Needs `"permissions": { "ffiAddress": true }`; throws `NotAllowedError` without it. */
  fromAddress(address: bigint | number): NativePointer;
}

/** What a callback is called with and what it answers. A block of memory arrives as a `pointer`. */
export interface CallbackSignature {
  readonly args?: readonly Exclude<FFIType, "void" | "buffer">[];
  /** `"void"` when left out. */
  readonly returns?: Exclude<FFIType, "buffer" | Text>;
}

type Received<T> = T extends Text ? string | null : T extends "pointer" ? NativePointer : ValueOf<T>;
type ReceivedEach<A extends readonly unknown[]> = { -readonly [I in keyof A]: Received<A[I]> };
type ReceivedBy<S extends CallbackSignature> = S["args"] extends readonly unknown[] ? ReceivedEach<S["args"]> : [];
type AnsweredBy<S extends CallbackSignature> =
  S["returns"] extends Exclude<FFIType, "void"> ? ArgOf<S["returns"]> : void;

/**
 * A JS function as a C function pointer. It takes one of a fixed number of slots and holds the
 * function until `close()`, whatever else still refers to it.
 */
export interface NativeCallback {
  /** What to pass to the library. Throws `InvalidStateError` once closed. */
  readonly pointer: NativePointer;
  readonly closed: boolean;
  /** Gives the slot back. A call the library makes afterwards runs nothing and returns 0. */
  close(): void;
}

export interface NativeCallbackConstructor {
  readonly prototype: NativeCallback;
  new <const S extends CallbackSignature>(
    callback: (...args: ReceivedBy<S>) => AnsweredBy<S>,
    signature: S,
  ): NativeCallback;
  /** How many callbacks are open, of the 128 there can be. */
  live(): number;
}

interface HostLibrary {
  readonly path: string;
  readonly closed: boolean;
  call(index: number, ...args: unknown[]): unknown;
  has(index: number): boolean;
  close(): void;
}

declare const __sk_std: {
  ffi: {
    NativePointer: NativePointerStatics;
    NativeCallback: NativeCallbackConstructor;
    open(name: string, definitions: readonly unknown[]): HostLibrary;
    struct(entries: readonly (readonly [string, FieldType])[], fields: StructFields): StructType;
  };
};

export const NativePointer: NativePointerStatics = __sk_std.ffi.NativePointer;
export const NativeCallback: NativeCallbackConstructor = __sk_std.ffi.NativeCallback;

/** A C struct of `fields`, in the order they are written. */
export function struct<const F extends StructFields>(fields: F): StructType<F> {
  if (!fields || typeof fields !== "object")
    throw new TypeError("struct: the fields must be an object of { name: type }");
  return __sk_std.ffi.struct(Object.entries(fields), fields) as StructType<F>;
}

/**
 * A function of exactly `arity` parameters: a rest parameter would make an array on every call.
 * A call with another number of arguments goes as it is, for the host to refuse.
 */
function fixed(host: HostLibrary, index: number, arity: number): (...args: unknown[]) => unknown {
  const any = (...args: unknown[]): unknown => host.call(index, ...args);
  switch (arity) {
    case 0:
      return function () {
        return arguments.length === 0 ? host.call(index) : any(...arguments);
      };
    case 1:
      return function (a: unknown) {
        return arguments.length === 1 ? host.call(index, a) : any(...arguments);
      };
    case 2:
      return function (a: unknown, b: unknown) {
        return arguments.length === 2 ? host.call(index, a, b) : any(...arguments);
      };
    case 3:
      return function (a: unknown, b: unknown, c: unknown) {
        return arguments.length === 3 ? host.call(index, a, b, c) : any(...arguments);
      };
    case 4:
      return function (a: unknown, b: unknown, c: unknown, d: unknown) {
        return arguments.length === 4 ? host.call(index, a, b, c, d) : any(...arguments);
      };
    case 5:
      return function (a: unknown, b: unknown, c: unknown, d: unknown, e: unknown) {
        return arguments.length === 5 ? host.call(index, a, b, c, d, e) : any(...arguments);
      };
    case 6:
      return function (a: unknown, b: unknown, c: unknown, d: unknown, e: unknown, f: unknown) {
        return arguments.length === 6 ? host.call(index, a, b, c, d, e, f) : any(...arguments);
      };
    default:
      return any;
  }
}

export class NativeLibrary<S extends Record<string, Signature> = Record<string, Signature>> {
  readonly name: string;
  readonly symbols: SymbolsOf<S>;
  readonly #host: HostLibrary;

  /** @internal Use `dlopen()`. */
  constructor(name: string, signatures: S) {
    if (!signatures || typeof signatures !== "object") {
      throw new TypeError("dlopen: the symbols must be an object of { name: { args, returns } }");
    }
    const names = Object.keys(signatures);
    const host = __sk_std.ffi.open(
      name,
      names.map((symbol) => {
        const s = signatures[symbol] as Signature;
        return [
          symbol,
          s.args ?? [],
          s.returns ?? "void",
          s.errno === true,
          s.optional === true,
          s.variadic,
          s.nonblocking === true,
        ];
      }),
    );
    const symbols: Record<string, unknown> = {};
    names.forEach((symbol, index) => {
      if (!host.has(index)) {
        symbols[symbol] = null;
        return;
      }
      const direct = fixed(host, index, (signatures[symbol] as Signature).args?.length ?? 0);
      const settled = async (...args: unknown[]): Promise<unknown> => host.call(index, ...args);
      const call = (signatures[symbol] as Signature).nonblocking === true ? settled : direct;
      Object.defineProperty(call, "name", { value: symbol });
      Object.defineProperty(call, "signature", { value: signatures[symbol] });
      symbols[symbol] = call;
    });
    this.name = name;
    this.symbols = Object.freeze(symbols) as SymbolsOf<S>;
    this.#host = host;
  }

  /** The file the library was loaded from. */
  get path(): string {
    return this.#host.path;
  }

  get closed(): boolean {
    return this.#host.closed;
  }

  /** Later calls throw `InvalidStateError`. The library itself stays loaded. */
  close(): void {
    this.#host.close();
  }
}

/**
 * Loads the library sk.config.json lists under `name` in `"permissions.ffi"` and looks up `symbols`.
 * Throws `NotAllowedError` for a name that is not listed and `NotFoundError` for a library or a
 * symbol that is not there.
 */
export function dlopen<const S extends Record<string, Signature>>(name: string, symbols: S): NativeLibrary<S> {
  return new NativeLibrary(name, symbols);
}
