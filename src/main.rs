//! Xcode start page for TontooOS.
//!
//! Example start window (560x780) built with TontooUI on Vello/WGPU:
//! no traffic lights, only one round toolbar button with an `xmark`
//! glyph at the top left (closes the window), the centered app icon
//! from `Resources/icon.tico` rendered through CoreIcon in its normal
//! (light) variant, the `Xcode` title with a `Version 27.0` line,
//! three example capsule buttons (`Open...`, `Clone...`, `New Project`)
//! and a static recents box mirroring the reference screenshot
//! (SwiftIU, Tux, Tux.zip, C Maps with the first row selected).
//!
//! All text uses SF Pro (system font) with `en_us` and `de_de` strings
//! from `lang/` via Accessibility. The theme follows the settings
//! daemon live through `ThemeWatcher` (Dark `#1B2022` / Light
//! `#FFFFFF`).

mod icon;
mod lang;

sdk::preinclude!();

use std::cell::Cell;
use std::rc::Rc;

use TontooUI::elements::{
  Align, BasicText, BasicToolbar, Button, ButtonShape, FileImage, HStack,
  HorizontalDivider, ImageFit, RoundedRectangle, SFSymbolImage, ShapeFill,
  TextAlignment, TextForeground, TextStyle, ToolbarPlacement, View, VStack,
  BUTTON_BG_DARK, BUTTON_BG_LIGHT,
};
use TontooUI::renderer::window::{App, Viewport, WindowCommand, run};
use TontooUI::renderer::{FontSystem, ImageLoader};
use TontooUI::theme::{ThemeMode, ThemeWatcher};
use vello::Scene;
use vello::peniko::Color;

const WINDOW_WIDTH: u32 = 560;
const WINDOW_HEIGHT: u32 = 780;
const ICON_PX: f32 = 120.0;
const ICON_RADIUS: f32 = 28.0;
const CONTENT_WIDTH: f32 = 440.0;
const TEXT_WIDTH: f32 = 440.0;
const ROW_H: f32 = 64.0;
const DIV_H: f32 = 1.0;
const BOX_PAD: f32 = 8.0;
const ROW_PAD_X: f32 = 12.0;
const ROW_ICON: f32 = 40.0;
const RECENTS_BOX_H: f32 = 340.0;
const VERSION: &str = "27.0";

/// One static recent row: icon plus name/path text column.
fn recent_row(symbol: &str, name: String, path: String) -> HStack {
  let texts = VStack::new()
    .spacing(2.0)
    .align(Align::Leading)
    .child(BasicText::new(name).style(TextStyle::Callout))
    .child(
      BasicText::new(path)
        .style(TextStyle::Caption)
        .foreground(TextForeground::Secondary),
    );
  HStack::new()
    .spacing(12.0)
    .align(Align::Center)
    .child(SFSymbolImage::new(symbol).size(ROW_ICON))
    .child(texts)
}

struct StartPage {
  close_bar: BasicToolbar,
  close_requested: Rc<Cell<bool>>,
  icon: FileImage,
  title: BasicText,
  version: BasicText,
  actions: HStack,
  rows: Vec<HStack>,
  dividers: Vec<HorizontalDivider>,
  box_bg: RoundedRectangle,
  selection_bg: RoundedRectangle,
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

    let actions = HStack::new()
      .spacing(12.0)
      .align(Align::Center)
      .child(
        Button::new(lang::t("action.open"))
          .shape(ButtonShape::Capsule)
          .on_press(|| println!("open pressed (example)")),
      )
      .child(
        Button::new(lang::t("action.clone"))
          .shape(ButtonShape::Capsule)
          .on_press(|| println!("clone pressed (example)")),
      )
      .child(
        Button::new(lang::t("action.new_project"))
          .shape(ButtonShape::Capsule)
          .icon("chevron.down")
          .on_press(|| println!("new project pressed (example)")),
      );

    let rows = vec![
      recent_row(
        "hammer.fill",
        lang::t("recent.swiftiu"),
        lang::t("recent.swiftiu_path"),
      ),
      recent_row("doc.fill", lang::t("recent.tux"), lang::t("recent.tux_path")),
      recent_row(
        "archivebox.fill",
        lang::t("recent.tux_zip"),
        lang::t("recent.tux_zip_path"),
      ),
      recent_row(
        "doc.fill",
        lang::t("recent.cmaps"),
        lang::t("recent.cmaps_path"),
      ),
    ];
    let dividers = vec![
      HorizontalDivider::new(),
      HorizontalDivider::new(),
      HorizontalDivider::new(),
    ];

    Self {
      close_bar,
      close_requested,
      icon: FileImage::new(app_icon, ICON_PX, ICON_PX)
        .radius(ICON_RADIUS)
        .fit(ImageFit::Cover),
      title,
      version,
      actions,
      rows,
      dividers,
      box_bg: RoundedRectangle::new(CONTENT_WIDTH, RECENTS_BOX_H, 16.0)
        .fill(BUTTON_BG_DARK),
      selection_bg: RoundedRectangle::new(CONTENT_WIDTH, ROW_H, 10.0)
        .fill(Color::from_rgb8(0x00, 0x7a, 0xff)),
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
    self.selection_bg.set_fill(ShapeFill::Solid(palette.accent));
    self.selection_bg.set_focused(focused);

    // Recent rows: first row is selected (white text/icon on the
    // accent fill), the rest follow the theme. Static example data:
    // no click handling, display only.
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

    // Layout: close button top left, everything else centered below.
    let (vx, vy, vw) = (viewport.x, viewport.y, viewport.width);
    self.close_bar.place(fonts, vx + 16.0, vy + 16.0, 36.0, 36.0);
    self.close_bar.draw(scene, fonts, images);

    let mut cy = vy + 80.0;
    let center = |w: f32| vx + ((vw - w) / 2.0).max(0.0);

    let (icon_w, icon_h) = self.icon.measure(fonts);
    self.icon.place(fonts, center(icon_w), cy, icon_w, icon_h);
    self.icon.draw(scene, fonts, images);
    cy += icon_h + 12.0;

    let (title_w, title_h) = self.title.measure(fonts);
    self.title.place(fonts, center(title_w), cy, title_w, title_h);
    self.title.draw(scene, fonts, images);
    cy += title_h + 4.0;

    let (version_w, version_h) = self.version.measure(fonts);
    self.version.place(fonts, center(version_w), cy, version_w, version_h);
    self.version.draw(scene, fonts, images);
    cy += version_h + 16.0;

    let (actions_w, actions_h) = self.actions.measure(fonts);
    self.actions.place(fonts, center(actions_w), cy, actions_w, actions_h);
    self.actions.draw(scene, fonts, images);
    cy += actions_h + 18.0;

    // Recents box with the selected first row on accent fill.
    let bx = center(CONTENT_WIDTH);
    self.box_bg.place(fonts, bx, cy, CONTENT_WIDTH, RECENTS_BOX_H);
    self.box_bg.draw(scene, fonts, images);
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
        divider.place(fonts, bx + BOX_PAD + ROW_PAD_X, ry, row_w, DIV_H);
        divider.draw(scene, fonts, images);
        ry += DIV_H;
      }
    }
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
    // Top strip drags the window; the close pill area is excluded so
    // its clicks never start a window drag.
    Some((60.0, 0.0, 500.0, 56.0))
  }

  fn poll_window_command(&mut self) -> Option<WindowCommand> {
    if self.close_requested.take() {
      return Some(WindowCommand::Close);
    }
    None
  }

  fn mouse_down(&mut self, x: f64, y: f64) {
    self.close_bar.mouse_down(x, y);
    self.each_action_button(|button| button.mouse_down(x, y));
  }

  fn mouse_up(&mut self, x: f64, y: f64) {
    self.close_bar.mouse_up(x, y);
    self.each_action_button(|button| button.mouse_up(x, y));
  }

  fn mouse_move(&mut self, x: f64, y: f64) {
    self.close_bar.mouse_move(x as f32, y as f32);
    self.each_action_button(|button| button.set_hover(x as f32, y as f32));
  }

  fn set_focused(&mut self, focused: bool) {
    self.focused = focused;
    self.close_bar.set_focused(focused);
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
