//! Project editor window for Xcode (separate big window).
//!
//! A static Xcode-like IDE card built on the `Sidebar` element: working
//! traffic lights (owned by the sidebar), a dead Run/Stop pill pair at
//! the sidebar top right (no callbacks, nothing is ever built or run),
//! a non-collapsible file navigator with one preselected file, a live
//! search capsule stretched across the sidebar bottom (every content
//! hit across files, warnings filtered on their tab) and real editable
//! file content. Code pages accept clicks and typing like a normal
//! text field and auto-save their backing file 400ms after typing
//! stops; folders and binary files are read-only.
//!
//! Handoff without CLI: the starter sets `XCODE_PROJECT_NAME` on a
//! spawned copy of this binary and closes its own window at once;
//! `open_project_window` waits ~600ms first (old window visibly
//! closes, short gap), then opens the 1100x700 editor fresh.
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::process::Child;
use std::rc::Rc;
use std::sync::{
  Arc, Mutex,
  atomic::{AtomicBool, Ordering},
  mpsc::{Receiver, TryRecvError},
};
use std::time::Instant;

use crate::check::{
  CHECK_IDLE_DELAY, CheckResult, diagnostic_title, spawn_check,
};
use crate::code_editor::{CodeEditor, example_rust_code};
use crate::project_files::{FileEntry, list_project_files, load_file_text};
use crate::TontooUI::elements::{
  Align, BasicOutlineGroup, BasicText, BasicToolbar, BarSwitcher,
  BarSwitcherItem, FileImage, HorizontalDivider, HStack, MenuItem,
  NestedMenu, OutlineNode, RoundedRectangle, SearchField, SFSymbolImage,
  ShapeFill, Sidebar, SidebarItem, TextForeground, TextStyle, ToolbarItem,
  TrafficAction, View, VStack, GROUP_BG_LIGHT, MENU_BTN_PAD_X,
  MENU_CHEV_GAP, MENU_CHEV_W, SIDEBAR_BG_DARK,
};
use crate::TontooUI::renderer::window::{App, CursorKind, Key, Viewport, WindowCommand, run};
use crate::TontooUI::renderer::{FontSystem, ImageLoader};
use crate::TontooUI::theme::{ThemeMode, ThemeWatcher, desaturate};
use crate::lang;
use crate::scaffold::system_username;
use vello::Scene;
use vello::kurbo::{Affine, Rect};
use vello::peniko::{Brush, Color, Fill};

/// Env handoff carrying the project name into the editor process.
pub const PROJECT_ENV: &str = "XCODE_PROJECT_NAME";
/// Env handoff carrying the project path into the editor process.
pub const PROJECT_PATH_ENV: &str = "XCODE_PROJECT_PATH";
/// Gap between the old window closing and the editor opening.
const OPEN_DELAY_MS: u64 = 600;
/// Editor window size: much bigger than the 420x585 start page.
const EDITOR_W: u32 = 1100;
const EDITOR_H: u32 = 700;
/// Navigator file row index (preselected once).
const FILE_INDEX: usize = 4;
/// Device glyph size next to the menu and its gap.
const DEVICE_ICON: f32 = 22.0;
const DEVICE_GAP: f32 = 8.0;
/// Device menu text size (matches `button_font` below).
const DEVICE_FONT: f32 = 15.0;
/// Glass padding beyond the widest side.
const GLASS_PAD: f32 = 16.0;
/// Bottom search height: deliberately small; the width always spans
/// the full sidebar column.
const SEARCH_H: f32 = 28.0;
const SEARCH_PAD: f32 = 8.0;
/// Compact navigator rows (defaults 34.5/22/10/14 stay chunkier);
/// folders and files share one size and differ by icon only.
const NAV_ROW_H: f32 = 28.0;
const NAV_ROW_ICON: f32 = 18.0;
const NAV_ROW_GAP: f32 = 8.0;
const NAV_ROW_LABEL: f32 = 12.0;
/// Navigator tab switcher geometry: 36px pill in the blank gap
/// between the 64px toolbar zone and the file rows at 108px.
const NAV_TABS_Y: f32 = 66.0;
const NAV_TABS_H: f32 = 36.0;
/// Warnings overlay: rows start below the picker, one 52px row per
/// issue with a 6px gap.
const WARN_TOP: f32 = 106.0;
const WARN_ROW_H: f32 = 52.0;
const WARN_GAP: f32 = 6.0;
const WARN_ICON: f32 = 20.0;
/// Warning icon tints (macOS system colors, both themes).
const WARN_TINT: Color = Color::from_rgb8(0xff, 0xcc, 0x00);
const ERROR_TINT: Color = Color::from_rgb8(0xff, 0x3b, 0x30);

/// One example navigator issue: severity icon plus title, file row
/// index and 1-based line for the jump to code.
struct WarnItem {
  error: bool,
  title: String,
  file: usize,
  line: usize,
}

impl WarnItem {
  fn icon(&self) -> &'static str {
    if self.error {
      "xmark.octagon.fill"
    } else {
      "exclamationmark.triangle.fill"
    }
  }

  fn tint(&self) -> Color {
    if self.error {
      ERROR_TINT
    } else {
      WARN_TINT
    }
  }

  fn subtitle(&self, files: &[String]) -> String {
    let name = files.get(self.file).cloned().unwrap_or_default();
    format!("{name}:{}", self.line)
  }
}

/// One warnings overlay row: tinted severity icon plus title and
/// file/line subtitle.
fn warn_row(icon: &str, title: &str, subtitle: &str) -> HStack {
  let texts = VStack::new()
    .spacing(1.5)
    .align(Align::Leading)
    .child(BasicText::new(title).style(TextStyle::Caption))
    .child(
      BasicText::new(subtitle)
        .style(TextStyle::Caption2)
        .foreground(TextForeground::Secondary),
    );
  HStack::new()
    .spacing(9.0)
    .align(Align::Center)
    .child(SFSymbolImage::new(icon).size(WARN_ICON))
    .child(texts)
}

/// File stem rule mirroring the reference: alphanumeric name plus an
/// `App` suffix unless it already ends in `app` (`test` -> `testApp`,
/// `MyApp` -> `MyApp`).
pub fn file_stem(display_name: &str) -> String {
  let flat: String = display_name
    .chars()
    .filter(|c| c.is_ascii_alphanumeric())
    .collect();
  if flat.is_empty() {
    return "MyApp".to_string();
  }
  if flat.to_lowercase().ends_with("app") {
    flat
  } else {
    format!("{flat}App")
  }
}

fn example_code(file: &str, project: &str, user: &str) -> String {
  example_rust_code(file, project, user)
}

/// One bound editor page per navigator row: folders open as empty
/// read-only pages, files load their real text (editable) or a
/// read-only binary placeholder, memory-only rows without a real
/// path keep the fallback `code`.
fn page_for_entry(entry: &FileEntry, code: &str) -> CodeEditor {
  if entry.path.as_os_str().is_empty() {
    return CodeEditor::new(code.to_string());
  }
  if entry.is_dir {
    let mut page = CodeEditor::new(String::new());
    page.set_file(None, true);
    return page;
  }
  match load_file_text(&entry.path) {
    Some(text) => {
      let mut page = CodeEditor::new(text);
      page.set_file(Some(entry.path.clone()), false);
      page
    }
    None => {
      let mut page = CodeEditor::new(lang::t("ed.binary"));
      page.set_file(Some(entry.path.clone()), true);
      page
    }
  }
}

/// File type icon plus tint by extension: `.rs` rust orange, `.proj`
/// rotate purple, `.toml` gear gray, `.json` tray yellow, everything
/// else the normal file icon following the accent.
pub const FILE_RUST_TINT: Color = Color::from_rgb8(0xce, 0x42, 0x2b);
pub const FILE_PROJ_TINT: Color = Color::from_rgb8(0xaf, 0x52, 0xde);
pub const FILE_TOML_TINT: Color = Color::from_rgb8(0x8e, 0x8e, 0x93);
pub const FILE_JSON_TINT: Color = Color::from_rgb8(0xff, 0xcc, 0x00);

fn file_icon(name: &str) -> (&'static str, Option<Color>) {
  let ext = name
    .trim()
    .rsplit('.')
    .next()
    .filter(|_| name.trim().contains('.'))
    .unwrap_or("")
    .to_lowercase();
  match ext.as_str() {
    "rs" => ("rust", Some(FILE_RUST_TINT)),
    "proj" => ("rotate.3d", Some(FILE_PROJ_TINT)),
    "toml" => ("gear", Some(FILE_TOML_TINT)),
    "json" => ("tray.2.fill", Some(FILE_JSON_TINT)),
    _ => ("doc.fill", None),
  }
}

/// Build outline roots plus flat index paths from walker entries.
/// Folders nest recursively (open), files carry type icons; flat
/// order is the entries order, so index `i` owns page `i`.
fn build_tree(entries: &[FileEntry]) -> (Vec<OutlineNode>, Vec<Vec<usize>>) {
  fn level(
    entries: &[FileEntry],
    pos: &mut usize,
    flat_paths: &mut Vec<Vec<usize>>,
    path: Vec<usize>,
    depth: usize,
  ) -> Vec<OutlineNode> {
    let mut nodes = Vec::new();
    while *pos < entries.len() && entries[*pos].depth == depth {
      let entry = &entries[*pos];
      let mut node_path = path.clone();
      node_path.push(nodes.len());
      let name = entry.label.trim().to_string();
      if entry.is_dir {
        *pos += 1;
        flat_paths.push(node_path.clone());
        let kids = level(entries, pos, flat_paths, node_path, depth + 1);
        let mut folder = OutlineNode::folder(name).expanded(true);
        for kid in kids {
          folder = folder.child(kid);
        }
        nodes.push(folder);
      } else {
        let (icon, tint) = file_icon(&entry.label);
        let mut file = OutlineNode::file(name).icon(icon);
        if let Some(tint) = tint {
          file = file.icon_tint(tint);
        }
        nodes.push(file);
        flat_paths.push(node_path);
        *pos += 1;
      }
    }
    nodes
  }

  let mut flat_paths = Vec::with_capacity(entries.len());
  let mut pos = 0usize;
  let roots = level(entries, &mut pos, &mut flat_paths, Vec::new(), 0);
  (roots, flat_paths)
}

/// Point inside a logical rect.
fn point_in_rect(r: (f32, f32, f32, f32), x: f32, y: f32) -> bool {
  x >= r.0 && x <= r.0 + r.2 && y >= r.1 && y <= r.1 + r.3
}

impl EditorUi {
  /// Sidebar background over the navigator rows (both tab overlays
  /// share it so the flat rows never show through).
  fn paint_column_bg(
    scene: &mut Scene,
    px: impl Fn(f32) -> f64 + Copy,
    x: f32,
    y0: f32,
    w: f32,
    y1: f32,
    dark: bool,
    focused: bool,
  ) {
    let bg = if dark { SIDEBAR_BG_DARK } else { GROUP_BG_LIGHT };
    let bg = if focused { bg } else { desaturate(bg) };
    scene.fill(
      Fill::NonZero,
      Affine::IDENTITY,
      &Brush::Solid(bg),
      None,
      &Rect::new(px(x), px(y0), px(x + w), px(y1)),
    );
  }
}

/// Resolve a bundled `Resources/<file>` raster (device glyphs).
fn resource_path(file: &str) -> std::path::PathBuf {
  let mut candidates = Vec::new();
  if let Ok(env) = std::env::var("APP_RESOURCES_DIR") {
    if !env.is_empty() {
      candidates.push(std::path::PathBuf::from(env).join(file));
    }
  }
  if let Ok(cwd) = std::env::current_dir() {
    candidates.push(cwd.join("Resources").join(file));
  }
  if let Ok(exe) = std::env::current_exe() {
    if let Some(parent) = exe.parent() {
      candidates.push(parent.join("Resources").join(file));
      if let Some(grand) = parent.parent() {
        candidates.push(grand.join("Resources").join(file));
      }
    }
  }
  candidates.into_iter().find(|p| p.is_file()).unwrap_or_else(|| {
    std::path::PathBuf::from(format!("__xcode_missing_{file}__"))
  })
}

/// Resolve the bundled `Resources/computer.png` device glyph.
fn computer_icon_path() -> std::path::PathBuf {
  resource_path("computer.png")
}

/// Absolute path string for a bundled raster, for menu row icons.
/// Missing files resolve to a sentinel path: the row then draws its
/// plain label without an icon.
fn png(file: &str) -> String {
  resource_path(file).to_string_lossy().to_string()
}

/// One running background `cargo check`: exactly one job exists at a
/// time. A keystroke after the start revision cancels it (the child
/// is killed) and a fresh job starts once typing stops again.
struct ActiveCheck {
  /// Content revision the job started on.
  revision: u64,
  rx: Receiver<CheckResult>,
  cancel: Arc<AtomicBool>,
  child: Arc<Mutex<Option<Child>>>,
}

/// History navigation request from the chevron pills.
#[derive(Clone, Copy, PartialEq, Eq)]
enum NavDir {
  Back,
  Fwd,
}

/// Max content search hits (the results list stays usable).
pub const MAX_SEARCH_HITS: usize = 200;
/// Max chars of the matched code line shown per search row.
pub const SEARCH_SNIPPET_CHARS: usize = 90;

/// One content search hit: file row plus 1-based line, byte column
/// and the matched code line snippet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
  pub file: usize,
  pub line: usize,
  pub col: usize,
  pub snippet: String,
}

pub struct EditorUi {
  sidebar: Sidebar,
  search: SearchField,
  /// Divider between the topbar pills and the editor below.
  top_div: HorizontalDivider,
  /// Dead Run/Stop pair at the sidebar top right edge (hover/press
  /// tint only, no callbacks).
  run_stop: BasicToolbar,
  /// Centered device text-menu with sections (example selection only).
  /// Transparent body over a longer empty glass pill below. Reflects
  /// the last picked row (label plus icon, display only).
  device: NestedMenu,
  /// Row icons parallel to the menu items (`None` for headers).
  device_icons: Vec<Option<String>>,
  /// Row labels parallel to the menu items (for the top reflection).
  device_labels: Vec<String>,
  /// Picked row index, applied to button label and glyph in `update`.
  device_sel: Rc<Cell<Option<usize>>>,
  /// Device glyph left of the menu (plain `computer.png` raster).
  computer: FileImage,
  /// Glass pill behind the device menu (the actual toolbar look).
  menu_glass: BasicToolbar,
  /// Back/forward chevron pills at the content left (file visit
  /// history, round circles, greyed out at the history ends).
  chev_back: BasicToolbar,
  chev_fwd: BasicToolbar,
  /// Pending history navigation from the chevron pills.
  nav_request: Rc<Cell<Option<NavDir>>>,
  /// File visit history (sidebar row indices, capped).
  back_stack: Vec<usize>,
  forward_stack: Vec<usize>,
  /// Last recorded selection (external changes push history).
  history_top: usize,  /// Performance pill at the content right
  /// (`chart.line.uptrend.xyaxis`, round circle): toggles the bottom
  /// panel open or closed on every page.
  inspector: BasicToolbar,
  /// Bottom panel master switch, flipped by the inspector pill.
  panel_open: Rc<Cell<bool>>,
  /// Last applied switch state (divider taps stay per page).
  last_panel_open: Cell<bool>,
  /// Navigator tab switcher (`Files` default, `Warnings & Errors`).
  nav_tabs: BarSwitcher,
  /// Active navigator tab, flipped by the picker.
  nav_tab: Rc<Cell<usize>>,
  /// Example navigator issues (3 warnings plus 2 errors).
  warn_items: Vec<WarnItem>,
  /// File names parallel to the sidebar items (for subtitles).
  warn_files: Vec<String>,
  /// Overlay rows parallel to the issues.
  warn_rows: Vec<HStack>,
  /// Accent wash behind the jumped-to warning.
  warn_bg: RoundedRectangle,
  /// Last jumped-to warning.
  selected_warn: usize,
  /// Placed warning row rects for tap jumps.
  warn_rects: Vec<(f32, f32, f32, f32)>,
  /// Content search hits for the current query (row order, then line).
  search_hits: Vec<SearchHit>,
  /// Overlay rows parallel to the search hits.
  search_rows: Vec<HStack>,
  /// Placed search row rects for tap jumps.
  search_rects: Vec<(f32, f32, f32, f32)>,
  /// Last jumped-to search hit.
  selected_search: usize,
  /// Last indexed query (recomputes only on change).
  last_query: String,
  /// Placeholder line while a query matches nothing.
  search_empty: BasicText,
  /// Navigator page count (one file page per row).
  page_total: usize,
  /// Real filesystem path per row (empty for memory-only fallbacks).
  file_paths: Vec<PathBuf>,
  /// Project root for `cargo check` (`None` without a real project).
  project_root: Option<PathBuf>,
  /// Running background check, if any (never more than one).
  check_job: Option<ActiveCheck>,
  /// Latest spawned check generation (stale results are dropped).
  check_generation: u64,
  /// Content revision covered by the last applied check result.
  content_at_check: u64,
  /// Last content-changing keystroke (2.5s idle starts a check).
  last_change: Option<Instant>,
  /// Glass pill plus label showing `check.indexing` while a check runs.
  check_glass: BasicToolbar,
  check_text: BasicText,
  /// Nested outline tree for the Files tab (covers the flat rows).
  nav_tree: BasicOutlineGroup,
  /// Flat row index to outline path (entries order).
  tree_paths: Vec<Vec<usize>>,
  /// Pending outline selection from `on_select`.
  tree_sel: Rc<RefCell<Option<Vec<usize>>>>,
  /// Placed tree rect for hit routing.
  tree_rect: (f32, f32, f32, f32),
  /// Cached I-beam state for code pages (updated on hover).
  code_ibeam: Cell<bool>,
  /// Cached divider resize state for code pages (updated on hover).
  divider_cursor: Cell<bool>,
  watcher: ThemeWatcher,
  focused: bool,
  bg: Color,
  command: Option<WindowCommand>,
}

impl EditorUi {
  pub fn new(project: &str, code: String) -> Self {
    // Built-in example rows (fallback without a project path,
    // memory only, nothing is saved).
    let file = file_stem(project);
    let empty = PathBuf::new();
    let entries = vec![
      FileEntry { label: project.to_string(), is_dir: true, depth: 0, path: empty.clone() },
      FileEntry { label: "Assets".to_string(), is_dir: true, depth: 1, path: empty.clone() },
      FileEntry { label: "ContentView".to_string(), is_dir: false, depth: 1, path: empty.clone() },
      FileEntry { label: "Info".to_string(), is_dir: false, depth: 1, path: empty.clone() },
      FileEntry { label: file, is_dir: false, depth: 1, path: empty },
    ];
    let warn_specs = vec![
      (false, lang::t("issue.unused_var"), 4, 36),
      (true, lang::t("issue.type_mismatch"), 4, 33),
      (false, lang::t("issue.trailing_ws"), 4, 9),
      (true, lang::t("issue.unresolved_import"), 2, 1),
      (false, lang::t("issue.missing_docs"), 4, 22),
    ];
    Self::from_entries(None, entries, code, FILE_INDEX, warn_specs)
  }

  /// Open a real project root: every file row loads its real text
  /// content (binary and oversized files open read-only), folders
  /// open as empty read-only pages. The first `.rs` row is
  /// preselected. Text edits auto-save 400ms after typing stops;
  /// nothing is ever built or run.
  pub fn open(project: &str, root: &Path) -> Result<Self, String> {
    if !root.is_dir() {
      return Err(format!("missing project dir: {}", root.display()));
    }
    let entries = list_project_files(root);
    let user = system_username();
    let file = file_stem(project);
    let code = example_code(&file, project, &user);
    let select = entries
      .iter()
      .position(|entry| !entry.is_dir && entry.label.trim().ends_with(".rs"))
      .unwrap_or(0);
    let secondary = if entries.len() > 1 {
      (select + 1) % entries.len()
    } else {
      select
    };
    let warn_specs = vec![
      (false, lang::t("issue.unused_var"), select, 36),
      (true, lang::t("issue.type_mismatch"), select, 33),
      (false, lang::t("issue.trailing_ws"), select, 9),
      (true, lang::t("issue.unresolved_import"), secondary, 1),
      (false, lang::t("issue.missing_docs"), select, 22),
    ];
    Ok(Self::from_entries(Some(root.to_path_buf()), entries, code, select, warn_specs))
  }

  /// Shared constructor from walker entries: flat sidebar rows plus
  /// one bound page per row (real file text, read-only folders and
  /// binary placeholders), warning names plus specs, and the nested
  /// outline tree mapping flat rows to outline paths. `code` is the
  /// fallback text for memory-only rows without a real file. `root`
  /// enables the background `cargo check` (plus one initial run once
  /// the window idles).
  fn from_entries(
    root: Option<PathBuf>,
    entries: Vec<FileEntry>,
    code: String,
    select: usize,
    warn_specs: Vec<(bool, String, usize, usize)>,
  ) -> Self {
    let items = entries
      .iter()
      .map(|entry| {
        SidebarItem::new(
          entry.label.clone(),
          if entry.is_dir { "folder.fill" } else { "doc.fill" },
        )
      })
      .collect::<Vec<_>>();
    let pages = entries
      .iter()
      .map(|entry| page_for_entry(entry, &code))
      .collect::<Vec<_>>();
    let warn_files = entries
      .iter()
      .map(|entry| entry.label.trim().to_string())
      .collect::<Vec<_>>();
    let file_paths = entries.iter().map(|entry| entry.path.clone()).collect::<Vec<_>>();
    let (tree_nodes, flat_paths) = build_tree(&entries);
    let mut ui =
      Self::assemble(items, pages, select, warn_files, warn_specs, tree_nodes, flat_paths);
    ui.file_paths = file_paths;
    ui.project_root = root.clone();
    if root.is_some() {
      // Kick one initial check once the fresh window idles: the
      // sentinel revision never matches real content, so the 2.5s
      // timer fires one run even before the first keystroke.
      ui.content_at_check = u64::MAX;
      ui.last_change = Some(Instant::now());
    }
    ui
  }

  fn assemble(
    items: Vec<SidebarItem>,
    pages: Vec<CodeEditor>,
    select: usize,
    warn_files: Vec<String>,
    warn_specs: Vec<(bool, String, usize, usize)>,
    tree_nodes: Vec<OutlineNode>,
    flat_paths: Vec<Vec<usize>>,
  ) -> Self {
    let page_total = pages.len();
    // Nested outline tree for the Files tab (trailing chevron,
    // remembered open state, animated). Selection reports through
    // `tree_sel` and lands on the flat page in `draw`.
    let tree_sel = Rc::new(RefCell::new(None::<Vec<usize>>));
    let pending = tree_sel.clone();
    let mut nav_tree = BasicOutlineGroup::new(tree_nodes)
      .trailing_chevron(true)
      .on_select(move |path| {
        pending.borrow_mut().replace(path);
      });
    if let Some(path) = flat_paths.get(select).cloned() {
      nav_tree.select(path);
    }
    // Row icons parallel to the menu items below (`None` for headers
    // and dividers): the picked row shows here on top, display only.
    let device_icons = vec![
      None,
      Some(png("computer.png")),
      None,
      None,
      Some(png("wrench.png")),
      Some(png("wrench.png")),
      None,
      None,
      Some(png("up.png")),
    ];
    let device_sel = Rc::new(Cell::new(None));
    let picked = device_sel.clone();
    let device_labels = vec![
      lang::t("menu.devices"),
      lang::t("menu.device"),
      String::new(),
      lang::t("menu.build"),
      lang::t("menu.prod"),
      lang::t("menu.dev"),
      String::new(),
      lang::t("menu.utils"),
      lang::t("menu.export"),
    ];
    let code_pages = pages;
    let mut sidebar = Sidebar::new(items);    for page in code_pages {
      sidebar = sidebar.page(page);
    }
    sidebar = sidebar
      .search_field(false)
      .toggle_button(false)
      .collapsible(false);
    // No content title (neither scheme nor item label): the topbar
    // holds history chevrons left, the device menu center and the
    // performance pill right instead.
    sidebar.set_title(String::new());
    sidebar.select(select);
    // Compact rows for the file list (folders and files alike).
    sidebar.set_row_metrics(NAV_ROW_H, NAV_ROW_ICON, NAV_ROW_GAP, NAV_ROW_LABEL);
    // The element defaults item labels to hand-set white, which wins
    // over the theme in light mode: clear the override once so labels
    // follow `set_theme` (dark `#d8d9d9`, light `#272727`).
    sidebar.set_item_text(None);
    // Bottom panel master switch: the performance pill flips it,
    // `draw` applies edges to every page (divider taps stay per
    // page, see `apply_panel_open`). Starts closed, nothing is
    // generated, placeholders show instead.
    let panel_open = Rc::new(Cell::new(false));
    let panel_toggle = panel_open.clone();
    // Navigator tabs: `Files` default plus `Warnings & Errors`.
    // Bar switcher with icon plus label cells in the blank gap
    // above the file rows; the warnings tab covers the rows with
    // the example issue list.
    let nav_tab = Rc::new(Cell::new(0));
    let tab_flip = nav_tab.clone();
    let nav_tabs = BarSwitcher::from_items(vec![
      BarSwitcherItem::both("folder.fill", lang::t("nav.files")),
      BarSwitcherItem::both(
        "exclamationmark.triangle.fill",
        lang::t("nav.warnings"),
      ),
    ])
    .selected(0)
    .on_select(move |index| tab_flip.set(index));
    let warn_items = warn_specs
      .into_iter()
      .map(|(error, title, file, line)| WarnItem { error, title, file, line })
      .collect::<Vec<_>>();
    let warn_rows = warn_items
      .iter()
      .map(|item| warn_row(item.icon(), &item.title, &item.subtitle(&warn_files)))
      .collect();
    let inspector =
      BasicToolbar::from_items(vec![ToolbarItem::icon("chart.line.uptrend.xyaxis")])
        .round(true)
        .on_action(move |_| panel_toggle.set(!panel_toggle.get()));
    // History navigation requests from the chevron pills.
    let nav_request = Rc::new(Cell::new(None));
    Self {
      sidebar,
      search: SearchField::new(lang::t("ed.search")),
      top_div: HorizontalDivider::new(),
      run_stop: BasicToolbar::from_items(vec![
        ToolbarItem::icon("play.fill"),
        ToolbarItem::divider(),
        ToolbarItem::icon("stop.fill"),
      ]),
      computer: FileImage::new(computer_icon_path(), DEVICE_ICON, DEVICE_ICON).radius(5.0),
      device: NestedMenu::new(
        lang::t("menu.device"),
        vec![
          MenuItem::section(lang::t("menu.devices")),
          MenuItem::action(lang::t("menu.device")).icon(png("computer.png")),
          MenuItem::divider(),
          MenuItem::section(lang::t("menu.build")),
          MenuItem::action(lang::t("menu.prod")).icon(png("wrench.png")),
          MenuItem::action(lang::t("menu.dev")).icon(png("wrench.png")),
          MenuItem::divider(),
          MenuItem::section(lang::t("menu.utils")),
          MenuItem::action(lang::t("menu.export")).icon(png("up.png")),
        ],
      )
      .on_action(move |path| {
        println!("device menu {path:?} (example)");
        if let Some(&index) = path.first() {
          picked.set(Some(index));
        }
      })
      .transparent_button(true)
      .button_font(DEVICE_FONT),
      device_icons,
      device_labels,
      device_sel,
      menu_glass: BasicToolbar::new(),
      chev_back: BasicToolbar::from_items(vec![ToolbarItem::icon("chevron.left")])
        .round(true)
        .on_action({
          let request = nav_request.clone();
          move |_| request.set(Some(NavDir::Back))
        }),
      chev_fwd: BasicToolbar::from_items(vec![ToolbarItem::icon("chevron.right")])
        .round(true)
        .on_action({
          let request = nav_request.clone();
          move |_| request.set(Some(NavDir::Fwd))
        }),
      nav_request,
      back_stack: Vec::new(),
      forward_stack: Vec::new(),
      history_top: select,
      inspector,
      panel_open,
      last_panel_open: Cell::new(false),
      nav_tabs,
      nav_tab,
      warn_items,
      warn_files,
      warn_rows,
      warn_bg: RoundedRectangle::new(200.0, WARN_ROW_H, 8.0)
        .fill(Color::from_rgb8(0x00, 0x7a, 0xff)),
      selected_warn: 0,
      warn_rects: Vec::new(),
      search_hits: Vec::new(),
      search_rows: Vec::new(),
      search_rects: Vec::new(),
      selected_search: 0,
      last_query: String::new(),
      search_empty: BasicText::new(lang::t("search.no_results")),
      page_total,
      file_paths: Vec::new(),
      project_root: None,
      check_job: None,
      check_generation: 0,
      content_at_check: 0,
      last_change: None,
      check_glass: BasicToolbar::new(),
      check_text: BasicText::new(lang::t("check.indexing")),
      nav_tree,
      tree_paths: flat_paths,
      tree_sel,
      tree_rect: (0.0, 0.0, 0.0, 0.0),
      code_ibeam: Cell::new(false),
      divider_cursor: Cell::new(false),
      watcher: ThemeWatcher::new(),
      focused: true,
      bg: crate::TontooUI::renderer::window::BACKGROUND,
      command: None,
    }
  }

  fn theme_code_pages(&mut self, accent: Color, mode: ThemeMode, dark: bool, focused: bool) {
    self.each_page(|ed| {
      ed.set_theme(accent, mode, dark);
      ed.set_focused(focused);
    });
  }

  /// Hand every page its own file diagnostics (line plus
  /// severity) so marked lines render badge, number and wash.
  fn sync_diagnostics(&mut self) {
    for index in 0..self.page_total {
      let markers: Vec<(usize, bool)> = self
        .warn_items
        .iter()
        .filter(|item| item.file == index)
        .map(|item| (item.line, item.error))
        .collect();
      if let Some(page) = self.sidebar.page_mut(index) {
        if let Some(ed) = page.as_any_mut().downcast_mut::<CodeEditor>() {
          ed.set_diagnostics(&markers);
        }
      }
    }
  }

  /// Frame clock hook for every page: placeholder timing plus the
  /// 400ms file auto-save. Nothing is built or run.
  fn tick_code_pages(&mut self, now_secs: f64) {
    self.each_page(|ed| {
      ed.tick(now_secs);
      ed.poll_autosave();
    });
  }

  /// Sum of all page mutation counters (drives check scheduling).
  fn content_revision(&mut self) -> u64 {
    let mut sum = 0u64;
    self.each_page(|ed| sum += ed.edit_revision());
    sum
  }

  /// Record a content-changing keystroke for the 2.5s check delay,
  /// but only when the active page is really editable.
  fn note_change_if_editable(&mut self) {
    let selected = self.sidebar.selected_index();
    let editable = self
      .sidebar
      .page_mut(selected)
      .and_then(|page| page.as_any_mut().downcast_mut::<CodeEditor>())
      .is_some_and(|ed| !ed.is_read_only());
    if editable {
      self.last_change = Some(Instant::now());
    }
  }

  /// Row index for a rustc file path (relative to the project root).
  /// Foreign crate diagnostics match nothing and are skipped.
  fn match_row(&self, root: &Path, file: &str) -> Option<usize> {
    let absolute = root.join(file);
    self.file_paths.iter().position(|path| *path == absolute)
  }

  /// Single-job scheduler, called every frame: reaps finished runs,
  /// cancels a running job once newer keystrokes exist (killing its
  /// child), and starts a fresh run after 2.5s idle when the content
  /// moved past the last applied result. Files are flushed to disk
  /// before every spawn so the check sees the latest text.
  fn poll_check(&mut self) {
    let mut finished: Option<CheckResult> = None;
    let mut gone = false;
    if let Some(job) = &self.check_job {
      match job.rx.try_recv() {
        Ok(result) => finished = Some(result),
        Err(TryRecvError::Empty) => {}
        Err(TryRecvError::Disconnected) => gone = true,
      }
    }
    if let Some(result) = finished {
      self.check_job = None;
      if result.generation == self.check_generation {
        self.apply_check_result(result);
      }
    } else if gone {
      // The worker died without a result (e.g. cargo missing):
      // back off until the next keystroke instead of respawning
      // every frame.
      self.check_job = None;
      self.content_at_check = self.content_revision();
    }
    if let Some(job) = self.check_job.take() {
      // A newer keystroke revision cancels the running job: the
      // worker drops its result and the child is killed.
      let revision = self.content_revision();
      if revision != job.revision {
        job.cancel.store(true, Ordering::Relaxed);
        if let Ok(mut slot) = job.child.lock() {
          if let Some(mut child) = slot.take() {
            let _ = child.kill();
            let _ = child.wait();
          }
        }
      } else {
        self.check_job = Some(job);
      }
    }
    if self.check_job.is_some() {
      return;
    }
    let Some(root) = self.project_root.clone() else {
      return;
    };
    if !root.join("Cargo.toml").is_file() {
      return;
    }
    if self.content_revision() == self.content_at_check {
      return;
    }
    let quiet = self
      .last_change
      .is_some_and(|at| at.elapsed() >= CHECK_IDLE_DELAY);
    if !quiet {
      return;
    }
    self.save_all_now();
    let revision = self.content_revision();
    self.start_check(root, revision);
  }

  /// Spawn one background `cargo check` for `revision`.
  fn start_check(&mut self, root: PathBuf, revision: u64) {
    self.check_generation += 1;
    let generation = self.check_generation;
    let cancel = Arc::new(AtomicBool::new(false));
    let child = Arc::new(Mutex::new(None));
    let (tx, rx) = std::sync::mpsc::channel();
    spawn_check(root, generation, revision, cancel.clone(), child.clone(), tx);
    self.check_job = Some(ActiveCheck { revision, rx, cancel, child });
  }

  /// Replace the warnings list and gutter markers with real check
  /// diagnostics (What plus file and line per row).
  fn apply_check_result(&mut self, result: CheckResult) {
    self.content_at_check = result.revision;
    let Some(root) = self.project_root.clone() else {
      return;
    };
    let mut items = Vec::new();
    for diag in result.diagnostics.iter().take(crate::check::MAX_DIAGNOSTICS) {
      if let Some(index) = self.match_row(&root, &diag.file) {
        items.push(WarnItem {
          error: diag.error,
          title: diagnostic_title(diag),
          file: index,
          line: diag.line.max(1),
        });
      }
    }
    items.sort_by(|a, b| (a.file, a.line).cmp(&(b.file, b.line)));
    self.selected_warn = 0;
    self.warn_rows = items
      .iter()
      .map(|item| warn_row(item.icon(), &item.title, &item.subtitle(&self.warn_files)))
      .collect();
    self.warn_items = items;
    self.sync_diagnostics();
  }

  /// Write every dirty bound file now (used on quit paths).
  pub fn save_all_now(&mut self) {
    self.each_page(|ed| {
      ed.save_now();
    });
  }

  /// Fold or unfold the bottom panel on every page (the inspector
  /// pill flips the switch, `draw` applies its edges).
  fn apply_panel_open(&mut self, open: bool) {
    self.each_page(|ed| ed.set_collapsed(!open));
  }

  /// Same as the inspector pill click (flips the switch and applies
  /// it at once, so tests need no window).
  pub fn toggle_panel(&mut self) {
    let open = !self.panel_open.get();
    self.panel_open.set(open);
    self.last_panel_open.set(open);
    self.apply_panel_open(open);
  }

  /// True while stepping back through visited files is possible.
  pub fn can_go_back(&self) -> bool {
    !self.back_stack.is_empty()
  }

  /// True while stepping forward through visited files is possible.
  pub fn can_go_forward(&self) -> bool {
    !self.forward_stack.is_empty()
  }

  /// Record an external file selection into the visit history
  /// (called every frame): a new file pushes the previous one back
  /// and drops the forward trail.
  fn record_selection(&mut self) {
    let selected = self.sidebar.selected_index();
    if selected != self.history_top {
      self.back_stack.push(self.history_top);
      if self.back_stack.len() > 100 {
        self.back_stack.remove(0);
      }
      self.forward_stack.clear();
      self.history_top = selected;
    }
  }

  /// Select a file with tree sync, without touching the history
  /// (history navigation drives this itself).
  fn select_file_synced(&mut self, index: usize) {
    if index >= self.page_total {
      return;
    }
    if let Some(path) = self.tree_paths.get(index).cloned() {
      for depth in 1..path.len() {
        self.nav_tree.set_expanded(&path[..depth], true);
      }
      self.nav_tree.select(path);
    }
    self.sidebar.select(index);
    self.history_top = index;
  }

  /// Step back to the previously visited file.
  pub fn go_back(&mut self) {
    if let Some(previous) = self.back_stack.pop() {
      self.forward_stack.push(self.history_top);
      self.select_file_synced(previous);
    }
  }

  /// Step forward to the file left by going back.
  pub fn go_forward(&mut self) {
    if let Some(next) = self.forward_stack.pop() {
      self.back_stack.push(self.history_top);
      self.select_file_synced(next);
    }
  }

  /// Apply a pending chevron pill request, if any.
  fn poll_nav_request(&mut self) {
    if let Some(direction) = self.nav_request.take() {
      match direction {
        NavDir::Back => self.go_back(),
        NavDir::Fwd => self.go_forward(),
      }
    }
  }

  /// Jump to an issue: select its file and move the caret to its
  /// line. Stays on the current tab (only the editor content moves).
  /// Runs at once on press so the content never flashes the wrong file.
  pub(crate) fn jump_to_issue(&mut self, index: usize) {
    let Some(item) = self.warn_items.get(index) else {
      return;
    };
    let (file, line) = (item.file, item.line);
    if file >= self.warn_files.len() {
      return;
    }
    self.selected_warn = index;
    // Expand the ancestor folders so the jumped-to row is visible.
    if let Some(path) = self.tree_paths.get(file).cloned() {
      for depth in 1..path.len() {
        self.nav_tree.set_expanded(&path[..depth], true);
      }
      self.nav_tree.select(path);
    }
    self.sidebar.select(file);
    if let Some(page) = self.sidebar.page_mut(file) {
      if let Some(ed) = page.as_any_mut().downcast_mut::<CodeEditor>() {
        ed.goto_line(line);
      }
    }
  }

  /// Apply a pending outline selection to the flat page.
  fn poll_tree_select(&mut self) {
    if let Some(path) = self.tree_sel.borrow_mut().take() {
      if let Some(flat) = self.tree_paths.iter().position(|p| *p == path) {
        self.sidebar.select(flat);
      }
    }
  }

  /// True while the search capsule holds a query: the results overlay
  /// covers the navigator rows (Files tab) or filters the warnings.
  pub fn search_active(&self) -> bool {
    !self.search.text_value().trim().is_empty()
  }

  /// Rebuild the content search index when the query changed
  /// (case-insensitive, every occurrence, row order then line order,
  /// read-only pages skipped, capped at `MAX_SEARCH_HITS`).
  pub(crate) fn recompute_search(&mut self) {
    let query = self.search.text_value().trim().to_string();
    if query == self.last_query {
      return;
    }
    self.last_query = query.clone();
    self.search_hits.clear();
    self.search_rows.clear();
    self.selected_search = 0;
    let needle = query.to_lowercase();
    if needle.is_empty() {
      return;
    }
    let mut pages: Vec<(String, bool)> = Vec::new();
    self.each_page(|ed| pages.push((ed.text_value().to_string(), ed.is_read_only())));
    for (index, (text, read_only)) in pages.iter().enumerate() {
      if *read_only || self.search_hits.len() >= MAX_SEARCH_HITS {
        continue;
      }
      for (ln, line) in text.split('\n').enumerate() {
        let lower = line.to_lowercase();
        let mut from = 0usize;
        while from <= lower.len() {
          let Some(off) = lower[from..].find(needle.as_str()) else {
            break;
          };
          let chars_before = lower[..from + off].chars().count();
          let byte_col = line
            .char_indices()
            .nth(chars_before)
            .map(|(b, _)| b)
            .unwrap_or(line.len());
          let mut snippet: String = line
            .trim()
            .replace('\t', "  ")
            .chars()
            .take(SEARCH_SNIPPET_CHARS)
            .collect();
          if line.trim().chars().count() > SEARCH_SNIPPET_CHARS {
            snippet.push_str("...");
          }
          self.search_hits.push(SearchHit {
            file: index,
            line: ln + 1,
            col: byte_col,
            snippet,
          });
          if self.search_hits.len() >= MAX_SEARCH_HITS {
            break;
          }
          from += off + needle.len();
        }
        if self.search_hits.len() >= MAX_SEARCH_HITS {
          break;
        }
      }
    }
    self.search_rows = self
      .search_hits
      .iter()
      .map(|hit| {
        let name = self
          .warn_files
          .get(hit.file)
          .cloned()
          .unwrap_or_default();
        warn_row("doc.fill", &format!("{name}:{}", hit.line), &hit.snippet)
      })
      .collect();
  }

  /// Warning indices visible under the current query (all without a
  /// query, title or file/line match with one).
  pub(crate) fn visible_warn_indices(&self) -> Vec<usize> {
    let query = self.search.text_value().trim().to_lowercase();
    if query.is_empty() {
      return (0..self.warn_items.len()).collect();
    }
    self
      .warn_items
      .iter()
      .enumerate()
      .filter(|(_, item)| {
        item.title.to_lowercase().contains(&query)
          || item.subtitle(&self.warn_files).to_lowercase().contains(&query)
      })
      .map(|(index, _)| index)
      .collect()
  }

  /// Jump to a content search hit: select its file and move the caret
  /// to its line and column. Runs at once on press.
  pub(crate) fn jump_to_hit(&mut self, pos: usize) {
    let Some(hit) = self.search_hits.get(pos).cloned() else {
      return;
    };
    if hit.file >= self.warn_files.len() {
      return;
    }
    self.selected_search = pos;
    if let Some(path) = self.tree_paths.get(hit.file).cloned() {
      for depth in 1..path.len() {
        self.nav_tree.set_expanded(&path[..depth], true);
      }
      self.nav_tree.select(path);
    }
    self.sidebar.select(hit.file);
    if let Some(page) = self.sidebar.page_mut(hit.file) {
      if let Some(ed) = page.as_any_mut().downcast_mut::<CodeEditor>() {
        ed.goto_line_col(hit.line, hit.col);
      }
    }
  }

  /// Jump to the first visible hit (Enter in the search capsule):
  /// first content hit on the Files tab, first matching warning on
  /// the Warnings tab.
  pub(crate) fn jump_to_first_search_hit(&mut self) {
    if self.nav_tab.get() == 1 {
      if let Some(&real) = self.visible_warn_indices().first() {
        self.jump_to_issue(real);
      }
    } else if !self.search_hits.is_empty() {
      self.jump_to_hit(0);
    }
  }

  /// Active navigator tab (`0` files, `1` warnings).
  pub fn nav_index(&self) -> usize {
    self.nav_tab.get()
  }

  /// Run over every file page (navigator rows own them 1:1).
  fn each_page(&mut self, mut f: impl FnMut(&mut CodeEditor)) {
    for index in 0..self.page_total {
      if let Some(page) = self.sidebar.page_mut(index) {
        if let Some(ed) = page.as_any_mut().downcast_mut::<CodeEditor>() {
          f(ed);
        }
      }
    }
  }

  fn refresh_code_ibeam(&mut self) {
    let mut hovered = false;
    let mut divider = false;
    self.each_page(|ed| {
      if ed.wants_text_cursor() {
        hovered = true;
      }
      if ed.wants_divider_cursor() {
        divider = true;
      }
    });
    self.code_ibeam.set(hovered);
    self.divider_cursor.set(divider);
  }

  pub fn draw(
    &mut self,
    scene: &mut Scene,
    fonts: &mut FontSystem,
    images: &mut ImageLoader<'_>,
    viewport: Viewport,
    time_secs: f64,
  ) {
    self.watcher.poll(time_secs);
    self.watcher.set_focused(self.focused, time_secs);
    let palette = self.watcher.palette(time_secs);
    let theme = self.watcher.theme();
    let dark = theme.mode == ThemeMode::Dark;
    let (mode, focused) = (theme.mode, self.focused);
    self.bg = palette.bg;

    // Background check scheduling (reap, cancel on new keystrokes,
    // spawn after 2.5s idle). Files were flushed before every spawn.
    self.poll_check();
    // Content search index follows the query (cached on change).
    self.recompute_search();

    self.sidebar.set_theme(palette.accent, dark);
    self.sidebar.set_glass(theme.mode, theme.glass);
    self.sidebar.set_focused(focused);
    self.search.set_theme(theme.mode, palette.accent, theme.glass);
    self.search.set_focused(focused);
    self.top_div.set_theme(palette.divider, dark);
    self.top_div.set_focused(focused);
    self.run_stop.set_theme(theme.mode, theme.glass);
    self.run_stop.set_focused(focused);
    self.computer.set_theme(dark);
    self.computer.set_focused(focused);
    self.device.set_theme(palette.accent, dark);
    self.device.set_glass(theme.mode, theme.glass);
    self.device.set_focused(focused);
    self.menu_glass.set_theme(theme.mode, theme.glass);
    self.menu_glass.set_focused(focused);
    self.chev_back.set_theme(theme.mode, theme.glass);
    self.chev_back.set_focused(focused);
    self.chev_back.set_disabled(!self.can_go_back());
    self.chev_fwd.set_theme(theme.mode, theme.glass);
    self.chev_fwd.set_focused(focused);
    self.chev_fwd.set_disabled(!self.can_go_forward());
    self.inspector.set_theme(theme.mode, theme.glass);
    self.inspector.set_focused(focused);
    self.check_glass.set_theme(theme.mode, theme.glass);
    self.check_glass.set_focused(focused);
    self.check_text.set_theme(theme.mode);
    self.check_text.set_focused(focused);
    self.nav_tabs.set_theme(theme.mode, theme.glass);
    self.nav_tabs.set_focused(focused);
    self.nav_tree.set_theme(palette.accent, dark);
    self.nav_tree.set_focused(focused);
    self.warn_bg.set_fill(ShapeFill::Solid(palette.accent));
    self.warn_bg.set_focused(focused);
    // Warning rows: the jumped-to row shows white text and icon on
    // the accent fill, the rest follow the theme with tinted icons.
    for (index, row) in self.warn_rows.iter_mut().enumerate() {
      let selected = index == self.selected_warn;
      let tint = self.warn_items.get(index).map(|item| item.tint());
      if let Some(symbol) = row.child_mut::<SFSymbolImage>(0) {
        if selected {
          symbol.set_color(Some(Color::WHITE));
        } else {
          symbol.set_color(tint);
          symbol.set_theme(palette.text, dark);
        }
        symbol.set_focused(focused);
      }
      if let Some(texts) = row.child_mut::<VStack>(1) {
        if let Some(title) = texts.child_mut::<BasicText>(0) {
          if selected {
            title.set_foreground(TextForeground::Color(Color::WHITE));
          } else {
            title.set_foreground(TextForeground::Primary);
            title.set_theme(theme.mode);
          }
          title.set_focused(focused);
        }
        if let Some(sub) = texts.child_mut::<BasicText>(1) {
          if selected {
            sub.set_foreground(TextForeground::Color(Color::from_rgba8(
              255, 255, 255, 220,
            )));
          } else {
            sub.set_foreground(TextForeground::Secondary);
            sub.set_theme(theme.mode);
          }
          sub.set_focused(focused);
        }
      }
    }
    // Search rows: same selected wash, a plain file icon for every
    // hit, theme text otherwise.
    for (pos, row) in self.search_rows.iter_mut().enumerate() {
      let selected = pos == self.selected_search;
      if let Some(symbol) = row.child_mut::<SFSymbolImage>(0) {
        if selected {
          symbol.set_color(Some(Color::WHITE));
        } else {
          symbol.set_color(None);
          symbol.set_theme(palette.text, dark);
        }
        symbol.set_focused(focused);
      }
      if let Some(texts) = row.child_mut::<VStack>(1) {
        if let Some(title) = texts.child_mut::<BasicText>(0) {
          if selected {
            title.set_foreground(TextForeground::Color(Color::WHITE));
          } else {
            title.set_foreground(TextForeground::Primary);
            title.set_theme(theme.mode);
          }
          title.set_focused(focused);
        }
        if let Some(sub) = texts.child_mut::<BasicText>(1) {
          if selected {
            sub.set_foreground(TextForeground::Color(Color::from_rgba8(
              255, 255, 255, 220,
            )));
          } else {
            sub.set_foreground(TextForeground::Secondary);
            sub.set_theme(theme.mode);
          }
          sub.set_focused(focused);
        }
      }
    }
    self.search_empty.set_theme(theme.mode);
    self.search_empty.set_focused(focused);
    // Picked menu row reflects on top (label plus glyph, display
    // only, no function).
    if let Some(index) = self.device_sel.take() {
      if let Some(label) = self.device_labels.get(index) {
        if !label.is_empty() {
          self.device.set_button(label.clone());
        }
      }
      if let Some(icon) = self.device_icons.get(index).and_then(|o| o.clone()) {
        self.computer.set_path(icon);
      }
    }
    // Code pages follow the theme with live highlight colors.
    self.theme_code_pages(palette.accent, mode, dark, focused);
    // Diagnostic markers per file (badge, number, wash).
    self.sync_diagnostics();
    // Fake stats and logs advance with the frame time.
    self.tick_code_pages(time_secs);
    // Apply inspector pill edges to every page (divider taps stay
    // per page and never touch the switch).
    let open = self.panel_open.get();
    if open != self.last_panel_open.get() {
      self.last_panel_open.set(open);
      self.apply_panel_open(open);
    }
    // Outline selection lands on the flat page (same frame, so the
    // content never flashes the wrong file). External selection
    // changes feed the visit history, then chevron requests navigate.
    self.poll_tree_select();
    self.record_selection();
    self.poll_nav_request();

    // No titlebar: the sidebar owns the decoration (traffic lights
    // live in it) and fills the whole viewport.
    self.sidebar.place(fonts, viewport.x, viewport.y, viewport.width, viewport.height);
    self.sidebar.draw(scene, fonts, images);
    let col_w = self.sidebar.width_value();
    let content_x = viewport.x + col_w;
    let content_w = (viewport.width - col_w).max(0.0);
    // Navigator tab switcher over the blank gap above the file rows.
    let tabs_w = (col_w - SEARCH_PAD * 2.0).max(0.0);
    self.nav_tabs.place(
      fonts,
      viewport.x + SEARCH_PAD,
      viewport.y + NAV_TABS_Y,
      tabs_w,
      NAV_TABS_H,
    );
    self.nav_tabs.draw(scene, fonts, images);
    let search_top = viewport.y + viewport.height - SEARCH_PAD - SEARCH_H;
    // Files tab with a query: content search hits cover the tree
    // (plain file icon plus `file:line` plus the matched snippet).
    // A click jumps straight into the editor.
    if self.nav_tab.get() == 0 && self.search_active() {
      let bg_y = viewport.y + WARN_TOP - 2.0;
      let scale = fonts.scale as f64;
      let px = |v: f32| v as f64 * scale;
      Self::paint_column_bg(scene, px, viewport.x, bg_y, col_w, search_top, dark, focused);
      let row_w = col_w - SEARCH_PAD * 2.0 - 12.0;
      let mut rects = Vec::new();
      if self.search_hits.is_empty() {
        let (empty_w, empty_h) = self.search_empty.measure(fonts);
        self.search_empty.place(
          fonts,
          viewport.x + SEARCH_PAD + 6.0,
          viewport.y + WARN_TOP,
          empty_w,
          empty_h,
        );
        self.search_empty.draw(scene, fonts, images);
      }
      for (pos, row) in self.search_rows.iter_mut().enumerate() {
        let ry = viewport.y + WARN_TOP + pos as f32 * (WARN_ROW_H + WARN_GAP);
        if pos == self.selected_search {
          self.warn_bg.place(
            fonts,
            viewport.x + SEARCH_PAD,
            ry,
            (col_w - SEARCH_PAD * 2.0).max(0.0),
            WARN_ROW_H,
          );
          self.warn_bg.draw(scene, fonts, images);
        }
        row.place(fonts, viewport.x + SEARCH_PAD + 6.0, ry, row_w.max(0.0), WARN_ROW_H);
        row.draw(scene, fonts, images);
        rects.push((viewport.x + SEARCH_PAD, ry, (col_w - SEARCH_PAD * 2.0).max(0.0), WARN_ROW_H));
      }
      self.search_rects = rects;
    } else if self.nav_tab.get() == 0 {
      let scale = fonts.scale as f64;
      let px = |v: f32| v as f64 * scale;
      let bg_y = viewport.y + WARN_TOP - 2.0;
      Self::paint_column_bg(scene, px, viewport.x, bg_y, col_w, search_top, dark, focused);
      let tree_y = viewport.y + WARN_TOP;
      let tree_h = (search_top - tree_y).max(0.0);
      self.nav_tree.place(fonts, viewport.x, tree_y, col_w, tree_h);
      let clip = Rect::new(
        px(viewport.x),
        px(tree_y),
        px(viewport.x + col_w),
        px(tree_y + tree_h),
      );
      scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &clip);
      self.nav_tree.draw(scene, fonts, images);
      scene.pop_layer();
      self.tree_rect = (viewport.x, tree_y, col_w, tree_h);
    }
    // Warnings tab: sidebar background over the file rows plus the
    // issue rows with an accent wash behind the jumped-to row. A
    // query filters the list (title or file and line). The content
    // keeps showing the selected file.
    if self.nav_tab.get() == 1 {
      let bg_y = viewport.y + WARN_TOP - 2.0;
      let scale = fonts.scale as f64;
      let px = |v: f32| v as f64 * scale;
      Self::paint_column_bg(scene, px, viewport.x, bg_y, col_w, search_top, dark, focused);
      let row_w = col_w - SEARCH_PAD * 2.0 - 12.0;
      let visible = self.visible_warn_indices();
      let mut rects = Vec::new();
      for (pos, &real) in visible.iter().enumerate() {
        let ry = viewport.y + WARN_TOP + pos as f32 * (WARN_ROW_H + WARN_GAP);
        if real == self.selected_warn {
          self.warn_bg.place(
            fonts,
            viewport.x + SEARCH_PAD,
            ry,
            (col_w - SEARCH_PAD * 2.0).max(0.0),
            WARN_ROW_H,
          );
          self.warn_bg.draw(scene, fonts, images);
        }
        let row = &mut self.warn_rows[real];
        row.place(fonts, viewport.x + SEARCH_PAD + 6.0, ry, row_w.max(0.0), WARN_ROW_H);
        row.draw(scene, fonts, images);
        rects.push((viewport.x + SEARCH_PAD, ry, (col_w - SEARCH_PAD * 2.0).max(0.0), WARN_ROW_H));
      }
      self.warn_rects = rects;
    }
    // Topbar row in the content toolbar zone: the pill centers on
    // the content middle and wraps a compact group (glyph plus gap
    // plus button text plus gap plus chevron) with equal padding,
    // so icon, text and chevron sit centered inside. The button
    // width derives from the button text only, not the widest
    // dropdown row, and the menu stays vertically centered in the
    // pill. Two round chevron pills step through the visit history
    // at the content left (greyed out at the history ends).
    self.device.set_viewport(viewport.x, viewport.y, viewport.width, viewport.height);
    let (_, menu_h) = self.device.measure(fonts);
    let label = self.device.button_text().to_string();
    let (tw, _) = FontSystem::layout_size(&fonts.layout_text(&label, DEVICE_FONT, Color::WHITE, None));
    let text_w = tw / fonts.scale;
    let middle = content_x + content_w / 2.0;
    // Compact button width: text plus padding plus chevron.
    // NestedMenu draws button text at `menu_x + MENU_BTN_PAD_X`
    // and the chevron at the right padding edge.
    let menu_w = text_w + MENU_BTN_PAD_X * 2.0 + MENU_CHEV_GAP + MENU_CHEV_W;
    let group_w = DEVICE_ICON + DEVICE_GAP + menu_w;
    let icon_x = middle - group_w / 2.0;
    let menu_x = icon_x + DEVICE_ICON + DEVICE_GAP;
    // Glass body symmetric around the group, 36px tall like the
    // other pills.
    let glass_h = 36.0;
    let glass_y = viewport.y + 14.0;
    let menu_y = glass_y + ((glass_h - menu_h) / 2.0).max(0.0);
    let icon_y = glass_y + ((glass_h - DEVICE_ICON) / 2.0).max(0.0);
    self.computer.place(fonts, icon_x, icon_y, DEVICE_ICON, DEVICE_ICON);
    self.computer.draw(scene, fonts, images);
    self.menu_glass.place(
      fonts,
      middle - group_w / 2.0 - GLASS_PAD,
      glass_y,
      group_w + GLASS_PAD * 2.0,
      glass_h,
    );
    self.menu_glass.draw(scene, fonts, images);
    self.device.place(fonts, menu_x, menu_y, menu_w, menu_h);
    self.device.draw(scene, fonts, images);
    let (back_w, _) = self.chev_back.measure(fonts);
    self.chev_back.place(fonts, content_x + SEARCH_PAD, viewport.y + 14.0, back_w, 36.0);
    self.chev_back.draw(scene, fonts, images);
    let (fwd_w, _) = self.chev_fwd.measure(fonts);
    self.chev_fwd.place(
      fonts,
      content_x + SEARCH_PAD + back_w + 8.0,
      viewport.y + 14.0,
      fwd_w,
      36.0,
    );
    self.chev_fwd.draw(scene, fonts, images);
    // Check status pill left of the performance pill: a glass body
    // with the `check.indexing` label, only while a check runs.
    let (insp_w, _) = self.inspector.measure(fonts);
    let insp_x = content_x + content_w - SEARCH_PAD - insp_w;
    if self.check_job.is_some() {
      let (text_w, text_h) = self.check_text.measure(fonts);
      let pad = 14.0;
      let pill_w = text_w + pad * 2.0;
      let pill_x = (insp_x - 8.0 - pill_w).max(content_x + SEARCH_PAD);
      self.check_glass.place(fonts, pill_x, viewport.y + 14.0, pill_w, 36.0);
      self.check_glass.draw(scene, fonts, images);
      self.check_text.place(
        fonts,
        pill_x + pad,
        viewport.y + 14.0 + ((36.0 - text_h) / 2.0).max(0.0),
        text_w,
        text_h,
      );
      self.check_text.draw(scene, fonts, images);
    }
    self.inspector.place(fonts, insp_x, viewport.y + 14.0, insp_w, 36.0);
    self.inspector.draw(scene, fonts, images);
    // Divider between the topbar pills above and the editor below.
    self.top_div.place(fonts, content_x + SEARCH_PAD, viewport.y + 54.0, content_w - SEARCH_PAD * 2.0, 1.0);
    self.top_div.draw(scene, fonts, images);
    let col_w = self.sidebar.width_value();
    // Dead Run/Stop pair at the sidebar top right edge, vertically
    // centered on the traffic lights row like the old pills.
    let (pill_w, _) = self.run_stop.measure(fonts);
    self.run_stop.place(
      fonts,
      viewport.x + col_w - SEARCH_PAD - pill_w,
      viewport.y + 12.5,
      pill_w,
      36.0,
    );
    self.run_stop.draw(scene, fonts, images);
    // Functionless search capsule pinned to the sidebar bottom:
    // small height, full column width even while resizing.
    self.search.place(
      fonts,
      viewport.x + SEARCH_PAD,
      viewport.y + viewport.height - SEARCH_PAD - SEARCH_H,
      (col_w - SEARCH_PAD * 2.0).max(0.0),
      SEARCH_H,
    );
    self.search.draw(scene, fonts, images);
  }

  pub fn background(&self) -> Color {
    self.bg
  }

  pub fn wants_backdrop(&self) -> bool {
    // Frosted menu popup needs the blur pass while open.
    self.sidebar.wants_backdrop() || self.device.is_open()
  }

  pub fn drag_rect(&self) -> (f32, f32, f32, f32) {
    self.sidebar.drag_rect()
  }

  pub fn press_traffic(&mut self, x: f64, y: f64) -> bool {
    match self.sidebar.press(x, y) {
      Some(TrafficAction::Close) => self.command = Some(WindowCommand::Close),
      Some(TrafficAction::Minimize) => self.command = Some(WindowCommand::Minimize),
      Some(TrafficAction::Maximize) => self.command = Some(WindowCommand::ToggleMaximize),
      None => return false,
    }
    true
  }

  pub fn poll_command(&mut self) -> Option<WindowCommand> {
    let command = self.command.take()?;
    if matches!(command, WindowCommand::Close) {
      self.save_all_now();
    }
    Some(command)
  }

  /// Hover only: traffic light glyphs, row highlights, pill tints
  /// and the menu react.
  pub fn hover(&mut self, x: f32, y: f32) {
    self.sidebar.set_hover(x, y);
    self.run_stop.mouse_move(x, y);
    self.device.mouse_move(x as f64, y as f64);
    self.chev_back.mouse_move(x, y);
    self.chev_fwd.mouse_move(x, y);
    self.inspector.mouse_move(x, y);
    self.nav_tabs.mouse_move(x, y);
    self.refresh_code_ibeam();
  }

  fn hit_warn(&self, x: f32, y: f32) -> Option<usize> {
    if self.nav_tab.get() != 1 {
      return None;
    }
    // Filtered position back to the real warning index.
    self
      .warn_rects
      .iter()
      .position(|r| x >= r.0 && x <= r.0 + r.2 && y >= r.1 && y <= r.1 + r.3)
      .and_then(|pos| self.visible_warn_indices().get(pos).copied())
  }

  fn hit_search(&self, x: f32, y: f32) -> Option<usize> {
    if self.nav_tab.get() != 0 || !self.search_active() {
      return None;
    }
    self.search_rects.iter().position(|r| {
      x >= r.0 && x <= r.0 + r.2 && y >= r.1 && y <= r.1 + r.3
    })
  }

  pub fn mouse_down(&mut self, x: f64, y: f64) {
    // Native element feel (press states, selection, resize): clicks
    // trigger no actions, there are no callbacks anywhere. The search
    // capsule filters and finds across files (see `mouse_up` jumps).
    // The inspector pill only toggles the bottom panel. A warning or
    // search press jumps to its code at once so the content never
    // flashes the wrong file. Tree presses skip the flat rows (the
    // poll lands the page); the resize edge keeps the sidebar. While
    // search results cover the tree, the tree ignores presses.
    let tree_hit = self.nav_tab.get() == 0
      && !self.search_active()
      && point_in_rect(self.tree_rect, x as f32, y as f32);
    let resize = self.sidebar.wants_resize_cursor(x, y);
    if !tree_hit || resize {
      self.sidebar.mouse_down(x, y);
    }
    self.search.mouse_down(x, y);
    self.run_stop.mouse_down(x, y);
    self.device.mouse_down(x, y);
    self.chev_back.mouse_down(x, y);
    self.chev_fwd.mouse_down(x, y);
    self.inspector.mouse_down(x, y);
    self.nav_tabs.mouse_down(x, y);
    if tree_hit && !resize {
      self.nav_tree.mouse_down(x, y);
    }
    if let Some(pos) = self.hit_search(x as f32, y as f32) {
      self.jump_to_hit(pos);
    } else if let Some(index) = self.hit_warn(x as f32, y as f32) {
      self.jump_to_issue(index);
    }
  }

  pub fn mouse_up(&mut self, x: f64, y: f64) {
    self.sidebar.mouse_up(x, y);
    // No `on_action` on the pill pairs: the press tint releases into
    // nothing, by design. The device menu keeps its example selection.
    self.run_stop.mouse_up(x, y);
    self.device.mouse_up(x, y);
    self.chev_back.mouse_up(x, y);
    self.chev_fwd.mouse_up(x, y);
    self.inspector.mouse_up(x, y);
    self.nav_tabs.mouse_up(x, y);
    self.nav_tree.mouse_up(x, y);
    // Tap jumps land on release (rects refresh every frame).
    self.recompute_search();
    if let Some(pos) = self.hit_search(x as f32, y as f32) {
      self.jump_to_hit(pos);
    } else if let Some(index) = self.hit_warn(x as f32, y as f32) {
      self.jump_to_issue(index);
    }
  }

  pub fn mouse_wheel(&mut self, dx: f64, dy: f64) {
    self.sidebar.mouse_wheel(dx, dy);
  }

  pub fn type_text(&mut self, content: &str) {
    // Search first while it holds focus, else the active code page.
    // Bound files auto-save 400ms after typing stops. Search typing
    // never arms the background check.
    self.search.type_text(content);
    self.sidebar.page_text(content);
    if !self.search.is_selected() {
      self.note_change_if_editable();
    }
  }

  pub fn key(&mut self, key: Key) -> bool {
    // Search shortcuts while the capsule holds focus: Enter jumps to
    // the first visible hit, ESC clears the query (a second ESC
    // leaves the field as usual).
    if self.search.is_selected() {
      match key {
        Key::Enter => {
          if self.search_active() {
            self.recompute_search();
            self.jump_to_first_search_hit();
            return true;
          }
        }
        Key::Escape => {
          if !self.search.text_value().is_empty() {
            self.search.set_text(String::new());
            return true;
          }
        }
        _ => {}
      }
    }
    if self.search.key(key) {
      return true;
    }
    let consumed = self.sidebar.page_key(key);
    if consumed {
      match key {
        Key::Backspace | Key::Enter | Key::Cut | Key::Paste | Key::Undo | Key::Redo => {
          self.note_change_if_editable();
        }
        _ => {}
      }
    }
    consumed
  }

  pub fn wants_text_cursor(&self) -> bool {
    self.search.wants_text_cursor() || self.code_ibeam.get()
  }

  pub fn wants_resize(&self, x: f64, y: f64) -> bool {
    self.sidebar.wants_resize_cursor(x, y) || self.divider_cursor.get()
  }

  pub fn set_focused(&mut self, focused: bool) {
    self.focused = focused;
    self.sidebar.set_focused(focused);
    self.search.set_focused(focused);
    self.top_div.set_focused(focused);
    self.run_stop.set_focused(focused);
    self.device.set_focused(focused);
    self.menu_glass.set_focused(focused);
    self.chev_back.set_focused(focused);
    self.chev_fwd.set_focused(focused);
    self.inspector.set_focused(focused);
    self.check_glass.set_focused(focused);
    self.check_text.set_focused(focused);
    self.nav_tabs.set_focused(focused);
    self.nav_tree.set_focused(focused);
    self.warn_bg.set_focused(focused);
    for row in self.warn_rows.iter_mut() {
      row.set_focused(focused);
    }
    for row in self.search_rows.iter_mut() {
      row.set_focused(focused);
    }
    self.search_empty.set_focused(focused);
  }

  /// Bottom panel master switch flipped by the inspector pill.
  pub fn panel_open(&self) -> bool {
    self.panel_open.get()
  }
}

struct EditorApp {
  ui: EditorUi,
}

impl EditorApp {
  fn new(project: String, path: Option<String>) -> Self {
    let user = system_username();
    let file = file_stem(&project);
    let code = example_code(&file, &project, &user);
    // Real project rows when the handoff carries a valid dir,
    // built-in example rows otherwise.
    let ui = match path.filter(|path| Path::new(path).is_dir()) {
      Some(path) => EditorUi::open(&project, Path::new(&path))
        .unwrap_or_else(|_| EditorUi::new(&project, code)),
      None => EditorUi::new(&project, code),
    };
    Self { ui }
  }
}

impl App for EditorApp {
  fn draw(
    &mut self,
    scene: &mut Scene,
    fonts: &mut FontSystem,
    images: &mut ImageLoader<'_>,
    viewport: Viewport,
    time_secs: f64,
  ) {
    self.ui.draw(scene, fonts, images, viewport, time_secs);
  }

  fn background(&self) -> Color {
    self.ui.background()
  }

  fn wants_backdrop(&self) -> bool {
    self.ui.wants_backdrop()
  }

  fn drag_region(&self) -> Option<(f32, f32, f32, f32)> {
    Some(self.ui.drag_rect())
  }

  fn poll_window_command(&mut self) -> Option<WindowCommand> {
    self.ui.poll_command()
  }

  fn mouse_down(&mut self, x: f64, y: f64) {
    // Only the traffic lights act; anything else is element feel
    // without actions (no callbacks registered anywhere).
    if !self.ui.press_traffic(x, y) {
      self.ui.mouse_down(x, y);
    }
  }

  fn mouse_up(&mut self, x: f64, y: f64) {
    self.ui.mouse_up(x, y);
  }

  fn mouse_move(&mut self, x: f64, y: f64) {
    self.ui.hover(x as f32, y as f32);
  }

  fn mouse_wheel(&mut self, dx: f64, dy: f64) {
    self.ui.mouse_wheel(dx, dy);
  }

  fn cursor(&self, x: f64, y: f64) -> CursorKind {
    // Grab handles win over the I-beam (narrow zones by design).
    if self.ui.wants_resize(x, y) {
      CursorKind::ResizeColumn
    } else if self.ui.wants_text_cursor() {
      CursorKind::Text
    } else {
      CursorKind::Default
    }
  }

  fn text(&mut self, content: &str) {
    self.ui.type_text(content);
  }

  fn key(&mut self, key: Key) {
    let _ = self.ui.key(key);
  }

  fn set_focused(&mut self, focused: bool) {
    self.ui.set_focused(focused);
  }
}

/// Open the big editor window for a project: waits out the gap after
/// the starter window closed, then runs fresh (no CLI involved).
pub fn open_project_window(project: String, path: Option<String>) {
  std::thread::sleep(std::time::Duration::from_millis(OPEN_DELAY_MS));
  let title = project.clone();
  let app = EditorApp::new(project, path);
  if let Err(err) = run(&title, EDITOR_W, EDITOR_H, app) {
    eprintln!("error: {err}");
    std::process::exit(1);
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn file_stem_appends_app_once() {
    assert_eq!(file_stem("test"), "testApp");
    assert_eq!(file_stem("MyApp"), "MyApp");
    assert_eq!(file_stem("My App"), "MyApp");
    assert_eq!(file_stem("  !!!  "), "MyApp");
  }

  #[test]
  fn example_code_is_40_line_rust() {
    let code = example_code("testApp", "test", "arlo");
    assert_eq!(code.lines().count(), 40);
    assert!(code.contains("testApp"));
    assert!(code.contains("Created by arlo"));
    assert!(code.contains("fn main"));
    assert!(!code.contains("import SwiftUI"));
  }

  #[test]
  fn highlight_kinds_resolve_colors() {
    use crate::DocumentKit::SpanKind;
    // Every kind has a distinct dark color (keyword pink, string red).
    let kw = SpanKind::Keyword.color(true);
    let st = SpanKind::Str.color(true);
    assert_ne!(kw, st);
    // Light variants differ from dark ones.
    assert_ne!(kw, SpanKind::Keyword.color(false));
  }

  #[test]
  fn project_env_key_is_stable() {
    // The starter and the editor child agree on these keys instead
    // of CLI arguments.
    assert_eq!(PROJECT_ENV, "XCODE_PROJECT_NAME");
    assert_eq!(PROJECT_PATH_ENV, "XCODE_PROJECT_PATH");
  }

  #[test]
  fn computer_glyph_resolves_in_project() {
    // `cargo test` runs with the package root as cwd, so the bundled
    // `Resources/computer.png` must resolve (no placeholder fallback).
    let path = computer_icon_path();
    assert!(path.is_file(), "missing {}", path.display());
    assert_eq!(
      path.file_name().and_then(|s| s.to_str()),
      Some("computer.png")
    );
  }

  #[test]
  fn performance_pill_toggles_bottom_panel() {
    // Fresh editors keep the bottom panel folded away on every page;
    // the performance pill opens it and folds it back.
    let code = example_code("testApp", "test", "arlo");
    let mut ui = EditorUi::new("test", code);
    assert!(!ui.panel_open());
    ui.toggle_panel();
    assert!(ui.panel_open());
    for index in 0..ui.page_total {
      let page = ui.sidebar.page_mut(index).expect("page");
      let ed = page
        .as_any_mut()
        .downcast_mut::<CodeEditor>()
        .expect("code page");
      assert!(!ed.panel_collapsed());
    }
    ui.toggle_panel();
    assert!(!ui.panel_open());
    for index in 0..ui.page_total {
      let page = ui.sidebar.page_mut(index).expect("page");
      let ed = page
        .as_any_mut()
        .downcast_mut::<CodeEditor>()
        .expect("code page");
      assert!(ed.panel_collapsed());
    }
  }

  #[test]
  fn history_records_external_selections() {
    let code = example_code("testApp", "test", "arlo");
    let mut ui = EditorUi::new("test", code);
    assert_eq!(ui.history_top, FILE_INDEX);
    assert!(!ui.can_go_back());
    assert!(!ui.can_go_forward());
    ui.sidebar.select(2);
    ui.record_selection();
    assert!(ui.can_go_back());
    assert!(!ui.can_go_forward());
    assert_eq!(ui.history_top, 2);
    ui.sidebar.select(3);
    ui.record_selection();
    assert_eq!(ui.history_top, 3);
    // Same selection never pushes twice.
    ui.record_selection();
    assert_eq!(ui.back_stack, vec![FILE_INDEX, 2]);
  }

  #[test]
  fn go_back_forward_navigates_history() {
    let code = example_code("testApp", "test", "arlo");
    let mut ui = EditorUi::new("test", code);
    ui.sidebar.select(1);
    ui.record_selection();
    ui.sidebar.select(2);
    ui.record_selection();
    ui.go_back();
    assert_eq!(ui.sidebar.selected_index(), 1);
    assert!(ui.can_go_back());
    assert!(ui.can_go_forward());
    ui.go_back();
    assert_eq!(ui.sidebar.selected_index(), FILE_INDEX);
    assert!(!ui.can_go_back());
    assert!(ui.can_go_forward());
    // Empty back stack is a no-op.
    ui.go_back();
    assert_eq!(ui.sidebar.selected_index(), FILE_INDEX);
    ui.go_forward();
    assert_eq!(ui.sidebar.selected_index(), 1);
    ui.go_forward();
    assert_eq!(ui.sidebar.selected_index(), 2);
    assert!(!ui.can_go_forward());
    // A new external selection drops the forward trail.
    ui.go_back();
    ui.sidebar.select(0);
    ui.record_selection();
    assert!(!ui.can_go_forward());
    assert_eq!(ui.history_top, 0);
  }

  #[test]
  fn chevron_request_navigates_on_poll() {
    let code = example_code("testApp", "test", "arlo");
    let mut ui = EditorUi::new("test", code);
    ui.sidebar.select(1);
    ui.record_selection();
    ui.nav_request.set(Some(NavDir::Back));
    ui.poll_nav_request();
    assert_eq!(ui.sidebar.selected_index(), FILE_INDEX);
    assert!(ui.nav_request.take().is_none());
    ui.nav_request.set(Some(NavDir::Fwd));
    ui.poll_nav_request();
    assert_eq!(ui.sidebar.selected_index(), 1);
  }

  #[test]
  fn file_icon_maps_types_with_tints() {
    assert_eq!(file_icon("  app.rs").0, "rust");
    assert!(file_icon("app.rs").1.is_some());
    assert_eq!(file_icon("tontoo.proj").0, "rotate.3d");
    assert_eq!(file_icon("Cargo.toml").0, "gear");
    assert_eq!(file_icon("en_us.json").0, "tray.2.fill");
    assert_eq!(file_icon("README").0, "doc.fill");
    assert_eq!(file_icon("README").1, None);
    // Case-insensitive, indent-tolerant.
    assert_eq!(file_icon("  MAIN.RS").0, "rust");
  }

  #[test]
  fn open_lists_real_files_without_target() {
    let parent = std::env::temp_dir()
      .join(format!("xcode-open-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&parent);
    std::fs::create_dir_all(&parent).unwrap();
    let root = crate::scaffold::create_project(
      &parent,
      "My App",
      "",
      "1.0",
      "de.x",
    )
    .expect("scaffold");
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("target").join("x"), "x").unwrap();
    let mut ui = EditorUi::open("My App", &root).expect("open");
    // Real rows (more than the 5 example ones), no target row, and
    // the first `.rs` row preselected with the example warnings.
    assert!(ui.page_total > 5);
    for index in 0..ui.page_total {
      let page = ui.sidebar.page_mut(index).expect("page");
      assert!(page.as_any_mut().downcast_mut::<CodeEditor>().is_some());
    }
    let entries = crate::project_files::list_project_files(&root);
    assert_eq!(entries.len(), ui.page_total);
    assert!(!entries.iter().any(|entry| entry.label.contains("target")));
    let first_rs = entries
      .iter()
      .position(|entry| !entry.is_dir && entry.label.trim().ends_with(".rs"))
      .expect("rs file");
    assert_eq!(ui.sidebar.selected_index(), first_rs);
    // Warnings attach to real rows only.
    for item in &ui.warn_items {
      assert!(item.file < ui.page_total);
    }
    let _ = std::fs::remove_dir_all(&parent);
  }

  #[test]
  fn open_binds_real_file_contents() {
    let parent = std::env::temp_dir()
      .join(format!("xcode-bind-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&parent);
    std::fs::create_dir_all(&parent).unwrap();
    let root = crate::scaffold::create_project(
      &parent,
      "My App",
      "",
      "1.0",
      "de.x",
    )
    .expect("scaffold");
    let entries = crate::project_files::list_project_files(&root);
    let mut ui = EditorUi::open("My App", &root).expect("open");
    for (index, entry) in entries.iter().enumerate() {
      let page = ui.sidebar.page_mut(index).expect("page");
      let ed = page
        .as_any_mut()
        .downcast_mut::<CodeEditor>()
        .expect("code page");
      if entry.is_dir {
        assert!(ed.is_read_only());
        assert!(ed.file_path().is_none());
      } else {
        let disk = crate::project_files::load_file_text(&entry.path);
        match disk {
          Some(text) => {
            assert!(!ed.is_read_only());
            assert_eq!(ed.file_path(), Some(entry.path.clone()));
            assert_eq!(ed.text_value(), text);
          }
          None => {
            assert!(ed.is_read_only());
            assert_eq!(ed.text_value(), crate::lang::t("ed.binary"));
          }
        }
      }
    }
    let _ = std::fs::remove_dir_all(&parent);
  }

  #[test]
  fn save_all_writes_dirty_pages() {
    let parent = std::env::temp_dir()
      .join(format!("xcode-saveall-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&parent);
    std::fs::create_dir_all(&parent).unwrap();
    let root = crate::scaffold::create_project(
      &parent,
      "My App",
      "",
      "1.0",
      "de.x",
    )
    .expect("scaffold");
    let mut ui = EditorUi::open("My App", &root).expect("open");
    let sel = ui.sidebar.selected_index();
    let path = {
      let page = ui.sidebar.page_mut(sel).expect("page");
      let ed = page
        .as_any_mut()
        .downcast_mut::<CodeEditor>()
        .expect("code page");
      assert!(!ed.is_read_only());
      ed.goto_line(1);
      ed.type_text("// saved");
      assert!(ed.is_dirty());
      ed.file_path().expect("bound file")
    };
    ui.save_all_now();
    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.contains("// saved"));
    let _ = std::fs::remove_dir_all(&parent);
  }

  #[test]
  fn open_rejects_missing_dir() {
    let missing = std::path::Path::new("/definitely/not/here-xcode");
    assert!(EditorUi::open("x", missing).is_err());
  }

  #[test]
  fn search_finds_every_occurrence_across_files() {
    let (parent, root) = scaffold_root("search");
    let mut ui = EditorUi::open("My App", &root).expect("open");
    ui.search.set_text("tontooui");
    ui.recompute_search();
    assert!(!ui.search_hits.is_empty());
    assert_eq!(ui.search_rows.len(), ui.search_hits.len());
    // Spans at least two files, every hit carries file, line and snippet.
    let mut files: Vec<usize> = ui.search_hits.iter().map(|hit| hit.file).collect();
    files.sort();
    files.dedup();
    assert!(files.len() >= 2);
    for hit in &ui.search_hits {
      assert!(hit.line >= 1);
      assert!(!hit.snippet.is_empty());
      assert!(hit.snippet.to_lowercase().contains("tontooui"));
    }
    // Case-insensitive: upper case finds the same hits.
    ui.search.set_text("TontooUI");
    ui.recompute_search();
    let upper = ui.search_hits.len();
    ui.search.set_text("tontooui");
    ui.recompute_search();
    assert_eq!(ui.search_hits.len(), upper);
    // Click-jump lands on the hit file and line.
    let first = ui.search_hits[0].clone();
    ui.jump_to_hit(0);
    assert_eq!(ui.sidebar.selected_index(), first.file);
    assert_eq!(ui.selected_search, 0);
    let page = ui.sidebar.page_mut(first.file).expect("page");
    let ed = page
      .as_any_mut()
      .downcast_mut::<CodeEditor>()
      .expect("code page");
    let (line, _) = ed.line_col();
    assert_eq!(line, first.line - 1);
    let _ = std::fs::remove_dir_all(&parent);
  }

  #[test]
  fn search_skips_read_only_pages() {
    let (parent, root) = scaffold_root("skipbin");
    std::fs::write(root.join("blob.bin"), b"hello\x00world").unwrap();
    let entries = crate::project_files::list_project_files(&root);
    let blob = entries
      .iter()
      .position(|entry| entry.label.trim() == "blob.bin")
      .expect("blob row");
    let mut ui = EditorUi::open("My App", &root).expect("open");
    ui.search.set_text("hello");
    ui.recompute_search();
    assert!(!ui.search_hits.is_empty());
    assert!(ui.search_hits.iter().all(|hit| hit.file != blob));
    let _ = std::fs::remove_dir_all(&parent);
  }

  #[test]
  fn warnings_filter_by_query() {
    let code = example_code("testApp", "test", "arlo");
    let mut ui = EditorUi::new("test", code);
    assert_eq!(ui.visible_warn_indices(), vec![0, 1, 2, 3, 4]);
    ui.search.set_text("unused");
    assert_eq!(ui.visible_warn_indices(), vec![0]);
    ui.search.set_text("MISMATCH");
    assert_eq!(ui.visible_warn_indices(), vec![1]);
    // File and line subtitles match too.
    ui.search.set_text("contentview");
    assert!(!ui.visible_warn_indices().is_empty());
    ui.search.set_text("zzz-no-such-warning");
    assert!(ui.visible_warn_indices().is_empty());
  }

  #[test]
  fn search_enter_jumps_and_escape_clears() {
    use crate::TontooUI::renderer::FontSystem;
    let (parent, root) = scaffold_root("keys");
    let mut ui = EditorUi::open("My App", &root).expect("open");
    let mut fonts = FontSystem::new();
    ui.search.place(&mut fonts, 0.0, 0.0, 200.0, 28.0);
    ui.search.mouse_down(10.0, 10.0);
    assert!(ui.search.is_selected());
    ui.search.set_text("hello");
    ui.recompute_search();
    assert!(!ui.search_hits.is_empty());
    let first = ui.search_hits[0].clone();
    assert!(ui.key(Key::Enter));
    assert_eq!(ui.sidebar.selected_index(), first.file);
    // ESC clears the query, a second ESC leaves the field as usual.
    assert!(ui.key(Key::Escape));
    assert_eq!(ui.search.text_value(), "");
    assert!(!ui.search_active());
    assert!(ui.key(Key::Escape));
    assert!(!ui.search.is_selected());
    // Search typing never arms the background check.
    assert!(ui.last_change.is_some());
    let armed = ui.last_change;
    ui.search.mouse_down(10.0, 10.0);
    ui.type_text("zzz");
    assert_eq!(ui.last_change, armed);
    let _ = std::fs::remove_dir_all(&parent);
  }

  fn scaffold_root(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let parent = std::env::temp_dir()
      .join(format!("xcode-check-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&parent);
    std::fs::create_dir_all(&parent).unwrap();
    let root = crate::scaffold::create_project(&parent, "My App", "", "1.0", "de.x")
      .expect("scaffold");
    (parent, root)
  }

  #[test]
  fn open_arms_initial_check() {
    let (parent, root) = scaffold_root("initial");
    let ui = EditorUi::open("My App", &root).expect("open");
    // Sentinel revision plus fresh timer: the first 2.5s idle window
    // fires one check even before the first keystroke.
    assert_eq!(ui.content_at_check, u64::MAX);
    assert!(ui.last_change.is_some());
    assert!(ui.check_job.is_none());
    assert!(ui.project_root.is_some());
    let _ = std::fs::remove_dir_all(&parent);
  }

  #[test]
  fn check_maps_diagnostics_to_rows_and_gutters() {
    use crate::check::{CheckDiagnostic, CheckResult};
    let (parent, root) = scaffold_root("map");
    let entries = crate::project_files::list_project_files(&root);
    let rs = entries
      .iter()
      .position(|entry| !entry.is_dir && entry.label.trim().ends_with(".rs"))
      .expect("rs file");
    let rel = entries[rs]
      .path
      .strip_prefix(&root)
      .expect("relative")
      .to_string_lossy()
      .replace('\\', "/");
    let mut ui = EditorUi::open("My App", &root).expect("open");
    let result = CheckResult {
      generation: 1,
      revision: 7,
      diagnostics: vec![
        CheckDiagnostic {
          file: rel,
          line: 3,
          col: 5,
          error: true,
          message: "mismatched types".to_string(),
          code: Some("E0308".to_string()),
        },
        CheckDiagnostic {
          file: "some/foreign/crate.rs".to_string(),
          line: 1,
          col: 1,
          error: true,
          message: "foreign".to_string(),
          code: None,
        },
      ],
    };
    ui.check_generation = 1;
    ui.apply_check_result(result);
    // Only the project file survives, with What plus file and line.
    assert_eq!(ui.warn_items.len(), 1);
    assert_eq!(ui.warn_items[0].file, rs);
    assert_eq!(ui.warn_items[0].line, 3);
    assert!(ui.warn_items[0].error);
    assert!(ui.warn_items[0].title.contains("mismatched types"));
    assert!(ui.warn_items[0].title.contains("E0308"));
    assert_eq!(ui.warn_rows.len(), 1);
    assert_eq!(ui.content_at_check, 7);
    // Gutter markers land on the right page and line.
    let page = ui.sidebar.page_mut(rs).expect("page");
    let ed = page
      .as_any_mut()
      .downcast_mut::<CodeEditor>()
      .expect("code page");
    assert_eq!(ed.marker_at(3), Some(true));
    assert_eq!(ed.marker_at(2), None);
    let _ = std::fs::remove_dir_all(&parent);
  }

  #[test]
  fn poll_check_stays_idle_without_changes() {
    let (parent, root) = scaffold_root("idle");
    let mut ui = EditorUi::open("My App", &root).expect("open");
    // Fresh revision matches the applied one: no spawn, no thread.
    ui.content_at_check = ui.content_revision();
    ui.poll_check();
    assert!(ui.check_job.is_none());
    let _ = std::fs::remove_dir_all(&parent);
  }

  #[test]
  fn poll_check_cancels_job_on_new_keystrokes() {
    let (parent, root) = scaffold_root("cancel");
    let mut ui = EditorUi::open("My App", &root).expect("open");
    // Fake running job without a thread (cancelled channel).
    let (_tx, rx) = std::sync::mpsc::channel();
    ui.check_job = Some(ActiveCheck {
      revision: ui.content_revision(),
      rx,
      cancel: Arc::new(AtomicBool::new(false)),
      child: Arc::new(Mutex::new(None)),
    });
    // Typing on the selected editable page moves the revision.
    let sel = ui.sidebar.selected_index();
    {
      let page = ui.sidebar.page_mut(sel).expect("page");
      let ed = page
        .as_any_mut()
        .downcast_mut::<CodeEditor>()
        .expect("code page");
      assert!(!ed.is_read_only());
      ed.goto_line(1);
    }
    ui.type_text("// late keystroke");
    ui.poll_check();
    // Cancelled and, still inside the idle window, not respawned.
    assert!(ui.check_job.is_none());
    let _ = std::fs::remove_dir_all(&parent);
  }

  #[test]
  fn diagnostics_reach_their_pages() {
    let code = example_code("testApp", "test", "arlo");
    let mut ui = EditorUi::new("test", code);
    ui.sync_diagnostics();
    // File 4 carries 4 markers (error on line 33), file 2 one.
    let page = ui.sidebar.page_mut(4).expect("page");
    let ed = page
      .as_any_mut()
      .downcast_mut::<CodeEditor>()
      .expect("code page");
    assert_eq!(ed.marker_at(33), Some(true));
    assert_eq!(ed.marker_at(36), Some(false));
    assert_eq!(ed.marker_at(1), None);
    let page = ui.sidebar.page_mut(2).expect("page");
    let ed = page
      .as_any_mut()
      .downcast_mut::<CodeEditor>()
      .expect("code page");
    assert_eq!(ed.marker_at(1), Some(true));
    let page = ui.sidebar.page_mut(0).expect("page");
    let ed = page
      .as_any_mut()
      .downcast_mut::<CodeEditor>()
      .expect("code page");
    assert_eq!(ed.marker_at(1), None);
  }

  #[test]
  fn navigator_starts_on_files_with_five_issues() {
    let code = example_code("testApp", "test", "arlo");
    let ui = EditorUi::new("test", code);
    assert_eq!(ui.nav_index(), 0);
    assert_eq!(ui.nav_tabs.selected_index(), 0);
    assert_eq!(ui.warn_items.len(), 5);
    assert_eq!(ui.warn_rows.len(), 5);
    // Compact rows, one size for folders and files.
    assert_eq!(ui.sidebar.row_metrics(), (28.0, 18.0, 8.0, 12.0));
    // 3 warnings plus 2 errors, all pointing at real rows.
    let errors = ui.warn_items.iter().filter(|item| item.error).count();
    assert_eq!(errors, 2);
  }

  #[test]
  fn warning_jump_selects_file_and_line() {
    let code = example_code("testApp", "test", "arlo");
    let mut ui = EditorUi::new("test", code);
    // Pretend the warnings tab is open, then jump to the first
    // issue (file 4, line 36). The tab stays, only the editor moves.
    ui.nav_tab.set(1);
    ui.jump_to_issue(0);
    assert_eq!(ui.nav_index(), 1);
    assert_eq!(ui.sidebar.selected_index(), 4);
    assert_eq!(ui.selected_warn, 0);
    let page = ui.sidebar.page_mut(4).expect("page");
    let ed = page
      .as_any_mut()
      .downcast_mut::<CodeEditor>()
      .expect("code page");
    let (line, col) = ed.line_col();
    assert_eq!((line, col), (35, 0));
    // Out of range jumps never touch the tab.
    ui.nav_tab.set(1);
    ui.jump_to_issue(99);
    assert_eq!(ui.nav_index(), 1);
  }

  #[test]
  fn tree_paths_mirror_flat_rows() {
    let code = example_code("testApp", "test", "arlo");
    let ui = EditorUi::new("test", code);
    // Example rows: root folder with 4 children.
    assert_eq!(ui.tree_paths.len(), ui.page_total);
    assert_eq!(ui.tree_paths[0], vec![0]);
    assert_eq!(ui.tree_paths[4], vec![0, 3]);
    assert!(ui.nav_tree.is_expanded(&[0]));
  }

  #[test]
  fn tree_selection_lands_on_flat_page() {
    let code = example_code("testApp", "test", "arlo");
    let mut ui = EditorUi::new("test", code);
    // Outline callback fires synchronously; the poll maps the path.
    ui.nav_tree.select(vec![0, 2]);
    ui.poll_tree_select();
    assert_eq!(ui.sidebar.selected_index(), 3);
    // Unknown paths never touch the selection.
    ui.nav_tree.select(vec![0, 2]);
    ui.tree_sel.borrow_mut().replace(vec![9, 9]);
    ui.poll_tree_select();
    assert_eq!(ui.sidebar.selected_index(), 3);
  }

  #[test]
  fn jump_expands_ancestor_folders() {
    let parent = std::env::temp_dir()
      .join(format!("xcode-ancestors-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&parent);
    std::fs::create_dir_all(&parent).unwrap();
    let root = crate::scaffold::create_project(
      &parent,
      "My App",
      "",
      "1.0",
      "de.x",
    )
    .expect("scaffold");
    let mut ui = EditorUi::open("My App", &root).expect("open");
    ui.nav_tree.collapse_all();
    ui.nav_tab.set(1);
    ui.jump_to_issue(0);
    // Ancestors of the jumped-to file stand open again, the tab stays.
    let flat = ui.sidebar.selected_index();
    let path = ui.tree_paths[flat].clone();
    assert!(path.len() > 1);
    for depth in 1..path.len() {
      assert!(ui.nav_tree.is_expanded(&path[..depth]));
    }
    assert_eq!(ui.nav_index(), 1);
    let _ = std::fs::remove_dir_all(&parent);
  }
}
