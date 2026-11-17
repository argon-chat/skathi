/** One searchable record: a docs section, a reference entry or a config option. */
export type SearchRecord = [
  group: number,
  title: string,
  crumb: string,
  url: string,
  summary: string,
  body: string,
  aliases: string,
  kind: string,
  status: string,
];

export interface SearchData {
  groups: string[];
  records: SearchRecord[];
}

export interface Segment {
  text: string;
  hit: boolean;
}

export interface SearchHit {
  id: number;
  group: string;
  title: Segment[];
  crumb: string;
  url: string;
  snippet: Segment[];
  kind: string;
  status: string;
  section: boolean;
}

export interface SearchGroup {
  label: string;
  hits: SearchHit[];
}

const enum Field {
  Title,
  Aliases,
  Crumb,
  Summary,
  Body,
}

const fieldWeights = [10, 6, 2, 3, 1.4] as const;
const perGroup = 6;
const perPage = 3;

/** "appWindow.setTitleBarOverlay()" and "<SkWindow backdrop>" compare as "appwindow.settitlebaroverlay", "skwindow backdrop". */
export function normalizeName(text: string): string {
  return text
    .toLowerCase()
    .replace(/[`"'<>]/g, "")
    .replace(/\(\s*\)|\(…\)/g, "")
    .replace(/\s+/g, " ")
    .trim();
}

function trimToken(chunk: string): string {
  return chunk.replace(/[.:\-#]+$/, "").replace(/^[.\-]+/, "");
}

/** Index terms: each word as written, plus its dotted, hyphenated and camelCase parts. */
export function indexTerms(text: string): string[] {
  const terms: string[] = [];
  for (const raw of text.split(/[^\p{L}\p{N}_@:.\-#$]+/u)) {
    const chunk = trimToken(raw);
    if (!chunk) continue;
    terms.push(chunk.toLowerCase());
    if (!/[.:\-@#$_]|\p{Ll}\p{Lu}/u.test(chunk)) continue;
    for (const segment of chunk.split(/[.:\-@#$_]+/)) {
      if (segment.length > 1 && segment.length < chunk.length) terms.push(segment.toLowerCase());
      if (!/\p{Ll}\p{Lu}/u.test(segment)) continue;
      for (const part of segment.split(/(?<=[\p{Ll}\p{N}])(?=\p{Lu})/u))
        if (part.length > 1) terms.push(part.toLowerCase());
    }
  }
  return terms;
}

export function queryTerms(query: string): string[] {
  return query
    .replace(/[()<>"'`,;[\]{}]/g, " ")
    .split(/\s+/)
    .map((chunk) => trimToken(chunk).toLowerCase())
    .filter((term) => term.length > 0);
}

/** Optimal string alignment distance, or `max + 1` once it exceeds `max`. */
function distance(a: string, b: string, max: number): number {
  if (Math.abs(a.length - b.length) > max) return max + 1;
  let before: number[] = [];
  let previous = Array.from({ length: b.length + 1 }, (_, j) => j);
  for (let i = 1; i <= a.length; i++) {
    const row = [i];
    let best = i;
    for (let j = 1; j <= b.length; j++) {
      const cost = a[i - 1] === b[j - 1] ? 0 : 1;
      let value = Math.min((previous[j] ?? 0) + 1, (row[j - 1] ?? 0) + 1, (previous[j - 1] ?? 0) + cost);
      if (i > 1 && j > 1 && a[i - 1] === b[j - 2] && a[i - 2] === b[j - 1])
        value = Math.min(value, (before[j - 2] ?? 0) + 1);
      row.push(value);
      best = Math.min(best, value);
    }
    if (best > max) return max + 1;
    before = previous;
    previous = row;
  }
  return previous[b.length] ?? max + 1;
}

function lowerBound(sorted: readonly string[], value: string): number {
  let low = 0;
  let high = sorted.length;
  while (low < high) {
    const mid = (low + high) >> 1;
    if ((sorted[mid] ?? "") < value) low = mid + 1;
    else high = mid;
  }
  return low;
}

function escapeRegExp(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function segments(text: string, pattern: RegExp | undefined): Segment[] {
  if (!pattern) return [{ text, hit: false }];
  const out: Segment[] = [];
  let last = 0;
  for (const match of text.matchAll(pattern)) {
    const start = match.index;
    if (start > last) out.push({ text: text.slice(last, start), hit: false });
    out.push({ text: match[0], hit: true });
    last = start + match[0].length;
  }
  if (last < text.length) out.push({ text: text.slice(last), hit: false });
  return out;
}

function snippet(text: string, pattern: RegExp | undefined, length = 150): Segment[] {
  if (text.length <= length) return segments(text, pattern);
  let first = 0;
  if (pattern) {
    pattern.lastIndex = 0;
    first = pattern.exec(text)?.index ?? 0;
    pattern.lastIndex = 0;
  }
  let start = Math.max(0, first - 45);
  if (start > 0) start = text.indexOf(" ", start) + 1 || start;
  let end = Math.min(text.length, start + length);
  if (end < text.length) end = text.lastIndexOf(" ", end) > start ? text.lastIndexOf(" ", end) : end;
  return segments(`${start > 0 ? "…" : ""}${text.slice(start, end)}${end < text.length ? "…" : ""}`, pattern);
}

interface Scored {
  id: number;
  score: number;
  tier: number;
  matched: number;
  terms: Set<string>;
}

export class SearchIndex {
  private readonly records: SearchRecord[];
  private readonly groups: string[];
  private readonly terms: string[];
  private readonly postings = new Map<string, Map<number, number>>();
  private readonly names: string[][];

  constructor(data: SearchData) {
    this.records = data.records;
    this.groups = data.groups;
    const lengths = this.records.map((record) => record[5].length);
    const average = lengths.reduce((sum, length) => sum + length, 0) / Math.max(1, lengths.length) || 1;
    this.records.forEach((record, id) => {
      const fields = [record[1], record[6], record[2], record[4], record[5]];
      const weights = new Map<string, number>();
      fields.forEach((text, field) => {
        if (!text) return;
        const counts = new Map<string, number>();
        for (const term of indexTerms(text)) counts.set(term, (counts.get(term) ?? 0) + 1);
        const norm = field === Field.Body ? 1.2 * (0.25 + (0.75 * text.length) / average) : 0;
        for (const [term, count] of counts) {
          const weight =
            field === Field.Body ? (fieldWeights[field] * count * 2.2) / (count + norm) : (fieldWeights[field] ?? 1);
          weights.set(term, (weights.get(term) ?? 0) + weight);
        }
      });
      for (const [term, weight] of weights) {
        let posting = this.postings.get(term);
        if (!posting) this.postings.set(term, (posting = new Map()));
        posting.set(id, weight);
      }
    });
    this.terms = [...this.postings.keys()].sort();
    this.names = this.records.map((record) => {
      const title = normalizeName(record[1]);
      const names = [title];
      const dot = title.lastIndexOf(".");
      if (dot > 0 && dot < title.length - 1) names.push(title.slice(dot + 1));
      for (const alias of record[6].split("\n")) if (alias) names.push(normalizeName(alias));
      return names;
    });
  }

  /** Index terms for a query term with their weight: exact, prefix (stronger while it is still being typed), typo. */
  private expand(term: string, typing: boolean): Map<string, number> {
    const found = new Map<string, number>();
    if (this.postings.has(term)) found.set(term, 1);
    let index = lowerBound(this.terms, term);
    for (let taken = 0; taken < 80 && index < this.terms.length; index++) {
      const candidate = this.terms[index] ?? "";
      if (!candidate.startsWith(term)) break;
      if (candidate === term) continue;
      found.set(candidate, (typing ? 0.55 : 0.25) + (0.3 * term.length) / candidate.length);
      taken++;
    }
    const max = term.length < 4 ? 0 : term.length < 7 ? 1 : 2;
    if (max === 0) return found;
    for (const candidate of this.terms) {
      if (found.has(candidate) || Math.abs(candidate.length - term.length) > max) continue;
      const d = distance(term, candidate, max);
      if (d <= max) found.set(candidate, d === 1 ? 0.45 : 0.25);
    }
    return found;
  }

  private tier(id: number, query: string): number {
    const names = this.names[id] ?? [];
    if (names[0] === query) return 4;
    if (names.includes(query)) return 3;
    if (names[0]?.startsWith(query) && query.length > 1) return 2;
    if (names.some((name) => name.startsWith(query)) && query.length > 2) return 1;
    return 0;
  }

  search(query: string): SearchGroup[] {
    const terms = [...new Set(queryTerms(query))];
    if (!terms.length) return [];
    const full = normalizeName(query);
    const scores = new Map<number, Scored>();
    terms.forEach((term, position) => {
      const best = new Map<number, [number, string]>();
      for (const [candidate, factor] of this.expand(term, position === terms.length - 1)) {
        const posting = this.postings.get(candidate);
        if (!posting) continue;
        const idf = Math.log(1 + this.records.length / posting.size);
        for (const [id, weight] of posting) {
          const score = factor * idf * weight;
          const current = best.get(id);
          if (!current || current[0] < score) best.set(id, [score, candidate]);
        }
      }
      for (const [id, [score, candidate]] of best) {
        const entry = scores.get(id) ?? { id, score: 0, tier: 0, matched: 0, terms: new Set<string>() };
        entry.score += score;
        entry.matched++;
        entry.terms.add(candidate);
        scores.set(id, entry);
      }
    });
    let ranked = [...scores.values()];
    const complete = ranked.filter((entry) => entry.matched === terms.length);
    if (complete.length) ranked = complete;
    const phrase = terms.length > 1 ? terms.join(" ") : "";
    for (const entry of ranked) {
      entry.tier = this.tier(entry.id, full);
      const record = this.records[entry.id];
      if (!record) continue;
      if (!record[7] && !record[3].includes("#")) entry.score *= 1.15;
      if (phrase && `${record[1]} ${record[4]} ${record[5]}`.toLowerCase().includes(phrase)) entry.score *= 1.6;
    }
    ranked.sort((a, b) => b.matched - a.matched || b.tier - a.tier || b.score - a.score);

    const floor = Math.max(...ranked.map((entry) => entry.score), 0) * 0.08;
    const groups = new Map<number, SearchHit[]>();
    const pages = new Map<string, number>();
    for (const entry of ranked) {
      const record = this.records[entry.id];
      if (!record || (entry.tier === 0 && entry.score < floor)) continue;
      const [group, title, crumb, url, summary, body, , kind, status] = record;
      const hits = groups.get(group) ?? [];
      if (hits.length >= perGroup) continue;
      const page = url.split("#")[0] ?? url;
      if (!kind) {
        const seen = pages.get(page) ?? 0;
        if (seen >= perPage) continue;
        pages.set(page, seen + 1);
      }
      const words = [...entry.terms].sort((a, b) => b.length - a.length).map(escapeRegExp);
      const short = words.filter((word) => word.length < 4);
      const long = words.filter((word) => word.length >= 4);
      const alternatives = [...long, ...short.map((word) => `(?<![\\p{L}\\p{N}])${word}`)];
      const pattern = alternatives.length ? new RegExp(alternatives.join("|"), "giu") : undefined;
      const probe = pattern && new RegExp(pattern.source, "iu");
      const text = summary && (!probe || probe.test(summary) || !probe.test(body)) ? summary : body;
      hits.push({
        id: entry.id,
        group: this.groups[group] ?? "",
        title: segments(title, pattern),
        crumb,
        url,
        snippet: snippet(text, pattern),
        kind,
        status,
        section: url.includes("#"),
      });
      groups.set(group, hits);
    }
    return [...groups].map(([group, hits]) => ({ label: this.groups[group] ?? "", hits }));
  }
}
