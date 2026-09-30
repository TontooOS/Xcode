# Editor

The example project editor is a big 1100x700 window in its own
process, built on the `Sidebar` element: working traffic lights
(owned by the sidebar), a dead Run/Stop pill pair, a
non-collapsible file navigator preselected on the file row, a
functionless search capsule stretched across the sidebar bottom and
editable example Rust code.
The element feels alive (hover, press states, selection, resize,
wheel, search focus and typing); code pages accept clicks and typing
like a normal text field: no callbacks are registered, all pages
start identical and nothing is ever saved, built or run. No CLI
anywhere, no TBuild calls.

## Processes

One persistent main process supervises two window children:

| Process | Env | Window | Ends with |
|---|---|---|---|
| supervisor | none | none | editor exit code, or start exit code on quit |
| start child | `XCODE_MODE=start` | 420x585 start page | `0` on quit, `42` after printing `OPEN:<name>` |
| editor child | `XCODE_PROJECT_NAME=<name>` | 1100x700 editor | its own exit code |

```rust
pub const PROJECT_ENV: &str = "XCODE_PROJECT_NAME";
```

### Rules

- Double-clicking a project row or finishing `Create` prints
  `OPEN:<name>` (flushed explicitly, piped stdout is block-buffered),
  closes the start window at once and exits `42`.
- The supervisor reads the line, then runs the editor child, which
  waits ~600ms first: the old window visibly closes, short gap, then
  the big window opens fresh.
- Quitting the start window (exit `0`) quits the app; quitting the
  editor ends the supervisor with the editor code.
- `XCODE_MODE` only ever equals `start`; an empty project name falls
  back to the start page.

## Layout

The `Sidebar` fills the whole viewport and owns the decoration
(traffic lights live in it, like the reference). Viewport-adaptive
pages: narrow windows squeeze the code, wide windows use everything.

| Area | Element | Description |
|---|---|---|
| `bar` | `Sidebar` traffic | All three traffic lights working via `press()`; drags via `drag_rect()` |
| `pills` | `BasicToolbar` | Single dead Run/Stop pair (`play.fill`, divider, `stop.fill`) at the sidebar top right edge, centered on the traffic row: hover/press tint only, nothing fires; content title via `set_title` (`{project} › My Mac`, `ed.device`) |
| `topbar` | `FileImage` + `BasicToolbar` + `NestedMenu` + pills | 22px `Resources/computer.png` glyph left of the `My Computer` text-menu at 15px (`button_font`, `menu.*` keys, sections Devices/Build/Utilities with `computer.png`/`wrench.png`/`up.png` row icons via the new `MenuItem::icon` API); the picked row reflects on top (button label plus glyph, display only) on a 36px glass pill: the pill centers on the content middle right of the sidebar and wraps a compact group (glyph plus gap plus button text plus gap plus chevron) with equal padding, so icon, text and chevron sit centered inside; the button width derives from the button text only (not the widest dropdown row) and the menu stays vertically centered in the pill; dead chevron pair (`chevron.left`, divider, `chevron.right`) at the content left; no content title, no collapse button; `HorizontalDivider` between pills and editor; all example only |
| `navigator` | `SidebarItem` | Project root plus `Assets`, `ContentView`, `Info` and file rows, preselected on the file row; `collapsible(false)` (never collapses, no collapse button), search hidden (`search_field(false)`), toggle pill hidden; mouse reaches the element natively (hover, selection, wheel, resize cursor); `set_item_text(None)` clears the element's white default so labels follow the theme (light `#272727`) |
| `search` | `SearchField` | Functionless capsule (`ed.search`) pinned to the sidebar bottom: 28px tall, full column width even while resizing; takes focus and typing, never searches |
| `editor` | `CodeEditor` pages | Editable 40-line Rust hello example through the DocumentKit Rust tokenizer (`highlight` → `Span`s: bold keywords, underlined strings, dimmed italic comments, gaps plain; per line, example code uses no multi-line constructs); gray `Footnote` gutter numbers share the code text size so they stay glued to their lines; spans rebuild on every edit and on theme text change; click inside focuses with a caret, typing inserts, `Backspace` deletes, `Enter` splits, arrows move, `ESC` unfocuses; edits stay in memory only. No breadcrumb, no status bar (removed) |
| `status` | page `HStack` | `Filter` (`ed.filter`), `Spacer`, `Line: 1  Col: 1` (`ed.status`) |

```rust
pub fn file_stem(display_name: &str) -> String
```

### Rules

- `file_stem` keeps ASCII alphanumerics and appends `App` unless the
  stem already ends in `app` (`test` -> `testApp`, `MyApp` ->
  `MyApp`); empty input becomes `MyApp`.
- The code header reads `Created by {user}` from the OS username.
- `example_code` returns exactly 40 lines of Rust hello code naming
  `{file}`, `{project}` and `{user}`.
- Each of the 5 items owns an identical `CodeEditor` page starting
  from the same 40-line text; edits stay per page in memory only and
  are never saved, built or run.
- `CodeEditor` is a custom Xcode element (user-approved: TontooUI has
  no code editor with live highlight, only plain `TextEditor`): it
  reuses TontooUI `BasicText` plus `FormattedText` rows with
  DocumentKit spans and a caret rect, following normal text field
  behavior (click to focus, I-beam hover, typing, `Backspace`,
  `Enter`, arrows, `ESC` to unfocus).
- `EditorUi` owns its `ThemeWatcher`; text and keys reach the active
  code page through `sidebar.page_text` and `sidebar.page_key` when
  the search field is not selected; the cursor shows I-beam over
  search or a hovered code page.
- Row interaction on the start page: single tap selects
  (`selected`), double tap (same row within 450ms) hands over to the
  supervisor. Project rows are excluded from the background drag
  region so taps reach the app.
- After `Create`, the new project hands over immediately (plus joining
  the list).

## Usage / Example

Double-click any project row, or create a project and press the second
`Create`: the start window closes at once and the big editor opens
after a short gap. Red closes it (then the whole app quits).

## Cross References

- [StartPage.md](StartPage.md) – start window, row selection, sheet embedding
- [Projects.md](Projects.md) – CoreData store, scaffolding, folder chooser flow
- [MAIN.md](MAIN.md) – project overview and quick start
- [RULE.md](RULE.md) – wiki design system and repo rules
