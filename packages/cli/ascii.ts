// Makes a production bundle pure ASCII, so JavaScriptCore holds its source as 8-bit: one non-ASCII
// character makes WTF keep the whole source as UTF-16 for the life of the process, and Bun's bundler
// has no ASCII-only output. This pass rewrites the finished bundle, turning each non-ASCII character
// into the escape its context reads as the same thing (`\uXXXX` in strings, templates, regexes,
// comments; `\u{…}` for astral identifiers; LS / PS outside a literal become line feeds).
//
// A `\` before the character goes with it; a `\` before LS / PS in a string is a line continuation,
// so the pair is dropped. In a TAGGED template the tag reads `strings.raw`, so those characters are
// left (the result counts them). The lexer must guess whether a `/` divides or starts a regex; if it
// ends in an impossible state the bundle is returned unchanged. prod.ts also checks the result with
// Bun's parser (`checkAscii`).

export interface AsciiResult {
  code: string;
  escaped: number;
  kept: number;
  failed?: string;
}

const LS = 0x2028;
const PS = 0x2029;

const hex4 = (u: number) => "\\u" + u.toString(16).toUpperCase().padStart(4, "0");

/** `\uXXXX`, or a surrogate pair: one escape that a string, template and regular expression all read as `cp`. */
function escapeUnits(cp: number): string {
  if (cp <= 0xffff) return hex4(cp);
  const v = cp - 0x10000;
  return hex4(0xd800 + (v >> 10)) + hex4(0xdc00 + (v & 0x3ff));
}

function escapeIdent(cp: number): string {
  return cp <= 0xffff ? hex4(cp) : "\\u{" + cp.toString(16).toUpperCase() + "}";
}

/** Keywords after which a `/` starts a regular expression. Not `of`: a minifier may name a variable `of`. */
const BEFORE_EXPRESSION = new Set([
  "return",
  "typeof",
  "instanceof",
  "in",
  "new",
  "delete",
  "void",
  "throw",
  "case",
  "do",
  "else",
  "yield",
  "await",
  "extends",
]);
const PAREN_STATEMENT = new Set(["if", "while", "for", "with"]);

const isIdentPart = (c: number) =>
  (c >= 48 && c <= 57) || (c >= 65 && c <= 90) || (c >= 97 && c <= 122) || c === 36 || c === 95 || c === 92;

export function asciiOnly(src: string): AsciiResult {
  const out: string[] = [];
  let copied = 0; // src[copied, i) is still to be copied verbatim
  let escaped = 0;
  let kept = 0;
  let i = 0;
  const n = src.length;

  const put = (from: number, to: number, text: string) => {
    if (from > copied) out.push(src.slice(copied, from));
    out.push(text);
    copied = to;
    escaped++;
  };
  const cpAt = (at: number): [number, number] => {
    const cp = src.codePointAt(at)!;
    return [cp, cp > 0xffff ? 2 : 1];
  };

  let regexOk = true;
  const parens: boolean[] = [];
  const braces: ("block" | "expr" | "tpl")[] = [];
  const templates: boolean[] = []; // tagged? per open `${`
  let lastWord = "";
  let last: "word" | "value" | "punct" = "punct";
  let lastPunct = "";

  const literalChar = (at: number, backslash: boolean, kind: "string" | "template" | "regex"): number => {
    const from = backslash ? at - 1 : at;
    const [cp, len] = cpAt(at);
    if (backslash && (cp === LS || cp === PS) && kind !== "regex") put(from, at + len, "");
    else put(from, at + len, escapeUnits(cp));
    return at + len;
  };

  const templateText = (tagged: boolean): boolean => {
    while (i < n) {
      const c = src.charCodeAt(i);
      if (c === 92) {
        const d = src.charCodeAt(i + 1);
        if (d > 0x7f) {
          if (tagged) {
            kept++;
            i += cpAt(i + 1)[1] + 1;
          } else i = literalChar(i + 1, true, "template");
        } else i += 2;
        continue;
      }
      if (c === 96) {
        i++;
        last = "value";
        regexOk = false;
        return true;
      }
      if (c === 36 && src.charCodeAt(i + 1) === 123) {
        i += 2;
        braces.push("tpl");
        templates.push(tagged);
        last = "punct";
        lastPunct = "${";
        regexOk = true;
        return true;
      }
      if (c > 0x7f) {
        if (tagged) {
          kept++;
          i += cpAt(i)[1];
        } else i = literalChar(i, false, "template");
        continue;
      }
      i++;
    }
    return false; // unterminated
  };

  while (i < n) {
    const c = src.charCodeAt(i);

    if (c === 32 || c === 9 || c === 10 || c === 13 || c === 11 || c === 12) {
      i++;
      continue;
    }

    if (c === 47 && src.charCodeAt(i + 1) === 47) {
      i += 2;
      while (i < n) {
        const d = src.charCodeAt(i);
        if (d === 10 || d === 13 || d === LS || d === PS) break;
        if (d > 0x7f) {
          const [cp, len] = cpAt(i);
          put(i, i + len, escapeUnits(cp));
          i += len;
        } else i++;
      }
      continue;
    }
    if (c === 47 && src.charCodeAt(i + 1) === 42) {
      const end = src.indexOf("*/", i + 2);
      if (end < 0) return { code: src, escaped: 0, kept: 0, failed: "an unterminated /* comment" };
      for (let k = i + 2; k < end;) {
        const d = src.charCodeAt(k);
        if (d > 0x7f) {
          const [cp, len] = cpAt(k);
          put(k, k + len, cp === LS || cp === PS ? "\n" : escapeUnits(cp));
          k += len;
        } else k++;
      }
      i = end + 2;
      continue;
    }

    if (c === 34 || c === 39) {
      i++;
      for (;;) {
        if (i >= n) return { code: src, escaped: 0, kept: 0, failed: "an unterminated string" };
        const d = src.charCodeAt(i);
        if (d === c) break;
        if (d === 92) {
          if (src.charCodeAt(i + 1) > 0x7f) i = literalChar(i + 1, true, "string");
          else i += 2;
          continue;
        }
        if (d === 10 || d === 13) return { code: src, escaped: 0, kept: 0, failed: "a line break inside a string" };
        if (d > 0x7f) i = literalChar(i, false, "string");
        else i++;
      }
      i++;
      last = "value";
      regexOk = false;
      continue;
    }

    if (c === 96) {
      const tagged = last === "word" ? !BEFORE_EXPRESSION.has(lastWord) : last === "value";
      i++;
      if (!templateText(tagged)) return { code: src, escaped: 0, kept: 0, failed: "an unterminated template" };
      continue;
    }

    if (c === 47) {
      if (!regexOk) {
        i++;
        if (src.charCodeAt(i) === 61) i++; // `/=`
        last = "punct";
        lastPunct = "/";
        regexOk = true;
        continue;
      }
      i++;
      let inClass = false;
      for (;;) {
        if (i >= n) return { code: src, escaped: 0, kept: 0, failed: "an unterminated regular expression" };
        const d = src.charCodeAt(i);
        if (d === 10 || d === 13 || d === LS || d === PS) {
          return { code: src, escaped: 0, kept: 0, failed: `a line break inside a regular expression at ${i}` };
        }
        if (d === 92) {
          if (src.charCodeAt(i + 1) > 0x7f) i = literalChar(i + 1, true, "regex");
          else i += 2;
          continue;
        }
        if (d === 91) inClass = true;
        else if (d === 93) inClass = false;
        else if (d === 47 && !inClass) break;
        if (d > 0x7f) i = literalChar(i, false, "regex");
        else i++;
      }
      i++;
      while (i < n && isIdentPart(src.charCodeAt(i))) i++; // flags
      last = "value";
      regexOk = false;
      continue;
    }

    if (c === 40) {
      parens.push(last === "word" && PAREN_STATEMENT.has(lastWord));
      i++;
      last = "punct";
      lastPunct = "(";
      regexOk = true;
      continue;
    }
    if (c === 41) {
      if (!parens.length) return { code: src, escaped: 0, kept: 0, failed: `an unmatched ) at ${i}` };
      const statement = parens.pop()!;
      i++;
      last = statement ? "punct" : "value";
      lastPunct = ")";
      regexOk = statement;
      continue;
    }
    if (c === 123) {
      const block =
        last === "word"
          ? !BEFORE_EXPRESSION.has(lastWord) || lastWord === "else" || lastWord === "do"
          : last === "punct"
            ? [")", ";", "{", "}", "=>", ""].includes(lastPunct)
            : false;
      braces.push(block ? "block" : "expr");
      i++;
      last = "punct";
      lastPunct = "{";
      regexOk = true;
      continue;
    }
    if (c === 125) {
      const kind = braces.pop();
      if (!kind) return { code: src, escaped: 0, kept: 0, failed: `an unmatched } at ${i}` };
      i++;
      if (kind === "tpl") {
        if (!templateText(templates.pop()!)) {
          return { code: src, escaped: 0, kept: 0, failed: "an unterminated template" };
        }
        continue;
      }
      last = kind === "block" ? "punct" : "value";
      lastPunct = "}";
      regexOk = kind === "block";
      continue;
    }
    if (c === 91) {
      parens.push(false);
      i++;
      last = "punct";
      lastPunct = "[";
      regexOk = true;
      continue;
    }
    if (c === 93) {
      if (!parens.length) return { code: src, escaped: 0, kept: 0, failed: `an unmatched ] at ${i}` };
      parens.pop();
      i++;
      last = "value";
      lastPunct = "]";
      regexOk = false;
      continue;
    }

    if (isIdentPart(c) || c > 0x7f) {
      if (c > 0x7f) {
        const [cp, len] = cpAt(i);
        if (cp === LS || cp === PS) {
          put(i, i + len, "\n");
          i += len;
          continue;
        }
        if (/\s/u.test(String.fromCodePoint(cp))) {
          put(i, i + len, " ");
          i += len;
          continue;
        }
      }
      const start = i;
      while (i < n) {
        const d = src.charCodeAt(i);
        if (d > 0x7f) {
          const [cp, len] = cpAt(i);
          if (cp === LS || cp === PS || /\s/u.test(String.fromCodePoint(cp))) break;
          put(i, i + len, escapeIdent(cp));
          i += len;
        } else if (isIdentPart(d)) i++;
        else break;
      }
      const word = src.slice(start, i);
      if (c >= 48 && c <= 57) {
        while (i < n) {
          const d = src.charCodeAt(i);
          if (d === 46 || isIdentPart(d)) i++;
          else if ((d === 43 || d === 45) && /[eE]$/.test(src.slice(start, i)) && !/^0[xX]/.test(word)) i++;
          else break;
        }
        last = "value";
        regexOk = false;
        continue;
      }
      // After a `.` it is a property name, never a keyword: `Symbol.for(x)`, `it.return()`.
      const property = last === "punct" && lastPunct === ".";
      lastWord = property ? "" : word;
      last = "word";
      regexOk = !property && BEFORE_EXPRESSION.has(word);
      continue;
    }

    if (c === 46 && src.charCodeAt(i + 1) >= 48 && src.charCodeAt(i + 1) <= 57) {
      i++;
      while (i < n && (isIdentPart(src.charCodeAt(i)) || src.charCodeAt(i) === 46)) i++;
      last = "value";
      regexOk = false;
      continue;
    }

    if ((c === 43 || c === 45) && src.charCodeAt(i + 1) === c) {
      i += 2;
      continue;
    }

    let p = src[i]!;
    if (c === 61 && src.charCodeAt(i + 1) === 62) p = "=>";
    i += p.length;
    last = "punct";
    lastPunct = p;
    regexOk = true;
  }

  if (parens.length || braces.length) {
    return { code: src, escaped: 0, kept: 0, failed: `${parens.length} ( / [ and ${braces.length} { left open` };
  }
  if (copied < n) out.push(src.slice(copied));
  return { code: out.join(""), escaped, kept };
}

/**
 * Checks an escaped bundle against the original with Bun's parser: `null` when both are the same
 * program, otherwise why not. Regexes and tagged templates print as written, so those are compared
 * after both prints go through [`asciiOnly`].
 */
export function checkAscii(before: string, after: string): string | null {
  const t = new Bun.Transpiler({ loader: "js" });
  let a: string;
  let b: string;
  try {
    a = t.transformSync(before);
    b = t.transformSync(after);
  } catch (e) {
    return "the bundle does not parse (" + (e instanceof Error ? e.message : String(e)) + ")";
  }
  if (a === b) return null;
  const na = asciiOnly(a);
  const nb = asciiOnly(b);
  if (na.failed || nb.failed) return `the re-printed bundle did not lex (${na.failed ?? nb.failed})`;
  if (na.code === nb.code) return null;
  let at = 0;
  while (at < na.code.length && na.code[at] === nb.code[at]) at++;
  return `the escaped bundle reads differently near ${JSON.stringify(na.code.slice(Math.max(0, at - 40), at + 40))}`;
}

export function nonAsciiCount(s: string): number {
  let count = 0;
  for (let i = 0; i < s.length; i++) if (s.charCodeAt(i) > 0x7f) count++;
  return count;
}
