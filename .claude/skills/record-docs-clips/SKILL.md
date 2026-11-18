---
name: record-docs-clips
description: Record or re-record the docs site's demo clips (webm, mp4, poster) of a Skathi app window on the secondary monitor, with acrylic, synthetic input and a drawn cursor. Use when a docs page needs a new clip, a clip is outdated, or a demo app changed.
---

# Recording docs demo clips

Clips come from a demo app in `apps/` driven by a plan in `scripts/record/plans/<plan>.ts`, captured by
`scripts/record/record.ts` and written to `docs/public/media/<group>/` as `<clip>.webm` (AV1),
`<clip>.mp4` (H.264) and `<clip>-poster.webp`, 1280×834 at 30 fps. Pages show them with
`<Demo id="<group>/<clip>">`.

## Rules

- **Acrylic, always.** Every recorded window has `"backdrop": "acrylic"` in its `sk.config.json`, no
  solid `backgroundColor`, and translucent page backgrounds (solid only as a fallback when there is
  no backdrop). The secondary monitor carries the Skathi wallpaper so the acrylic shows it. The
  harness refuses a monitor with a plain-colour wallpaper and a window whose backdrop is `none`;
  never work around those checks, fix the app.
- **Everything in git.** The demo app, its generated assets' generator, the plan and any harness change
  are committed with the media. Nothing a clip depends on lives only in a scratch or capture folder.
- **Re-record in place.** A new take overwrites the existing files under the same names, so pages need
  no edits.
- **Hands off the desktop.** Only the secondary monitor is used; the harness refuses the primary.
  Windows are placed there before they first show and are never activated. Input is synthetic
  (`driver.ts`) and the cursor is drawn by the app; never move the real mouse or type on the real
  keyboard.
- **Nothing that dates.** Text in a demo page names no platform, backend or version ("Windows only",
  "WebView2"): support changes and the clip would have to be re-recorded. Platform notes belong on the
  docs page.
- **Loops.** A plan returns `{ start, end }`: two moments with the same picture. Continuous animations
  on the page need periods that divide that window, so the clip loops without a jump.

## Steps

1. Build the host (or pass `--build`):
   `cargo build --target x86_64-pc-windows-msvc -p sk-app`, plus `--example surface` for a clip with
   `host: "surface"`.
2. Look before recording: `bun scripts/record/record.ts <plan> [clip...] --out <dir> --shots`, then
   read the stills in `<dir>/shots/`. Check the wallpaper shows through, nothing is cut off, and the
   page is in the state the clip starts from.
3. Record, from the repo root:
   - one clip: `bun scripts/record/record.ts elements video --out <dir>`
   - all of a plan's clips: `bun scripts/record/record.ts elements --out <dir>`

   `--out` takes the lossless captures, which are large: put them on a drive with room, outside the
   repo. `--encode-only` re-encodes them, `--media <dir>` writes the clips elsewhere,
   `--display \\.\DISPLAYn` picks the monitor when there is more than one secondary, `--hold <s>`
   launches and waits for poking at the page with devtools.

4. Review: the poster plus a few frames of each clip (ffmpeg `-ss <t> -frames:v 1`), and the first and
   last frame match. Then make sure every app, example and ffmpeg process the run started has exited.
5. Commit the media together with the app and plan changes (`docs(<group>): …`). Check `git status`
   for `??` first and stage only the run's own files.

## A new clip or app

- New clip in an existing app: add a page to the app (with `data-demo="…"` hooks for the driver), then
  a `clips` entry in its plan with `plan`, `poster` (fraction of the clip) and, if the page needs time
  to load, `settle` in ms.
- New app: copy `apps/elements` (its `demo.ts` exposes `__demo.show(clip)`, `CursorLayer.vue` draws the
  cursor), add it to `apps/tsconfig.json`, and add `scripts/record/plans/<plan>.ts` with `app`, `group`
  (the media folder) and `window` size.
- An element that opens an OS window (the colour dialog, a file picker) can't be captured: show the
  same result through in-page controls instead.
