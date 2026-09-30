//! Editable Rust code page for Xcode (user-approved custom element).
//!
//! TontooUI ships plain `TextEditor` elements but no code editor with
//! live Rust highlighting, so this file owns a small custom `View`
//! built from TontooUI primitives (`VStack` plus `HStack` rows with
//! `BasicText` gutters and `FormattedText` code) plus DocumentKit
//! spans, a caret rect and a selection wash. Behavior follows a normal
//! text field: click inside focuses with a caret, drag selects,
//! double-click selects the word, typing replaces the highlight,
//! `Backspace` deletes, `Enter` splits, arrows move (`Shift` extends),
//! `ESC` unfocuses, `Ctrl+A/C/X/V/Z/Y` selects all, copies, cuts,
//! pastes, undoes and redoes through an in-memory clipboard. Edits
//! stay in memory only.

use std::any::Any;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use vello::Scene;
use vello::kurbo::{Affine, Rect};
use vello::peniko::{Brush, Color, Fill};

use crate::DocumentKit::{SyntaxLang, highlight_syntax as highlight};
use crate::TontooUI::elements::{
  Align, BasicText, FormattedText, HStack, Scrollbar, Span, TextAlignment,
  TextForeground, TextStyle, View, VStack, SCROLLBAR_W_HOVER,
};
use crate::TontooUI::renderer::window::Key;
use crate::TontooUI::renderer::{FontSystem, ImageLoader, RichSpan};
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
/// Undo/redo depth.
pub const CODE_UNDO_LIMIT: usize = 100;
/// Selection wash alpha (0-1 of the accent).
pub const CODE_SELECTION_ALPHA: f32 = 0.3;
/// Double-click gap in seconds for word select.
pub const CODE_DOUBLE_TAP_SECONDS: f64 = 0.4;

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
  // `split` keeps the trailing empty line so the line count stays
  // stable while typing past the final newline.
  for (no, line) in text.split('\n').enumerate() {
    rows = rows.child(code_row(no, highlight_spans(line, dark)));
  }
  rows
}

/// Advance of `line[..upto]` in logical px, measured through the
/// same rich monospace DocumentKit layout the code rows render with
/// (bold runs included). The plain proportional probe used before
/// made the selection wash and the caret drift off the letters.
///
/// A trailing `"X"` sentinel in the prefix style keeps whitespace
/// prefixes measurable: CoreText collapses whitespace-only strings
/// to zero width, which snapped clicks at an indent to the first
/// visible glyph instead of the line start.
fn rich_advance(
  fonts: &mut FontSystem,
  line: &str,
  dark: bool,
  upto: usize,
) -> f32 {
  let upto = upto.min(line.len());
  if upto == 0 {
    return 0.0;
  }
  let mut content = String::new();
  let mut rich = Vec::new();
  let mut style = (false, false);
  for span in highlight_spans(line, dark) {
    if content.len() >= upto {
      break;
    }
    let take = (upto - content.len()).min(span.text.len());
    let Some(piece) = span.text.get(..take) else {
      break;
    };
    let start = content.len();
    content.push_str(piece);
    style = (span.bold, span.italic);
    rich.push(RichSpan {
      range: start..content.len(),
      bold: span.bold,
      italic: span.italic,
      monospace: true,
      ..Default::default()
    });
  }
  if content.is_empty() {
    return 0.0;
  }
  let sentinel_at = content.len();
  content.push('X');
  rich.push(RichSpan {
    range: sentinel_at..content.len(),
    bold: style.0,
    italic: style.1,
    monospace: true,
    ..Default::default()
  });
  let frame =
    fonts.layout_rich_text(&content, CODE_FONT_SIZE, Color::WHITE, None, &rich);
  let full = FontSystem::layout_size(&frame).0 / fonts.scale;
  let solo = fonts.layout_rich_text(
    "X",
    CODE_FONT_SIZE,
    Color::WHITE,
    None,
    &[RichSpan {
      range: 0..1,
      bold: style.0,
      italic: style.1,
      monospace: true,
      ..Default::default()
    }],
  );
  full - FontSystem::layout_size(&solo).0 / fonts.scale
}

/// Byte caret at visual `goal_x` inside `line` (nearest advance,
/// measured with the rich monospace layout).
fn col_at_x(
  fonts: &mut FontSystem,
  line: &str,
  dark: bool,
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
      Some(_) => rich_advance(fonts, line, dark, bounds[k]),
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

/// Word boundaries around a byte caret: alphanumeric plus `_`
/// counts as word chars. Collapsed when the caret sits between
/// words (double-click on a gap just places the caret).
fn word_range(text: &str, caret: usize) -> (usize, usize) {
  let caret = caret.min(text.len());
  let is_word = |c: char| c.is_alphanumeric() || c == '_';
  let mut a = caret;
  while a > 0 {
    match text[..a].chars().next_back() {
      Some(c) if is_word(c) => a -= c.len_utf8(),
      _ => break,
    }
  }
  let mut b = caret;
  while b < text.len() {
    match text[b..].chars().next() {
      Some(c) if is_word(c) => b += c.len_utf8(),
      _ => break,
    }
  }
  (a, b)
}

fn clipboard_buffer() -> &'static Mutex<String> {
  static BUFFER: OnceLock<Mutex<String>> = OnceLock::new();
  BUFFER.get_or_init(|| Mutex::new(String::new()))
}

fn clipboard_get() -> Option<String> {
  clipboard_buffer().lock().ok().and_then(|guard| {
    if guard.is_empty() {
      None
    } else {
      Some(guard.clone())
    }
  })
}

fn clipboard_set(text: &str) {
  if let Ok(mut guard) = clipboard_buffer().lock() {
    *guard = text.to_string();
  }
}

#[derive(Clone)]
struct UndoState {
  text: String,
  caret: usize,
  anchor: usize,
  sel: bool,
}

/// Overlay bar width in logical px. Matches the hover width of the
/// standard `Scrollbar` so the thumb never covers more than needed.
pub const CODE_BAR_W: f32 = SCROLLBAR_W_HOVER;

/// Editable code page: TontooUI rows plus caret, selection and the
/// standard overlay `Scrollbar`.
pub struct CodeEditor {
  text: String,
  caret: usize,
  anchor: usize,
  sel: bool,
  selected: bool,
  pressing: bool,
  hovered: bool,
  dark: bool,
  mode: ThemeMode,
  focused: bool,
  accent: Color,
  rows: VStack,
  rect: (f32, f32, f32, f32),
  bar: Scrollbar,
  last_model: (f32, f32),
  pending: Option<(f32, f32)>,
  drag: Option<(f32, f32)>,
  press_time: Option<Instant>,
  undo: Vec<UndoState>,
  redo: Vec<UndoState>,
}

impl CodeEditor {
  pub fn new(initial: String) -> Self {
    let caret = initial.len();
    let rows = build_rows(&initial, true);
    Self {
      text: initial,
      caret,
      anchor: caret,
      sel: false,
      selected: false,
      pressing: false,
      hovered: false,
      dark: true,
      mode: ThemeMode::Dark,
      focused: true,
      accent: Color::from_rgb8(0x00, 0x7a, 0xff),
      rows,
      rect: (0.0, 0.0, 0.0, 0.0),
      bar: Scrollbar::new(),
      last_model: (-1.0, -1.0),
      pending: None,
      drag: None,
      press_time: None,
      undo: Vec::new(),
      redo: Vec::new(),
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
    self.bar.set_theme(accent, dark);
  }

  pub fn set_focused(&mut self, focused: bool) {
    self.focused = focused;
    self.bar.set_focused(focused);
  }

  /// Current scroll offset in logical px (`0.0` is the top).
  pub fn offset(&self) -> f32 {
    self.bar.offset()
  }

  pub fn max_offset(&self) -> f32 {
    self.bar.max_offset()
  }

  pub fn scrollable(&self) -> bool {
    self.bar.scrollable()
  }

  fn rebuild(&mut self) {
    self.rows = build_rows(&self.text, self.dark);
  }

  fn lines(&self) -> Vec<String> {
    self.text.split('\n').map(|l| l.to_string()).collect()
  }

  fn content_h(&self) -> f32 {
    self.lines().len() as f32 * CODE_LINE_H
  }

  /// True while the pointer is over the overlay bar (same generous
  /// grab area the `Scrollbar` itself uses). Presses there drive the
  /// bar and never reach the text underneath.
  fn over_bar(&self, x: f32, y: f32) -> bool {
    if !self.bar.scrollable() || self.rect.2 <= 0.0 || self.rect.3 <= 0.0
    {
      return false;
    }
    let bx = self.rect.0 + self.rect.2 - CODE_BAR_W;
    x >= bx - 2.0
      && x <= self.rect.0 + self.rect.2 + 2.0
      && y >= self.rect.1
      && y <= self.rect.1 + self.rect.3
  }

  /// Sync the bar model with the content: `set_content` flashes the
  /// bar, so it only runs when the model changed (every-frame calls
  /// would keep the bar awake forever). The bar stays the single
  /// source of truth for the offset.
  fn sync_bar(&mut self, fonts: &mut FontSystem) {
    let model = (self.content_h(), self.rect.3);
    if model != self.last_model {
      self.last_model = model;
      self.bar.set_content(model.0, model.1);
    }
    let bw = CODE_BAR_W.min(self.rect.2).max(0.0);
    self.bar.set_rect(
      self.rect.0 + (self.rect.2 - bw).max(0.0),
      self.rect.1,
      bw,
      self.rect.3,
    );
    let content_h = model.0.max(self.rect.3);
    self.rows.place(
      fonts,
      self.rect.0,
      self.rect.1 - self.bar.offset(),
      self.rect.2,
      content_h,
    );
  }

  /// Keep the caret visible inside the viewport.
  fn track_caret(&mut self) {
    let (line, _) = self.line_col();
    let top = line as f32 * CODE_LINE_H;
    let offset = self.bar.offset();
    if top - offset < 0.0 {
      self.bar.set_offset(top.max(0.0));
    } else if top + CODE_LINE_H - offset > self.rect.3 {
      self.bar.set_offset((top + CODE_LINE_H - self.rect.3).max(0.0));
    }
  }

  /// Wheel scroll in logical px (down positive, like the shell).
  /// The bar clamps and flashes; rows re-place on the next draw.
  pub fn scroll(&mut self, dx: f64, dy: f64) {
    self.bar.mouse_wheel(dx, dy);
  }

  fn line_col(&self) -> (usize, usize) {
    let caret = self.caret.min(self.text.len());
    let mut start = 0usize;
    for (i, part) in self.text.split('\n').enumerate() {
      let end = start + part.len();
      if caret <= end {
        return (i, caret - start);
      }
      start = end + 1;
    }
    (self.lines().len().saturating_sub(1), 0)
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

  fn snapshot(&self) -> UndoState {
    UndoState {
      text: self.text.clone(),
      caret: self.caret,
      anchor: self.anchor,
      sel: self.sel,
    }
  }

  fn push_undo(&mut self) {
    let state = self.snapshot();
    if self
      .undo
      .last()
      .is_none_or(|top| top.text != state.text || top.caret != state.caret)
    {
      self.undo.push(state);
      if self.undo.len() > CODE_UNDO_LIMIT {
        self.undo.remove(0);
      }
    }
    self.redo.clear();
  }

  fn restore(&mut self, state: UndoState) {
    self.text = state.text;
    self.caret = state.caret.min(self.text.len());
    self.anchor = state.anchor.min(self.text.len());
    self.sel = state.sel && self.anchor != self.caret;
    self.rebuild();
    self.track_caret();
  }

  pub fn undo_edit(&mut self) -> bool {
    let Some(top) = self.undo.pop() else {
      return false;
    };
    self.redo.push(self.snapshot());
    self.restore(top);
    true
  }

  pub fn redo_edit(&mut self) -> bool {
    let Some(top) = self.redo.pop() else {
      return false;
    };
    self.undo.push(self.snapshot());
    self.restore(top);
    true
  }

  fn selection_range(&self) -> Option<(usize, usize)> {
    if self.sel && self.anchor != self.caret {
      let a = self.anchor.min(self.text.len());
      let b = self.caret.min(self.text.len());
      Some((a.min(b), a.max(b)))
    } else {
      None
    }
  }

  fn selected_text(&self) -> String {
    match self.selection_range() {
      Some((a, b)) => self.text[a..b].to_string(),
      None => String::new(),
    }
  }

  pub fn select_all(&mut self) {
    self.anchor = 0;
    self.caret = self.text.len();
    self.sel = !self.text.is_empty();
    self.track_caret();
  }

  fn collapse(&mut self) {
    self.anchor = self.caret;
    self.sel = false;
  }

  fn delete_selection(&mut self) -> bool {
    let Some((a, b)) = self.selection_range() else {
      return false;
    };
    self.push_undo();
    self.text.replace_range(a..b, "");
    self.caret = a;
    self.anchor = a;
    self.sel = false;
    self.rebuild();
    true
  }

  pub fn copy_selection(&mut self) -> bool {
    let selected = self.selected_text();
    if selected.is_empty() {
      return false;
    }
    clipboard_set(&selected);
    true
  }

  pub fn cut_selection(&mut self) -> bool {
    if !self.copy_selection() {
      return false;
    }
    self.delete_selection()
  }

  pub fn paste_clipboard(&mut self) {
    if let Some(text) = clipboard_get() {
      self.insert(&text);
    }
  }

  fn insert(&mut self, content: &str) {
    let clean: String = content
      .chars()
      .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
      .collect();
    if clean.is_empty() {
      return;
    }
    self.push_undo();
    if let Some((a, b)) = self.selection_range() {
      self.text.replace_range(a..b, "");
      self.caret = a;
    }
    let caret = self.caret.min(self.text.len());
    self.text.insert_str(caret, &clean);
    self.caret = caret + clean.len();
    self.anchor = self.caret;
    self.sel = false;
    self.rebuild();
    self.track_caret();
  }

  fn backspace(&mut self) {
    if self.delete_selection() {
      self.track_caret();
      return;
    }
    let caret = self.caret.min(self.text.len());
    if caret == 0 {
      return;
    }
    self.push_undo();
    let prev = self.text[..caret]
      .char_indices()
      .last()
      .map(|(i, _)| i)
      .unwrap_or(0);
    self.text.remove(prev);
    self.caret = prev;
    self.anchor = prev;
    self.rebuild();
    self.track_caret();
  }

  fn step_left(&mut self, extend: bool) {
    if !extend {
      if let Some((a, _)) = self.selection_range() {
        self.caret = a;
        self.collapse();
        self.track_caret();
        return;
      }
      let caret = self.caret.min(self.text.len());
      if caret > 0 {
        self.caret = self.text[..caret]
          .char_indices()
          .last()
          .map(|(i, _)| i)
          .unwrap_or(0);
      }
      self.collapse();
      self.track_caret();
      return;
    }
    if !self.sel {
      self.anchor = self.caret;
    }
    let caret = self.caret.min(self.text.len());
    if caret > 0 {
      self.caret = self.text[..caret]
        .char_indices()
        .last()
        .map(|(i, _)| i)
        .unwrap_or(0);
    }
    self.sel = self.anchor != self.caret;
    self.track_caret();
  }

  fn step_right(&mut self, extend: bool) {
    if !extend {
      if let Some((_, b)) = self.selection_range() {
        self.caret = b;
        self.collapse();
        self.track_caret();
        return;
      }
      let caret = self.caret.min(self.text.len());
      if caret < self.text.len() {
        let rest = &self.text[caret..];
        self.caret = rest
          .char_indices()
          .nth(1)
          .map(|(i, _)| caret + i)
          .unwrap_or(self.text.len());
      }
      self.collapse();
      self.track_caret();
      return;
    }
    if !self.sel {
      self.anchor = self.caret;
    }
    let caret = self.caret.min(self.text.len());
    if caret < self.text.len() {
      let rest = &self.text[caret..];
      self.caret = rest
        .char_indices()
        .nth(1)
        .map(|(i, _)| caret + i)
        .unwrap_or(self.text.len());
    }
    self.sel = self.anchor != self.caret;
    self.track_caret();
  }

  fn move_up_down_simple(&mut self, up: bool, extend: bool) {
    let before = self.caret;
    let was_sel = self.sel;
    let lines = self.lines();
    let (line, col) = self.line_col();
    let next = if up {
      line.saturating_sub(1)
    } else {
      (line + 1).min(lines.len().saturating_sub(1))
    };
    let target_len = lines.get(next).map(|s| s.len()).unwrap_or(0);
    self.caret = self.caret_from_line_col(next, col.min(target_len));
    if extend {
      if !was_sel {
        self.anchor = before;
      }
      self.sel = self.anchor != self.caret;
    } else {
      self.collapse();
    }
    self.track_caret();
  }

  fn caret_at_point(
    &mut self,
    fonts: &mut FontSystem,
    x: f32,
    y: f32,
  ) -> usize {
    let lines = self.lines();
    let rel = (y - self.rect.1 + self.bar.offset()).max(0.0);
    let line = ((rel / CODE_LINE_H) as usize).min(lines.len() - 1);
    let code_x = self.rect.0 + CODE_GUTTER_W + CODE_GAP;
    let goal = (x - code_x).max(0.0);
    let target = lines.get(line).cloned().unwrap_or_default();
    let col = col_at_x(fonts, &target, self.dark, goal);
    self.caret_from_line_col(line, col)
  }

  fn finish_press(&mut self, caret: usize, now: Instant) {
    let caret = caret.min(self.text.len());
    let double = matches!(self.press_time, Some(t)
      if now.duration_since(t).as_secs_f64() < CODE_DOUBLE_TAP_SECONDS);
    if double {
      self.press_time = None;
      let (a, b) = word_range(&self.text, caret);
      if a != b {
        self.anchor = a;
        self.caret = b;
        self.sel = true;
        self.track_caret();
        return;
      }
    } else {
      self.press_time = Some(now);
    }
    self.caret = caret;
    self.anchor = caret;
    self.sel = false;
    self.track_caret();
  }

  fn finish_drag(&mut self, caret: usize) {
    self.caret = caret.min(self.text.len());
    self.sel = self.anchor != self.caret;
    self.track_caret();
  }

  /// Press handling: presses over the overlay bar drive the bar
  /// and never reach the text; clicks inside arm a focus plus caret
  /// resolve on the next draw (caret mapping needs fonts); outside
  /// unfocuses at once.
  pub fn press(&mut self, x: f64, y: f64) {
    self.bar.mouse_down(x, y);
    let (x, y) = (x as f32, y as f32);
    let (rx, ry, rw, rh) = self.rect;
    if self.over_bar(x, y) {
      return;
    }
    if x >= rx && x <= rx + rw && y >= ry && y <= ry + rh {
      self.selected = true;
      self.pressing = true;
      self.pending = Some((x, y));
      self.drag = None;
    } else {
      self.selected = false;
      self.pressing = false;
      self.pending = None;
      self.drag = None;
    }
  }

  /// Type text at the caret while focused.
  pub fn type_text(&mut self, content: &str) {
    if self.selected {
      self.insert(content);
    }
  }

  /// Key handling while focused: editing, selection, clipboard,
  /// undo and redo. Returns true when consumed.
  pub fn press_key(&mut self, key: Key) -> bool {
    if !self.selected {
      return false;
    }
    match key {
      Key::Backspace => self.backspace(),
      Key::Left => self.step_left(false),
      Key::Right => self.step_right(false),
      Key::Up => self.move_up_down_simple(true, false),
      Key::Down => self.move_up_down_simple(false, false),
      Key::SelectLeft => self.step_left(true),
      Key::SelectRight => self.step_right(true),
      Key::SelectUp => self.move_up_down_simple(true, true),
      Key::SelectDown => self.move_up_down_simple(false, true),
      Key::Enter => self.insert("\n"),
      Key::SelectAll => self.select_all(),
      Key::Copy => {
        self.copy_selection();
      }
      Key::Cut => {
        self.cut_selection();
      }
      Key::Paste => self.paste_clipboard(),
      Key::Undo => {
        self.undo_edit();
      }
      Key::Redo => {
        self.redo_edit();
      }
      Key::Escape => {
        self.selected = false;
        self.sel = false;
        self.pressing = false;
        return true;
      }
    }
    true
  }
}

impl View for CodeEditor {
  fn measure(&mut self, fonts: &mut FontSystem) -> (f32, f32) {
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
    self.sync_bar(fonts);
  }

  fn draw(
    &mut self,
    scene: &mut Scene,
    fonts: &mut FontSystem,
    images: &mut ImageLoader<'_>,
  ) {
    if let Some((px, py)) = self.pending.take() {
      if self.selected {
        let caret = self.caret_at_point(fonts, px, py);
        self.finish_press(caret, Instant::now());
      }
    }
    if let Some((dx, dy)) = self.drag.take() {
      if self.selected && self.pressing {
        let caret = self.caret_at_point(fonts, dx, dy);
        self.finish_drag(caret);
      }
    }
    // Re-sync every frame: wheel and thumb drags change the offset
    // without a new place call, and the text may have grown.
    self.sync_bar(fonts);
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
    let scale = fonts.scale as f64;
    let px = |v: f32| v as f64 * scale;
    let (rx, ry, rw, rh) = self.rect;
    if rw <= 0.0 || rh <= 0.0 {
      return;
    }
    let clip = Rect::new(px(rx), px(ry), px(rx + rw), px(ry + rh));
    scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &clip);
    self.rows.draw(scene, fonts, images);
    // Selection wash, segmented per line.
    if let Some((sa, sb)) = self.selection_range() {
      let wash = {
        let c = self.accent.to_rgba8();
        Color::from_rgba8(
          c.r,
          c.g,
          c.b,
          (c.a as f32 * CODE_SELECTION_ALPHA).round() as u8,
        )
      };
      let offset = self.bar.offset();
      let lines = self.lines();
      let mut start = 0usize;
      for (index, line) in lines.iter().enumerate() {
        let end = start + line.len();
        let lo = sa.max(start).min(end);
        let hi = sb.max(start).min(end);
        if lo < hi {
          let dark = self.dark;
          let x0 = rich_advance(fonts, line, dark, lo - start);
          let x1 = rich_advance(fonts, line, dark, hi - start);
          let cy =
            self.rect.1 + index as f32 * CODE_LINE_H - offset;
          if cy + CODE_LINE_H >= self.rect.1
            && cy <= self.rect.1 + self.rect.3
          {
            let cx = self.rect.0 + CODE_GUTTER_W + CODE_GAP;
            scene.fill(
              Fill::NonZero,
              Affine::IDENTITY,
              &Brush::Solid(wash),
              None,
              &Rect::new(
                px(cx + x0),
                px(cy + 1.0),
                px(cx + x1),
                px(cy + CODE_LINE_H - 3.0),
              ),
            );
          }
        }
        start = end + 1;
      }
    }
    if self.selected {
      let (line, col) = self.line_col();
      let lines = self.lines();
      let current = lines.get(line).cloned().unwrap_or_default();
      let dark = self.dark;
      let upto = col.min(current.len());
      let cx = self.rect.0
        + CODE_GUTTER_W
        + CODE_GAP
        + rich_advance(fonts, &current, dark, upto);
      let cy =
        self.rect.1 + line as f32 * CODE_LINE_H - self.bar.offset();
      if cy + CODE_LINE_H >= self.rect.1
        && cy <= self.rect.1 + self.rect.3
      {
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
    scene.pop_layer();
    self.bar.draw(scene, fonts, images);
  }

  fn mouse_down(&mut self, x: f64, y: f64) {
    self.press(x, y);
  }

  fn mouse_up(&mut self, x: f64, y: f64) {
    self.bar.mouse_up(x, y);
    self.pressing = false;
    self.pending = None;
    self.drag = None;
  }

  fn set_hover(&mut self, x: f32, y: f32) {
    self.bar.mouse_move(x as f64, y as f64);
    let (rx, ry, rw, rh) = self.rect;
    self.hovered = x >= rx && x <= rx + rw && y >= ry && y <= ry + rh;
    if self.pressing && self.selected {
      self.drag = Some((x, y));
    }
  }

  fn text(&mut self, content: &str) {
    self.type_text(content);
  }

  fn key(&mut self, key: Key) -> bool {
    self.press_key(key)
  }

  fn mouse_wheel(&mut self, dx: f64, dy: f64) {
    self.scroll(dx, dy);
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
    ed.anchor = 1;
    ed.insert("X");
    assert_eq!(ed.text_value(), "aXb");
    ed.backspace();
    assert_eq!(ed.text_value(), "ab");
    ed.insert("\n");
    assert_eq!(ed.text_value().split('\n').count(), 2);
  }

  #[test]
  fn select_all_copies_and_pastes() {
    let mut ed = CodeEditor::new("hello".to_string());
    ed.selected = true;
    assert!(ed.press_key(Key::SelectAll));
    assert_eq!(ed.selected_text(), "hello");
    assert!(ed.press_key(Key::Copy));
    ed.press_key(Key::Right);
    assert!(ed.press_key(Key::Paste));
    assert_eq!(ed.text_value(), "hellohello");
  }

  #[test]
  fn cut_and_undo_redo_roundtrip() {
    let mut ed = CodeEditor::new("hello".to_string());
    ed.selected = true;
    ed.press_key(Key::SelectAll);
    assert!(ed.press_key(Key::Cut));
    assert_eq!(ed.text_value(), "");
    assert!(ed.press_key(Key::Undo));
    assert_eq!(ed.text_value(), "hello");
    assert!(ed.press_key(Key::Redo));
    assert_eq!(ed.text_value(), "");
  }

  #[test]
  fn shift_extends_selection() {
    let mut ed = CodeEditor::new("abcd".to_string());
    ed.selected = true;
    ed.caret = 1;
    ed.anchor = 1;
    ed.sel = false;
    assert!(ed.press_key(Key::SelectRight));
    assert_eq!(ed.selected_text(), "b");
    assert!(ed.press_key(Key::SelectRight));
    assert_eq!(ed.selected_text(), "bc");
  }

  #[test]
  fn wheel_scrolls_and_clamps() {
    let text = (0..100).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
    let mut ed = CodeEditor::new(text);
    ed.rect = (0.0, 0.0, 300.0, 200.0);
    ed.bar.set_content(ed.content_h(), 200.0);
    assert_eq!(ed.offset(), 0.0);
    ed.scroll(0.0, -100.0);
    assert!(ed.offset() > 0.0);
    let max = ed.max_offset();
    assert!(max > 0.0);
    ed.scroll(0.0, -100000.0);
    assert_eq!(ed.offset(), max);
    ed.scroll(0.0, 100000.0);
    assert_eq!(ed.offset(), 0.0);
  }

  #[test]
  fn caret_tracking_keeps_last_line_visible() {
    let text = (0..100).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
    let mut ed = CodeEditor::new(text);
    ed.rect = (0.0, 0.0, 300.0, 200.0);
    ed.bar.set_content(ed.content_h(), 200.0);
    ed.selected = true;
    ed.caret = ed.text_value().len();
    ed.anchor = ed.caret;
    ed.sel = false;
    ed.track_caret();
    assert_eq!(ed.offset(), ed.max_offset());
  }

  #[test]
  fn bar_hides_when_content_fits() {
    let mut ed = CodeEditor::new("hi".to_string());
    ed.rect = (0.0, 0.0, 300.0, 200.0);
    ed.bar.set_content(ed.content_h(), 200.0);
    assert!(!ed.scrollable());
    assert_eq!(ed.max_offset(), 0.0);
  }

  #[test]
  fn rich_advance_matches_rendered_layout() {
    let mut fonts = FontSystem::new();
    // A line with bold keyword runs: the wash must use the same
    // rich monospace layout the row renders with, not a plain
    // proportional probe.
    let line = "use std::collections::HashMap;";
    let full = rich_advance(&mut fonts, line, true, line.len());
    let mut content = String::new();
    let mut rich = Vec::new();
    for span in highlight_spans(line, true) {
      let start = content.len();
      content.push_str(&span.text);
      rich.push(RichSpan {
        range: start..content.len(),
        bold: span.bold,
        italic: span.italic,
        monospace: true,
        ..Default::default()
      });
    }
    let frame = fonts.layout_rich_text(
      &content,
      CODE_FONT_SIZE,
      Color::WHITE,
      None,
      &rich,
    );
    let (tw, _) = FontSystem::layout_size(&frame);
    assert!((full - tw / fonts.scale).abs() < 0.01);
    assert_eq!(rich_advance(&mut fonts, line, true, 0), 0.0);
    // Advances grow monotonically over char boundaries.
    let mut prev = 0.0;
    let mut at = 0usize;
    while at < line.len() {
      at += line[at..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
      let w = rich_advance(&mut fonts, line, true, at);
      assert!(w >= prev);
      prev = w;
    }
    assert!(prev > 0.0);
  }

  #[test]
  fn col_at_x_roundtrips_rich_layout() {
    let mut fonts = FontSystem::new();
    let line = "    title: String,";
    assert_eq!(col_at_x(&mut fonts, line, true, -10.0), 0);
    assert_eq!(col_at_x(&mut fonts, line, true, 1e6), line.len());
    // The middle of the line maps back near its own advance.
    let mid = line.len() / 2;
    let x = rich_advance(&mut fonts, line, true, mid);
    let hit = col_at_x(&mut fonts, line, true, x);
    assert!((hit as isize - mid as isize).abs() <= 1);
  }
}
