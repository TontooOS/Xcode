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
| `topbar` | `FileImage` + `BasicToolbar` + `NestedMenu` + pills | 22px `Resources/computer.png` glyph left of the `My Computer` text-menu at 15px (`button_font`, `menu.*` keys, sections Devices/Build/Utilities with `computer.png`/`wrench.png`/`up.png` row icons via the new `MenuItem::icon` API); the picked row reflects on top (button label plus glyph, display only) on a 36px glass pill: the pill centers on the content middle right of the sidebar and wraps a compact group (glyph plus gap plus button text plus gap plus chevron) with equal padding, so icon, text and chevron sit centered inside; the button width derives from the button text only (not the widest dropdown row) and the menu stays vertically centered in the pill; dead chevron pair (`chevron.left`, divider, `chevron.right`) at the content left; notes placeholder pill (`chart.line.uptrend.xyaxis`, round 36px circle) at the content right (hover and press tint only, toggles the bottom panel open or closed); no content title, no collapse button; `HorizontalDivider` between pills and editor; all example only |
| `navigator` | `SidebarItem` + `BarSwitcher` + warnings overlay | Project root plus `Assets`, `ContentView`, `Info` and file rows, preselected on the file row; `collapsible(false)` (never collapses, no collapse button), search hidden (`search_field(false)`), toggle pill hidden; a `BarSwitcher` tab switcher with icon plus label cells (`folder` `Files` default, warning triangle `Warnings & Errors`) sits in the gap above the rows; the warnings tab covers the rows with an example list (3 yellow warnings plus 2 red errors with tinted icons, file and line subtitles) while the content keeps showing the selected file; clicking a warning jumps back to `Files`, selects its file and moves the caret to its line; mouse reaches the element natively (hover, selection, wheel, resize cursor); `set_item_text(None)` clears the element's white default so labels follow the theme (light `#272727`) |
| `search` | `SearchField` | Functionless capsule (`ed.search`) pinned to the sidebar bottom: 28px tall, full column width even while resizing; takes focus and typing, never searches |
| `editor` | `CodeEditor` pages | Editable 40-line Rust hello example through the DocumentKit Rust tokenizer (`highlight` → `Span`s: bold keywords, underlined strings, dimmed italic comments, gaps plain; per line, example code uses no multi-line constructs); gray `Footnote` gutter numbers share the code text size so they stay glued to their lines; lines carrying a warning or error always show a tinted gutter badge, a tinted line number and a matching line wash like the reference (red errors, yellow warnings); spans rebuild on every edit and on theme text change; spans rebuild on every edit and on theme text change; click inside focuses with a caret, drag selects, double-click selects the word, typing replaces the highlight, `Backspace` deletes, `Enter` splits, arrows move (`Shift` extends),   `ESC` unfocuses, `Ctrl+A/C/X/V/Z/Y` selects all, copies, cuts, pastes, undoes and redoes through an in-memory clipboard; caret, click mapping and the selection wash measure through the same rich monospace DocumentKit layout the code rows render with (bold runs included), so the   highlight hugs the letters exactly; caret, click mapping,
  selection, gutter badges and line washes all use the row pitch
  measured from the laid-out rows (never a fixed px guess), so the
  highlight cannot drift off the lines further down; the wheel scrolls vertically with caret tracking so the caret stays visible; edits stay in memory only. No breadcrumb, no status bar (removed) |
| `bottom` | divider + `CodeEditor` panel | Bottom area below the code with a drag divider: click collapses or expands it like a sidebar, dragging resizes it between 100px and 420px (180px default); the divider turns 2px accent while hovering or dragging like the sidebar edge and shows the resize cursor; left `Performance` stats as a divider-free 2x2 grid (CPU top left, GPU top right, RAM bottom left in MB, bottom right split into Disk in MB/s and Network in Mb/s) with one colorful sparkline plus live value per metric sampling every 1s (fake in-memory data, custom Vello paths, user-approved); right `Logs` with a fake cargo build loop growing every 1s (capped at 300 lines) on the standard overlay `Scrollbar` with wheel, thumb drag, track jump and bottom stick; all example only, nothing runs |
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
  DocumentKit spans, a caret rect, a selection wash and the standard
  overlay `Scrollbar`, following normal text field behavior (click to
  focus, drag or `Shift` plus arrows to select, double-click for word
  select, I-beam hover, typing, `Backspace`, `Enter`, arrows, `ESC`
  to unfocus, `Ctrl+A/C/X/V/Z/Y` for select, clipboard, undo and redo
  with an in-memory clipboard capped at 100 undo steps).
- Below the code sits a bottom panel with a drag divider: click
  collapses or expands it like a sidebar, dragging resizes it
  (100px..420px, 180px default); the divider turns 2px accent while
  hovering or dragging like the sidebar edge and the cursor turns
  into the resize cursor there. The left `Performance` stats form a
  divider-free 2x2 grid (CPU top left, GPU top right, RAM bottom
  left in MB, bottom right split into Disk in MB/s and Network in
  Mb/s) with one colorful
  sparkline plus live value per metric (Vello paths, user-approved
  custom drawing; graph colors are fixed per metric so the lines
  stay distinguishable) sampling fake in-memory values every 1s; the
  right `Logs`
  shows a fake cargo build loop growing every 1s (capped at 300
  lines) on its own standard `Scrollbar` sticking to the bottom.
  `EditorUi` ticks the active code page with the frame time and
  the performance pill toggles the bottom panel on every page; all
  example only, nothing runs.
- `EditorUi` owns its `ThemeWatcher`; text and keys reach the active
  code page through `sidebar.page_text` and `sidebar.page_key` when
  the search field is not selected; the cursor shows I-beam over
  search or a hovered code page.
- The navigator tab switch shares one `Sidebar`: the picker flips a
  `nav_tab` cell, the warnings overlay paints the sidebar background
  over the file rows and draws example rows with tinted icons; a
  warning click jumps (`Files` tab, file select, `goto_line`) at
  once so the content never flashes the wrong file.
- Every page shows its own file diagnostics inline:
  `EditorUi` hands each `CodeEditor` its warning and error lines
  per file every frame; marked lines keep a tinted gutter badge, a
  tinted number and a line wash in both themes.
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
