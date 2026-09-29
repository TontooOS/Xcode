//! New-project sheet content for Xcode.
//!
//! A fixed 380x266 form drawn inside a `BasicSheet` card: title, an
//! EN/DE `SegmentedPicker` switching between the English and the German
//! app name field, an app version field, a bundle ID (organization)
//! field prefilled with `de.arlomu`, a live bundle identifier preview
//! (`{org}.{english_name}`, `AppName` while empty; English only) and a
//! `Cancel` / `Create` button row. `Cancel` raises a flag the app polls
//! to dismiss the sheet; `Create` is an example and only prints.

use std::cell::Cell;
use std::rc::Rc;

use crate::TontooUI::elements::{
  BasicText, BasicTextField, Button, ButtonShape, SegmentedPicker,
  TextForeground, TextStyle, View, BUTTON_BG_DARK, BUTTON_BG_LIGHT,
};
use crate::TontooUI::renderer::window::Key;
use crate::TontooUI::renderer::{FontSystem, ImageLoader};
use crate::TontooUI::theme::ThemeMode;
use crate::lang;
use vello::Scene;
use vello::peniko::Color;

const FORM_W: f32 = 380.0;
const FORM_H: f32 = 266.0;
const PAD: f32 = 20.0;
const LABEL_W: f32 = 104.0;
const GAP: f32 = 8.0;
const FIELD_ROW_H: f32 = 28.0;
const CONTENT_W: f32 = FORM_W - PAD * 2.0;
const FIELD_X: f32 = PAD + LABEL_W + GAP;
const FIELD_W: f32 = CONTENT_W - LABEL_W - GAP;

pub struct NewProjectForm {
  title: BasicText,
  locale: SegmentedPicker,
  name_label: BasicText,
  name_en: BasicTextField,
  name_de: BasicTextField,
  version_label: BasicText,
  version: BasicTextField,
  org_label: BasicText,
  org: BasicTextField,
  preview: BasicText,
  cancel: Button,
  create: Button,
  last_locale: usize,
  x: f32,
  y: f32,
}

impl NewProjectForm {
  pub fn new(cancel_flag: Rc<Cell<bool>>) -> Self {
    let cancel_pressed = cancel_flag.clone();
    let mut org = BasicTextField::new("de.arlomu");
    org.set_text("de.arlomu");
    Self {
      title: BasicText::new(lang::t("sheet.title")).style(TextStyle::Body),
      locale: SegmentedPicker::from_slice("", &["EN", "DE"]),
      name_label: BasicText::new(lang::t("sheet.app_name"))
        .style(TextStyle::Callout),
      name_en: BasicTextField::new(lang::t("sheet.ph_name")),
      name_de: BasicTextField::new(lang::t("sheet.ph_name")),
      version_label: BasicText::new(lang::t("sheet.app_version"))
        .style(TextStyle::Callout),
      version: BasicTextField::new(lang::t("sheet.ph_version")),
      org_label: BasicText::new(lang::t("sheet.bundle_id"))
        .style(TextStyle::Callout),
      org,
      preview: BasicText::new("de.arlomu.AppName")
        .style(TextStyle::Caption)
        .foreground(TextForeground::Secondary),
      cancel: Button::new(lang::t("sheet.cancel"))
        .shape(ButtonShape::Capsule)
        .on_press(move || cancel_pressed.set(true)),
      create: Button::new(lang::t("sheet.create"))
        .shape(ButtonShape::Capsule)
        .on_press(|| println!("create pressed (example)")),
      last_locale: 0,
      x: 0.0,
      y: 0.0,
    }
  }

  fn active_is_en(&self) -> bool {
    self.locale.selected_index() == 0
  }

  /// Live theme plus derived state. Refreshes the bundle identifier
  /// preview from the organization and the English name field and
  /// deselects the name field that just got hidden by a locale switch.
  pub fn update(&mut self, accent: Color, dark: bool, mode: ThemeMode, focused: bool) {
    self.title.set_theme(mode);
    self.title.set_focused(focused);
    for label in [&mut self.name_label, &mut self.version_label, &mut self.org_label] {
      label.set_theme(mode);
      label.set_focused(focused);
    }
    self.locale.set_theme(accent, dark);
    self.locale.set_focused(focused);
    for field in [&mut self.name_en, &mut self.name_de, &mut self.version, &mut self.org] {
      field.set_theme(accent, dark);
      field.set_focused(focused);
    }
    self.preview.set_theme(mode);
    self.preview.set_focused(focused);
    let (button_bg, button_text) = if dark {
      (BUTTON_BG_DARK, Color::WHITE)
    } else {
      (BUTTON_BG_LIGHT, Color::BLACK)
    };
    for button in [&mut self.cancel, &mut self.create] {
      button.set_palette(button_bg, button_text);
      button.set_theme(accent, dark);
      button.set_focused(focused);
    }

    let index = self.locale.selected_index();
    if index != self.last_locale {
      // The hidden field keeps no stale selection: typing always goes
      // to the visible name field.
      if self.last_locale == 0 {
        self.name_en.key(Key::Escape);
      } else {
        self.name_de.key(Key::Escape);
      }
      self.last_locale = index;
    }

    let org = self.org.text_value().trim();
    let org = if org.is_empty() { "de.arlomu" } else { org };
    let name = self.name_en.text_value().trim();
    let name = if name.is_empty() { "AppName" } else { name };
    self.preview.set_text(format!("{org}.{name}"));
  }

  pub fn wants_text_cursor(&self) -> bool {
    self.name_en.wants_text_cursor()
      || self.name_de.wants_text_cursor()
      || self.version.wants_text_cursor()
      || self.org.wants_text_cursor()
  }
}

impl View for NewProjectForm {
  fn measure(&mut self, _fonts: &mut FontSystem) -> (f32, f32) {
    (FORM_W, FORM_H)
  }

  fn place(&mut self, _fonts: &mut FontSystem, x: f32, y: f32, _w: f32, _h: f32) {
    self.x = x;
    self.y = y;
    let mut cy = y + PAD;
    self.title.place(_fonts, x + PAD, cy, CONTENT_W, 22.0);
    cy += 22.0 + 10.0;
    self.name_label.place(_fonts, x + PAD, cy + 4.0, LABEL_W, 20.0);
    self.locale.place(_fonts, x + FIELD_X, cy + 2.0, FIELD_W, 24.0);
    cy += FIELD_ROW_H + GAP;
    self.name_en.place(_fonts, x + PAD, cy, CONTENT_W, FIELD_ROW_H);
    self.name_de.place(_fonts, x + PAD, cy, CONTENT_W, FIELD_ROW_H);
    cy += FIELD_ROW_H + GAP;
    self.version_label.place(_fonts, x + PAD, cy + 4.0, LABEL_W, 20.0);
    self.version.place(_fonts, x + FIELD_X, cy, FIELD_W, FIELD_ROW_H);
    cy += FIELD_ROW_H + GAP;
    self.org_label.place(_fonts, x + PAD, cy + 4.0, LABEL_W, 20.0);
    self.org.place(_fonts, x + FIELD_X, cy, FIELD_W, FIELD_ROW_H);
    cy += FIELD_ROW_H + GAP;
    self.preview.place(_fonts, x + PAD, cy, CONTENT_W, 16.0);
    cy += 16.0 + 12.0;
    self.cancel.place(_fonts, x + PAD, cy, 90.0, 30.0);
    self.create.place(
      _fonts,
      x + FORM_W - PAD - 90.0,
      cy,
      90.0,
      30.0,
    );
  }

  fn draw(
    &mut self,
    scene: &mut Scene,
    fonts: &mut FontSystem,
    images: &mut ImageLoader<'_>,
  ) {
    self.title.draw(scene, fonts, images);
    self.name_label.draw(scene, fonts, images);
    self.locale.draw(scene, fonts, images);
    if self.active_is_en() {
      self.name_en.draw(scene, fonts, images);
    } else {
      self.name_de.draw(scene, fonts, images);
    }
    self.version_label.draw(scene, fonts, images);
    self.version.draw(scene, fonts, images);
    self.org_label.draw(scene, fonts, images);
    self.org.draw(scene, fonts, images);
    self.preview.draw(scene, fonts, images);
    self.cancel.draw(scene, fonts, images);
    self.create.draw(scene, fonts, images);
  }

  fn mouse_down(&mut self, x: f64, y: f64) {
    self.locale.mouse_down(x, y);
    if self.active_is_en() {
      self.name_en.mouse_down(x, y);
    } else {
      self.name_de.mouse_down(x, y);
    }
    self.version.mouse_down(x, y);
    self.org.mouse_down(x, y);
    self.cancel.mouse_down(x, y);
    self.create.mouse_down(x, y);
  }

  fn mouse_up(&mut self, x: f64, y: f64) {
    self.locale.mouse_up(x, y);
    self.cancel.mouse_up(x, y);
    self.create.mouse_up(x, y);
  }

  fn set_hover(&mut self, x: f32, y: f32) {
    self.locale.mouse_move(x as f64, y as f64);
    self.cancel.set_hover(x, y);
    self.create.set_hover(x, y);
  }

  fn set_focused(&mut self, focused: bool) {
    self.title.set_focused(focused);
    self.locale.set_focused(focused);
    self.name_label.set_focused(focused);
    self.name_en.set_focused(focused);
    self.name_de.set_focused(focused);
    self.version_label.set_focused(focused);
    self.version.set_focused(focused);
    self.org_label.set_focused(focused);
    self.org.set_focused(focused);
    self.preview.set_focused(focused);
    self.cancel.set_focused(focused);
    self.create.set_focused(focused);
  }

  fn text(&mut self, content: &str) {
    // Only the visible name field takes typing; the hidden locale
    // field was deselected on switch (see `update`).
    if self.active_is_en() {
      self.name_en.type_text(content);
    } else {
      self.name_de.type_text(content);
    }
    self.version.type_text(content);
    self.org.type_text(content);
  }

  fn key(&mut self, key: Key) -> bool {
    // Route to the selected field; unselected fields report false.
    let active = if self.active_is_en() {
      self.name_en.key(key)
    } else {
      self.name_de.key(key)
    };
    active || self.version.key(key) || self.org.key(key)
  }

  fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
    self
  }
}
