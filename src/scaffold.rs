//! Project scaffolding for Xcode.
//!
//! `create_project` writes a fresh TontooOS app skeleton into
//! `<parent>/<en_name>/`: `Cargo.toml`, `tontoo.proj`, `.gitignore`,
//! `src/app.rs`, `src/content_view.rs` (lowercase: Rust module
//! `content_view` only resolves to a lowercase file on Linux),
//! `lang/` (build input: TBuild reads localized names from here) plus
//! a `Resources/lang/` runtime mirror (TBuild stages `Resources/` into
//! the bundle, where the running app looks its strings up) and an
//! empty `Resources/` icon slot.

use std::path::{Path, PathBuf};

/// Required-field validation for step 1 of the sheet. Returns the lang
/// key of the error hint, or `None` when everything required is set.
pub fn validate(en_name: &str, version: &str, bundle_id: &str) -> Option<&'static str> {
  if en_name.trim().is_empty() || version.trim().is_empty() || bundle_id.trim().is_empty() {
    return Some("sheet.err_required");
  }
  if sanitize_cargo_name(en_name).is_empty() {
    return Some("sheet.err_name");
  }
  if normalize_version(version).is_none() {
    return Some("sheet.err_version");
  }
  None
}

/// Cargo needs full semver (`1.0.0`): short numeric forms are padded
/// (`1` and `1.0` become `1.0.0`), anything else is rejected.
pub fn normalize_version(raw: &str) -> Option<String> {
  let parts: Vec<&str> = raw.trim().split('.').collect();
  if parts.is_empty()
    || parts.len() > 3
    || !parts
      .iter()
      .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
  {
    return None;
  }
  let mut out = parts.join(".");
  while out.split('.').count() < 3 {
    out.push_str(".0");
  }
  Some(out)
}

/// Cargo-safe package name: lowercase ASCII, everything else becomes
/// `-`. Empty when nothing usable remains (then `validate` rejects).
pub fn sanitize_cargo_name(en_name: &str) -> String {
  let mut out = String::new();
  for c in en_name.trim().to_lowercase().chars() {
    if c.is_ascii_alphanumeric() {
      out.push(c);
    } else if c == '-' || c == '_' || c == ' ' {
      out.push('-');
    }
  }
  out.trim_matches('-').to_string()
}

/// Default location root for new projects (`~/Documents`).
pub fn documents_dir() -> PathBuf {
  if let Ok(home) = std::env::var("HOME") {
    if !home.trim().is_empty() {
      return PathBuf::from(home).join("Documents");
    }
  }
  std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn cargo_toml(cargo_name: &str, version: &str) -> String {
  format!(
    "[package]\nname = \"{cargo_name}\"\nversion = \"{version}\"\nedition = \"2021\"\n\n\n\n[[bin]]\nname = \"{cargo_name}\"\npath = \"src/app.rs\"\n\n[dependencies]\n# TontooOS SDK: frameworks via features (on-device: /Library/System/sdk).\nsdk = {{ path = \"/Library/System/sdk\", features = [\"TontooUI\", \"Foundation\", \"Accessibility\", \"CoreData\"] }}\nvello = \"0.10\"\n"
  )
}

fn tontoo_proj(bundle_id: &str, en_name: &str, version: &str) -> String {
  format!(
    "{{\n  \"bundle_id\": \"{bundle_id}\",\n  \"name\": \"{en_name}\",\n  \"version\": \"{version}\",\n  \"icon\": \"Resources/icon.tico\"\n}}\n"
  )
}

fn gitignore() -> &'static str {
  "target/\nCargo.lock\n"
}

fn app_rs(en_name: &str) -> String {
  format!(
    "//! {en_name} - built with Xcode.\n\nmod content_view;\n\nsdk::preinclude!();\n\nuse TontooUI::elements::{{Titlebar, TrafficAction}};\nuse TontooUI::renderer::window::{{App, Viewport, WindowCommand, run}};\nuse TontooUI::renderer::{{FontSystem, ImageLoader}};\nuse TontooUI::theme::ThemeWatcher;\nuse vello::Scene;\nuse vello::peniko::Color;\n\nuse content_view::ContentView;\n\nstruct {en_name}App {{\n  bar: Titlebar,\n  card: ContentView,\n  watcher: ThemeWatcher,\n  focused: bool,\n  bg: Color,\n  command: Option<WindowCommand>,\n}}\n\nimpl App for {en_name}App {{\n  fn draw(\n    &mut self,\n    scene: &mut Scene,\n    fonts: &mut FontSystem,\n    images: &mut ImageLoader<'_>,\n    viewport: Viewport,\n    time_secs: f64,\n  ) {{\n    self.watcher.poll(time_secs);\n    self.watcher.set_focused(self.focused, time_secs);\n    let palette = self.watcher.palette(time_secs);\n    self.bg = palette.bg;\n    self.bar.set_palette(palette.titlebar_bg, palette.titlebar_text, palette.divider);\n    self.bar.set_rect(viewport.x, viewport.y, viewport.width);\n    self.bar.draw(scene, fonts);\n    let top = viewport.y + 31.0;\n    let (w, h) = self.card.measure(fonts);\n    let x = viewport.x + ((viewport.width - w) / 2.0).max(0.0);\n    self.card.place(fonts, x, top + 40.0, w, h);\n    self.card.draw(scene, fonts, images);\n  }}\n\n  fn background(&self) -> Color {{\n    self.bg\n  }}\n\n  fn drag_region(&self) -> Option<(f32, f32, f32, f32)> {{\n    Some(self.bar.drag_rect())\n  }}\n\n  fn poll_window_command(&mut self) -> Option<WindowCommand> {{\n    self.command.take()\n  }}\n\n  fn mouse_down(&mut self, x: f64, y: f64) {{\n    match self.bar.press(x as f32, y as f32) {{\n      Some(TrafficAction::Close) => self.command = Some(WindowCommand::Close),\n      Some(TrafficAction::Minimize) => self.command = Some(WindowCommand::Minimize),\n      Some(TrafficAction::Maximize) => self.command = Some(WindowCommand::ToggleMaximize),\n      None => self.card.mouse_down(x, y),\n    }}\n  }}\n\n  fn mouse_up(&mut self, x: f64, y: f64) {{\n    self.card.mouse_up(x, y);\n  }}\n\n  fn mouse_move(&mut self, x: f64, y: f64) {{\n    self.bar.set_hover(x as f32, y as f32);\n    self.card.set_hover(x, y);\n  }}\n\n  fn set_focused(&mut self, focused: bool) {{\n    self.focused = focused;\n    self.bar.set_focused(focused);\n  }}\n}}\n\nfn main() {{\n  let app = {en_name}App {{\n    bar: Titlebar::new(\"{en_name}\"),\n    card: ContentView::new(),\n    watcher: ThemeWatcher::new(),\n    focused: true,\n    bg: TontooUI::renderer::window::BACKGROUND,\n    command: None,\n  }};\n  if let Err(err) = run(\"{en_name}\", 800, 600, app) {{\n    eprintln!(\"error: {{err}}\");\n    std::process::exit(1);\n  }}\n}}\n"
  )
}

fn content_view_rs() -> &'static str {
  "//! Example content card: Hello World text plus a button.\n\nuse crate::TontooUI::elements::{\n  Align, BasicText, Button, ButtonShape, TextStyle, View, VStack,\n};\nuse crate::TontooUI::renderer::{FontSystem, ImageLoader};\nuse vello::Scene;\n\npub struct ContentView {\n  stack: VStack,\n}\n\nimpl ContentView {\n  pub fn new() -> Self {\n    Self {\n      stack: VStack::new()\n        .spacing(12.0)\n        .align(Align::Center)\n        .child(BasicText::new(\"Hello World\").style(TextStyle::Title2))\n        .child(\n          Button::new(\"Tap Me\")\n            .shape(ButtonShape::Capsule)\n            .on_press(|| println!(\"tapped\")),\n        ),\n    }\n  }\n\n  pub fn measure(&mut self, fonts: &mut FontSystem) -> (f32, f32) {\n    self.stack.measure(fonts)\n  }\n\n  pub fn place(&mut self, fonts: &mut FontSystem, x: f32, y: f32, w: f32, h: f32) {\n    self.stack.place(fonts, x, y, w, h);\n  }\n\n  pub fn draw(&mut self, scene: &mut Scene, fonts: &mut FontSystem, images: &mut ImageLoader<'_>) {\n    self.stack.draw(scene, fonts, images);\n  }\n\n  pub fn mouse_down(&mut self, x: f64, y: f64) {\n    for index in 0..self.stack.len() {\n      if let Some(button) = self.stack.child_mut::<Button>(index) {\n        button.mouse_down(x, y);\n      }\n    }\n  }\n\n  pub fn mouse_up(&mut self, x: f64, y: f64) {\n    for index in 0..self.stack.len() {\n      if let Some(button) = self.stack.child_mut::<Button>(index) {\n        button.mouse_up(x, y);\n      }\n    }\n  }\n\n  pub fn set_hover(&mut self, x: f64, y: f64) {\n    for index in 0..self.stack.len() {\n      if let Some(button) = self.stack.child_mut::<Button>(index) {\n        button.set_hover(x as f32, y as f32);\n      }\n    }\n  }\n}\n"
}

fn lang_json(lang: &str, name: &str) -> String {
  format!(
    "{{\n  \"lang\": \"{lang}\",\n  \"translations\": {{\n    \"name\": \"{name}\"\n  }}\n}}\n"
  )
}

/// Write the skeleton into `<parent>/<en_name>/`. Fails when the
/// target exists or any write fails. Returns the project root.
pub fn create_project(
  parent: &Path,
  en_name: &str,
  de_name: &str,
  version: &str,
  bundle_id: &str,
) -> Result<PathBuf, String> {
  let en_name = en_name.trim();
  let de_name = de_name.trim();
  let de_name = if de_name.is_empty() { en_name } else { de_name };
  let version = normalize_version(version).ok_or("sheet.err_version")?;
  let bundle_id = bundle_id.trim();
  if let Some(key) = validate(en_name, &version, bundle_id) {
    return Err(key.to_string());
  }
  let cargo_name = sanitize_cargo_name(en_name);
  // Struct name for the generated app: ASCII alphanumeric, capitalized.
  let struct_name: String = en_name
    .chars()
    .filter(|c| c.is_ascii_alphanumeric())
    .collect();
  let struct_name = if struct_name.is_empty() {
    "MyApp".to_string()
  } else {
    struct_name
  };

  let root = parent.join(en_name);
  if root.exists() {
    return Err("sheet.err_exists".to_string());
  }
  let write = |path: PathBuf, content: &str| -> Result<(), String> {
    std::fs::write(&path, content)
      .map_err(|e| format!("cannot write {}: {e}", path.display()))
  };
  std::fs::create_dir_all(root.join("src"))
    .map_err(|e| format!("cannot create src: {e}"))?;
  std::fs::create_dir_all(root.join("Resources").join("lang"))
    .map_err(|e| format!("cannot create Resources/lang: {e}"))?;
  std::fs::create_dir_all(root.join("lang"))
    .map_err(|e| format!("cannot create lang: {e}"))?;
  write(root.join("Cargo.toml"), &cargo_toml(&cargo_name, &version))?;
  write(root.join("tontoo.proj"), &tontoo_proj(bundle_id, en_name, &version))?;
  write(root.join(".gitignore"), gitignore())?;
  write(root.join("src").join("app.rs"), &app_rs(&struct_name))?;
  write(root.join("src").join("content_view.rs"), content_view_rs())?;
  write(root.join("lang").join("en_us.json"), &lang_json("en_us", en_name))?;
  write(root.join("lang").join("de_de.json"), &lang_json("de_de", de_name))?;
  // Runtime mirror inside the staged bundle (same content as `lang/`).
  write(
    root.join("Resources").join("lang").join("en_us.json"),
    &lang_json("en_us", en_name),
  )?;
  write(
    root.join("Resources").join("lang").join("de_de.json"),
    &lang_json("de_de", de_name),
  )?;
  Ok(root)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn validation_requires_fields() {
    assert_eq!(validate("", "1.0", "de.x"), Some("sheet.err_required"));
    assert_eq!(validate("A", "", "de.x"), Some("sheet.err_required"));
    assert_eq!(validate("A", "1.0", ""), Some("sheet.err_required"));
    assert_eq!(validate("!!!", "1.0", "de.x"), Some("sheet.err_name"));
    assert_eq!(validate("A", "1.0", "de.x"), None);
    assert_eq!(validate("My App", "1.0", "de.arlomu"), None);
    assert_eq!(validate("A", "x.y", "de.x"), Some("sheet.err_version"));
    assert_eq!(validate("A", "1.0.0.0", "de.x"), Some("sheet.err_version"));
  }

  #[test]
  fn version_normalizes_to_semver() {
    assert_eq!(normalize_version("1"), Some("1.0.0".to_string()));
    assert_eq!(normalize_version("1.0"), Some("1.0.0".to_string()));
    assert_eq!(normalize_version("27.0.0"), Some("27.0.0".to_string()));
    assert_eq!(normalize_version("x"), None);
    assert_eq!(normalize_version("1.0.0.0"), None);
  }

  #[test]
  fn cargo_name_is_lowercase_safe() {
    assert_eq!(sanitize_cargo_name("My Cool App!"), "my-cool-app");
    assert_eq!(sanitize_cargo_name("  spaced  "), "spaced");
    assert_eq!(sanitize_cargo_name("!!!"), "");
  }

  #[test]
  fn scaffold_writes_all_files() {
    let parent = std::env::temp_dir().join(format!("xcode-scaffold-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&parent);
    std::fs::create_dir_all(&parent).unwrap();
    let root = create_project(&parent, "My App", "Meine App", "1.0", "de.arlomu.myapp")
      .expect("scaffold");
    assert_eq!(root, parent.join("My App"));
    for rel in [
      "Cargo.toml",
      "tontoo.proj",
      ".gitignore",
      "src/app.rs",
      "src/content_view.rs",
      "lang/en_us.json",
      "lang/de_de.json",
      "Resources/lang/en_us.json",
      "Resources/lang/de_de.json",
    ] {
      assert!(root.join(rel).is_file(), "missing {rel}");
    }
    assert!(root.join("Resources").is_dir());
    let cargo = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(cargo.contains("name = \"my-app\""));
    assert!(cargo.contains("version = \"1.0.0\""));
    assert!(cargo.contains("path = \"src/app.rs\""));
    let proj = std::fs::read_to_string(root.join("tontoo.proj")).unwrap();
    assert!(proj.contains("\"bundle_id\": \"de.arlomu.myapp\""));
    assert!(proj.contains("\"name\": \"My App\""));
    assert!(proj.contains("\"version\": \"1.0.0\""));
    assert!(proj.contains("\"icon\": \"Resources/icon.tico\""));
    let de = std::fs::read_to_string(root.join("lang/de_de.json")).unwrap();
    assert!(de.contains("\"name\": \"Meine App\""));
    let app = std::fs::read_to_string(root.join("src/app.rs")).unwrap();
    // Module and file agree in lowercase (Linux is case-sensitive).
    assert!(app.contains("mod content_view;"));
    assert!(app.contains("use content_view::ContentView;"));
    assert!(!app.contains("BasicText"), "unused template import");
    let view = std::fs::read_to_string(root.join("src/content_view.rs")).unwrap();
    assert!(view.contains("Hello World"));
    assert!(view.contains("Tap Me"));
    // Child module addresses the preincluded frameworks by crate path.
    assert!(view.contains("crate::TontooUI"));
    // Second run on the same name refuses (target exists).
    assert_eq!(
      create_project(&parent, "My App", "", "1.0", "de.x"),
      Err("sheet.err_exists".to_string())
    );
    // Keep the tree for manual end-to-end checks (build it with cargo
    // in WSL): `XCODE_KEEP_SCAFFOLD=1 cargo test scaffold_writes`.
    if std::env::var("XCODE_KEEP_SCAFFOLD").is_ok() {
      eprintln!("scaffold kept at {}", root.display());
    } else {
      let _ = std::fs::remove_dir_all(&parent);
    }
  }
}
