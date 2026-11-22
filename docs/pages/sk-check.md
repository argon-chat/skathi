---
title: sk check
label: sk check codes
description: "What sk check looks for and every code it reports: what the finding means, an example that triggers it and the fix. Errors that break an AOT build, code that runs slower in an image, typing, CSS the renderer does not apply, Vue and Svelte features, and permissions."
section: Reference
order: 7
since: "0.7"
---

# sk check

`sk check` reads the app the way a production build does and reports what will not work, or will
work differently, before the app runs. It looks at the files the bundle includes, the app's own and
those of its dependencies: the scripts, the stylesheets, the `<style>` blocks and `style`
attributes of components, the effect shaders, and the permissions in sk.config.json.

```sh
bunx sk check                  # the app in this folder
bunx sk check apps/notes       # another one
bunx sk check --deny-warnings  # for CI: warnings fail too
bunx sk check --no-deps        # only the app's own code
bunx sk check --format json    # one JSON report on stdout
```

Every finding has a code. `SK2005` on a line of the report is a page here:
[`/sk-check/sk2005/`](/sk-check/sk2005/). The same text is in the terminal:

```sh
bunx sk explain SK2005
bunx sk explain --list
```

## When it runs

- **By hand or in CI**, as above.
- **In `sk build` and `sk publish`.** The CSS codes are printed with the build, one line each.
- **In a build with AOT.** The whole check runs first, and the build stops on its errors: an
  `SK1xxx` finding is code that throws in an image.

## Exit codes

| Exit | Meaning                                                                         |
| ---- | ------------------------------------------------------------------------------- |
| `0`  | No errors. Warnings do not fail the check unless it runs with `--deny-warnings` |
| `1`  | Errors, or warnings with `--deny-warnings`                                      |
| `2`  | The check could not run as asked: a wrong flag, a wrong configuration, no build |

## Allowing a finding

A finding that is known and accepted is hidden with
[`check.allow`](/sk-config/#check.allow) in sk.config.json. It maps a code to package names and to
globs of the app's own files:

```json title="sk.config.json"
{
  "check": {
    "allow": {
      "SK2001": ["some-package"],
      "SK3003": ["src/legacy/**"]
    }
  }
}
```

The report counts what was hidden as `suppressed`. A code that does not exist stops the check with
exit code 2, so a typo cannot allow everything. Two kinds of code cannot be allowed: the ones that
stop the check, and the errors in the app's permissions. Each code's page says which it is.

## The JSON report

`--format json` prints one object, schema `skathi.check` version 1. Each entry of `diagnostics` has
the `code`, the `severity`, the `message`, the `file` relative to the app with its `range` (lines and
characters from zero, in UTF-16 units, as an editor's protocol counts them), the `package` the file
belongs to or `null` for the app's own code, a `help` line and `related` locations. `summary` has the
counts and, under `typing`, how many of the declarations the app reaches from each package are `any`.
