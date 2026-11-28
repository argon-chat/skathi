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
bun run check:layout         # build, then check the landing's card rows and intros (headless Chromium)
bun run preview              # inspect the production site and search
bun run preview:cloudflare   # build and serve through Wrangler
```

`check:layout` renders the landing at 1440, 1100 and 390 px in both themes and fails when the columns of a `CardRow` end more than 1px apart or a grid of cards bypasses `CardRow`, and when a section heading sits outside `SectionIntro` or an intro (other than a centred one) leaves the right of the row empty or ends its text and aside apart. It needs a Chromium: `CHROMIUM_PATH`, a Playwright download (`bunx playwright install chromium`), or an installed Chrome or Edge. `bun scripts/check-layout.ts --url http://localhost:4321/` checks a running dev server instead of a build.

Requires Bun >=1.4.4 and a Node version supported by Astro (>=22.23.4; Node 24 recommended). Bun installs, runs scripts, formats supported files, and runs integration tests. Astro's compiler and Vite build the site; `Bun.build` is not a replacement for the Astro/MDX/Vue pipeline. No Vite Plus wrapper is required. `bun --bun run build` is also verified and runs the build using Bun as the JavaScript runtime.

## Content

### API references

The API reference reads the SDK checkout during the site build. On a fresh clone, run
`git submodule update --init vendor/redjs` from the SDK root. HTML reference pages, Markdown downloads
and search results use the same source data.

| Section                    | Where to change the API                                                                                                                                                                 | What the site extracts                                                                                                                                                                                                                                                          |
| -------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Web                        | The host's Rust bindings and the app-facing declarations in `packages/runtime/env.d.ts`, `src/engine.d.ts` and `src/indexeddb.d.ts` under `packages/runtime`                            | Global values, interfaces, overloads and JSDoc. Matching Rust `ClassSpec` registrations supply the prototype's methods, getters, setters and implementation path.                                                                                                               |
| Standard library           | The public `skathi:*` module exports in `packages/runtime/env.d.ts` and the definitions they re-export                                                                                  | Exported types and values grouped by module, signatures and JSDoc. A native class can also expose its Rust prototype in its reference page.                                                                                                                                     |
| Shader library and effects | Public declarations and `///` comments in `packages/cli/effects/sk.slang` and `packages/cli/effects/sk/*.slang`. Built-in effect structs and attributes in `crates/sk-effects/shaders/` | Library overloads, types and members come from Slang source. Effect names, parameter order, ranges and defaults come from the checked-in compiler reflection in `crates/sk-effects/generated/effects.json`. Run the existing `bun run shaders` after editing a built-in effect. |
| CSS                        | `CATALOG` in `crates/sk-css/src/catalog.rs` and the prefix allowlist in `crates/sk-css/src/prefix.rs`                                                                                   | A page per catalogued property with mapper status, example values it applies or refuses, implementation notes and accepted aliases. Linux CI compares the extracted records with the compiled Rust catalog.                                                                     |

For example, `DeadZoneTarget` and its JSDoc appear under Types on `/std/gamepads/` because the gamepad API
uses it. A search for the type opens its definition there. Exports without a guide or an associated
type section appear in their module's catalog. The overview at `/std/` takes each module's description
from the corresponding property of `stdModules` in `packages/runtime/src/std/index.ts`. Adding a public library function in
`sk/math.slang` creates a shader page with its overloads and first `///` paragraph. Existing guides
can keep their URLs and add usage examples to those signatures.

Web coverage pages include the SDK signatures and associated types. A registered Web interface without
a guide can appear under Native interfaces. Runtime availability, feature flags, release history, platform differences
and examples belong on the relevant coverage page. Rust prototype details cover statically identifiable
registrations. Dynamic names, macros and members installed separately can still be described in the public
declaration. To document a Rust class at its implementation, add a `///` comment to a named `ClassSpec`
constant and pass it to the registration function in the same file. See `TimeSpan` in
`crates/sk-app/src/stdlib/time.rs`.

CSS selectors, media queries and at-rules have their own reference entries outside the property catalog.
Both the CSS and Web pages look up MDN links, experimental flags and deprecations in the pinned
`@mdn/browser-compat-data` package at build time. Run `bun run bcd:check` in `docs/` to check key overrides.

Run `bun run docs:api:check` from the SDK root to validate extraction, or `bun run docs:build` to render the site.

### Guides and Markdown pages

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

Sections are `Getting started`, `Guides`, `Devtools`, `Embedding`, `Reference`, and `Contributing`. Navigation and previous/next links follow the section and `order`. `label` is optional. `pages/index.md` is `/`; `pages/guides/windows.md` is `/guides/windows/`. Use level-two and level-three headings for the outline; the layout already supplies the page title.

Use site paths for links (`/css/`). Relative `.md` and `.mdx` links are also converted to site paths for HTML output. Sätteri, Astro 7's native Markdown processor, handles both Markdown and MDX. MDX supports imported Vue components with `client:visible`, `client:load`, and other Astro directives. See `pages/authoring.mdx` for a working example and fences for `slang`, `js`, `ts`, and `rust`.

Slang highlighting uses the official `shader-slang/slang-vscode-extension` TextMate grammar, vendored in `vendor/slang/` with its license and pinned revision. Other languages use Shiki's built-in grammars. No highlighter is shipped to the browser.

Pagefind indexes the built article content and loads only when a reader searches. Development uses a local content search; use `bun run preview` to test Pagefind itself. The website includes responsive navigation, a sticky outline, code copying, persistent light/dark/system themes, and a custom 404 page.

## Agents and SEO

Every page is rendered to HTML and exported at `/<page-id>.md` (`/index.md` for the home page). `/llms.txt` lists pages and `/llms-full.txt` includes every page body. HTML advertises Markdown through `rel="alternate"`; **Copy page** uses the same source. MDX exports intentionally preserve source imports and component markup.

Titles, descriptions, canonical URLs, Open Graph/Twitter tags, a social image, JSON-LD, `robots.txt`, and a sitemap are generated. Set `SITE_URL` before a production build; local development defaults to `http://localhost:4321`. `.env.example` shows the variable. There is no assumed production domain.

## Cloudflare Workers

`wrangler.jsonc` publishes `dist/` as Worker static assets. A server adapter is unnecessary. `_headers` sets Markdown MIME types, cache headers for hashed assets, and basic response headers. `_redirects` keeps moved pages reachable at their old URLs. Unknown paths serve the generated `404.html` with a 404 response.

Authenticate once with `bunx wrangler login`, then set the real domain and deploy (PowerShell):

```powershell
$env:SITE_URL = 'https://your-docs-domain.example'
bun run deploy:dry-run
bun run deploy
```

For CI, supply `SITE_URL`, `CLOUDFLARE_API_TOKEN`, and `CLOUDFLARE_ACCOUNT_ID` as environment variables. Change the Worker name in `wrangler.jsonc` and configure its domain in Cloudflare as needed. The deploy command builds before uploading; no deployment is performed during scaffolding.
