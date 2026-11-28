/** The library interface is source-level. Effect reflection only sees parameters reachable from an entry point. */
export interface Declaration {
  name: string;
  kind: "struct" | "interface" | "enum" | "namespace" | "attribute" | "function" | "method" | "field" | "constant";
  signature: string;
  summary: string;
  members: Declaration[];
}

/** Walk declaration scopes, skipping function bodies, comments and strings as units. */
export function declarations(source: string): Declaration[] {
  const tokens =
    source.match(/\/\/[^\n]*|\/\*[\s\S]*?\*\/|"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|[{};]|[^/"'{};]+|./g) ?? [];
  let at = 0;

  function block(): string {
    const start = at;
    let depth = 1;
    while (at < tokens.length) {
      const token = tokens[at++];
      if (token === "{") depth++;
      if (token === "}" && --depth === 0) return tokens.slice(start, at - 1).join("");
    }
    throw new Error("Unclosed Slang declaration body");
  }

  function scope(parent?: Declaration): Declaration[] {
    const results: Declaration[] = [];
    let text = "";
    let doc: string[] = [];
    while (at < tokens.length) {
      const token = tokens[at++]!;
      if (token.startsWith("//") || token.startsWith("/*")) {
        if (token.startsWith("///")) doc.push(token.slice(3).trim());
        continue;
      }
      if (token === "}") break;
      if (token !== ";" && token !== "{") {
        text += token;
        continue;
      }
      const annotated = text.trim().replace(/\s+/g, " ");
      const head = annotated.replace(/^(?:\[+[^\]]+\]+\s*)+/, "");
      const summary = doc
        .join("\n")
        .split(/\n\s*\n/, 1)[0]!
        .replace(/\s+/g, " ")
        .trim();
      text = "";
      doc = [];
      if (!head.startsWith("public ") && !(head && parent?.kind === "interface")) {
        if (token === "{") block();
        continue;
      }
      const signature = head.replace(/^public /, "");
      const container = /^(struct|interface|enum|namespace|cbuffer)\s+(\w+)/.exec(signature);
      if (container && token === "{") {
        const kind = container[1]!;
        if (kind === "cbuffer") {
          block();
          continue;
        }
        const attribute = annotated.includes("__AttributeUsage(");
        const name = attribute ? `[${container[2]!.replace(/Attribute$/, "")}]` : container[2]!;
        const entry: Declaration = {
          name,
          kind: attribute ? "attribute" : (kind as "struct" | "interface" | "enum" | "namespace"),
          signature,
          summary,
          members: [],
        };
        if (kind === "enum") {
          entry.signature += ` { ${block()
            .replace(/\/\/[^\n]*|\/\*[\s\S]*?\*\//g, "")
            .trim()
            .replace(/\s+/g, " ")} }`;
        } else {
          entry.members = scope(entry);
        }
        results.push(entry);
        continue;
      }
      if (token === "{") block();
      // Registered resources belong to the renderer's private binding layout.
      if (/\s:\s*register\s*\(/.test(signature)) continue;
      const left = signature.split(/[=(]/, 1)[0]!.trim();
      const fn = signature.slice(left.length).trimStart().startsWith("(");
      const name = /(\w+)(?:<[^<>]*>)?(?:\[[^\]]*\])?$/.exec(left)?.[1];
      if (!name) throw new Error(`Unsupported public Slang declaration: ${head}`);
      results.push({
        name,
        kind: fn
          ? parent && parent.kind !== "namespace"
            ? "method"
            : "function"
          : /\bconst\b/.test(left)
            ? "constant"
            : "field",
        signature: signature + ";",
        summary,
        members: [],
      });
    }
    return results;
  }
  return scope();
}

/** A type's page includes its public members, including interface requirements without an explicit visibility. */
export function signatureOf(decl: Declaration, showDoc: (node: Declaration) => boolean): string {
  const comment = showDoc(decl) && decl.summary ? `/// ${decl.summary}\n` : "";
  if (decl.kind === "attribute") {
    const fields = decl.members.map((m) => m.signature.replace(/;$/, "")).join(", ");
    return fields ? `[${decl.name.slice(1, -1)}(${fields})]` : decl.name;
  }
  if (["struct", "interface", "namespace"].includes(decl.kind)) {
    if (!decl.members.length) return `${comment}${decl.signature} {}`;
    const body = decl.members
      .map((member) => signatureOf(member, showDoc))
      .join("\n")
      .replace(/^/gm, "    ");
    return `${comment}${decl.signature}\n{\n${body}\n}`;
  }
  return comment + decl.signature;
}
