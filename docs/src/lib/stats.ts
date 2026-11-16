export interface Stats {
  npm: number;
  stars: number;
}

const repo = "argon-chat/skathi";
const github = { headers: { "User-Agent": "skathi-docs", Accept: "application/vnd.github+json" } };

async function json<T>(url: string, init?: RequestInit): Promise<T | undefined> {
  try {
    const response = await fetch(url, init);
    return response.ok ? ((await response.json()) as T) : undefined;
  } catch {
    return undefined;
  }
}

// npm answers at most 18 months per range; every app install pulls the CLI, so it stands for installs.
async function npmDownloads(name: string): Promise<number | undefined> {
  const end = new Date();
  const start = new Date(end);
  start.setUTCMonth(start.getUTCMonth() - 18);
  const day = (date: Date) => date.toISOString().slice(0, 10);
  const body = await json<{ downloads?: number }>(
    `https://api.npmjs.org/downloads/point/${day(start)}:${day(end)}/${name}`,
  );
  return body?.downloads;
}

async function stars(): Promise<number | undefined> {
  return (await json<{ stargazers_count?: number }>(`https://api.github.com/repos/${repo}`, github))?.stargazers_count;
}

export async function fetchStats(): Promise<Stats | undefined> {
  const [npm, starCount] = await Promise.all([npmDownloads("@skathi-native/cli"), stars()]);
  if (npm === undefined || starCount === undefined) return undefined;
  return { npm, stars: starCount };
}
