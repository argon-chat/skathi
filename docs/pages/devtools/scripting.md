---
title: Scripting the app from outside
label: Scripting and agents
description: Drive a running app from the command line with sk ctl, the protocol the inspector itself uses. Read the tree, click, type, evaluate, record frames and take screenshots from scripts, tests and coding agents.
section: Devtools
order: 6
---

Everything the inspector shows, it gets over a small local protocol, and the CLI speaks it too. `sk ctl` sends one command to a running app and prints the reply as JSON. That makes the inspector's whole view of your app available to a shell script, a CI job, or a coding agent that cannot see the screen.

## One command, one answer

Start the app with `sk dev`, then, from the same folder:

```bash
bunx sk ctl text "button.primary"
```

```json
{
  "id": 1043,
  "text": "Count up · 3",
  "source": "textContent"
}
```

Run from the app's folder, it finds the app `sk dev` is running there by itself (`dist/.sk-dev`). Elsewhere, pass `--dist <app>/dist/.sk-dev`. For an app built with `sk build`, pass `--app <name>` with the `name` from `sk.config.json`; it attaches to the newest running copy. `--window <id>` addresses a pop-out window: the `windows` command lists them, and `<SkWindow>`'s `open` event gives the id.

The exit code is 0 when the app answered `ok`, 1 otherwise, so a script can stop at the first failure. A reply that does not come in 10 seconds is a failure too (`--timeout <ms>` changes it), so a frozen app cannot hang the script.

## A loop an agent can run

The input commands go through the app's real hit test, so a click lands on what is on top at that point, `pointer-events` and all, and the app's own handlers run. Act, then check:

```bash
bunx sk ctl click "button.primary"
bunx sk ctl text "button.primary"          # "Count up · 4"
bunx sk ctl logs                           # anything new in the console?
bunx sk ctl screenshot shot.png            # what does it look like now?
```

`screenshot` captures the window's client area without raising or focusing it; pass `--backend print_window` for the whole window with its backdrop composited in. `logs` returns a cursor, `next`; pass it back as `--since` to get only the lines after it.

When a check fails, the same script finds out why, the way you would in the inspector:

```bash
bunx sk ctl rules "button.primary"         # the matched rules, as in the Elements tab
bunx sk ctl styles "button.primary"        # the computed values
bunx sk ctl eval "localStorage.getItem('note')"
bunx sk ctl net                            # the requests, as in the Network tab
```

## What it can do

| Area        | Commands                                                                                         |
| ----------- | ------------------------------------------------------------------------------------------------ |
| Elements    | `tree`, `query`, `text`, `styles`, `rules`, `highlight`, `pick`                                  |
| Input       | `click`, `focus`, `type`, `ime`, `key`, `mouse`, `pen`, `touch`, `scroll`, `wheel`               |
| JavaScript  | `eval`, `logs`                                                                                   |
| Network     | `net`, `net_detail`, `net_cache`                                                                 |
| Storage     | `storage`, `indexeddb`                                                                           |
| Performance | `perf`, `trace` (record frames as in the Frames view), `mem`, `allocations`                      |
| Windows     | `windows`, `screenshot`, `raise`                                                                 |
| Effects     | `effects` (list, read, set and reset an element's effect)                                        |
| Other       | `ping`, `dialog` (answer file dialogs and prompts in tests), `permissions` (read and reset them) |

Run the script with `--help` for every command and its options.

## Running beside someone at work

An agent that runs the app on a developer's machine should not pull windows in front of them. `SK_WINDOW_FOCUS=never` keeps the app's window from taking the foreground, and `SK_DEVTOOLS_RAISE=off` keeps a second F12 or `appDevtools.open()` from bringing the inspector forward. The [testing guide](/guides/testing/#running-without-taking-over-your-desktop) lists the rest.

## As an MCP server

In a clone of the Skathi repository, `packages/cli/devtools/mcp.ts` serves the same commands over the Model Context Protocol, one tool per command named `sk_<command>`, for agents that prefer tools to a shell:

```bash
bun packages/cli/devtools/mcp.ts --dist <app>/dist/.sk-dev
bun packages/cli/devtools/mcp.ts --app <name>
```

It reads the app's handshake again on every call, so restarting the app does not need a restart of the server. It is not part of the npm package; `sk ctl` is.

## Tests

The [end-to-end testing](/guides/testing/) package drives the app over the same protocol, with the same real input, from `bun test`.
