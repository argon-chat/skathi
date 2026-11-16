---
title: CSS coverage
description: Supported CSS properties, layout, styling, and platform limitations in the native renderer.
section: Reference
order: 1
---

# CSS coverage

What a Skathi app can use from CSS, what is coming, and what is not. Stylesheets are compiled by
lightningcss, laid out by taffy and painted natively; there is no browser engine underneath. Most
unsupported declarations print an advisory while the app runs (`sk dev`, devtools); the few that are
accepted and ignored without one say so on their page.

Every property, selector, at-rule, function and unit has its own page: the support on Windows, macOS and
Linux, the release it arrived in, how it differs from a browser, and a link to MDN for the standard
behavior.
