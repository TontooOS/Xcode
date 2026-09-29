//! Example project editor window for Xcode (separate big window).
//!
//! A static Xcode-like IDE card built on the `Sidebar` element: working
//! traffic lights (owned by the sidebar), a dead Run/Stop pill pair at
//! the sidebar top right (no callbacks), a non-collapsible file
//! navigator with one preselected file, a functionless search capsule
//! stretched across the sidebar bottom, dead example Swift code plus a
//! dimmed minimap, breadcrumb and status bar. The element feels alive (hover, press states,
//! selection, resize) but clicks trigger no actions: there are no
//! callbacks, all pages are identical and nothing is ever saved, built
//! or run.
//!
//! Handoff without CLI: the starter sets `XCODE_PROJECT_NAME` on a
//! spawned copy of this binary and closes its own window at once;
//! `open_project_window` waits ~600ms first (old window visibly
//! closes, short gap), then opens the 1100x700 editor fresh.
use crate::TontooUI::elements::{
  Align, BasicText, BasicToolbar, FileImage, HStack, MenuItem, NestedMenu,
  SearchField, Sidebar, SidebarItem, Spacer, TextForeground, TextStyle,
  ToolbarItem, TrafficAction, View, VStack,
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

/// Resolve the bundled `Resources/computer.png` device glyph.
fn computer_icon_path() -> std::path::PathBuf {
  let mut candidates = Vec::new();
  if let Ok(env) = std::env::var("APP_RESOURCES_DIR") {
    if !env.is_empty() {
      candidates.push(std::path::PathBuf::from(env).join("computer.png"));
    }
  }
  if let Ok(cwd) = std::env::current_dir() {
    candidates.push(cwd.join("Resources").join("computer.png"));
  }
  if let Ok(exe) = std::env::current_exe() {
    if let Some(parent) = exe.parent() {
      candidates.push(parent.join("Resources").join("computer.png"));
      if let Some(grand) = parent.parent() {
        candidates.push(grand.join("Resources").join("computer.png"));
      }
    }
  }
  candidates.into_iter().find(|p| p.is_file()).unwrap_or_else(|| {
    std::path::PathBuf::from("__xcode_missing_computer_icon__")
  })
}

/// Static editor page: breadcrumb, code plus minimap, status bar.
fn editor_page(breadcrumb: String, code: String, filter: String, status: String) -> VStack {
  VStack::new()
    .spacing(8.0)
    .align(Align::Leading)
    .child(
      BasicText::new(breadcrumb)
        .style(TextStyle::Caption)
        .foreground(TextForeground::Secondary),
    )
    .child(
      HStack::new()
        .spacing(8.0)
        .align(Align::Leading)
        .child(BasicText::new(code.clone()).style(TextStyle::Footnote))
        .child(
          BasicText::new(code)
            .style(TextStyle::Caption2)
            .foreground(TextForeground::Secondary)
            .width(112.0),
        ),
    )
    .child(
      HStack::new()
        .spacing(8.0)
        .align(Align::Center)
        .child(
          BasicText::new(filter)
            .style(TextStyle::Caption)
            .foreground(TextForeground::Secondary),
        )
        .child(Spacer::new())
        .child(
          BasicText::new(status)
            .style(TextStyle::Caption)
            .foreground(TextForeground::Secondary),
        ),
    )
}

pub struct EditorUi {
  sidebar: Sidebar,
  search: SearchField,
  /// Dead Run/Stop pair at the sidebar top right edge (hover/press
  /// tint only, no callbacks).
  run_stop: BasicToolbar,
  /// Device glyph left of the menu (plain `computer.png` raster).
  computer: FileImage,
  /// Centered device text-menu with sections (example selection only).
  device: NestedMenu,
  /// Dead back/forward chevrons at the top right.
  chev: BasicToolbar,
  /// Dead collapse button at the very top right.
  collapse_btn: BasicToolbar,
  code_width: f32,
  watcher: ThemeWatcher,
  focused: bool,
  bg: Color,
  command: Option<WindowCommand>,
}

impl EditorUi {
  pub fn new(project: &str, breadcrumb: String, code: String, filter: String, status: String) -> Self {
    let file = file_stem(project);
    let items = vec![
      SidebarItem::new(project, "folder.fill"),
      SidebarItem::new("Assets", "folder.fill"),
      SidebarItem::new("ContentView", "doc.fill"),
      SidebarItem::new("Info", "doc.fill"),
      SidebarItem::new(file, "doc.fill"),
    ];
    let mut sidebar = Sidebar::new(items)
      .page(editor_page(breadcrumb.clone(), code.clone(), filter.clone(), status.clone()))
      .page(editor_page(breadcrumb.clone(), code.clone(), filter.clone(), status.clone()))
      .page(editor_page(breadcrumb.clone(), code.clone(), filter.clone(), status.clone()))
      .page(editor_page(breadcrumb.clone(), code.clone(), filter.clone(), status.clone()))
      .page(editor_page(breadcrumb, code, filter, status))
      .search_field(false)
      .toggle_button(false)
      .collapsible(false);
    sidebar.set_title(format!("{project} › {}", lang::t("ed.device")));
    sidebar.select(FILE_INDEX);
    // The element defaults item labels to hand-set white, which wins
    // over the theme in light mode: clear the override once so labels
    // follow `set_theme` (dark `#d8d9d9`, light `#272727`).
    sidebar.set_item_text(None);
    Self {
      sidebar,
      search: SearchField::new(lang::t("ed.search")),
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
          MenuItem::action(lang::t("menu.device")),
          MenuItem::divider(),
          MenuItem::section(lang::t("menu.build")),
          MenuItem::action(lang::t("menu.prod")),
          MenuItem::action(lang::t("menu.dev")),
          MenuItem::divider(),
          MenuItem::section(lang::t("menu.utils")),
          MenuItem::action(lang::t("menu.export")),
        ],
      )
      .on_action(|path| println!("device menu {path:?} (example)")),
      chev: BasicToolbar::from_items(vec![
        ToolbarItem::icon("chevron.left"),
        ToolbarItem::divider(),
        ToolbarItem::icon("chevron.right"),
      ]),
      collapse_btn: BasicToolbar::from_icons(vec!["sidebar.right".to_string()]).round(true),
      code_width: 600.0,
      watcher: ThemeWatcher::new(),
      focused: true,
      bg: crate::TontooUI::renderer::window::BACKGROUND,
      command: None,
    }
  }

  /// Wire one page: theme for all texts plus the code width for the
  /// current content size.
  fn wire_page(page: &mut dyn View, mode: ThemeMode, focused: bool, code_w: f32) {
    let Some(stack) = page.as_any_mut().downcast_mut::<VStack>() else {
      return;
    };
    if let Some(line) = stack.child_mut::<BasicText>(0) {
      line.set_theme(mode);
      line.set_focused(focused);
    }
    if let Some(row) = stack.child_mut::<HStack>(1) {
      if let Some(code) = row.child_mut::<BasicText>(0) {
        code.set_width(Some(code_w));
        code.set_theme(mode);
        code.set_focused(focused);
      }
      if let Some(mini) = row.child_mut::<BasicText>(1) {
        mini.set_theme(mode);
        mini.set_focused(focused);
      }
    }
    if let Some(bar) = stack.child_mut::<HStack>(2) {
      if let Some(left) = bar.child_mut::<BasicText>(0) {
        left.set_theme(mode);
        left.set_focused(focused);
      }
      if let Some(right) = bar.child_mut::<BasicText>(2) {
        right.set_theme(mode);
        right.set_focused(focused);
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
    self.run_stop.set_theme(theme.mode, theme.glass);
    self.run_stop.set_focused(focused);
    self.computer.set_theme(dark);
    self.computer.set_focused(focused);
    self.device.set_theme(palette.accent, dark);
    self.device.set_glass(theme.mode, theme.glass);
    self.device.set_focused(focused);
    self.chev.set_theme(theme.mode, theme.glass);
    self.chev.set_focused(focused);
    self.collapse_btn.set_theme(theme.mode, theme.glass);
    self.collapse_btn.set_focused(focused);
    // Code width follows the content size (window is fixed, maximize
    // still changes the viewport).
    self.code_width = (viewport.width - 48.0 - self.sidebar.width_value() - 136.0).max(40.0);
    let code_w = self.code_width;
    for index in 0..5 {
      if let Some(page) = self.sidebar.page_mut(index) {
        Self::wire_page(page, mode, focused, code_w);
      }
    }

    // No titlebar: the sidebar owns the decoration (traffic lights
    // live in it) and fills the whole viewport.
    self.sidebar.place(fonts, viewport.x, viewport.y, viewport.width, viewport.height);
    self.sidebar.draw(scene, fonts, images);
    let col_w = self.sidebar.width_value();
    let content_x = viewport.x + col_w;
    let content_w = (viewport.width - col_w).max(0.0);
    let right = viewport.x + viewport.width;
    // Topbar row in the content toolbar zone: device glyph plus
    // centered text-menu, dead chevron pair right, dead collapse pill
    // far right.
    self.device.set_viewport(viewport.x, viewport.y, viewport.width, viewport.height);
    let (menu_w, menu_h) = self.device.measure(fonts);
    let group_w = DEVICE_ICON + DEVICE_GAP + menu_w;
    let group_x = content_x + ((content_w - group_w) / 2.0).max(0.0);
    self.computer.place(
      fonts,
      group_x,
      viewport.y + 14.0 + ((menu_h - DEVICE_ICON) / 2.0).max(0.0),
      DEVICE_ICON,
      DEVICE_ICON,
    );
    self.computer.draw(scene, fonts, images);
    self.device.place(fonts, group_x + DEVICE_ICON + DEVICE_GAP, viewport.y + 14.0, menu_w, menu_h);
    self.device.draw(scene, fonts, images);
    let (chev_w, _) = self.chev.measure(fonts);
    self.chev.place(fonts, right - SEARCH_PAD - 36.0 - SEARCH_PAD - chev_w, viewport.y + 14.0, chev_w, 36.0);
    self.chev.draw(scene, fonts, images);
    self.collapse_btn.place(fonts, right - SEARCH_PAD - 36.0, viewport.y + 14.0, 36.0, 36.0);
    self.collapse_btn.draw(scene, fonts, images);
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
    self.sidebar.wants_backdrop()
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
    self.collapse_btn.mouse_move(x, y);
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
    self.collapse_btn.mouse_down(x, y);
  }

  pub fn mouse_up(&mut self, x: f64, y: f64) {
    self.sidebar.mouse_up(x, y);
    // No `on_action` on the pill pairs: the press tint releases into
    // nothing, by design. The device menu keeps its example selection.
    self.run_stop.mouse_up(x, y);
    self.device.mouse_up(x, y);
    self.chev.mouse_up(x, y);
    self.collapse_btn.mouse_up(x, y);
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
    self.run_stop.set_focused(focused);
    self.device.set_focused(focused);
    self.chev.set_focused(focused);
    self.collapse_btn.set_focused(focused);
  }
}

struct EditorApp {
  ui: EditorUi,
}

impl EditorApp {
  fn new(project: String) -> Self {
    let user = system_username();
    let file = file_stem(&project);
    let breadcrumb = format!(
      "{project} › {project} › {file} | {}",
      lang::t("ed.no_selection")
    );
    let code = example_code(&file, &project, &user);
    let ui = EditorUi::new(
      &project,
      breadcrumb,
      code,
      lang::t("ed.filter"),
      lang::t("ed.status"),
    );
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
