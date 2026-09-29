# Editor

The example project editor is a big 1100x700 window in its own
process, built on the `Sidebar` element: working traffic lights
(owned by the sidebar), one dead Run pill (no callback), a
non-collapsible file navigator preselected on the file row, a
functionless search capsule stretched across the sidebar bottom, dead
example Swift code plus a dimmed minimap, breadcrumb and status bar.
The element feels alive (hover, press states, selection, resize,
wheel, search focus and typing) but clicks trigger no actions: no
callbacks are registered, all pages are identical and nothing is ever
saved, built or run. No CLI anywhere, no TBuild calls.

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
| `topbar` | `FileImage` + `NestedMenu` + pills | 22px `Resources/computer.png` glyph left of the centered `My Computer` text-menu (`menu.*` keys, sections Devices/Build/Utilities, example selection), dead chevron pair (`chevron.left`, divider, `chevron.right`) right, dead round collapse pill (`sidebar.right`) far right; all example only |
| `navigator` | `SidebarItem` | Project root plus `Assets`, `ContentView`, `Info` and file rows, preselected on the file row; `collapsible(false)` (never collapses, no collapse button), search hidden (`search_field(false)`), toggle pill hidden; mouse reaches the element natively (hover, selection, wheel, resize cursor); `set_item_text(None)` clears the element's white default so labels follow the theme (light `#272727`) |
| `search` | `SearchField` | Functionless capsule (`ed.search`) pinned to the sidebar bottom: 28px tall, full column width even while resizing; takes focus and typing, never searches |
| `editor` | page `BasicText` | Static example Swift code (`Footnote`), file stem via `file_stem`, width follows the content size |
| `inspector` | page `BasicText` | Same code dimmed at `Caption2` as a faux minimap (112px) |
| `status` | page `HStack` | `Filter` (`ed.filter`), `Spacer`, `Line: 1  Col: 1` (`ed.status`) |

```rust
pub fn file_stem(display_name: &str) -> String
```

### Rules

- `file_stem` keeps ASCII alphanumerics and appends `App` unless the
  stem already ends in `app` (`test` -> `testApp`, `MyApp` ->
  `MyApp`); empty input becomes `MyApp`.
- The code header reads `Created by {user}` from the OS username.
- Each of the 5 items owns an identical editor page; per-frame wiring
  reaches the texts through `page_mut` downcasts (theme plus code
  width), so no page state is stored.
- `EditorUi` owns its `ThemeWatcher`; mouse only reaches `press()` and
  hover, never the pages.
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
