# Skathi documentation

Astro, Vue 3, strict TypeScript, Markdown/MDX, Shiki, and Pagefind. This package belongs to the Bun workspace in `crates/vue-native`; install dependencies there with `bun install`.

From the workspace root:

```sh
bun run docs:dev
bun run docs:check
bun run docs:build
```

From `docs/`:

```sh
bun run dev                  # localhost:4321
bun run check                # Astro and Vue TypeScript checks
bun fmt                      # Bun formatter; Prettier for .astro files
bun run fmt:check
bun run build                # static site and Pagefind search index in dist/
bun run test                 # integration checks against the built output
bun run preview              # inspect the production site and search
bun run preview:cloudflare   # build and serve through Wrangler
```

Requires Bun >=1.4.4 and a Node version supported by Astro (>=22.23.4; Node 24 recommended). Bun installs, runs scripts, formats supported files, and runs integration tests. Astro's compiler and Vite build the site; `Bun.build` is not a replacement for the Astro/MDX/Vue pipeline. No Vite Plus wrapper is required. `bun --bun run build` is also verified and runs the build using Bun as the JavaScript runtime.

## Content

Add `.md` or `.mdx` under `pages/`, including nested directories. The existing `css.md` and `web-platform.md` remain at their original paths and are also loaded. `README.md` is excluded. Frontmatter is validated:

```yaml
---
title: Window lifecycle
description: Open, update, and close a native window.
section: Reference
order: 3
label: Windows
---
```

Sections are `Getting started`, `Reference`, and `Contributing`. Navigation and previous/next links follow the section and `order`. `label` is optional. `pages/index.md` is `/`; `pages/guides/windows.md` is `/guides/windows/`. Use level-two and level-three headings for the outline; the layout already supplies the page title.

Use site paths for links (`/css/`). Relative `.md` and `.mdx` links are also converted to site paths for HTML output. Sätteri, Astro 7's native Markdown processor, handles both Markdown and MDX. MDX supports imported Vue components with `client:visible`, `client:load`, and other Astro directives. See `pages/authoring.mdx` for a working example and fences for `slang`, `js`, `ts`, and `rust`.

Slang highlighting uses the official `shader-slang/slang-vscode-extension` TextMate grammar, vendored in `vendor/slang/` with its license and pinned revision. Other languages use Shiki's built-in grammars. No highlighter is shipped to the browser.

Pagefind indexes the built article content and loads only when a reader searches. Development uses a local content search; use `bun run preview` to test Pagefind itself. The website includes responsive navigation, a sticky outline, code copying, persistent light/dark/system themes, and a custom 404 page.

## Agents and SEO

Every page is rendered to HTML and exported at `/<page-id>.md` (`/index.md` for the home page). `/llms.txt` lists pages and `/llms-full.txt` includes every page body. HTML advertises Markdown through `rel="alternate"`; **Copy page** uses the same source. MDX exports intentionally preserve source imports and component markup.

Titles, descriptions, canonical URLs, Open Graph/Twitter tags, a social image, JSON-LD, `robots.txt`, and a sitemap are generated. Set `SITE_URL` before a production build; local development defaults to `http://localhost:4321`. `.env.example` shows the variable. There is no assumed production domain.

## Cloudflare Workers

`wrangler.jsonc` publishes `dist/` as Worker static assets. A server adapter is unnecessary. `_headers` sets Markdown MIME types, cache headers for hashed assets, and basic response headers. Unknown paths serve the generated `404.html` with a 404 response.

Authenticate once with `bunx wrangler login`, then set the real domain and deploy (PowerShell):

```powershell
$env:SITE_URL = 'https://your-docs-domain.example'
bun run deploy:dry-run
bun run deploy
```

For CI, supply `SITE_URL`, `CLOUDFLARE_API_TOKEN`, and `CLOUDFLARE_ACCOUNT_ID` as environment variables. Change the Worker name in `wrangler.jsonc` and configure its domain in Cloudflare as needed. The deploy command builds before uploading; no deployment is performed during scaffolding.
