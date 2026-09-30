//! Editable Rust code page for Xcode (user-approved custom element).
//!
//! TontooUI ships plain `TextEditor` elements but no code editor with
//! live Rust highlighting, so this file owns a small custom `View`
//! built from TontooUI primitives (`VStack` plus `HStack` rows with
//! `BasicText` gutters and `FormattedText` code) plus DocumentKit
//! spans and a caret rect. Behavior follows a normal text field:
//! click inside focuses with a caret, typing inserts, `Backspace`
//! deletes, `Enter` splits the line, arrows move, `ESC` unfocuses,
//! hover shows the I-beam cursor. Edits stay in memory only.

use std::any::Any;

use vello::Scene;
use vello::kurbo::{Affine, Rect};
use vello::peniko::{Brush, Color, Fill};

use crate::DocumentKit::{SyntaxLang, highlight_syntax as highlight};
use crate::TontooUI::elements::{
  Align, BasicText, FormattedText, HStack, Span, TextAlignment,
  TextForeground, TextStyle, View, VStack,
};
use crate::TontooUI::renderer::window::Key;
use crate::TontooUI::renderer::{FontSystem, ImageLoader};
use crate::TontooUI::theme::ThemeMode;

/// Gray gutter width for line numbers.
pub const CODE_GUTTER_W: f32 = 30.0;
/// Gap between gutter and code.
pub const CODE_GAP: f32 = 8.0;
/// Fixed row height (Footnote 13px plus spacing).
pub const CODE_LINE_H: f32 = 18.0;
/// Caret width in logical px.
pub const CODE_CARET_W: f32 = 2.0;
/// Code text size (matches `TextStyle::Footnote`).
pub const CODE_FONT_SIZE: f32 = 13.0;

/// 40-line Rust hello example naming file, project and user.
pub fn example_rust_code(file: &str, project: &str, user: &str) -> String {
  format!(
    "//  {file}.rs\n//  {project}\n//\n//  Created by {user}.\n//  Hello Rust example, editable.\nuse std::collections::HashMap;\n\n/// App state.\npub struct HelloApp {{\n    title: String,\n    count: u64,\n    flags: HashMap<String, bool>,\n}}\n\nimpl HelloApp {{\n    pub fn new(title: &str) -> Self {{\n        let mut flags = HashMap::new();\n        flags.insert(\"ready\".to_string(), true);\n        Self {{ title: title.to_string(), count: 0, flags }}\n    }}\n\n    pub fn tick(&mut self) -> u64 {{\n        self.count += 1;\n        self.count\n    }}\n\n    pub fn title(&self) -> &str {{\n        &self.title\n    }}\n}}\n\nfn main() {{\n    let mut app = HelloApp::new(\"{project}\");\n    println!(\"hello: {{}}\", app.title());\n    for _ in 0..3 {{\n        let n = app.tick();\n        println!(\"tick {{n}}\");\n    }}\n}}\n// End of example."
  )
}

/// One source line: gray number plus DocumentKit-highlighted code.
fn code_row(no: usize, spans: Vec<Span>) -> HStack {
  HStack::new()
    .spacing(CODE_GAP)
    .align(Align::Center)
    .child(
      BasicText::new((no + 1).to_string())
        .style(TextStyle::Footnote)
        .foreground(TextForeground::Secondary)
        .alignment(TextAlignment::Trailing)
        .width(CODE_GUTTER_W),
    )
    .child(FormattedText::spans(spans).style(TextStyle::Footnote))
}

/// Map one source line through the DocumentKit Rust tokenizer into
/// `Span`s (gaps stay plain code). Colors follow `SpanKind` per theme.
pub(crate) fn highlight_spans(line: &str, dark: bool) -> Vec<Span> {
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

fn build_rows(text: &str, dark: bool) -> VStack {
  let mut rows = VStack::new().spacing(2.0).align(Align::Leading);
  for (no, line) in text.lines().enumerate() {
    rows = rows.child(code_row(no, highlight_spans(line, dark)));
  }
  // Keep at least one row so empty text stays clickable.
  if text.is_empty() {
    rows = rows.child(code_row(0, highlight_spans("", dark)));
  }
  rows
}

fn advance_of(fonts: &mut FontSystem, text: &str) -> f32 {
  if text.is_empty() {
    return 0.0;
  }
  let layout = fonts.layout_text(text, CODE_FONT_SIZE, Color::WHITE, None);
  FontSystem::layout_size(&layout).0 / fonts.scale
}

/// Byte caret at visual `goal_x` inside `line` (nearest advance).
fn col_at_x(
  fonts: &mut FontSystem,
  line: &str,
  goal_x: f32,
) -> usize {
  let goal = goal_x.max(0.0);
  let mut bounds = vec![0usize];
  let mut at = 0usize;
  while at < line.len() {
    match line[at..].chars().next() {
      Some(c) => {
        at += c.len_utf8();
        bounds.push(at);
      }
      None => break,
    }
  }
  let mut adv = |k: usize| -> f32 {
    match line.get(..bounds[k]) {
      Some(slice) => advance_of(fonts, slice),
      None => 0.0,
    }
  };
  let mut lo = 0usize;
  let mut hi = bounds.len().saturating_sub(1);
  while lo < hi {
    let mid = (lo + hi + 1) / 2;
    if adv(mid) <= goal {
      lo = mid;
    } else {
      hi = mid - 1;
    }
  }
  let mut caret = bounds[lo];
  if lo + 1 < bounds.len() {
    let a0 = adv(lo);
    let a1 = adv(lo + 1);
    if (a0 + a1) / 2.0 < goal {
      caret = bounds[lo + 1];
    }
  }
  caret
}

/// Editable code page: TontooUI rows plus a caret rect.
pub struct CodeEditor {
  text: String,
  caret: usize,
  selected: bool,
  hovered: bool,
  dark: bool,
  mode: ThemeMode,
  focused: bool,
  accent: Color,
  rows: VStack,
  rect: (f32, f32, f32, f32),
  pending: Option<(f32, f32)>,
}

impl CodeEditor {
  pub fn new(initial: String) -> Self {
    let caret = initial.len();
    let rows = build_rows(&initial, true);
    Self {
      text: initial,
      caret,
      selected: false,
      hovered: false,
      dark: true,
      mode: ThemeMode::Dark,
      focused: true,
      accent: Color::from_rgb8(0x00, 0x7a, 0xff),
      rows,
      rect: (0.0, 0.0, 0.0, 0.0),
      pending: None,
    }
  }

  pub fn text_value(&self) -> &str {
    &self.text
  }

  pub fn wants_text_cursor(&self) -> bool {
    self.hovered || self.selected
  }

  pub fn set_theme(&mut self, accent: Color, mode: ThemeMode, dark: bool) {
    if self.dark != dark {
      self.dark = dark;
      self.rows = build_rows(&self.text, dark);
    }
    self.accent = accent;
    self.mode = mode;
  }

  pub fn set_focused(&mut self, focused: bool) {
    self.focused = focused;
  }

  fn rebuild(&mut self) {
    self.rows = build_rows(&self.text, self.dark);
  }

  fn lines(&self) -> Vec<String> {
    let mut out: Vec<String> =
      self.text.lines().map(|l| l.to_string()).collect();
    if out.is_empty() {
      out.push(String::new());
    }
    out
  }

  fn line_col(&self) -> (usize, usize) {
    let caret = self.caret.min(self.text.len());
    let mut line = 0usize;
    let mut start = 0usize;
    for (i, part) in self.text.split('\n').enumerate() {
      let end = start + part.len();
      if caret <= end || i == self.text.split('\n').count() - 1 {
        line = i;
        return (line, caret - start);
      }
      start = end + 1;
    }
    (line, 0)
  }

  fn caret_from_line_col(&self, line: usize, col: usize) -> usize {
    let mut start = 0usize;
    for (i, part) in self.text.split('\n').enumerate() {
      if i == line {
        return start + col.min(part.len());
      }
      start += part.len() + 1;
    }
    self.text.len()
  }

  fn insert(&mut self, content: &str) {
    let clean: String = content
      .chars()
      .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
      .collect();
    if clean.is_empty() {
      return;
    }
    let caret = self.caret.min(self.text.len());
    self.text.insert_str(caret, &clean);
    self.caret = caret + clean.len();
    self.rebuild();
  }

  fn backspace(&mut self) {
    let caret = self.caret.min(self.text.len());
    if caret == 0 {
      return;
    }
    let prev = self.text[..caret]
      .char_indices()
      .last()
      .map(|(i, _)| i)
      .unwrap_or(0);
    self.text.remove(prev);
    self.caret = prev;
    self.rebuild();
  }

  fn step_left(&mut self) {
    let caret = self.caret.min(self.text.len());
    if caret > 0 {
      self.caret = self.text[..caret]
        .char_indices()
        .last()
        .map(|(i, _)| i)
        .unwrap_or(0);
    }
  }

  fn step_right(&mut self) {
    let caret = self.caret.min(self.text.len());
    if caret < self.text.len() {
      let rest = &self.text[caret..];
      self.caret = rest
        .char_indices()
        .nth(1)
        .map(|(i, _)| caret + i)
        .unwrap_or(self.text.len());
    }
  }

  fn move_up_down_simple(&mut self, up: bool) {
    let lines = self.lines();
    let (line, col) = self.line_col();
    let next = if up {
      line.saturating_sub(1)
    } else {
      (line + 1).min(lines.len().saturating_sub(1))
    };
    let target_len = lines.get(next).map(|s| s.len()).unwrap_or(0);
    self.caret = self.caret_from_line_col(next, col.min(target_len));
  }

  fn caret_at_point(
    &mut self,
    fonts: &mut FontSystem,
    x: f32,
    y: f32,
  ) -> usize {
    let lines = self.lines();
    let rel = (y - self.rect.1).max(0.0);
    let line = ((rel / CODE_LINE_H) as usize).min(lines.len() - 1);
    let code_x = self.rect.0 + CODE_GUTTER_W + CODE_GAP;
    let goal = (x - code_x).max(0.0);
    let target = lines.get(line).cloned().unwrap_or_default();
    let col = col_at_x(fonts, &target, goal);
    self.caret_from_line_col(line, col)
  }

  /// Press handling: click inside arms a focus plus caret resolve
  /// on the next draw (caret mapping needs fonts); outside
  /// unfocuses at once.
  pub fn press(&mut self, x: f64, y: f64) {
    let (x, y) = (x as f32, y as f32);
    let (rx, ry, rw, rh) = self.rect;
    if x >= rx && x <= rx + rw && y >= ry && y <= ry + rh {
      self.selected = true;
      self.pending = Some((x, y));
    } else {
      self.selected = false;
      self.pending = None;
    }
  }

  /// Type text at the caret while focused.
  pub fn type_text(&mut self, content: &str) {
    if self.selected {
      self.insert(content);
    }
  }

  /// Key handling while focused: `Backspace` deletes, arrows move,
  /// `Enter` splits the line, `ESC` unfocuses. Returns true consumed.
  pub fn press_key(&mut self, key: Key) -> bool {
    if !self.selected {
      return false;
    }
    match key {
      Key::Backspace => self.backspace(),
      Key::Left => self.step_left(),
      Key::Right => self.step_right(),
      Key::Up => self.move_up_down_simple(true),
      Key::Down => self.move_up_down_simple(false),
      Key::Enter => self.insert("\n"),
      Key::Escape => {
        self.selected = false;
        return true;
      }
      _ => return false,
    }
    true
  }
}

impl View for CodeEditor {
  fn measure(
    &mut self,
    fonts: &mut FontSystem,
  ) -> (f32, f32) {
    let (w, h) = self.rows.measure(fonts);
    (w.max(300.0), h.max(self.lines().len() as f32 * CODE_LINE_H))
  }

  fn place(
    &mut self,
    fonts: &mut FontSystem,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
  ) {
    self.rect = (x, y, width, height);
    self.rows.place(fonts, x, y, width, height);
  }

  fn draw(
    &mut self,
    scene: &mut Scene,
    fonts: &mut FontSystem,
    images: &mut ImageLoader<'_>,
  ) {
    if let Some((px, py)) = self.pending.take() {
      if self.selected {
        self.caret = self.caret_at_point(fonts, px, py);
      }
    }
    for index in 0..self.rows.len() {
      if let Some(row) = self.rows.child_mut::<HStack>(index) {
        if let Some(gutter) = row.child_mut::<BasicText>(0) {
          gutter.set_theme(self.mode);
          gutter.set_focused(self.focused);
        }
        if let Some(code) = row.child_mut::<FormattedText>(1) {
          code.set_theme(self.mode);
          code.set_focused(self.focused);
        }
      }
    }
    self.rows.draw(scene, fonts, images);
    if self.selected {
      let (line, col) = self.line_col();
      let lines = self.lines();
      let current = lines.get(line).cloned().unwrap_or_default();
      let prefix = current.get(..col.min(current.len())).unwrap_or("");
      let cx =
        self.rect.0 + CODE_GUTTER_W + CODE_GAP + advance_of(fonts, prefix);
      let cy = self.rect.1 + line as f32 * CODE_LINE_H;
      let scale = fonts.scale as f64;
      let px = |v: f32| v as f64 * scale;
      let accent = if self.focused {
        self.accent
      } else {
        Color::from_rgb8(0x9a, 0x9a, 0x9e)
      };
      scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        &Brush::Solid(accent),
        None,
        &Rect::new(
          px(cx),
          px(cy + 1.0),
          px(cx + CODE_CARET_W),
          px(cy + CODE_LINE_H - 3.0),
        ),
      );
    }
  }

  fn mouse_down(&mut self, x: f64, y: f64) {
    self.press(x, y);
  }

  fn mouse_up(&mut self, _x: f64, _y: f64) {}

  fn set_hover(&mut self, x: f32, y: f32) {
    let (rx, ry, rw, rh) = self.rect;
    self.hovered =
      x >= rx && x <= rx + rw && y >= ry && y <= ry + rh;
  }

  fn text(&mut self, content: &str) {
    self.type_text(content);
  }

  fn key(&mut self, key: Key) -> bool {
    self.press_key(key)
  }

  fn as_any_mut(&mut self) -> &mut dyn Any {
    self
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn rust_example_is_40_lines() {
    let code = example_rust_code("testApp", "test", "arlo");
    assert_eq!(code.lines().count(), 40);
    assert!(code.contains("testApp"));
    assert!(code.contains("Created by arlo"));
    assert!(!code.contains("import SwiftUI"));
    assert!(code.contains("fn main"));
  }

  #[test]
  fn highlight_spans_cover_everything() {
    assert!(!highlight_spans("", true).is_empty());
    assert_eq!(highlight_spans("hello", true).len(), 1);
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
  fn editor_inserts_and_deletes() {
    let mut ed = CodeEditor::new("ab".to_string());
    ed.selected = true;
    ed.caret = 1;
    ed.insert("X");
    assert_eq!(ed.text_value(), "aXb");
    ed.backspace();
    assert_eq!(ed.text_value(), "ab");
    ed.insert("\n");
    assert_eq!(ed.text_value().lines().count(), 2);
  }
}
