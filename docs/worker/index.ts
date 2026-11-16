import { fetchStats } from "../src/lib/stats";

interface Env {
  ASSETS: { fetch(request: Request): Promise<Response> };
  GITHUB_TOKEN?: string;
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

export default {
  async fetch(request: Request, env: Env, context: Context): Promise<Response> {
    if (new URL(request.url).pathname === "/api/stats") return stats(request, env, context);
    return env.ASSETS.fetch(request);
  },
};
