// Two exported functions, hand-assembled: `mix(n)`, n rounds of xorshift32 over locals, and
// `sieve(n)`, the primes below n by a byte-per-number sieve in linear memory (n <= 4 MiB).

const leb = (n: number): number[] => {
  const out: number[] = [];
  do {
    let byte = n & 0x7f;
    n >>>= 7;
    if (n) byte |= 0x80;
    out.push(byte);
  } while (n);
  return out;
};
function sleb(n: number): number[] {
  const out: number[] = [];
  for (;;) {
    const byte = n & 0x7f;
    n >>= 7;
    if ((n === 0 && !(byte & 0x40)) || (n === -1 && byte & 0x40)) return [...out, byte];
    out.push(byte | 0x80);
  }
}
const vec = (items: number[][]): number[] => [...leb(items.length), ...items.flat()];
const section = (id: number, body: number[]): number[] => [id, ...leb(body.length), ...body];
const name = (s: string): number[] => [...leb(s.length), ...[...s].map((c) => c.charCodeAt(0))];

const PLAIN: Record<string, number[]> = {
  end: [0x0b],
  "i32.add": [0x6a],
  "i32.xor": [0x73],
  "i32.shl": [0x74],
  "i32.shr_u": [0x76],
  "i32.lt_s": [0x48],
  "i32.ge_u": [0x4f],
  "i32.eqz": [0x45],
  "i32.load8_u": [0x2d, 0, 0],
  "i32.store8": [0x3a, 0, 0],
  "memory.fill": [0xfc, 0x0b, 0x00],
  block: [0x02, 0x40],
  loop: [0x03, 0x40],
  if: [0x04, 0x40],
};
const WITH_INDEX: Record<string, number> = {
  "local.get": 0x20,
  "local.set": 0x21,
  "local.tee": 0x22,
  br: 0x0c,
  br_if: 0x0d,
};

/** Assembles whitespace-separated instructions; blocks are void, memory accesses unaligned at offset 0. */
function code(text: string): number[] {
  const tokens = text.trim().split(/\s+/);
  const out: number[] = [];
  for (let i = 0; i < tokens.length; i++) {
    const op = tokens[i]!;
    if (op === "i32.const") out.push(0x41, ...sleb(Number(tokens[++i])));
    else if (op in WITH_INDEX) out.push(WITH_INDEX[op]!, ...leb(Number(tokens[++i])));
    else if (op in PLAIN) out.push(...PLAIN[op]!);
    else throw Error(`unknown instruction ${op}`);
  }
  return out;
}

// mix(n): locals 1 = i, 2 = x
const mix = code(`
  i32.const 1 local.set 2
  loop
    local.get 2 local.get 2 i32.const 13 i32.shl i32.xor local.set 2
    local.get 2 local.get 2 i32.const 17 i32.shr_u i32.xor local.set 2
    local.get 2 local.get 2 i32.const 5 i32.shl i32.xor local.set 2
    local.get 1 i32.const 1 i32.add local.tee 1 local.get 0 i32.lt_s br_if 0
  end
  local.get 2
  end`);

// sieve(n): locals 1 = i, 2 = j, 3 = count
const sieve = code(`
  i32.const 0 i32.const 0 local.get 0 memory.fill
  i32.const 2 local.set 1
  block loop
    local.get 1 local.get 0 i32.ge_u br_if 1
    local.get 1 i32.load8_u i32.eqz
    if
      local.get 3 i32.const 1 i32.add local.set 3
      local.get 1 local.get 1 i32.add local.set 2
      block loop
        local.get 2 local.get 0 i32.ge_u br_if 1
        local.get 2 i32.const 1 i32.store8
        local.get 2 local.get 1 i32.add local.set 2
        br 0
      end end
    end
    local.get 1 i32.const 1 i32.add local.set 1
    br 0
  end end
  local.get 3
  end`);

const I32 = 0x7f;
const body = (locals: number, instructions: number[]) => {
  const inner = [...vec([[...leb(locals), I32]]), ...instructions];
  return [...leb(inner.length), ...inner];
};

export const WASM = Uint8Array.from([
  ...[0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00],
  ...section(1, vec([[0x60, ...vec([[I32]]), ...vec([[I32]])]])),
  ...section(3, vec([[0], [0]])),
  ...section(5, vec([[0x00, ...leb(64)]])),
  ...section(
    7,
    vec([
      [...name("mix"), 0x00, 0],
      [...name("sieve"), 0x00, 1],
    ]),
  ),
  ...section(10, [...leb(2), ...body(2, mix), ...body(3, sieve)]),
]);
