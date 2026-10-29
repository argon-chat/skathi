// The Network details pane's pure half (`../src/netfmt.ts`): how a captured
// body becomes lines and hex, which header values are masked and how, and what
// "Copy as curl" / "Copy as fetch" put on the clipboard.
//
//   bun test apps/devtools/test
//
// No host, no Vue: these are the functions that turn `net_detail`'s reply
// (crates/vn-app/src/devtools/net.rs `Ring::detail`) into what a person reads,
// and the copies are the part most worth pinning — a curl line that quotes one
// character wrong is a request that silently differs from the one the app made.

import { describe, expect, test } from "bun:test";
import {
  asCurl,
  asFetch,
  bodyAsText,
  bodySummary,
  formatBytes,
  fromBase64,
  hexDump,
  hexLine,
  hexRows,
  isJsonMime,
  maskHeader,
  parseForm,
  prettyJson,
  queryOf,
  requestHeaderRows,
  responseHeaderRows,
  shellQuote,
  textLines,
} from "../src/netfmt";
import type { NetDetail } from "../src/protocol";

const b64 = (s: string) => Buffer.from(s, "binary").toString("base64");

function detail(over: Partial<NetDetail> = {}): NetDetail {
  return {
    entry: { seq: 7, rid: 3, kind: "fetch", url: "https://api.test/v1/items?x=1", method: "POST", at: 0 },
    rid: 3,
    request: { headers: [], credentials: "include", mode: "cors", redirect: "follow", cache: "default" },
    response: { status: 200, headers: [] },
    limits: { bodyCap: 262144, budget: 12582912, used: 0, evicted: 0 },
    ...over,
  };
}

describe("bytes", () => {
  test("base64 and base64url decode, padding and whitespace ignored", () => {
    expect(new TextDecoder().decode(fromBase64("aGVsbG8="))).toBe("hello");
    expect(new TextDecoder().decode(fromBase64("aGVs\nbG8"))).toBe("hello");
    expect(Array.from(fromBase64("-_8"))).toEqual(Array.from(fromBase64("+/8")));
    expect(fromBase64("").length).toBe(0);
  });

  test("a hex row is offset, two groups of eight, then the printable bytes", () => {
    const bytes = new Uint8Array(18).map((_, i) => (i === 1 ? 0x41 : i));
    expect(hexRows(bytes.length)).toBe(2);
    expect(hexLine(bytes, 0)).toBe(
      "00000000  00 41 02 03 04 05 06 07  08 09 0a 0b 0c 0d 0e 0f  .A..............",
    );
    // A short last row is padded so the ASCII column stays aligned.
    expect(hexLine(bytes, 1)).toBe(
      "00000010  10 11                                             ..",
    );
    expect(hexLine(bytes, 1).indexOf("..")).toBe(hexLine(bytes, 0).indexOf(".A"));
    expect(hexDump(bytes).split("\n")).toHaveLength(2);
  });
});

describe("text", () => {
  test("lines are hard-wrapped at the column count, tabs become two spaces", () => {
    expect(textLines("abcdefghij\nxy", 8)).toEqual(["abcdefgh", "ij", "xy"]);
    expect(textLines("a\tb", 8)).toEqual(["a  b"]);
    expect(textLines("one\r\ntwo", 80)).toEqual(["one", "two"]);
    expect(textLines("x\n".repeat(10), 80, 4)).toHaveLength(4);
  });

  test("JSON is recognised by type and pretty-printed only when it parses", () => {
    expect(isJsonMime("application/json; charset=utf-8")).toBe(true);
    expect(isJsonMime("application/problem+json")).toBe(true);
    expect(isJsonMime("application/ion")).toBe(false);
    expect(prettyJson('{"a":[1,2]}')).toBe('{\n  "a": [\n    1,\n    2\n  ]\n}');
    expect(prettyJson("{nope")).toBeNull();
    expect(prettyJson('"a string"')).toBeNull();
  });

  test("form bodies and query strings decode + and percent-escapes", () => {
    expect(parseForm("a=1&b=two+words&c=%E2%9C%93&flag")).toEqual([
      ["a", "1"],
      ["b", "two words"],
      ["c", "✓"],
      ["flag", ""],
    ]);
    expect(queryOf("https://h.test/p?q=a%20b&n=2#frag")).toEqual([
      ["q", "a b"],
      ["n", "2"],
    ]);
    expect(queryOf("https://h.test/p")).toEqual([]);
  });

  test("sizes", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(262144)).toBe("256 KB");
    expect(formatBytes(12 * 1024 * 1024)).toBe("12.0 MB");
  });
});

describe("masking", () => {
  test("a Cookie header keeps every name and hides every value", () => {
    expect(maskHeader("Cookie", "ArgonSecure=abc123; theme=dark")).toBe(
      "ArgonSecure=•••••• (6); theme=•••••• (4)",
    );
  });

  test("a Set-Cookie line keeps its attributes", () => {
    expect(maskHeader("set-cookie", "ArgonSecure=abc123; Path=/; HttpOnly")).toBe(
      "ArgonSecure=•••••• (6); Path=/; HttpOnly",
    );
  });

  test("Authorization keeps its scheme", () => {
    expect(maskHeader("Authorization", "Bearer eyJhbGciOi")).toBe("Bearer •••••• (10)");
  });
});

describe("header rows", () => {
  test("the jar's Cookie follows the app's own, and an implied Content-Type is added", () => {
    const d = detail({
      request: {
        headers: [["x-trace", "1"]],
        credentials: "include",
        bodyInfo: { kind: "text", implied: "text/plain;charset=UTF-8" },
      },
      cookies: { sent: "sid=abc", set: [] },
    });
    const rows = requestHeaderRows(d);
    expect(rows.map((r) => [r.name, r.value, r.sensitive])).toEqual([
      ["x-trace", "1", false],
      ["content-type", "text/plain;charset=UTF-8", false],
      ["cookie", "sid=abc", true],
    ]);
    expect(rows[2].note).toBe("from the cookie jar");
  });

  test("a script-set Cookie is kept first, then the jar's", () => {
    const d = detail({
      request: { headers: [["Cookie", "a=1"]] },
      cookies: { sent: "sid=abc", set: [] },
    });
    expect(requestHeaderRows(d)[0].value).toBe("a=1; sid=abc");
  });

  test("reconstructed Set-Cookie lines are appended to the response headers, marked", () => {
    const d = detail({
      response: { status: 200, headers: [["content-type", "application/json"]] },
      cookies: { sent: null, set: ["sid=new; Path=/"] },
    });
    const rows = responseHeaderRows(d);
    expect(rows).toHaveLength(2);
    expect(rows[1]).toMatchObject({ name: "set-cookie", sensitive: true, note: "reconstructed from the jar" });
  });
});

describe("bodies", () => {
  test("the summary says what was cut, and from how much", () => {
    expect(bodySummary(undefined, 0)).toBe("not read yet");
    expect(bodySummary(null, 0)).toBe("no body");
    expect(bodySummary({ stored: 300, mime: "application/json" }, 0)).toBe("application/json · 300 B");
    expect(bodySummary({ stored: 262144, truncated: true, total: 1048576 }, 0)).toBe(
      "truncated: first 256 KB of 1.0 MB",
    );
    expect(bodySummary({ stored: 262144, truncated: true, total: 300000, unit: "chars" }, 0)).toBe(
      "truncated: first 256 KB of 300000 chars",
    );
    expect(bodySummary({ stored: 0, evicted: true }, 12 * 1024 * 1024)).toContain("evicted");
  });

  test("a binary body copies as its hex dump, text as itself", () => {
    expect(bodyAsText({ stored: 2, encoding: "utf8", data: "hi" })).toBe("hi");
    expect(bodyAsText({ stored: 2, encoding: "base64", data: b64("\x00\xff") })).toBe(
      "00000000  00 ff                                             ..",
    );
    expect(bodyAsText({ stored: 0, evicted: true })).toBe("");
  });
});

describe("copy as curl", () => {
  test("method, headers, the jar's cookies as -b, and a text body", () => {
    const d = detail({
      request: {
        headers: [["content-type", "application/json"]],
        credentials: "include",
        body: { stored: 9, encoding: "utf8", data: `{"a":"'"}` },
      },
      cookies: { sent: "sid=abc", set: [] },
    });
    expect(asCurl(d)).toBe(
      [
        "curl 'https://api.test/v1/items?x=1'",
        "-X 'POST'",
        "-H 'content-type: application/json'",
        "-b 'sid=abc'",
        `--data-raw '{"a":"'\\''"}'`,
      ].join(" \\\n  "),
    );
  });

  test("binary bodies use $'…' escapes", () => {
    const d = detail({
      request: { headers: [], body: { stored: 3, encoding: "base64", data: b64("a\x00'") } },
    });
    expect(asCurl(d)).toEndWith("--data-binary $'a\\x00\\''");
  });

  test("multipart is re-built as -F fields, files named, its boundary header dropped", () => {
    const d = detail({
      request: {
        headers: [["Content-Type", "multipart/form-data; boundary=xyz"]],
        bodyInfo: {
          kind: "formdata",
          fields: [
            { name: "title", value: "hi" },
            { name: "pic", file: { name: "a.png", size: 10, type: "image/png" } },
          ],
        },
        body: { stored: 0, kind: "formdata", note: "multipart" },
      },
    });
    const out = asCurl(d);
    expect(out).not.toContain("boundary");
    expect(out).toContain("-F 'title=hi'");
    expect(out).toContain("-F 'pic=@a.png;type=image/png'");
    expect(out).toStartWith("# NOTE: file parts are named but their bytes were not captured\n");
  });

  test("a truncated body is copied with a NOTE saying so", () => {
    const d = detail({
      request: { headers: [], body: { stored: 262144, truncated: true, encoding: "utf8", data: "x" } },
    });
    expect(asCurl(d)).toStartWith("# NOTE: the body was cut at 256 KB by the devtools tee\n");
  });

  test("a GET has no -X", () => {
    const d = detail({ entry: { seq: 1, kind: "fetch", url: "https://h.test/", method: "GET", at: 0 } });
    expect(asCurl(d)).toBe("curl 'https://h.test/'");
  });

  test("shell quoting closes and reopens around a quote", () => {
    expect(shellQuote("it's")).toBe(`'it'\\''s'`);
  });
});

describe("copy as fetch", () => {
  test("no Cookie header — credentials carries the cookies, as in a browser", () => {
    const d = detail({
      request: {
        headers: [["accept", "application/json"]],
        credentials: "include",
        body: { stored: 7, encoding: "utf8", data: '{"a":1}' },
      },
      cookies: { sent: "sid=abc", set: [] },
    });
    const out = asFetch(d);
    expect(out).not.toContain("sid=abc");
    expect(out).toContain('"accept": "application/json"');
    expect(out).toContain('"body": "{\\"a\\":1}"');
    expect(out).toContain('"method": "POST"');
    expect(out).toContain('"credentials": "include"');
    expect(out.startsWith('fetch("https://api.test/v1/items?x=1", {')).toBe(true);
    expect(out.endsWith("});")).toBe(true);
  });

  test("the copy is a valid fetch call", () => {
    const d = detail({
      request: { headers: [["x", "1"]], body: { stored: 2, encoding: "base64", data: b64("\x01\x02") } },
    });
    let seen: unknown[] = [];
    // Evaluated against a stub `fetch`: the text must parse and pass what it says.
    new Function("fetch", asFetch(d))((...args: unknown[]) => (seen = args));
    expect(seen[0]).toBe("https://api.test/v1/items?x=1");
    const init = seen[1] as { body: Uint8Array; method: string; headers: Record<string, string> };
    expect(Array.from(init.body)).toEqual([1, 2]);
    expect(init.method).toBe("POST");
    expect(init.headers).toEqual({ x: "1" });
  });
});
