# Xcode – Wiki

Xcode is the TontooOS start page for building apps: a 420x585 TontooUI
card on Vello/WGPU (~25% smaller than the Apple reference) with no
traffic lights (one round toolbar button with an `xmark` glyph closes
the window), the centered 90px app icon from `Resources/icon.tico`
rendered through CoreIcon in its normal (light) variant, the `Xcode`
title with a `Version 27.0` line, three example capsule buttons and a
static scaled-down recents box. It follows the live system color scheme
through `ThemeWatcher` and loads `en_us`/`de_de` strings from `lang/`
via Accessibility.

- Repository: https://github.com/TontooOS/TontooOS
- License: TCL v26.1
- Version: 27.0.0

## Feature Index

| Feature | File | Description |
|---|---|---|
| Main index | [MAIN.md](MAIN.md) | This page |
| Rules | [RULE.md](RULE.md) | Development and usage rules |
| StartPage | [StartPage.md](StartPage.md) | Start window layout, CoreIcon icon pipeline, resources and localization |

## Quick Start

Run the start page from the repository root:

```bash
cargo run
```

The window follows the settings daemon theme live (Dark `#1B2022`, Light
`#FFFFFF`) and picks German strings when `LANG` starts with `de`.

See [StartPage.md](StartPage.md) for details.

## Changelog

- 2026-09-29: Initial Xcode start page (X close toolbar button, centered `icon.tico` in normal variant, `Open...` / `Clone...` / `New Project` example buttons, static recents box, `lang/en_us.json` and `lang/de_de.json`).
- 2026-09-29: Scaled the whole window ~25% down to 420x585 (90px icon, 330x255 recents box, smaller row text).
