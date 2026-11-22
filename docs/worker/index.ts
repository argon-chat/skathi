import { fetchStats } from "../src/lib/stats";

interface Env {
  ASSETS: { fetch(request: Request): Promise<Response> };
  GITHUB_TOKEN?: string;
  VISITS?: { writeDataPoint(point: { blobs: string[]; doubles: number[]; indexes: string[] }): void };
}

interface Context {
  waitUntil(promise: Promise<unknown>): void;
}

async function stats(request: Request, env: Env, context: Context): Promise<Response> {
  const cache = await caches.open("stats");
  const key = new Request(new URL("/api/stats", request.url));
  const cached = await cache.match(key);
  if (cached) return cached;
  const body = await fetchStats(env.GITHUB_TOKEN);
  const complete = body.npm !== undefined && body.stars !== undefined;
  const response = Response.json(body, {
    status: Object.keys(body).length ? 200 : 502,
    headers: {
      "Cache-Control": `public, max-age=${complete ? 3600 : 300}`,
      "Access-Control-Allow-Origin": "*",
    },
  });
  if (response.ok) context.waitUntil(cache.put(key, response.clone()));
  return response;
}

/** A visit that arrived by a campaign link: its utm_ values and the page, nothing about the visitor. */
function visit(request: Request, env: Env): Response {
  const query = new URL(request.url).searchParams;
  const field = (name: string) => (query.get(name) ?? "").slice(0, 96);
  const source = field("utm_source");
  if (request.method === "POST" && source) {
    env.VISITS?.writeDataPoint({
      blobs: [source, field("utm_medium"), field("utm_campaign"), field("utm_content"), field("path")],
      doubles: [1],
      indexes: [source.slice(0, 32)],
    });
  }
  return new Response(null, { status: 204 });
}

export default {
  async fetch(request: Request, env: Env, context: Context): Promise<Response> {
    const { pathname } = new URL(request.url);
    if (pathname === "/api/stats") return stats(request, env, context);
    if (pathname === "/api/visit") return visit(request, env);
    return env.ASSETS.fetch(request);
  },
};
