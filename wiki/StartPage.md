# StartPage

The start page renders the Xcode welcome window: a fixed 420x585 card
(~25% smaller than the Apple reference) with no `Titlebar` and no
traffic lights. The only chrome at the top is a single round
`BasicToolbar` pill with one `xmark` icon that closes the window. Below
it, the app icon from `Resources/icon.tico` shows centered at 90px,
followed by the bold `Xcode` title, a `Version 27.0` line, two example
capsule buttons and a static scaled-down recents box mirroring the
reference screenshot.

## Layout

```rust
const WINDOW_WIDTH: u32 = 420;
const WINDOW_HEIGHT: u32 = 585;
```

### Structure

| Row | Element | Description |
|---|---|---|
| `close` | `BasicToolbar` | Round single-`xmark` pill at (12, 12), 36x36; `on_action` requests `WindowCommand::Close` |
| `icon` | `FileImage` | 90x90, 21px radius, centered; shows the CoreIcon PNG (see `## Icon Pipeline`) |
| `title` | `BasicText` | `app.title` in `TextStyle::Headline` (17 semibold), centered |
| `version` | `BasicText` | `app.version` in `TextStyle::Caption` with `TextForeground::Secondary`, centered |
| `actions` | `HStack` | Two capsule `Button` elements: `action.open`, `action.new_project` (plain label, no icon); example `on_press` handlers only print |
| `recents` | `RoundedRectangle` + rows | 330x255 container (`BUTTON_BG_DARK` / `BUTTON_BG_LIGHT`) with four static 48px rows (30px symbols, `Caption` name + `Caption2` path) and three `HorizontalDivider` elements; the first row sits on an accent `RoundedRectangle` selection fill |

### Rules

- There is no `Titlebar`: traffic lights are never constructed, so no
  red/yellow/green lights can appear.
- The drag region is the whole window (`(0, 0, 420, 585)`), so a press
  anywhere on the background drags the app. A press inside the drag
  region starts a system drag instead of a click, therefore no control
  ever sees `mouse_down`; `mouse_up` synthesizes down+up at the release
  position instead, so releasing over the close pill or an action button
  fires it while releasing anywhere else (a real drag) does nothing.
- The recents box is display-only: rows never handle mouse events. Row
  text is scaled down with the box (`Caption` 12px names, `Caption2`
  11px paths, 30px symbols in 48px rows).
- The first row is selected: white name/path text and white symbol on
  the live `palette.accent` fill. Other rows follow the theme text.
- Action buttons use `set_palette` (`BUTTON_BG_DARK`/`WHITE` in dark
  mode, `BUTTON_BG_LIGHT`/`BLACK` in light mode) plus
  `set_theme(palette.accent, dark)`.
- `wants_backdrop` returns `true`: the round close control is a `Lens`
  glass pill and refracts the backdrop.
- Returns `WindowCommand::Close` from `poll_window_command` when the
  close pill fires; action buttons never close.

## Icon Pipeline

`icon::display_icon` resolves the display icon.

```rust
pub fn display_icon() -> PathBuf
```

### Rules

- Probes the bundled `Resources/icon.tico` (`APP_RESOURCES_DIR`
  override, `Resources/icon.tico` next to the working directory and the
  executable, then the `.app` bundle layout).
- `.tico` entries render through `Tico::load` plus
  `render(512, None)`: full-resolution composite with the Apple
  app-icon finish and no tint, i.e. the normal (light) variant as
  authored.
- Plain files run through `AppIcon::from_file(raw).light()` so they
  become a proper squircle with the Liquid Glass finish in the light
  variant.
- The finished PNG is written to `std::env::temp_dir()` as
  `xcode-icon-Xcode-normal.png` (or `-light.png`) and shown with
  `FileImage`.
- Returns the `__xcode_missing_icon__` sentinel when no bundle icon
  exists; `FileImage` then draws its theme placeholder box.
- Returns the raw path when rendering fails (callers fall back to the
  raw icon).

## Resources and Localization

| Path | Description |
|---|---|
| `Resources/icon.tico` | Bundled app icon, rendered in the normal variant |
| `lang/en_us.json` | English strings (Accessibility shape) |
| `lang/de_de.json` | German strings (Accessibility shape) |
| `Resources/lang/en_us.json` | Bundled mirror of the English strings |
| `Resources/lang/de_de.json` | Bundled mirror of the German strings |
| `tontoo.proj` | Bundle manifest (`bundle_id: com.tontoo.xcode`, `version: 27.0.0`) |

`lang::t` looks up a localized string and returns the key itself when
missing.

```rust
pub fn t(key: &str) -> String
```

### Rules

- Only `en_us` and `de_de` are supported; anything else falls back to
  `en_us`.
- Locale detection order: `LANGUAGE`, `LC_ALL`, `LANG` (prefix `de`
  selects German), then `/etc/locale.conf`, else `en_us`.
- Lookup order: `$XCODE_LANG_DIR`, `./lang`, `./Resources/lang`,
  executable-relative `lang` folders, the `.app` bundle
  `Resources/lang`, then `/usr/share/xcode/lang`.
- Placeholders (`{version}`) are replaced by the callers.
- No serde usage in this crate: parsing goes through Accessibility
  `LangFile` / `LangStore`.

## Usage / Example

```bash
cargo run
```

Prints `open pressed (example)` or `new project pressed (example)` to
stdout when an example button is pressed. Closing works through the
`xmark` pill only.

## Cross References

- [MAIN.md](MAIN.md) – project overview and quick start
- [RULE.md](RULE.md) – wiki design system and repo rules
