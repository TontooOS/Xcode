# StartPage

The start page renders the Xcode welcome window: a fixed 420x585 card
(~25% smaller than the Apple reference) with no `Titlebar` and no
traffic lights. The only chrome at the top is a single round
`BasicToolbar` pill with one `xmark` icon that closes the window. Below
it, the app icon from `Resources/icon.tico` shows centered at 90px,
followed by the bold `Xcode` title, a `Version 27.0.0` line, two example
capsule buttons and a project list (`No Projects` while empty).
`New Project` opens a modal options sheet on top (see
`## NewProject Sheet`); the start page stays visible behind the dimmed
backdrop.

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
| `recents` | `RoundedRectangle` + rows | 330x255 container (`BUTTON_BG_DARK` / `BUTTON_BG_LIGHT`); saved CoreData projects as 48px rows (folder symbol, `Caption` name + `Caption2` path, first row accent-selected, up to 4 shown, display only), or a centered dim `project.empty` (`No Projects`) line while empty |
| `sheet` | `BasicSheet<NewProjectForm>` | Modal options card (`SheetSize::Small`, 380x266) over the dimmed start page; opened by `New Project`, closed by `Cancel` or ESC (see `## NewProject Sheet`) |

### Rules

- There is no `Titlebar`: traffic lights are never constructed, so no
  red/yellow/green lights can appear.
- The drag region is the whole window (`(0, 0, 420, 585)`), so a press
  anywhere on the background drags the app. A press inside the drag
  region starts a system drag instead of a click, therefore no control
  ever sees `mouse_down`; `mouse_up` synthesizes down+up at the release
  position instead, so releasing over the close pill or an action button
  fires it while releasing anywhere else (a real drag) does nothing.
- The recents box is display-only: saved projects from CoreData (see
  [Projects.md](Projects.md)), first row accent-selected, no click
  handling and no open action yet. While empty it shows the centered
  dim `project.empty` line.
- The drag region is background-only: the renderer polls it on every
  press, and the hover-tracked cursor decides. Over the close pill or
  an action button (placed rects from the last frame) it returns `None`
  so the press reaches the app as a real down/up click; anywhere else
  it returns the full window and the press starts a system drag.
- An open sheet disables the drag region (`None`): every press reaches
  the form fields, and dragging pauses until `Cancel` or ESC.
- Action buttons use `set_palette` (`BUTTON_BG_DARK`/`WHITE` in dark
  mode, `BUTTON_BG_LIGHT`/`BLACK` in light mode) plus
  `set_theme(palette.accent, dark)`.
- `wants_backdrop` returns `true`: the round close control is a `Lens`
  glass pill and refracts the backdrop.
- Returns `WindowCommand::Close` from `poll_window_command` when the
  close pill fires; action buttons never close.

## NewProject Sheet

`sheet::NewProjectForm` is the card content inside the `BasicSheet`
with two steps. Step 1 is the fixed 380x292 options form: title, EN/DE
locale switch, name/version/bundle ID fields, live bundle identifier
preview, error line and the button row. Step 2 is a dynamically sized
folder chooser (see [Projects.md](Projects.md)).

```rust
pub fn update(&mut self, accent: Color, dark: bool, mode: ThemeMode, focused: bool)
```

### Structure

| Row | Element | Description |
|---|---|---|
| `title` | `BasicText` | `sheet.title` in `TextStyle::Body` |
| `locale` | `SegmentedPicker` | `EN` / `DE` switch next to the `sheet.app_name` label; selects whether the English or the German name field shows |
| `name` | `BasicTextField` | English or German app name (placeholder `sheet.ph_name`); only the visible one takes typing |
| `version` | `BasicTextField` | `sheet.app_version` label plus version field (placeholder `sheet.ph_version`) |
| `org` | `BasicTextField` | `sheet.bundle_id` label plus organization field, prefilled with `dev.<username>` (`scaffold::default_bundle_id`) |
| `preview` | `BasicText` | Live `{org}.{english_name}` line (`Caption`, `Secondary`); shows `AppName` while the English name is empty and the `dev.<username>` default while the org is empty; English only |
| `error` | `BasicText` | Red `Caption` hint (`sheet.err_*`) after failed validation; empty otherwise |
| `buttons` | `HStack` | `Cancel` left, `Create` right (capsule `Button` elements) |
| `chooser` | step 2 | `location.title`, current path line, `..` plus up to 8 subdirectory buttons (`Plain` + `folder.fill`), `location.new_folder` target preview, `Cancel` / `Create` row |

### Rules

- The start page stays mounted behind the dimmed backdrop; the sheet
  fades in via `show()` and out via `dismiss()`.
- `New Project` raises a flag consumed in `draw`, which calls
  `sheet.show()`. `Cancel` raises a flag consumed the same way, which
  calls `sheet.dismiss()`. ESC also dismisses (handled by
  `BasicSheet::key`). Step 1 `Create` validates and enters the folder
  chooser; step 2 `Create` scaffolds the project, saves the CoreData
  record and closes (see [Projects.md](Projects.md)).
- `App::text` and `App::key` route into the form only while the sheet
  is visible; the I-beam cursor (`CursorKind::Text`) follows field
  selection through a polled flag.
- Switching the locale deselects the hidden name field, so typing never
  lands in the invisible field.
- No serde usage: all strings come from `lang::t`.

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

Prints `open pressed (example)` to stdout when `Open...` is pressed
(`New Project` opens the options sheet instead). Closing works through
the `xmark` pill or the sheet `Cancel` button only. See
[Projects.md](Projects.md) for the full create flow.

## Cross References

- [Projects.md](Projects.md) – CoreData store, scaffolding, folder chooser flow

## Cross References

- [MAIN.md](MAIN.md) – project overview and quick start
- [RULE.md](RULE.md) – wiki design system and repo rules
