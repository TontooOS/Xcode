//! Xcode start page for TontooOS.
//!
//! Example start window (420x585, ~25% smaller than the Apple reference)
//! built with TontooUI on Vello/WGPU:
//! no traffic lights, only one round toolbar button with an `xmark`
//! glyph at the top left (closes the window), the centered app icon
//! from `Resources/icon.tico` rendered through CoreIcon in its normal
//! (light) variant, the `Xcode` title with a `Version 27.0` line, two
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
mod sheet;

sdk::preinclude!();

use std::cell::Cell;
use std::rc::Rc;

use TontooUI::elements::{
  Align, BasicSheet, BasicText, BasicToolbar, Button, ButtonShape, FileImage,
  HStack, ImageFit, RoundedRectangle, ShapeFill, SheetSize, TextAlignment,
  TextForeground, TextStyle, ToolbarPlacement, View, BUTTON_BG_DARK,
  BUTTON_BG_LIGHT,
};
use TontooUI::renderer::window::{
  App, CursorKind, Key, Viewport, WindowCommand, run,
};
use TontooUI::renderer::{FontSystem, ImageLoader};
use TontooUI::theme::{ThemeMode, ThemeWatcher};
use vello::Scene;
use vello::peniko::Color;

use crate::sheet::NewProjectForm;

const WINDOW_WIDTH: u32 = 420;
const WINDOW_HEIGHT: u32 = 585;
const ICON_PX: f32 = 90.0;
const ICON_RADIUS: f32 = 21.0;
const CONTENT_WIDTH: f32 = 330.0;
const TEXT_WIDTH: f32 = 330.0;
const RECENTS_BOX_H: f32 = 255.0;
const VERSION: &str = "27.0";

struct StartPage {
  close_bar: BasicToolbar,
  close_requested: Rc<Cell<bool>>,
  icon: FileImage,
  title: BasicText,
  version: BasicText,
  actions: HStack,
  empty_label: BasicText,
  box_bg: RoundedRectangle,
  sheet: BasicSheet<NewProjectForm>,
  show_sheet: Rc<Cell<bool>>,
  cancel_sheet: Rc<Cell<bool>>,
  sheet_ibeam: Cell<bool>,
  watcher: ThemeWatcher,
  focused: bool,
  bg: Color,
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

    // The recents box starts empty: no example projects, only a dim
    // placeholder line. Rows are added here once projects exist.
    let empty_label = BasicText::new(lang::t("recent.empty"))
      .style(TextStyle::Callout)
      .foreground(TextForeground::Secondary)
      .alignment(TextAlignment::Center)
      .width(CONTENT_WIDTH - 32.0);

    let sheet = BasicSheet::new(NewProjectForm::new(cancel_sheet.clone()))
      .size(SheetSize::Small);

    Self {
      close_bar,
      close_requested,
      icon: FileImage::new(app_icon, ICON_PX, ICON_PX)
        .radius(ICON_RADIUS)
        .fit(ImageFit::Cover),
      title,
      version,
      actions,
      empty_label,
      box_bg: RoundedRectangle::new(CONTENT_WIDTH, RECENTS_BOX_H, 12.0)
        .fill(BUTTON_BG_DARK),
      sheet,
      show_sheet,
      cancel_sheet,
      sheet_ibeam: Cell::new(false),
      watcher: ThemeWatcher::new(),
      focused: true,
      bg: TontooUI::renderer::window::BACKGROUND,
    }
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
    self.empty_label.set_theme(theme.mode);
    self.empty_label.set_focused(focused);

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

    // Layout: close button top left, everything else centered below.
    let (vx, vy, vw, vh) = (viewport.x, viewport.y, viewport.width, viewport.height);
    self.sheet.set_viewport(vx, vy, vw, vh);
    self.close_bar.place(fonts, vx + 12.0, vy + 12.0, 36.0, 36.0);
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
    cy += actions_h + 14.0;

    // Empty recents box with a centered dim placeholder line.
    let bx = center(CONTENT_WIDTH);
    self.box_bg.place(fonts, bx, cy, CONTENT_WIDTH, RECENTS_BOX_H);
    self.box_bg.draw(scene, fonts, images);
    let (empty_w, empty_h) = self.empty_label.measure(fonts);
    self.empty_label.place(
      fonts,
      bx + ((CONTENT_WIDTH - empty_w) / 2.0).max(0.0),
      cy + ((RECENTS_BOX_H - empty_h) / 2.0).max(0.0),
      empty_w,
      empty_h,
    );
    self.empty_label.draw(scene, fonts, images);

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
    // Background drag only while no sheet is up: an open sheet needs
    // every press for its fields, so dragging pauses until Cancel/ESC.
    if self.sheet.is_visible() {
      return None;
    }
    // The whole window background drags the app: a press anywhere
    // starts a system drag instead of a click (see `mouse_up`, which
    // re-fires releases over controls so buttons keep working).
    Some((0.0, 0.0, WINDOW_WIDTH as f32, WINDOW_HEIGHT as f32))
  }

  fn poll_window_command(&mut self) -> Option<WindowCommand> {
    if self.close_requested.take() {
      return Some(WindowCommand::Close);
    }
    None
  }

  fn mouse_down(&mut self, _x: f64, _y: f64) {
    // Unreachable for the left button while no sheet is up: the drag
    // region covers the whole window, so every press starts a system
    // drag and never reaches the controls. Clicks are synthesized in
    // `mouse_up` instead. An open sheet disables the drag region, so
    // its presses arrive here and go straight into the form.
    if self.sheet.is_visible() {
      self.sheet.mouse_down(_x, _y);
    }
  }

  fn mouse_up(&mut self, x: f64, y: f64) {
    if self.sheet.is_visible() {
      self.sheet.mouse_up(x, y);
      return;
    }
    // No control ever sees `mouse_down` (see `drag_region` / `mouse_down`),
    // so synthesize down+up at the release position: releasing over the
    // close pill or an action button fires it, releasing anywhere else
    // (a real background drag) does nothing.
    self.close_bar.mouse_down(x, y);
    self.close_bar.mouse_up(x, y);
    self.each_action_button(|button| {
      button.mouse_down(x, y);
      button.mouse_up(x, y);
    });
  }

  fn mouse_move(&mut self, x: f64, y: f64) {
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
