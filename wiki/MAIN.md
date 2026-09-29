# Xcode – Wiki

Xcode is the TontooOS start page for building apps: a 420x585 TontooUI
card on Vello/WGPU (~25% smaller than the Apple reference) with no
traffic lights (one round toolbar button with an `xmark` glyph closes
the window), the centered 90px app icon from `Resources/icon.tico`
rendered through CoreIcon in its normal (light) variant, the `Xcode`
title with a `Version 27.0.0` line, two example capsule buttons and a
project list. `New Project` opens a modal options sheet on top
(EN/DE app name, version, bundle ID, live bundle identifier preview,
folder chooser, `Cancel` / `Create`): `Create` scaffolds a real app
skeleton on disk and stores the project in CoreData, so it survives
restarts. The list shows `No Projects` while empty. It follows the
live system color scheme through `ThemeWatcher` and loads
`en_us`/`de_de` strings from `lang/` via Accessibility.

- Repository: https://github.com/TontooOS/TontooOS
- License: TCL v27.0
- Version: 27.0.0

## Feature Index

| Feature | File | Description |
|---|---|---|
| Main index | [MAIN.md](MAIN.md) | This page |
| Rules | [RULE.md](RULE.md) | Development and usage rules |
| StartPage | [StartPage.md](StartPage.md) | Start window layout, CoreIcon icon pipeline, resources and localization |
| Projects | [Projects.md](Projects.md) | CoreData project store, scaffold generator, folder chooser flow |
| Editor | [Editor.md](Editor.md) | Example IDE window on the Sidebar element (dead except traffic lights), supervisor processes, double-click handover |

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
- 2026-09-29: Full-window background drag (press anywhere drags the app, releases over controls still click).
- 2026-09-29: Removed the `Clone...` button; `New Project` is a plain label without chevron icon.
- 2026-09-29: Empty recents box by default plus modal new-project sheet (EN/DE name switch, version, bundle ID, live `{org}.{english_name}` preview, `Cancel` closes, `Create` is an example).
- 2026-09-29: Real project creation (folder chooser from `~/Documents`, scaffold with `Cargo.toml`/`tontoo.proj`/`.gitignore`/`src`/`Resources`/`lang`, CoreData `Project` records, `No Projects` placeholder, first row selected).
- 2026-09-29: Scaffold generates lowercase `src/content_view.rs` (Linux module resolution) with trimmed template imports, `Resources/lang/` runtime mirror next to build-input `lang/`, Xcode version line shows `27.0.0`.
- 2026-09-29: Background-only drag (hover-tracked hit test over close pill and buttons, real down/up clicks, no more release synthesis).
- 2026-09-29: Default bundle ID is `dev.<username>` instead of `de.arlomu`.
- 2026-09-29: Drag hit test uses the real viewport origin (24px frame margin) instead of `(0, 0)` constants, fixing clicks on the close pill.
- 2026-09-29: Example editor view (in-process swap via row double-click or after Create, dead except traffic lights, no CLI, no TBuild).
- 2026-09-29: Persistent supervisor process (start/editor children, env handoff, `OPEN:` protocol, exit 42); editor is a big Sidebar window with dead pills.
- 2026-09-29: Editor feels alive but acts dead (native hover/press/selection/collapse/wheel routing, no callbacks, identical pages).
- 2026-09-29: Sidebar never collapses (no collapse button, Run pill only) plus a functionless 28px search capsule stretched across its bottom.
- 2026-09-29: Dead Run/Stop pill pair at the editor content top right (hover/press tint, no callbacks).
- 2026-09-29: `computer.png` device glyph left of the device menu (centered group).
- 2026-09-29: Device menu rows carry PNG icons (`MenuItem::icon`, new TontooUI API via CoreImage; `wrench.png` still missing, falls back to plain label).
- 2026-09-29: Single Run/Stop pair at the sidebar top right edge (left pill removed).
- 2026-09-29: Sidebar labels follow the theme via `set_item_text(None)` (element defaults to hand-set white, unreadable in light mode).
- 2026-09-29: Content topbar (centered device `NestedMenu` with Devices/Build/Utilities sections, dead chevron pair, dead far-right collapse pill; all example).
- 2026-09-29: Sheet `Cancel`/`Create` buttons use `SHEET_BUTTON_BG_DARK` in dark mode (`BUTTON_BG_DARK` melts into the sheet card, only the label showed).
