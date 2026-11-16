import { fetchStats } from "../src/lib/stats";

interface Env {
  ASSETS: { fetch(request: Request): Promise<Response> };
}

interface Context {
  waitUntil(promise: Promise<unknown>): void;
}

const statsTtl = 3600;

async function stats(request: Request, context: Context): Promise<Response> {
  const cache = await caches.open("stats");
  const key = new Request(new URL("/api/stats", request.url));
  const cached = await cache.match(key);
  if (cached) return cached;
  const body = await fetchStats();
  if (!body) return Response.json({ error: "npm or GitHub did not answer" }, { status: 502 });
  const response = Response.json(body, {
    headers: { "Cache-Control": `public, max-age=${statsTtl}`, "Access-Control-Allow-Origin": "*" },
  });
  context.waitUntil(cache.put(key, response.clone()));
  return response;
}

export default {
  async fetch(request: Request, env: Env, context: Context): Promise<Response> {
    if (new URL(request.url).pathname === "/api/stats") return stats(request, context);
    return env.ASSETS.fetch(request);
  },
};
