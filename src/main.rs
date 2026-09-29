//! Xcode start page for TontooOS.
//!
//! Example start window (420x585, ~25% smaller than the Apple reference)
//! built with TontooUI on Vello/WGPU:
//! no traffic lights, only one round toolbar button with an `xmark`
//! glyph at the top left (closes the window), the centered app icon
//! from `Resources/icon.tico` rendered through CoreIcon in its normal
//! (light) variant, the `Xcode` title with a `Version 27.0.0` line, two
//! example capsule buttons (`Open...`, `New Project`) and an empty
//! recents box. `New Project` opens a modal sheet on top with the
//! project options (EN/DE app name, version, bundle ID, live bundle
//! identifier preview, `Cancel` / `Create`); the start page stays
//! visible behind the dimmed backdrop.
//!
//! All text uses SF Pro (system font) with `en_us` and `de_de` strings
//! from `lang/` via Accessibility. The theme follows the settings
//! daemon live through `ThemeWatcher` (Dark `#1B2022` / Light
//! `#FFFFFF`).

mod icon;
mod lang;
mod projects;
mod scaffold;
mod sheet;

sdk::preinclude!();

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use TontooUI::elements::{
  Align, BasicSheet, BasicText, BasicToolbar, Button, ButtonShape, FileImage,
  HStack, HorizontalDivider, ImageFit, RoundedRectangle, SFSymbolImage,
  ShapeFill, SheetSize, TextAlignment, TextForeground, TextStyle,
  ToolbarPlacement, View, VStack, BUTTON_BG_DARK, BUTTON_BG_LIGHT,
};
use TontooUI::renderer::window::{
  App, CursorKind, Key, Viewport, WindowCommand, run,
};
use TontooUI::renderer::{FontSystem, ImageLoader};
use TontooUI::theme::{ThemeMode, ThemeWatcher};
use vello::Scene;
use vello::peniko::Color;

use crate::projects::{ProjectRecord, load_projects};
use crate::sheet::NewProjectForm;

const WINDOW_WIDTH: u32 = 420;
const WINDOW_HEIGHT: u32 = 585;
const ICON_PX: f32 = 90.0;
const ICON_RADIUS: f32 = 21.0;
const CONTENT_WIDTH: f32 = 330.0;
const TEXT_WIDTH: f32 = 330.0;
const RECENTS_BOX_H: f32 = 255.0;
const ROW_H: f32 = 48.0;
const ROW_PAD_X: f32 = 9.0;
const BOX_PAD: f32 = 6.0;
/// Visible project rows without scrolling (display only for now).
const MAX_ROWS: usize = 4;
/// Close pill geometry: 12px offset inside the viewport plus its size.
/// The viewport itself starts at a 24px frame margin, so these must be
/// added to the real viewport origin every frame (never constants).
const CLOSE_OFF: f32 = 12.0;
const CLOSE_S: f32 = 36.0;

fn hit_rect(rect: (f32, f32, f32, f32), x: f32, y: f32) -> bool {
  x >= rect.0 && x <= rect.0 + rect.2 && y >= rect.1 && y <= rect.1 + rect.3
}

/// One project row: folder icon plus display name/path text column.
fn project_row(name: &str, path: &str) -> HStack {
  let texts = VStack::new()
    .spacing(1.5)
    .align(Align::Leading)
    .child(BasicText::new(name).style(TextStyle::Caption))
    .child(
      BasicText::new(path)
        .style(TextStyle::Caption2)
        .foreground(TextForeground::Secondary),
    );
  HStack::new()
    .spacing(9.0)
    .align(Align::Center)
    .child(SFSymbolImage::new("folder.fill").size(30.0))
    .child(texts)
}
const VERSION: &str = "27.0.0";

struct StartPage {
  close_bar: BasicToolbar,
  close_requested: Rc<Cell<bool>>,
  icon: FileImage,
  title: BasicText,
  version: BasicText,
  actions: HStack,
  records: Vec<ProjectRecord>,
  rows: Vec<HStack>,
  dividers: Vec<HorizontalDivider>,
  empty_label: BasicText,
  box_bg: RoundedRectangle,
  selection_bg: RoundedRectangle,
  sheet: BasicSheet<NewProjectForm>,
  show_sheet: Rc<Cell<bool>>,
  cancel_sheet: Rc<Cell<bool>>,
  sheet_ibeam: Cell<bool>,
  /// Last cursor position (tracks hover): the drag region is only the
  /// background, so presses over controls reach the app as clicks.
  cursor: Cell<(f32, f32)>,
  /// Real viewport rect from the last frame: the renderer draws inside
  /// a 24px frame margin, so hit tests and the region must use this
  /// origin instead of (0, 0).
  viewport: Cell<(f32, f32, f32, f32)>,
  /// Placed button rects from the last frame for the hit test above.
  action_rects: Vec<(f32, f32, f32, f32)>,
  watcher: ThemeWatcher,
  focused: bool,
  bg: Color,
}

impl StartPage {
  fn rebuild_rows(&mut self) {
    self.rows = self
      .records
      .iter()
      .take(MAX_ROWS)
      .map(|r| project_row(r.display_name(), &r.path))
      .collect();
    self.dividers = (0..self.rows.len().saturating_sub(1))
      .map(|_| HorizontalDivider::new())
      .collect();
  }
}

impl StartPage {
  fn new(app_icon: std::path::PathBuf) -> Self {
    let close_requested = Rc::new(Cell::new(false));
    let flag = close_requested.clone();
    let close_bar = BasicToolbar::from_icons(vec!["xmark".to_string()])
      .round(true)
      .placement(ToolbarPlacement::Leading)
      .on_action(move |_| flag.set(true));

    let title = BasicText::new(lang::t("app.title"))
      .style(TextStyle::Headline)
      .alignment(TextAlignment::Center)
      .width(TEXT_WIDTH);
    let version = BasicText::new(
      lang::t("app.version").replace("{version}", VERSION),
    )
    .style(TextStyle::Caption)
    .foreground(TextForeground::Secondary)
    .alignment(TextAlignment::Center)
    .width(TEXT_WIDTH);

    let show_sheet = Rc::new(Cell::new(false));
    let show_pressed = show_sheet.clone();
    let cancel_sheet = Rc::new(Cell::new(false));

    let actions = HStack::new()
      .spacing(9.0)
      .align(Align::Center)
      .child(
        Button::new(lang::t("action.open"))
          .shape(ButtonShape::Capsule)
          .on_press(|| println!("open pressed (example)")),
      )
      .child(
        Button::new(lang::t("action.new_project"))
          .shape(ButtonShape::Capsule)
          .on_press(move || show_pressed.set(true)),
      );

    // Saved projects survive restarts (CoreData); a missing store
    // simply means no projects yet.
    let records = match load_projects() {
      Ok(records) => records,
      Err(err) => {
        eprintln!("projects: {err}");
        Vec::new()
      }
    };

    // The recents box starts empty when nothing was created yet: only
    // a dim placeholder line. Rows are added here once projects exist.
    let empty_label = BasicText::new(lang::t("project.empty"))
      .style(TextStyle::Callout)
      .foreground(TextForeground::Secondary)
      .alignment(TextAlignment::Center)
      .width(CONTENT_WIDTH - 32.0);

    let created = Rc::new(RefCell::new(None));
    let sheet = BasicSheet::new(NewProjectForm::new(cancel_sheet.clone(), created.clone()))
      .size(SheetSize::Small);

    let mut page = Self {
      close_bar,
      close_requested,
      icon: FileImage::new(app_icon, ICON_PX, ICON_PX)
        .radius(ICON_RADIUS)
        .fit(ImageFit::Cover),
      title,
      version,
      actions,
      records,
      rows: Vec::new(),
      dividers: Vec::new(),
      empty_label,
      box_bg: RoundedRectangle::new(CONTENT_WIDTH, RECENTS_BOX_H, 12.0)
        .fill(BUTTON_BG_DARK),
      selection_bg: RoundedRectangle::new(CONTENT_WIDTH, ROW_H, 8.0)
        .fill(Color::from_rgb8(0x00, 0x7a, 0xff)),
      sheet,
      show_sheet,
      cancel_sheet,
      sheet_ibeam: Cell::new(false),
      cursor: Cell::new((-1.0, -1.0)),
      viewport: Cell::new((0.0, 0.0, WINDOW_WIDTH as f32, WINDOW_HEIGHT as f32)),
      action_rects: Vec::new(),
      watcher: ThemeWatcher::new(),
      focused: true,
      bg: TontooUI::renderer::window::BACKGROUND,
    };
    page.rebuild_rows();
    page
  }

  fn each_action_button(&mut self, mut f: impl FnMut(&mut Button)) {
    for index in 0..self.actions.len() {
      if let Some(button) = self.actions.child_mut::<Button>(index) {
        f(button);
      }
    }
  }
}

impl App for StartPage {
  fn draw(
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
    let focused = self.focused;
    self.bg = palette.bg;

    // Live theme for the chrome: glass X button, icon placeholder,
    // title/version text, capsule buttons, box and selection fills.
    self.close_bar.set_theme(theme.mode, theme.glass);
    self.close_bar.set_focused(focused);
    self.icon.set_theme(dark);
    self.icon.set_focused(focused);
    self.title.set_theme(theme.mode);
    self.title.set_focused(focused);
    self.version.set_theme(theme.mode);
    self.version.set_focused(focused);
    let (button_bg, button_text) = if dark {
      (BUTTON_BG_DARK, Color::WHITE)
    } else {
      (BUTTON_BG_LIGHT, Color::BLACK)
    };
    self.each_action_button(|button| {
      button.set_palette(button_bg, button_text);
      button.set_theme(palette.accent, dark);
      button.set_focused(focused);
    });
    self.box_bg.set_fill(ShapeFill::Solid(if dark {
      BUTTON_BG_DARK
    } else {
      BUTTON_BG_LIGHT
    }));
    self.box_bg.set_focused(focused);
    self.selection_bg.set_fill(ShapeFill::Solid(palette.accent));
    self.selection_bg.set_focused(focused);
    self.empty_label.set_theme(theme.mode);
    self.empty_label.set_focused(focused);

    // Project rows: first row is selected (white text/icon on the
    // accent fill), the rest follow the theme. Display only for now:
    // no click handling, no open action yet.
    for (index, row) in self.rows.iter_mut().enumerate() {
      let selected = index == 0;
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
        if let Some(name) = texts.child_mut::<BasicText>(0) {
          if selected {
            name.set_foreground(TextForeground::Color(Color::WHITE));
          } else {
            name.set_foreground(TextForeground::Primary);
            name.set_theme(theme.mode);
          }
          name.set_focused(focused);
        }
        if let Some(path) = texts.child_mut::<BasicText>(1) {
          if selected {
            path.set_foreground(TextForeground::Color(Color::from_rgba8(
              255, 255, 255, 220,
            )));
          } else {
            path.set_foreground(TextForeground::Secondary);
            path.set_theme(theme.mode);
          }
          path.set_focused(focused);
        }
      }
    }
    for divider in self.dividers.iter_mut() {
      divider.set_theme(palette.divider, dark);
      divider.set_focused(focused);
    }

    // New-project sheet: live theme plus derived form state (bundle
    // identifier preview, locale switching). Open/close requests from
    // the buttons are polled here.
    self.sheet.set_theme(dark);
    self.sheet.set_focused(focused);
    self.sheet.child_mut().update(palette.accent, dark, theme.mode, focused);
    self.sheet.child_mut().set_focused(focused);
    if self.show_sheet.take() {
      self.sheet.show();
    }
    if self.cancel_sheet.take() {
      self.sheet.dismiss();
    }
    // A freshly scaffolded project joins the recents box; the form
    // resets once the sheet is fully gone.
    if let Some(record) = self.sheet.child_mut().take_created() {
      self.records.push(record);
      self.rebuild_rows();
    }
    if !self.sheet.is_visible() {
      self.sheet.child_mut().reset();
    }

    // Layout: close button top left, everything else centered below.
    let (vx, vy, vw, vh) = (viewport.x, viewport.y, viewport.width, viewport.height);
    self.viewport.set((vx, vy, vw, vh));
    self.sheet.set_viewport(vx, vy, vw, vh);
    self.close_bar.place(fonts, vx + CLOSE_OFF, vy + CLOSE_OFF, CLOSE_S, CLOSE_S);
    self.close_bar.draw(scene, fonts, images);

    let mut cy = vy + 60.0;
    let center = |w: f32| vx + ((vw - w) / 2.0).max(0.0);

    let (icon_w, icon_h) = self.icon.measure(fonts);
    self.icon.place(fonts, center(icon_w), cy, icon_w, icon_h);
    self.icon.draw(scene, fonts, images);
    cy += icon_h + 9.0;

    let (title_w, title_h) = self.title.measure(fonts);
    self.title.place(fonts, center(title_w), cy, title_w, title_h);
    self.title.draw(scene, fonts, images);
    cy += title_h + 3.0;

    let (version_w, version_h) = self.version.measure(fonts);
    self.version.place(fonts, center(version_w), cy, version_w, version_h);
    self.version.draw(scene, fonts, images);
    cy += version_h + 12.0;

    let (actions_w, actions_h) = self.actions.measure(fonts);
    self.actions.place(fonts, center(actions_w), cy, actions_w, actions_h);
    self.actions.draw(scene, fonts, images);
    // Snapshot for the background-only drag hit test below.
    let mut rects = Vec::new();
    self.each_action_button(|button| rects.push(button.rect()));
    self.action_rects = rects;
    cy += actions_h + 14.0;

    // Recents box: saved projects on top of the accent-selected first
    // row, or a centered dim placeholder when nothing exists yet.
    let bx = center(CONTENT_WIDTH);
    self.box_bg.place(fonts, bx, cy, CONTENT_WIDTH, RECENTS_BOX_H);
    self.box_bg.draw(scene, fonts, images);
    if self.rows.is_empty() {
      let (empty_w, empty_h) = self.empty_label.measure(fonts);
      self.empty_label.place(
        fonts,
        bx + ((CONTENT_WIDTH - empty_w) / 2.0).max(0.0),
        cy + ((RECENTS_BOX_H - empty_h) / 2.0).max(0.0),
        empty_w,
        empty_h,
      );
      self.empty_label.draw(scene, fonts, images);
    } else {
      self.selection_bg.place(
        fonts,
        bx + BOX_PAD,
        cy + BOX_PAD,
        CONTENT_WIDTH - BOX_PAD * 2.0,
        ROW_H,
      );
      self.selection_bg.draw(scene, fonts, images);
      let row_w = CONTENT_WIDTH - BOX_PAD * 2.0 - ROW_PAD_X * 2.0;
      let mut ry = cy + BOX_PAD;
      for (index, row) in self.rows.iter_mut().enumerate() {
        row.place(fonts, bx + BOX_PAD + ROW_PAD_X, ry, row_w, ROW_H);
        row.draw(scene, fonts, images);
        ry += ROW_H;
        if index < self.dividers.len() {
          let divider = &mut self.dividers[index];
          divider.place(fonts, bx + BOX_PAD + ROW_PAD_X, ry, row_w, 1.0);
          divider.draw(scene, fonts, images);
          ry += 1.0;
        }
      }
    }

    // Modal new-project sheet on top: dims the start page, fades in.
    self.sheet.draw(scene, fonts, images);
  }

  fn background(&self) -> Color {
    self.bg
  }

  fn wants_backdrop(&self) -> bool {
    // The round close control is a Lens glass pill and refracts the
    // backdrop, so the compositor must feed it the backdrop pass.
    true
  }

  fn drag_region(&self) -> Option<(f32, f32, f32, f32)> {
    // Background drag only: an open sheet needs every press for its
    // fields, and presses over the close pill or the action buttons
    // must reach the app as clicks instead of starting a drag. The
    // renderer polls this on every press, so the hover-tracked cursor
    // position decides.
    if self.sheet.is_visible() {
      return None;
    }
    let (mx, my) = self.cursor.get();
    let (vx, vy, vw, vh) = self.viewport.get();
    if hit_rect((vx + CLOSE_OFF, vy + CLOSE_OFF, CLOSE_S, CLOSE_S), mx, my) {
      return None;
    }
    if self.action_rects.iter().any(|r| hit_rect(*r, mx, my)) {
      return None;
    }
    Some((vx, vy, vw, vh))
  }

  fn poll_window_command(&mut self) -> Option<WindowCommand> {
    if self.close_requested.take() {
      return Some(WindowCommand::Close);
    }
    None
  }

  fn mouse_down(&mut self, x: f64, y: f64) {
    // Presses on the background never arrive here (system drag, see
    // `drag_region`); presses over controls and the open sheet do and
    // arm them normally.
    if self.sheet.is_visible() {
      self.sheet.mouse_down(x, y);
      return;
    }
    self.close_bar.mouse_down(x, y);
    self.each_action_button(|button| button.mouse_down(x, y));
  }

  fn mouse_up(&mut self, x: f64, y: f64) {
    if self.sheet.is_visible() {
      self.sheet.mouse_up(x, y);
      return;
    }
    self.close_bar.mouse_up(x, y);
    self.each_action_button(|button| button.mouse_up(x, y));
  }

  fn mouse_move(&mut self, x: f64, y: f64) {
    self.cursor.set((x as f32, y as f32));
    self.close_bar.mouse_move(x as f32, y as f32);
    self.each_action_button(|button| button.set_hover(x as f32, y as f32));
    if self.sheet.is_visible() {
      self.sheet.child_mut().set_hover(x as f32, y as f32);
      self.sheet_ibeam.set(self.sheet.child_mut().wants_text_cursor());
    } else {
      self.sheet_ibeam.set(false);
    }
  }

  fn text(&mut self, content: &str) {
    if self.sheet.is_visible() {
      self.sheet.child_mut().text(content);
      self.sheet_ibeam.set(self.sheet.child_mut().wants_text_cursor());
    }
  }

  fn key(&mut self, key: Key) {
    if self.sheet.is_visible() {
      // ESC dismisses the sheet; anything else goes to the form fields.
      if !self.sheet.key(key) {
        self.sheet.child_mut().key(key);
      }
      self.sheet_ibeam.set(self.sheet.child_mut().wants_text_cursor());
    }
  }

  fn cursor(&self, _x: f64, _y: f64) -> CursorKind {
    if self.sheet_ibeam.get() {
      CursorKind::Text
    } else {
      CursorKind::Default
    }
  }

  fn set_focused(&mut self, focused: bool) {
    self.focused = focused;
    self.close_bar.set_focused(focused);
    self.sheet.set_focused(focused);
    self.sheet.child_mut().set_focused(focused);
  }
}

fn main() {
  lang::init();
  let app_icon = icon::display_icon();
  let app = StartPage::new(app_icon);
  if let Err(err) = run("Xcode", WINDOW_WIDTH, WINDOW_HEIGHT, app) {
    eprintln!("error: {err}");
    std::process::exit(1);
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn hit_rect_covers_edges() {
    assert!(hit_rect((10.0, 10.0, 20.0, 20.0), 10.0, 10.0));
    assert!(hit_rect((10.0, 10.0, 20.0, 20.0), 30.0, 30.0));
    assert!(hit_rect((10.0, 10.0, 20.0, 20.0), 20.0, 20.0));
    assert!(!hit_rect((10.0, 10.0, 20.0, 20.0), 9.9, 15.0));
    assert!(!hit_rect((10.0, 10.0, 20.0, 20.0), 15.0, 30.1));
  }
}
