---
title: Memory use
description: 'Where a Skathi app''s memory goes, how to read the numbers, and the settings that lower them: "memory": "low", the AOT memory profile and what the GPU driver holds.'
section: Guides
order: 6.5
since: "0.7"
---

# Memory use

A Skathi app is one process: the host, the JS engine, the renderer and the GPU driver's user-mode part
all live in it. This page says which of them holds what, which numbers to compare, and what you can
turn down.

## Which number to read

Task Manager and the devtools show several figures for the same process. They answer different
questions.

| Figure                                        | What it counts                                                               | Use it for                                    |
| --------------------------------------------- | ---------------------------------------------------------------------------- | --------------------------------------------- |
| Private working set (Task Manager's "Memory") | Private pages that are in RAM right now                                      | What the app costs the machine at this moment |
| Private bytes (commit)                        | Private memory the process has reserved a place for, in RAM or the page file | What the app can grow to without asking again |
| Working set                                   | Private pages plus the shared ones: DLLs, fonts, mapped files                | Little: most of the shared part is not yours  |

Private bytes are usually well above the private working set. An allocator commits memory ahead of
need and keeps freed memory for reuse, and Windows charges commit for a whole mapping when it is
made, touched or not. So a high commit figure with a low working set is memory that is promised, not
used.

## See where it goes

The **Performance** tab of the [devtools](/devtools/performance/) has a memory breakdown by category
under the frame chart, with **Refresh memory**, **Measure RAM** and **Force GC**. The same report is
available to scripts as the [`mem` command](/devtools/scripting/). It is available on Windows and
macOS.

Press **Force GC** before you compare two readings: without it the JS heap holds garbage that the
next collection would free anyway.

This is the breakdown of a small app (one window, a few hundred elements) in a debug host on
Windows 11 with an NVIDIA card, after a forced GC. Your numbers will differ; the proportions are
what to look at.

| Category                             | Committed | Of it in use                            |
| ------------------------------------ | --------- | --------------------------------------- |
| JS engine                            | 64 MB     | 9 MB of live allocations, the rest held |
| Rust heap (renderer, layout, styles) | 14 MB     | 10 MB                                   |
| GPU driver and other native heaps    | 134 MB    | —                                       |
| Thread stacks                        | 5 MB      | 5 MB                                    |
| **Private bytes in total**           | 256 MB    | 119 MB in RAM (private working set)     |

Two things stand out. The JS engine holds seven times what it uses, and the largest part is not
Skathi's at all: it is the graphics driver's user-mode half, which every GPU-rendered app carries.

## "memory": "low"

The JS engine's allocator (mimalloc) commits a page whole when it takes one and keeps freed memory
for a moment, because it is likely to be asked for again. `"memory": "low"` in
[sk.config.json](/sk-config/#memory) turns both off:

```json
{
  "memory": "low"
}
```

- A page is committed as it fills.
- Freed memory goes back to the system at once.
- The same two settings are put into the process's environment (`MIMALLOC_PAGE_COMMIT_ON_DEMAND=1`,
  `MIMALLOC_PURGE_DELAY=0`) before the GPU driver loads, unless they are already set. NVIDIA's driver
  carries its own copy of the same allocator and reads them. Processes the app starts inherit them.
  On macOS and Linux, the standalone `sk-app` executable sets these defaults at launch. When calling
  `skathi::run_exe()` from your own executable, set them in its launch environment to configure the driver.

The same app as above:

|                         | `"default"` | `"low"` |
| ----------------------- | ----------- | ------- |
| Private bytes           | 256 MB      | 160 MB  |
| Private working set     | 119 MB      | 79 MB   |
| JS engine, committed    | 64 MB       | 29 MB   |
| GPU driver and the rest | 134 MB      | 74 MB   |

Of the 96 MB, 35 MB is the engine's and 60 MB the driver's. On a GPU whose driver does not use this
allocator, expect the engine's share only, and almost all of it in commit, not in RAM.

The price is allocation in bursts. A loop that makes 800,000 short-lived objects and 240 MB of
buffers took about 1.3 times as long in this mode. Steady work that allocates little is not
affected.

Choose `low` for an app that sits in the tray or stays open all day. Leave the default for one that
builds large data in bursts.

### Try it without rebuilding

The `SK_MEMORY` environment variable, set to `low` or `default`, decides over the config. Start the
app both ways and compare the breakdown after a forced GC:

```sh
SK_MEMORY=low ./my-app
```

## AOT builds

A process running an [ahead-of-time image](/guides/aot/) commits more than the same app under the
JIT, in pages it mostly never touches:

- With the `compact` memory profile, the default, the difference is about 35 to 50 MB of commit on
  Windows. Most of it is one block of about 58 MB per JavaScript VM, a worker's included, for the
  engine's property-cache tables. No option changes its size.
- With `SK_AOT_MEMORY_PROFILE=default` the engine keeps its own sizes, which commit about 325 MB
  more on Windows. Use it only when an app runs out of the compact capacities.

Neither raises the working set by much. The details are in
[Commit charge on Windows](/guides/aot/#commit-charge-on-windows).

## What the app itself holds

The settings above trim what is held in reserve. What is live is the app's own:

- **JS objects.** A list of 100,000 rows kept in reactive state costs memory whether it is on screen
  or not. [Virtualize long lists](/guides/best-practices/#virtualize-long-lists) so that only the
  visible rows have elements.
- **Images.** A decoded image is width × height × 4 bytes whatever the file weighs. Ship images at
  the size they are shown.
- **Native memory that grows.** The **Allocations** view of the
  [Performance tab](/devtools/performance/#allocations-what-is-holding-native-memory) groups the
  host's native allocations by site. Set a baseline, repeat an interaction and take a snapshot: a
  site that grows every time is the leak.
