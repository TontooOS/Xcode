//! Example project editor window for Xcode (separate big window).
//!
//! A static Xcode-like IDE card built on the `Sidebar` element: working
//! traffic lights (owned by the sidebar), a dead Run/Stop pill pair at
//! the sidebar top right (no callbacks), a non-collapsible file
//! navigator with one preselected file, a functionless search capsule
//! stretched across the sidebar bottom and editable example Rust code.
//! Code pages accept clicks and typing like a normal text field;
//! edits stay in memory only, nothing is ever saved, built or run.
//!
//! Handoff without CLI: the starter sets `XCODE_PROJECT_NAME` on a
//! spawned copy of this binary and closes its own window at once;
//! `open_project_window` waits ~600ms first (old window visibly
//! closes, short gap), then opens the 1100x700 editor fresh.
use std::cell::Cell;
use std::rc::Rc;

use crate::code_editor::{CodeEditor, example_rust_code};
use crate::TontooUI::elements::{
  Align, BasicText, BasicToolbar, FileImage, HorizontalDivider, HStack,
  MenuItem, NestedMenu, RoundedRectangle, SearchField, SFSymbolImage,
  SegmentedPicker, ShapeFill, Sidebar, SidebarItem,
  TextForeground, TextStyle, ToolbarItem, TrafficAction, View, VStack,
  GROUP_BG_LIGHT, MENU_BTN_PAD_X, MENU_CHEV_GAP, MENU_CHEV_W,
  SIDEBAR_BG_DARK,
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
/// Navigator tab switcher geometry: 24px picker in the blank gap
/// between the 64px toolbar zone and the file rows at 108px.
const NAV_TABS_Y: f32 = 70.0;
const NAV_TABS_H: f32 = 24.0;
/// Warnings overlay: rows start below the picker, one 52px row per
/// issue with a 6px gap.
const WARN_TOP: f32 = 100.0;
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
  /// Dead back/forward chevrons at the content left.
  chev: BasicToolbar,
  /// Performance pill at the content right
  /// (`chart.line.uptrend.xyaxis`, round circle): toggles the bottom
  /// panel open or closed on every page.
  inspector: BasicToolbar,
  /// Bottom panel master switch, flipped by the inspector pill.
  panel_open: Rc<Cell<bool>>,
  /// Last applied switch state (divider taps stay per page).
  last_panel_open: Cell<bool>,
  /// Navigator tab switcher (`Files` default, `Warnings & Errors`).
  nav_tabs: SegmentedPicker,
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
    let file = file_stem(project);
    let items = vec![
      SidebarItem::new(project, "folder.fill"),
      SidebarItem::new("Assets", "folder.fill"),
      SidebarItem::new("ContentView", "doc.fill"),
      SidebarItem::new("Info", "doc.fill"),
      SidebarItem::new(file, "doc.fill"),
    ];
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
    let code_pages = [
      CodeEditor::new(code.clone()),
      CodeEditor::new(code.clone()),
      CodeEditor::new(code.clone()),
      CodeEditor::new(code.clone()),
      CodeEditor::new(code),
    ];
    let mut sidebar = Sidebar::new(items);
    for page in code_pages {
      sidebar = sidebar.page(page);
    }
    sidebar = sidebar
      .search_field(false)
      .toggle_button(false)
      .collapsible(false);
    // No content title (neither scheme nor item label): the topbar
    // holds chevrons left, the device menu center and the collapse
    // pill right instead.
    sidebar.set_title(String::new());
    sidebar.select(FILE_INDEX);
    // The element defaults item labels to hand-set white, which wins
    // over the theme in light mode: clear the override once so labels
    // follow `set_theme` (dark `#d8d9d9`, light `#272727`).
    sidebar.set_item_text(None);
    // Bottom panel master switch: the performance pill flips it,
    // `draw` applies edges to every page (divider taps stay per
    // page, see `apply_panel_open`).
    let panel_open = Rc::new(Cell::new(true));
    let panel_toggle = panel_open.clone();
    // Navigator tabs: `Files` default plus `Warnings & Errors`. The
    // switcher overlays the blank gap above the file rows; the
    // warnings tab covers the rows with the example issue list.
    let nav_tab = Rc::new(Cell::new(0));
    let tab_flip = nav_tab.clone();
    let nav_tabs = SegmentedPicker::from_slice(
      "",
      &[&lang::t("nav.files"), &lang::t("nav.warnings")],
    )
    .selected(0)
    .on_select(move |index| tab_flip.set(index));
    let warn_files = vec![
      project.to_string(),
      "Assets".to_string(),
      "ContentView".to_string(),
      "Info".to_string(),
      file_stem(project),
    ];
    let warn_items = vec![
      WarnItem { error: false, title: lang::t("issue.unused_var"), file: 4, line: 36 },
      WarnItem { error: true, title: lang::t("issue.type_mismatch"), file: 4, line: 33 },
      WarnItem { error: false, title: lang::t("issue.trailing_ws"), file: 4, line: 9 },
      WarnItem { error: true, title: lang::t("issue.unresolved_import"), file: 2, line: 1 },
      WarnItem { error: false, title: lang::t("issue.missing_docs"), file: 4, line: 22 },
    ];
    let warn_rows = warn_items
      .iter()
      .map(|item| warn_row(item.icon(), &item.title, &item.subtitle(&warn_files)))
      .collect();
    let inspector =
      BasicToolbar::from_items(vec![ToolbarItem::icon("chart.line.uptrend.xyaxis")])
        .round(true)
        .on_action(move |_| panel_toggle.set(!panel_toggle.get()));
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
      chev: BasicToolbar::from_items(vec![
        ToolbarItem::icon("chevron.left"),
        ToolbarItem::divider(),
        ToolbarItem::icon("chevron.right"),
      ]),
      inspector,
      panel_open,
      last_panel_open: Cell::new(true),
      nav_tabs,
      nav_tab,
      warn_items,
      warn_files,
      warn_rows,
      warn_bg: RoundedRectangle::new(200.0, WARN_ROW_H, 8.0)
        .fill(Color::from_rgb8(0x00, 0x7a, 0xff)),
      selected_warn: 0,
      warn_rects: Vec::new(),
      code_ibeam: Cell::new(false),
      divider_cursor: Cell::new(false),
      watcher: ThemeWatcher::new(),
      focused: true,
      bg: crate::TontooUI::renderer::window::BACKGROUND,
      command: None,
    }
  }

  fn theme_code_pages(&mut self, accent: Color, mode: ThemeMode, dark: bool, focused: bool) {
    for index in 0..5 {
      if let Some(page) = self.sidebar.page_mut(index) {
        if let Some(ed) = page.as_any_mut().downcast_mut::<CodeEditor>() {
          ed.set_theme(accent, mode, dark);
          ed.set_focused(focused);
        }
      }
    }
  }

  /// Advance fake stats and logs on every page with the frame time
  /// (one sample plus one log line per second, example only).
  fn tick_code_pages(&mut self, now_secs: f64) {
    for index in 0..5 {
      if let Some(page) = self.sidebar.page_mut(index) {
        if let Some(ed) = page.as_any_mut().downcast_mut::<CodeEditor>() {
          ed.tick(now_secs);
        }
      }
    }
  }

  /// Fold or unfold the bottom panel on every page (the inspector
  /// pill flips the switch, `draw` applies its edges).
  fn apply_panel_open(&mut self, open: bool) {
    for index in 0..5 {
      if let Some(page) = self.sidebar.page_mut(index) {
        if let Some(ed) = page.as_any_mut().downcast_mut::<CodeEditor>() {
          ed.set_collapsed(!open);
        }
      }
    }
  }

  /// Same as the inspector pill click (flips the switch and applies
  /// it at once, so tests need no window).
  pub fn toggle_panel(&mut self) {
    let open = !self.panel_open.get();
    self.panel_open.set(open);
    self.last_panel_open.set(open);
    self.apply_panel_open(open);
  }

  /// Jump to an example issue: back to the `Files` tab, select its
  /// file and move the caret to its line. Runs at once on press so
  /// the content never flashes the wrong file.
  pub(crate) fn jump_to_issue(&mut self, index: usize) {
    let Some(item) = self.warn_items.get(index) else {
      return;
    };
    let (file, line) = (item.file, item.line);
    if file >= self.warn_files.len() {
      return;
    }
    self.selected_warn = index;
    self.nav_tab.set(0);
    self.nav_tabs.set_selected(0);
    self.sidebar.select(file);
    if let Some(page) = self.sidebar.page_mut(file) {
      if let Some(ed) = page.as_any_mut().downcast_mut::<CodeEditor>() {
        ed.goto_line(line);
      }
    }
  }

  /// Active navigator tab (`0` files, `1` warnings).
  pub fn nav_index(&self) -> usize {
    self.nav_tab.get()
  }

  fn refresh_code_ibeam(&mut self) {
    let mut hovered = false;
    let mut divider = false;
    for index in 0..5 {
      if let Some(page) = self.sidebar.page_mut(index) {
        if let Some(ed) = page.as_any_mut().downcast_mut::<CodeEditor>() {
          if ed.wants_text_cursor() {
            hovered = true;
          }
          if ed.wants_divider_cursor() {
            divider = true;
          }
          if hovered && divider {
            break;
          }
        }
      }
    }
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
    self.chev.set_theme(theme.mode, theme.glass);
    self.chev.set_focused(focused);
    self.inspector.set_theme(theme.mode, theme.glass);
    self.inspector.set_focused(focused);
    self.nav_tabs.set_theme(palette.accent, dark);
    self.nav_tabs.set_focused(focused);
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
    // Fake stats and logs advance with the frame time.
    self.tick_code_pages(time_secs);
    // Apply inspector pill edges to every page (divider taps stay
    // per page and never touch the switch).
    let open = self.panel_open.get();
    if open != self.last_panel_open.get() {
      self.last_panel_open.set(open);
      self.apply_panel_open(open);
    }

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
    // Warnings tab: sidebar background over the file rows plus the
    // example issue rows with an accent wash behind the jumped-to
    // row. The content keeps showing the selected file.
    if self.nav_tab.get() == 1 {
      let search_top = viewport.y + viewport.height - SEARCH_PAD - SEARCH_H;
      let bg_y = viewport.y + WARN_TOP - 2.0;
      let bg = if dark { SIDEBAR_BG_DARK } else { GROUP_BG_LIGHT };
      let bg = if focused { bg } else { desaturate(bg) };
      let scale = fonts.scale as f64;
      let px = |v: f32| v as f64 * scale;
      scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        &Brush::Solid(bg),
        None,
        &Rect::new(px(viewport.x), px(bg_y), px(viewport.x + col_w), px(search_top)),
      );
      let row_w = col_w - SEARCH_PAD * 2.0 - 12.0;
      let mut rects = Vec::new();
      for (index, row) in self.warn_rows.iter_mut().enumerate() {
        let ry = viewport.y + WARN_TOP + index as f32 * (WARN_ROW_H + WARN_GAP);
        if index == self.selected_warn {
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
      self.warn_rects = rects;
    }
    // Topbar row in the content toolbar zone: the pill centers on
    // the content middle and wraps a compact group (glyph plus gap
    // plus button text plus gap plus chevron) with equal padding,
    // so icon, text and chevron sit centered inside. The button
    // width derives from the button text only, not the widest
    // dropdown row, and the menu stays vertically centered in the
    // pill. Dead chevron pair at the content left.
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
    let (chev_w, _) = self.chev.measure(fonts);
    self.chev.place(fonts, content_x + SEARCH_PAD, viewport.y + 14.0, chev_w, 36.0);
    self.chev.draw(scene, fonts, images);
    // Notes placeholder pill at the content right: toggles a state
    // for the later notes area, no panel yet.
    let (insp_w, _) = self.inspector.measure(fonts);
    self.inspector.place(
      fonts,
      content_x + content_w - SEARCH_PAD - insp_w,
      viewport.y + 14.0,
      insp_w,
      36.0,
    );
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
    self.command.take()
  }

  /// Hover only: traffic light glyphs, row highlights, pill tints
  /// and the menu react.
  pub fn hover(&mut self, x: f32, y: f32) {
    self.sidebar.set_hover(x, y);
    self.run_stop.mouse_move(x, y);
    self.device.mouse_move(x as f64, y as f64);
    self.chev.mouse_move(x, y);
    self.inspector.mouse_move(x, y);
    self.nav_tabs.mouse_move(x as f64, y as f64);
    self.refresh_code_ibeam();
  }

  fn hit_warn(&self, x: f32, y: f32) -> Option<usize> {
    if self.nav_tab.get() != 1 {
      return None;
    }
    self.warn_rects.iter().position(|r| {
      x >= r.0 && x <= r.0 + r.2 && y >= r.1 && y <= r.1 + r.3
    })
  }

  pub fn mouse_down(&mut self, x: f64, y: f64) {
    // Native element feel (press states, selection, resize): clicks
    // trigger no actions, there are no callbacks anywhere. The search
    // only takes focus and typing, it never searches. The inspector
    // pill only toggles the bottom panel. A warning press jumps to
    // its code at once so the content never flashes the wrong file.
    self.sidebar.mouse_down(x, y);
    self.search.mouse_down(x, y);
    self.run_stop.mouse_down(x, y);
    self.device.mouse_down(x, y);
    self.chev.mouse_down(x, y);
    self.inspector.mouse_down(x, y);
    self.nav_tabs.mouse_down(x, y);
    if let Some(index) = self.hit_warn(x as f32, y as f32) {
      self.jump_to_issue(index);
    }
  }

  pub fn mouse_up(&mut self, x: f64, y: f64) {
    self.sidebar.mouse_up(x, y);
    // No `on_action` on the pill pairs: the press tint releases into
    // nothing, by design. The device menu keeps its example selection.
    self.run_stop.mouse_up(x, y);
    self.device.mouse_up(x, y);
    self.chev.mouse_up(x, y);
    self.inspector.mouse_up(x, y);
    self.nav_tabs.mouse_up(x, y);
  }

  pub fn mouse_wheel(&mut self, dx: f64, dy: f64) {
    self.sidebar.mouse_wheel(dx, dy);
  }

  pub fn type_text(&mut self, content: &str) {
    // Search first while it holds focus, else the active code page.
    // Edits stay in memory only.
    self.search.type_text(content);
    self.sidebar.page_text(content);
  }

  pub fn key(&mut self, key: Key) -> bool {
    if self.search.key(key) {
      return true;
    }
    self.sidebar.page_key(key)
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
    self.chev.set_focused(focused);
    self.inspector.set_focused(focused);
    self.nav_tabs.set_focused(focused);
    self.warn_bg.set_focused(focused);
    for row in self.warn_rows.iter_mut() {
      row.set_focused(focused);
    }
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
  fn new(project: String) -> Self {
    let user = system_username();
    let file = file_stem(&project);
    let code = example_code(&file, &project, &user);
    let ui = EditorUi::new(&project, code);
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
pub fn open_project_window(project: String) {
  std::thread::sleep(std::time::Duration::from_millis(OPEN_DELAY_MS));
  let title = project.clone();
  let app = EditorApp::new(project);
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
    // The starter and the editor child agree on this key instead of
    // CLI arguments.
    assert_eq!(PROJECT_ENV, "XCODE_PROJECT_NAME");
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
    // Fresh editors show the bottom panel; the performance pill
    // folds it away on every page and back.
    let code = example_code("testApp", "test", "arlo");
    let mut ui = EditorUi::new("test", code);
    assert!(ui.panel_open());
    ui.toggle_panel();
    assert!(!ui.panel_open());
    for index in 0..5 {
      let page = ui.sidebar.page_mut(index).expect("page");
      let ed = page
        .as_any_mut()
        .downcast_mut::<CodeEditor>()
        .expect("code page");
      assert!(ed.panel_collapsed());
    }
    ui.toggle_panel();
    assert!(ui.panel_open());
  }

  #[test]
  fn navigator_starts_on_files_with_five_issues() {
    let code = example_code("testApp", "test", "arlo");
    let ui = EditorUi::new("test", code);
    assert_eq!(ui.nav_index(), 0);
    assert_eq!(ui.nav_tabs.selected_index(), 0);
    assert_eq!(ui.warn_items.len(), 5);
    assert_eq!(ui.warn_rows.len(), 5);
    // 3 warnings plus 2 errors, all pointing at real rows.
    let errors = ui.warn_items.iter().filter(|item| item.error).count();
    assert_eq!(errors, 2);
  }

  #[test]
  fn warning_jump_selects_file_and_line() {
    let code = example_code("testApp", "test", "arlo");
    let mut ui = EditorUi::new("test", code);
    // Pretend the warnings tab is open, then jump to the first
    // issue (file 4, line 36).
    ui.nav_tab.set(1);
    ui.jump_to_issue(0);
    assert_eq!(ui.nav_index(), 0);
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
}
