# Editor

The project editor is a big 1100x700 window in its own process,
built on the `Sidebar` element: working traffic lights (owned by the
sidebar), a dead Run/Stop pill pair (nothing is ever built or run),
a non-collapsible file navigator preselected on the file row, a
functionless search capsule stretched across the sidebar bottom and
real editable file content.
The element feels alive (hover, press states, selection, resize,
wheel, search focus and typing); code pages accept clicks and typing
like a normal text field and auto-save their backing file 400ms
after typing stops: no callbacks are registered, folders and binary
files open read-only. No CLI anywhere, no TBuild calls.

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
| `navigator` | `Sidebar` + `BarSwitcher` + outline tree + warnings overlay | Project files below the tabs (see `open`) as a nested `BasicOutlineGroup` tree: folders nest recursively with trailing `>` chevrons, type icons plus tints and slide plus fade animation, open state remembered per folder; clicks select the flat page (folders toggle too), the flat `Sidebar` rows stay underneath for the pages; a `BarSwitcher` tab switcher with icon plus label cells (`folder` `Files` default, warning triangle `Warnings & Errors`) sits in the gap above the tree; the warnings tab covers the tree with an example list (3 yellow warnings plus 2 red errors with tinted icons, file and line subtitles) while the content keeps showing the selected file; clicking a warning expands its ancestors, jumps back to `Files`, selects its file and moves the caret to its line; `collapsible(false)` (never collapses, no collapse button), search hidden (`search_field(false)`), toggle pill hidden; mouse reaches the element natively (hover, selection, wheel, resize cursor); `set_item_text(None)` clears the element's white default so labels follow the theme (light `#272727`) |
| `search` | `SearchField` | Live capsule (`ed.search`) pinned to the sidebar bottom: 28px tall, full column width even while resizing; typing filters live: on the Files tab every content hit across files covers the tree (plain `doc.fill` icon plus `file:line` title plus the matched code snippet, capped at 200 rows, read-only pages skipped, `search.no_results` placeholder while empty), on the Warnings tab only matching issues stay; click or Enter jumps to the hit (file select plus caret to line and column), ESC clears the query; search typing never arms the background check |
| `editor` | `CodeEditor` pages | Editable 40-line Rust hello example through the DocumentKit Rust tokenizer (`highlight` → `Span`s: bold keywords, underlined strings, dimmed italic comments, gaps plain; per line, example code uses no multi-line constructs); gray `Footnote` gutter numbers share the code text size so they stay glued to their lines; lines carrying a warning or error always show a tinted gutter badge, a tinted line number and a matching line wash like the reference (red errors, yellow warnings); spans rebuild on every edit and on theme text change; spans rebuild on every edit and on theme text change; click inside focuses with a caret, drag selects, double-click selects the word, typing replaces the highlight, `Backspace` deletes, `Enter` splits, arrows move (`Shift` extends),   `ESC` unfocuses, `Ctrl+A/C/X/V/Z/Y` selects all, copies, cuts, pastes, undoes and redoes through an in-memory clipboard; caret, click mapping and the selection wash measure through the same rich monospace DocumentKit layout the code rows render with (bold runs included), so the   highlight hugs the letters exactly; caret, click mapping,
  selection, gutter badges and line washes all use the row pitch
  measured from the laid-out rows (never a fixed px guess), so the
  highlight cannot drift off the lines further down; the wheel scrolls vertically with caret tracking so the caret stays visible; text edits auto-save the bound file 400ms after typing stops, folders and binary files stay read-only. No breadcrumb, no status bar (removed) |
| `bottom` | divider + `CodeEditor` panel | Bottom area below the code with a drag divider: folded away by default, click expands or collapses it like a sidebar, dragging resizes it between 100px and 420px (180px default); the divider turns 2px accent while hovering or dragging like the sidebar edge and shows the resize cursor; left `Performance` stats as a divider-free 2x2 grid (CPU top left, GPU top right, RAM bottom left in MB, bottom right split into Disk in MB/s and Network in Mb/s) keeping labels and values with the localized `panel.no_metrics` (`No Performance Metrics`) text where graphs would sit, no graphs; right `Logs` with the localized `panel.no_logs` (`No Logs`) placeholder; nothing is generated, all example only, nothing runs |
| `status` | page `HStack` | `Filter` (`ed.filter`), `Spacer`, `Line: 1  Col: 1` (`ed.status`) |

```rust
pub fn file_stem(display_name: &str) -> String
```

`file_icon` maps file names to SF Symbols plus tints: `.rs`
`rust` orange, `.proj` `rotate.3d` purple, `.toml` `gear` gray,
`.json` `tray.2.fill` yellow, everything else the normal `doc.fill`
following the accent; folders keep `folder.fill`.

### Project files

`EditorUi::open` lists the opened project root for the navigator:
recursive walk with indented display names (two spaces per depth),
folders first then files, alphabetical; `target` folders, dotfiles
and dotfolders (including `.git`) plus symlinks are skipped, at most
500 rows. Every row owns one bound `CodeEditor` page with its real
file text (folders empty read-only, binary or oversized files with
the `ed.binary` placeholder read-only); the first `.rs` file row is
preselected and carries the example warnings. Without a valid
project path the
editor falls back to the built-in example rows. The supervisor hands
the path over as `OPEN:<name>\t<path>` (env `XCODE_PROJECT_PATH`
next to `XCODE_PROJECT_NAME`).

### Rules

- `file_stem` keeps ASCII alphanumerics and appends `App` unless the
  stem already ends in `app` (`test` -> `testApp`, `MyApp` ->
  `MyApp`); empty input becomes `MyApp`.
- The code header reads `Created by {user}` from the OS username.
- `example_code` returns exactly 40 lines of Rust hello code naming
  `{file}`, `{project}` and `{user}` (fallback rows without a real
  project path only).
- Each navigator row owns one bound `CodeEditor` page:
  `list_project_files` carries the real path per row,
  `load_file_text` loads text up to 1MB (UTF-8, no NUL byte),
  `page_for_entry` binds editable pages to their file, folders open
  as empty read-only pages and binary or oversized files show the
  localized `ed.binary` placeholder read-only. Text edits mark the
  page dirty and `poll_autosave` writes the file 400ms after typing
  stops (`AUTOSAVE_DELAY`); `save_all_now` flushes every page on
  window close. Nothing is built or run.
- `CodeEditor` is a custom Xcode element (user-approved: TontooUI has
  no code editor with live highlight, only plain `TextEditor`): it
  reuses TontooUI `BasicText` plus `FormattedText` rows with
  DocumentKit spans, a caret rect, a selection wash and the standard
  overlay `Scrollbar`, following normal text field behavior (click to
  focus, drag or `Shift` plus arrows to select, double-click for word
  select, I-beam hover, typing, `Backspace`, `Enter`, arrows, `ESC`
  to unfocus, `Ctrl+A/C/X/V/Z/Y` for select, clipboard, undo and redo
  with an in-memory clipboard capped at 100 undo steps).
  `goto_line_col` jumps to a 1-based line plus byte column for search
  hits (both clamped).
- Below the code sits a bottom panel with a drag divider: folded
  away by default, click expands or collapses it like a sidebar,
  dragging resizes it (100px..420px, 180px default); the divider
  turns 2px accent while hovering or dragging like the sidebar edge
  and the cursor turns into the resize cursor there. The left
  `Performance` stats form a divider-free 2x2 grid (CPU top left,
  GPU top right, RAM bottom left in MB, bottom right split into Disk
  in MB/s and Network in Mb/s) keeping labels and values with the
  localized `panel.no_metrics` text instead of graphs; the right
  `Logs` shows the localized `panel.no_logs` placeholder. Nothing is
  generated. `EditorUi` ticks the active code page with the frame
  time (no-op) and the performance pill toggles the bottom panel on
  every page; all example only, nothing runs.
- `EditorUi` owns its `ThemeWatcher`; text and keys reach the active
  code page through `sidebar.page_text` and `sidebar.page_key` when
  the search field is not selected; the cursor shows I-beam over
  search or a hovered code page.
- The navigator tab switch shares one `Sidebar`: the picker flips a
  `nav_tab` cell; the Files tab paints the sidebar background over
  the flat rows and draws the nested outline tree (`build_tree`
  maps flat rows to outline paths, `poll_tree_select` lands outline
  clicks on the flat page the same frame), or the search results
  overlay while the capsule holds a query (`recompute_search`
  indexes every case-insensitive occurrence over editable pages,
  `jump_to_hit` selects the file and moves the caret to line and
  column); tree presses skip the flat rows except on the resize edge.
  The warnings tab draws issue rows (filtered by the query when set)
  instead; a warning click expands its ancestors and jumps (`Files`
  tab, file select, `goto_line`) at once so the content never
  flashes the wrong file.
- Every page shows its own file diagnostics inline:
  `EditorUi` hands each `CodeEditor` its warning and error lines
  per file every frame; marked lines keep a tinted gutter badge, a
  tinted number and a line wash in both themes.
- Background check (`src/check.rs`, no new dependencies): 2.5s after
  the last content-changing keystroke (`CHECK_IDLE_DELAY`) and once
  after opening a project, the editor flushes all files and spawns
  exactly one `cargo check --message-format=json` worker thread in
  the project root. While it runs, a glass pill with the localized
  `check.indexing` (`Indexing Files...`) label sits left of the
  performance pill. A keystroke after the job start cancels it (the
  child is killed) and a fresh job starts once typing stops again;
  stale generations are dropped. Output lines collect into two
  buckets (`collect_diagnostics`): crate-relative paths first
  (own code, up to 2000), absolute dependency paths after (up to
  200), since dependencies check first and would otherwise flood
  the cap before the crate's own errors arrive. The cargo binary resolves through
  `cargo_program` (`CARGO` env, then `~/.cargo/bin/cargo`,
  `/root/.cargo/bin/cargo`, `/usr/local/cargo/bin/cargo`, then
  `PATH`, since the supervisor chain does not always inherit the
  shell path). The worker logs `[xcode-check] start`, `done` (with
  the diagnostic count) and `spawn failed` to stderr, so a missing
  binary or a hanging run is visible in the terminal. JSON parses through Foundation
  `JsonValue` (serde-free); only `compiler-message` entries with
  `error` or `warning` level survive, mapped to their primary span.
  `apply_check_result` replaces the warnings list (What plus file
  and line per row, diagnostic code appended, foreign crates
  skipped) and the gutter markers follow through
  `sync_diagnostics`. Nothing is ever built or run: Run/Stop stays
  dead.
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
