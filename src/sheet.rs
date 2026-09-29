//! New-project sheet content for Xcode.
//!
//! A `BasicSheet` card with two steps. Step 1 is the 380x292 options
//! form: title, an EN/DE `SegmentedPicker` switching between the
//! English and the German app name field, an app version field, a
//! bundle ID (organization) field prefilled with `dev.<username>`, a
//! live bundle identifier preview (`{org}.{english_name}`, `AppName` while
//! empty; English only), an error line and a `Cancel` / `Create`
//! button row. `Create` validates (EN name, version and bundle ID are
//! required) and moves to step 2. Step 2 is a folder chooser rooted at
//! `~/Documents`: subdirectory buttons plus `..`, a target preview
//! (`<dir>/<en_name>`) and `Cancel` / `Create`; `Create` scaffolds the
//! project, stores the record for the app and closes the sheet.
//! `Cancel` raises a flag the app polls to dismiss the sheet.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use crate::TontooUI::elements::{
  BasicText, BasicTextField, Button, ButtonShape, ButtonStyle, SegmentedPicker,
  TextForeground, TextStyle, View, BUTTON_BG_DARK, BUTTON_BG_LIGHT,
};
use crate::TontooUI::renderer::window::Key;
use crate::TontooUI::renderer::{FontSystem, ImageLoader};
use crate::TontooUI::theme::ThemeMode;
use crate::lang;
use crate::projects::{ProjectRecord, save_project};
use crate::scaffold::{create_project, default_bundle_id, documents_dir, validate};
use vello::Scene;
use vello::peniko::Color;

/// Error text color (system red).
const ERROR_RED: Color = Color::from_rgb8(0xff, 0x3b, 0x30);
/// Visible subdirectory buttons per folder page (no scrolling yet).
const MAX_ENTRIES: usize = 8;

const FORM_W: f32 = 380.0;
const FORM_H: f32 = 292.0;
const PAD: f32 = 20.0;
const LABEL_W: f32 = 104.0;
const GAP: f32 = 8.0;
const FIELD_ROW_H: f32 = 28.0;
const CONTENT_W: f32 = FORM_W - PAD * 2.0;
const FIELD_X: f32 = PAD + LABEL_W + GAP;
const FIELD_W: f32 = CONTENT_W - LABEL_W - GAP;
const ENTRY_H: f32 = 26.0;
const ENTRY_GAP: f32 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
  Form,
  Location,
}

#[derive(Clone, Copy, Debug)]
enum Nav {
  Up,
  Into(usize),
}

pub struct NewProjectForm {
  step: Step,
  title: BasicText,
  locale: SegmentedPicker,
  name_label: BasicText,
  name_en: BasicTextField,
  name_de: BasicTextField,
  version_label: BasicText,
  version: BasicTextField,
  org_label: BasicText,
  org: BasicTextField,
  org_default: String,
  preview: BasicText,
  error: BasicText,
  cancel: Button,
  create: Button,
  // Step 2 state.
  loc_title: BasicText,
  loc_path: BasicText,
  up_button: Button,
  dir_buttons: Vec<Button>,
  loc_target: BasicText,
  cwd: PathBuf,
  entries: Vec<String>,
  nav_flag: Rc<Cell<Option<Nav>>>,
  create_flag: Rc<Cell<bool>>,
  created: Rc<RefCell<Option<ProjectRecord>>>,
  cancel_flag: Rc<Cell<bool>>,
  last_locale: usize,
  x: f32,
  y: f32,
}

impl NewProjectForm {
  pub fn new(
    cancel_flag: Rc<Cell<bool>>,
    created: Rc<RefCell<Option<ProjectRecord>>>,
  ) -> Self {
    let cancel_pressed = cancel_flag.clone();
    let create_flag = Rc::new(Cell::new(false));
    let create_pressed = create_flag.clone();
    let nav_flag: Rc<Cell<Option<Nav>>> = Rc::new(Cell::new(None));
    let up_nav = nav_flag.clone();
    let org_default = default_bundle_id();
    let org_preview = format!("{org_default}.AppName");
    let mut org = BasicTextField::new(org_default.clone());
    org.set_text(org_default.clone());
    Self {
      step: Step::Form,
      title: BasicText::new(lang::t("sheet.title")).style(TextStyle::Body),
      locale: SegmentedPicker::from_slice("", &["EN", "DE"]),
      name_label: BasicText::new(lang::t("sheet.app_name")).style(TextStyle::Callout),
      name_en: BasicTextField::new(lang::t("sheet.ph_name")),
      name_de: BasicTextField::new(lang::t("sheet.ph_name")),
      version_label: BasicText::new(lang::t("sheet.app_version")).style(TextStyle::Callout),
      version: BasicTextField::new(lang::t("sheet.ph_version")),
      org_label: BasicText::new(lang::t("sheet.bundle_id")).style(TextStyle::Callout),
      org,
      org_default,
      preview: BasicText::new(org_preview)
        .style(TextStyle::Caption)
        .foreground(TextForeground::Secondary),
      error: BasicText::new("").style(TextStyle::Caption).foreground_color(ERROR_RED),
      cancel: Button::new(lang::t("sheet.cancel"))
        .shape(ButtonShape::Capsule)
        .on_press(move || cancel_pressed.set(true)),
      create: Button::new(lang::t("sheet.create"))
        .shape(ButtonShape::Capsule)
        .on_press(move || create_pressed.set(true)),
      loc_title: BasicText::new(lang::t("location.title")).style(TextStyle::Body),
      loc_path: BasicText::new(String::new())
        .style(TextStyle::Caption)
        .foreground(TextForeground::Secondary),
      up_button: Button::new("..")
        .style(ButtonStyle::Plain)
        .on_press(move || up_nav.set(Some(Nav::Up))),
      dir_buttons: Vec::new(),
      loc_target: BasicText::new(String::new())
        .style(TextStyle::Caption)
        .foreground(TextForeground::Secondary),
      cwd: PathBuf::new(),
      entries: Vec::new(),
      nav_flag,
      create_flag,
      created,
      cancel_flag,
      last_locale: 0,
      x: 0.0,
      y: 0.0,
    }
  }

  fn active_is_en(&self) -> bool {
    self.locale.selected_index() == 0
  }

  /// Back to the options form (keeps field contents).
  pub fn reset(&mut self) {
    self.step = Step::Form;
    self.error.set_text(String::new());
  }

  /// Take a freshly scaffolded record for the recents list, if any.
  pub fn take_created(&mut self) -> Option<ProjectRecord> {
    self.created.borrow_mut().take()
  }

  fn refresh_entries(&mut self) {
    let _ = std::fs::create_dir_all(&self.cwd);
    let mut names: Vec<String> = Vec::new();
    if let Ok(read) = std::fs::read_dir(&self.cwd) {
      for entry in read.flatten() {
        let path = entry.path();
        let is_dir = path.is_dir();
        if is_dir {
          if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
            if !name.starts_with('.') {
              names.push(name.to_string());
            }
          }
        }
      }
    }
    names.sort();
    names.truncate(MAX_ENTRIES);
    self.entries = names;
    self.dir_buttons.clear();
    for (index, name) in self.entries.iter().enumerate() {
      let nav = self.nav_flag.clone();
      self.dir_buttons.push(
        Button::new(name.clone())
          .style(ButtonStyle::Plain)
          .icon("folder.fill")
          .on_press(move || nav.set(Some(Nav::Into(index)))),
      );
    }
  }

  fn short_path(path: &str) -> String {
    if path.chars().count() > 48 {
      let chars: Vec<char> = path.chars().collect();
      let head: String = chars[..20].iter().collect();
      let tail: String = chars[chars.len() - 27..].iter().collect();
      format!("{head}…{tail}")
    } else {
      path.to_string()
    }
  }

  /// Live theme plus derived state. Consumes the create flag: on step 1
  /// it validates and enters the folder chooser, on step 2 it
  /// scaffolds the project into the chosen folder.
  pub fn update(&mut self, accent: Color, dark: bool, mode: ThemeMode, focused: bool) {
    self.title.set_theme(mode);
    self.title.set_focused(focused);
    self.loc_title.set_theme(mode);
    self.loc_title.set_focused(focused);
    for label in [
      &mut self.name_label,
      &mut self.version_label,
      &mut self.org_label,
    ] {
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
    self.loc_path.set_theme(mode);
    self.loc_path.set_focused(focused);
    self.loc_target.set_theme(mode);
    self.loc_target.set_focused(focused);
    // The error line keeps its hand-set red color (no `set_theme`).
    self.error.set_focused(focused);
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
    self.up_button.set_palette(button_bg, button_text);
    self.up_button.set_theme(accent, dark);
    self.up_button.set_focused(focused);
    for button in self.dir_buttons.iter_mut() {
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

    // Folder navigation from the chooser buttons.
    if let Some(nav) = self.nav_flag.take() {
      match nav {
        Nav::Up => {
          if let Some(parent) = self.cwd.parent() {
            self.cwd = parent.to_path_buf();
          }
        }
        Nav::Into(i) => {
          if let Some(name) = self.entries.get(i) {
            let next = self.cwd.join(name);
            if next.is_dir() {
              self.cwd = next;
            }
          }
        }
      }
      self.refresh_entries();
    }

    // Bundle identifier preview (English only).
    let org = self.org.text_value().trim().to_string();
    let org_show = if org.is_empty() {
      self.org_default.clone()
    } else {
      org
    };
    let name = self.name_en.text_value().trim().to_string();
    let name_show = if name.is_empty() { "AppName".to_string() } else { name };
    self.preview.set_text(format!("{org_show}.{name_show}"));

    if self.step == Step::Location {
      self.loc_path.set_text(Self::short_path(&self.cwd.to_string_lossy()));
      let target = self.cwd.join(name_show.trim());
      self.loc_target.set_text(format!(
        "{} {}",
        lang::t("location.new_folder"),
        Self::short_path(&target.to_string_lossy())
      ));
    }

    // Create button: validate on step 1, scaffold on step 2.
    if self.create_flag.take() {
      match self.step {
        Step::Form => {
          let en = self.name_en.text_value().trim().to_string();
          let version = self.version.text_value().trim().to_string();
          let org = self.org.text_value().trim().to_string();
          match validate(&en, &version, &org) {
            Some(key) => self.error.set_text(lang::t(key)),
            None => {
              self.error.set_text(String::new());
              self.step = Step::Location;
              self.cwd = documents_dir();
              self.refresh_entries();
            }
          }
        }
        Step::Location => {
          let en = self.name_en.text_value().trim().to_string();
          let de = self.name_de.text_value().trim().to_string();
          let version = self.version.text_value().trim().to_string();
          let org = self.org.text_value().trim().to_string();
          match create_project(&self.cwd, &en, &de, &version, &org) {
            Ok(root) => {
              self.error.set_text(String::new());
              let record = ProjectRecord::new(
                en,
                de,
                version,
                org,
                root.to_string_lossy().to_string(),
              );
              // Persist for restarts; the folder already exists, so a
              // store failure only logs and the project still shows
              // until quit.
              if let Err(err) = save_project(&record) {
                eprintln!("projects: {err}");
              }
              *self.created.borrow_mut() = Some(record);
              self.cancel_flag.set(true);
            }
            Err(key) => self.error.set_text(lang::t(&key)),
          }
        }
      }
    }
  }

  pub fn wants_text_cursor(&self) -> bool {
    self.step == Step::Form
      && (self.name_en.wants_text_cursor()
        || self.name_de.wants_text_cursor()
        || self.version.wants_text_cursor()
        || self.org.wants_text_cursor())
  }

  fn loc_height(&self) -> f32 {
    PAD + 22.0 + 10.0 + 16.0 + GAP + ENTRY_H + ENTRY_GAP
      + self.entries.len() as f32 * (ENTRY_H + ENTRY_GAP)
      + (GAP - ENTRY_GAP) + 16.0 + GAP + 30.0 + PAD
  }
}

impl View for NewProjectForm {
  fn measure(&mut self, _fonts: &mut FontSystem) -> (f32, f32) {
    match self.step {
      Step::Form => (FORM_W, FORM_H),
      Step::Location => (FORM_W, self.loc_height()),
    }
  }

  fn place(&mut self, _fonts: &mut FontSystem, x: f32, y: f32, _w: f32, _h: f32) {
    self.x = x;
    self.y = y;
    match self.step {
      Step::Form => {
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
        cy += 16.0 + 6.0;
        self.error.place(_fonts, x + PAD, cy, CONTENT_W, 14.0);
        cy += 14.0 + 10.0;
        self.cancel.place(_fonts, x + PAD, cy, 90.0, 30.0);
        self.create.place(_fonts, x + FORM_W - PAD - 90.0, cy, 90.0, 30.0);
      }
      Step::Location => {
        let mut cy = y + PAD;
        self.loc_title.place(_fonts, x + PAD, cy, CONTENT_W, 22.0);
        cy += 22.0 + 10.0;
        self.loc_path.place(_fonts, x + PAD, cy, CONTENT_W, 16.0);
        cy += 16.0 + GAP;
        self.up_button.place(_fonts, x + PAD, cy, CONTENT_W, ENTRY_H);
        cy += ENTRY_H + ENTRY_GAP;
        for button in self.dir_buttons.iter_mut() {
          button.place(_fonts, x + PAD, cy, CONTENT_W, ENTRY_H);
          cy += ENTRY_H + ENTRY_GAP;
        }
        cy += GAP - ENTRY_GAP;
        self.loc_target.place(_fonts, x + PAD, cy, CONTENT_W, 16.0);
        cy += 16.0 + GAP;
        self.cancel.place(_fonts, x + PAD, cy, 90.0, 30.0);
        self.create.place(_fonts, x + FORM_W - PAD - 90.0, cy, 90.0, 30.0);
      }
    }
  }

  fn draw(
    &mut self,
    scene: &mut Scene,
    fonts: &mut FontSystem,
    images: &mut ImageLoader<'_>,
  ) {
    match self.step {
      Step::Form => {
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
        self.error.draw(scene, fonts, images);
        self.cancel.draw(scene, fonts, images);
        self.create.draw(scene, fonts, images);
      }
      Step::Location => {
        self.loc_title.draw(scene, fonts, images);
        self.loc_path.draw(scene, fonts, images);
        self.up_button.draw(scene, fonts, images);
        for button in self.dir_buttons.iter_mut() {
          button.draw(scene, fonts, images);
        }
        self.loc_target.draw(scene, fonts, images);
        self.cancel.draw(scene, fonts, images);
        self.create.draw(scene, fonts, images);
      }
    }
  }

  fn mouse_down(&mut self, x: f64, y: f64) {
    match self.step {
      Step::Form => {
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
      Step::Location => {
        self.up_button.mouse_down(x, y);
        for button in self.dir_buttons.iter_mut() {
          button.mouse_down(x, y);
        }
        self.cancel.mouse_down(x, y);
        self.create.mouse_down(x, y);
      }
    }
  }

  fn mouse_up(&mut self, x: f64, y: f64) {
    self.locale.mouse_up(x, y);
    self.up_button.mouse_up(x, y);
    for button in self.dir_buttons.iter_mut() {
      button.mouse_up(x, y);
    }
    self.cancel.mouse_up(x, y);
    self.create.mouse_up(x, y);
  }

  fn set_hover(&mut self, x: f32, y: f32) {
    self.locale.mouse_move(x as f64, y as f64);
    self.up_button.set_hover(x, y);
    for button in self.dir_buttons.iter_mut() {
      button.set_hover(x, y);
    }
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
    self.error.set_focused(focused);
    self.cancel.set_focused(focused);
    self.create.set_focused(focused);
    self.loc_title.set_focused(focused);
    self.loc_path.set_focused(focused);
    self.up_button.set_focused(focused);
    for button in self.dir_buttons.iter_mut() {
      button.set_focused(focused);
    }
    self.loc_target.set_focused(focused);
  }

  fn text(&mut self, content: &str) {
    if self.step != Step::Form {
      return;
    }
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
    if self.step != Step::Form {
      return false;
    }
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
