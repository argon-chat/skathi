---
title: sk.config.json
description: Every key of an app's sk.config.json, with its type, default, allowed values, platform notes and an example.
section: Reference
order: 4
---

# sk.config.json

`sk.config.json` sits in the app's root, next to `package.json`. `sk dev`, `sk build`, `sk publish` and
`sk check` read it; the host reads the `window` block and the launch keys before the main window opens.
Every key is optional.

```json
{
  "$schema": "./node_modules/@skathi-native/runtime/sk.config.schema.json",
  "name": "notes",
  "productName": "Notes",
  "version": "1.0.0",
  "window": { "title": "Notes", "width": 960, "height": 640 },
  "windows": { "icon": "assets/icon.png" }
}
```

An unknown top-level key is a warning, since other tools may share the file. An unknown key under
`windows`, `macos`, `check` or a pack is an error, so a typo cannot ship silently. Inside `window`, an unknown
key is ignored. Keys that start with `$` are allowed everywhere.

This page is generated from the JSON schema in `@skathi-native/runtime` (`sk.config.schema.json`), the same
file editors use to complete and check the config.
