import { createRequire } from "node:module";

export type BcdData = Readonly<Record<string, unknown>>;
export const isRecord = (value: unknown): value is BcdData =>
  typeof value === "object" && value !== null && !Array.isArray(value);

// Resolved by Node during static rendering. Only the selected metadata reaches catalog islands.
export const browserCompatData: BcdData = createRequire(import.meta.url)("@mdn/browser-compat-data");

export interface CompatInfo {
  key: string;
  mdn?: string;
  experimental?: true;
  deprecated?: true;
}

/** A subfeature may inherit its MDN page from a parent. Status always comes from the subfeature itself. */
export function findCompat(data: BcdData, keys: readonly string[], firstPageIndex: number): CompatInfo | undefined {
  for (const key of keys) {
    let node: unknown = data;
    let mdn: string | undefined;
    for (const [index, part] of key.split(".").entries()) {
      node = isRecord(node) ? node[part] : undefined;
      const compat = isRecord(node) ? node["__compat"] : undefined;
      if (index >= firstPageIndex && isRecord(compat) && typeof compat["mdn_url"] === "string") mdn = compat["mdn_url"];
    }
    const compat = isRecord(node) ? node["__compat"] : undefined;
    if (!isRecord(compat)) continue;
    const status = compat["status"];
    return {
      key,
      ...(mdn !== undefined && { mdn }),
      ...(isRecord(status) && status["experimental"] === true && { experimental: true }),
      ...(isRecord(status) && status["deprecated"] === true && { deprecated: true }),
    };
  }
  return undefined;
}
