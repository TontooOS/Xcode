//! Example project editor window for Xcode (separate big window).
//!
//! A static Xcode-like IDE card built on the `Sidebar` element: working
//! traffic lights (owned by the sidebar), a dead Run/Stop pill pair at
//! the sidebar top right (no callbacks), a non-collapsible file
//! navigator with one preselected file, a functionless search capsule
//! stretched across the sidebar bottom, dead example Swift code,
//! breadcrumb and status bar. The element feels alive (hover, press states,
//! selection, resize) but clicks trigger no actions: there are no
//! callbacks, all pages are identical and nothing is ever saved, built
//! or run.
//!
//! Handoff without CLI: the starter sets `XCODE_PROJECT_NAME` on a
//! spawned copy of this binary and closes its own window at once;
//! `open_project_window` waits ~600ms first (old window visibly
//! closes, short gap), then opens the 1100x700 editor fresh.
use std::cell::Cell;
use std::rc::Rc;

use crate::DocumentKit::{SyntaxLang, highlight_syntax as highlight};
use crate::TontooUI::elements::{
  Align, BasicText, BasicToolbar, FileImage, FormattedText, HStack,
  HorizontalDivider, MenuItem, NestedMenu, SearchField, Sidebar, SidebarItem,
  Span, TextAlignment, TextForeground, TextStyle, ToolbarItem,
  TrafficAction, View, VStack, MENU_BTN_PAD_X,
};
use crate::TontooUI::renderer::window::{App, CursorKind, Key, Viewport, WindowCommand, run};
use crate::TontooUI::renderer::{FontSystem, ImageLoader};
use crate::TontooUI::theme::{ThemeMode, ThemeWatcher};
use crate::lang;
use crate::scaffold::system_username;
use vello::Scene;
use vello::peniko::Color;

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
  format!(
    "//  {file}.swift\n//  {project}\n//\n//  Created by {user}.\n//\nimport SwiftUI\n\n@main\nstruct {file}: App {{\n    var body: some Scene {{\n        WindowGroup {{\n            ContentView()\n        }}\n    }}\n}}\n"
  )
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

/// Gray gutter width for line numbers.
const GUTTER_W: f32 = 30.0;

/// One source line: gray number plus DocumentKit-highlighted code.
/// Both sides share one `TextStyle` size so numbers stay glued to
/// their lines. Highlighting runs per line (exact for `//` comments
/// and single-line strings; the example code uses no multi-line
/// constructs).
fn code_row(no: usize, spans: Vec<Span>) -> HStack {
  HStack::new()
    .spacing(8.0)
    .align(Align::Center)
    .child(
      BasicText::new((no + 1).to_string())
        .style(TextStyle::Footnote)
        .foreground(TextForeground::Secondary)
        .alignment(TextAlignment::Trailing)
        .width(GUTTER_W),
    )
    .child(FormattedText::spans(spans).style(TextStyle::Footnote))
}

/// Map one source line through the DocumentKit Rust tokenizer into
/// `Span`s (gaps stay plain code). Colors follow `SpanKind` per theme.
fn highlight_spans(line: &str, dark: bool) -> Vec<Span> {
  if line.is_empty() {
    return vec![Span::new(" ").code()];
  }
  let mut out = Vec::new();
  let mut cursor = 0;
  for s in highlight(SyntaxLang::Rust, line) {
    if s.start > cursor {
      if let Some(t) = line.get(cursor..s.start) {
        out.push(Span::new(t).code());
      }
    }
    if let Some(t) = line.get(s.start..s.end) {
      let mut span = Span::new(t).code().color(s.kind.color(dark));
      if s.bold {
        span = span.bold();
      }
      if s.italic {
        span = span.italic();
      }
      if s.underline {
        span = span.underline();
      }
      out.push(span);
    }
    cursor = cursor.max(s.end);
  }
  if let Some(t) = line.get(cursor..) {
    if !t.is_empty() {
      out.push(Span::new(t).code());
    }
  }
  if out.is_empty() {
    out.push(Span::new(" ").code());
  }
  out
}

/// Static editor page: highlighted code with gray line numbers.
fn editor_page(lines: &[String], dark: bool) -> VStack {
  let mut rows = VStack::new().spacing(2.0).align(Align::Leading);
  for (no, line) in lines.iter().enumerate() {
    rows = rows.child(code_row(no, highlight_spans(line, dark)));
  }
  rows
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
  /// Source lines behind the row views (rebuilt on theme change).
  code_lines: Vec<String>,
  /// Text color the row spans were built with.
  code_text: Color,
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
    let code_lines: Vec<String> = code.lines().map(|l| l.to_string()).collect();
    // Initial spans assume dark; the first draw rebuilds them in the
    // live theme (see `wire_page`).
    let mut sidebar = Sidebar::new(items)
      .page(editor_page(&code_lines, true))
      .page(editor_page(&code_lines, true))
      .page(editor_page(&code_lines, true))
      .page(editor_page(&code_lines, true))
      .page(editor_page(&code_lines, true))
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
      code_lines,
      code_text: Color::TRANSPARENT,
      watcher: ThemeWatcher::new(),
      focused: true,
      bg: crate::TontooUI::renderer::window::BACKGROUND,
      command: None,
    }
  }

  /// Wire one page: theme for all texts; code spans rebuild when the
  /// theme changed (fresh kind colors).
  fn wire_page(
    page: &mut dyn View,
    mode: ThemeMode,
    focused: bool,
    lines: &[String],
    dark: bool,
    recolor: bool,
  ) {
    let Some(rows) = page.as_any_mut().downcast_mut::<VStack>() else {
      return;
    };
    for (no, source) in lines.iter().enumerate() {
      if let Some(row) = rows.child_mut::<HStack>(no) {
        if let Some(gutter) = row.child_mut::<BasicText>(0) {
          gutter.set_theme(mode);
          gutter.set_focused(focused);
        }
          if let Some(code) = row.child_mut::<FormattedText>(1) {
            if recolor {
              code.set_source(highlight_spans(source, dark));
            }
          code.set_theme(mode);
          code.set_focused(focused);
        }
      }
    }
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
    // Spans rebuild on theme text change (fresh kind colors).
    let recolor = palette.text != self.code_text;
    if recolor {
      self.code_text = palette.text;
    }
    for index in 0..5 {
      if let Some(page) = self.sidebar.page_mut(index) {
        Self::wire_page(page, mode, focused, &self.code_lines, dark, recolor);
      }
    }

    // No titlebar: the sidebar owns the decoration (traffic lights
    // live in it) and fills the whole viewport.
    self.sidebar.place(fonts, viewport.x, viewport.y, viewport.width, viewport.height);
    self.sidebar.draw(scene, fonts, images);
    let col_w = self.sidebar.width_value();
    let content_x = viewport.x + col_w;
    let content_w = (viewport.width - col_w).max(0.0);
    // Topbar row in the content toolbar zone: the TEXT centers on
    // the window middle, the glyph hangs left of it, the glass pill
    // spans symmetric around the text. Dead chevron pair at the
    // content left.
    self.device.set_viewport(viewport.x, viewport.y, viewport.width, viewport.height);
    let (menu_w, menu_h) = self.device.measure(fonts);
    let label = self.device.button_text().to_string();
    let (tw, _) = FontSystem::layout_size(&fonts.layout_text(&label, DEVICE_FONT, Color::WHITE, None));
    let text_w = tw / fonts.scale;
    let middle = viewport.x + viewport.width / 2.0;
    // NestedMenu draws button text at `menu_x + MENU_BTN_PAD_X`.
    let menu_x = middle - MENU_BTN_PAD_X - text_w / 2.0;
    let icon_x = menu_x - DEVICE_GAP - DEVICE_ICON;
    let half = ((middle - icon_x + GLASS_PAD).max(menu_x + menu_w - middle + GLASS_PAD)).max(0.0);
    self.computer.place(
      fonts,
      icon_x,
      viewport.y + 14.0 + ((menu_h - DEVICE_ICON) / 2.0).max(0.0),
      DEVICE_ICON,
      DEVICE_ICON,
    );
    self.computer.draw(scene, fonts, images);
    // Glass body symmetric around the text, 36px tall like the other
    // pills.
    let glass_h = 36.0;
    let glass_y = viewport.y + 14.0 + ((menu_h - glass_h) / 2.0).max(0.0);
    self.menu_glass.place(fonts, middle - half, glass_y, half * 2.0, glass_h);
    self.menu_glass.draw(scene, fonts, images);
    self.device.place(fonts, menu_x, viewport.y + 14.0, menu_w, menu_h);
    self.device.draw(scene, fonts, images);
    let (chev_w, _) = self.chev.measure(fonts);
    self.chev.place(fonts, content_x + SEARCH_PAD, viewport.y + 14.0, chev_w, 36.0);
    self.chev.draw(scene, fonts, images);
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
  }

  pub fn mouse_down(&mut self, x: f64, y: f64) {
    // Native element feel (press states, selection, resize): clicks
    // trigger no actions, there are no callbacks anywhere. The search
    // only takes focus and typing, it never searches.
    self.sidebar.mouse_down(x, y);
    self.search.mouse_down(x, y);
    self.run_stop.mouse_down(x, y);
    self.device.mouse_down(x, y);
    self.chev.mouse_down(x, y);
  }

  pub fn mouse_up(&mut self, x: f64, y: f64) {
    self.sidebar.mouse_up(x, y);
    // No `on_action` on the pill pairs: the press tint releases into
    // nothing, by design. The device menu keeps its example selection.
    self.run_stop.mouse_up(x, y);
    self.device.mouse_up(x, y);
    self.chev.mouse_up(x, y);
  }

  pub fn mouse_wheel(&mut self, dx: f64, dy: f64) {
    self.sidebar.mouse_wheel(dx, dy);
  }

  pub fn type_text(&mut self, content: &str) {
    self.search.type_text(content);
  }

  pub fn key(&mut self, key: Key) -> bool {
    self.search.key(key)
  }

  pub fn wants_text_cursor(&self) -> bool {
    self.search.wants_text_cursor()
  }

  pub fn wants_resize(&self, x: f64, y: f64) -> bool {
    self.sidebar.wants_resize_cursor(x, y)
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
    if self.ui.wants_text_cursor() {
      CursorKind::Text
    } else if self.ui.wants_resize(x, y) {
      CursorKind::ResizeColumn
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
  fn example_code_mentions_names() {
    let code = example_code("testApp", "test", "arlo");
    assert!(code.contains("testApp"));
    assert!(code.contains("Created by arlo"));
    assert!(code.contains("import SwiftUI"));
  }

  #[test]
  fn highlight_spans_cover_everything() {
    // Empty lines stay renderable.
    assert!(!highlight_spans("", true).is_empty());
    // Plain text without tokens stays one code span.
    assert_eq!(highlight_spans("hello", true).len(), 1);
    // Gap filling is exact: mapped spans equal tokenizer spans plus
    // one plain span per uncovered gap, whatever the lexer finds.
    for line in ["fn x() {} // hi", "let s = \"hi\";", "    ", "}"] {
      let toks = highlight(SyntaxLang::Rust, line);
      let mut gaps = 0;
      let mut cursor = 0;
      for t in &toks {
        if t.start > cursor {
          gaps += 1;
        }
        cursor = cursor.max(t.end);
      }
      if cursor < line.len() {
        gaps += 1;
      }
      let mapped = highlight_spans(line, true).len();
      let expected = toks.len() + gaps;
      assert_eq!(mapped, expected.max(1), "line {line:?}");
    }
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
}
