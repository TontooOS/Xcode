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
use vello::kurbo::{Affine, BezPath, Cap, Circle, Join, Line, Rect, Stroke};
use vello::peniko::{Brush, Color, Fill};

use crate::DocumentKit::{SyntaxLang, highlight_syntax as highlight};
use crate::TontooUI::elements::{
  Align, BasicText, FormattedText, HStack, Scrollbar, Span, TextAlignment,
  TextForeground, TextStyle, View, VStack, DIVIDER_DARK, DIVIDER_LIGHT,
  SCROLLBAR_W_HOVER, TEXTFIELD_PLACEHOLDER_DARK,
  TEXTFIELD_PLACEHOLDER_LIGHT,
};
use crate::TontooUI::renderer::window::Key;
use crate::TontooUI::renderer::{FontSystem, ImageLoader, RichSpan, draw_layout};
use crate::TontooUI::theme::{ThemeMode, desaturate};

/// Gray gutter width for line numbers.
pub const CODE_GUTTER_W: f32 = 30.0;
/// Gap between gutter and code.
pub const CODE_GAP: f32 = 8.0;
/// Fallback row pitch in logical px until the first layout measures
/// the real one (keep in sync with `sync_pitch`).
pub const CODE_LINE_H: f32 = 18.0;
/// Row gap inside the rows stack (must match `build_rows`).
pub const CODE_ROW_SPACING: f32 = 2.0;
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
  let mut rows = VStack::new().spacing(CODE_ROW_SPACING).align(Align::Leading);
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

/// Point inside a logical rect.
fn point_in(r: (f32, f32, f32, f32), x: f32, y: f32) -> bool {
  x >= r.0 && x <= r.0 + r.2 && y >= r.1 && y <= r.1 + r.3
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
/// Bottom panel default height in logical px.
pub const PANEL_DEFAULT_H: f32 = 180.0;
/// Bottom panel minimum height while expanded.
pub const PANEL_MIN_H: f32 = 100.0;
/// Bottom panel maximum height.
pub const PANEL_MAX_H: f32 = 420.0;
/// Divider strip height between code and panel.
pub const PANEL_DIV_H: f32 = 10.0;
/// Stats share of the panel width (logs take the rest).
pub const PANEL_STATS_SPLIT: f32 = 0.6;
/// Drag distance in logical px before a divider press becomes a
/// resize instead of a collapse toggle.
pub const PANEL_DRAG_SLOP: f32 = 4.0;
/// Fake stats samples kept (60s at one sample per second).
pub const STATS_CAP: usize = 60;
/// Fake log lines kept.
pub const LOGS_CAP: usize = 300;
/// Seconds between fake samples and log lines.
pub const TICK_SECONDS: f64 = 1.0;
/// Log text size in logical px.
pub const LOG_FONT_SIZE: f32 = 12.0;
/// Log row height in logical px.
pub const LOG_LINE_H: f32 = 16.0;
/// Panel header height in logical px.
pub const PANEL_HEADER_H: f32 = 20.0;
/// Panel label text size in logical px.
pub const PANEL_LABEL_SIZE: f32 = 11.0;
/// Fixed graph colors per metric (macOS system palette) so the
/// sparkline lines stay distinguishable in both themes.
pub const STAT_CPU: Color = Color::from_rgb8(0x0a, 0x84, 0xff);
pub const STAT_GPU: Color = Color::from_rgb8(0xbf, 0x5a, 0xf2);
pub const STAT_RAM: Color = Color::from_rgb8(0x30, 0xd1, 0x58);
pub const STAT_DISK: Color = Color::from_rgb8(0xff, 0x9f, 0x0a);
pub const STAT_NET: Color = Color::from_rgb8(0x64, 0xd2, 0xff);
/// Diagnostic gutter and wash colors (match the navigator tints).
pub const DIAG_ERROR: Color = Color::from_rgb8(0xff, 0x3b, 0x30);
pub const DIAG_WARN: Color = Color::from_rgb8(0xff, 0xcc, 0x00);
/// Line wash alphas (0-1 of the marker color).
pub const DIAG_ERROR_WASH: f32 = 0.16;
pub const DIAG_WARN_WASH: f32 = 0.24;
/// Gutter badge radius in logical px.
pub const DIAG_DOT_R: f32 = 4.5;

/// Editable code page: TontooUI rows plus caret, selection, the
/// standard overlay `Scrollbar` and a collapsible bottom panel with
/// fake performance stats and logs.
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
  /// Measured row pitch (row height plus gap); refreshed from the
  /// laid-out rows on every place and draw, so caret, washes and
  /// clicks never drift off the lines.
  pitch: f32,
  pending: Option<(f32, f32)>,
  drag: Option<(f32, f32)>,
  press_time: Option<Instant>,
  undo: Vec<UndoState>,
  redo: Vec<UndoState>,
  /// Bottom panel height while expanded.
  panel_h: f32,
  /// Bottom panel folded away (divider only).
  collapsed: bool,
  /// Divider resize in progress.
  divider_drag: bool,
  /// Divider resize anchor: press Y plus panel height at press.
  divider_start: (f32, f32),
  divider_moved: bool,
  /// Divider strip hovered (accent highlight plus resize cursor).
  divider_hover: bool,
  /// Fake CPU/GPU/RAM/Disk/Net samples (0..100, one per second).
  samples_cpu: Vec<f32>,
  samples_gpu: Vec<f32>,
  samples_ram: Vec<f32>,
  samples_disk: Vec<f32>,
  samples_net: Vec<f32>,
  /// Fake cargo log lines.
  logs: Vec<String>,
  /// Logs overlay bar plus model guard.
  log_bar: Scrollbar,
  log_model: (f32, f32),
  /// Stick the logs to the bottom on new lines.
  stick_logs: bool,
  /// Fake tick counter and last tick time.
  tick_count: u64,
  last_tick: f64,
  /// Last hover point for wheel routing.
  last_hover: (f32, f32),
  /// Diagnostic markers: 1-based line plus severity (`true` error,
  /// `false` warning), sorted and deduplicated with error winning.
  markers: Vec<(usize, bool)>,
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
      pitch: CODE_LINE_H,
      pending: None,
      drag: None,
      press_time: None,
      undo: Vec::new(),
      redo: Vec::new(),
      panel_h: PANEL_DEFAULT_H,
      collapsed: false,
      divider_drag: false,
      divider_start: (0.0, 0.0),
      divider_moved: false,
      divider_hover: false,
      samples_cpu: Vec::new(),
      samples_gpu: Vec::new(),
      samples_ram: Vec::new(),
      samples_disk: Vec::new(),
      samples_net: Vec::new(),
      logs: Vec::new(),
      log_bar: Scrollbar::new(),
      log_model: (-1.0, -1.0),
      stick_logs: true,
      tick_count: 0,
      last_tick: 0.0,
      last_hover: (-1.0, -1.0),
      markers: Vec::new(),
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
    self.log_bar.set_focused(focused);
  }

  /// Panel state for tests and later wiring.
  pub fn panel_height(&self) -> f32 {
    self.panel_h
  }

  pub fn panel_collapsed(&self) -> bool {
    self.collapsed
  }

  /// Fold or unfold the bottom panel (the top-right performance
  /// pill drives this on every page).
  pub fn set_collapsed(&mut self, collapsed: bool) {
    self.collapsed = collapsed;
  }

  /// True while the divider strip hovers or a resize drag runs:
  /// the app shows the resize cursor then, like the sidebar edge.
  pub fn wants_divider_cursor(&self) -> bool {
    self.divider_drag || self.divider_hover
  }

  pub fn log_lines(&self) -> &[String] {
    &self.logs
  }

  pub fn sample_count(&self) -> usize {
    self.samples_cpu.len()
  }

  /// Zones inside the placed rect: code viewport, divider strip,
  /// panel area, stats area (panel left) and logs area (panel
  /// right). The panel area is empty while collapsed.
  fn zones(&self) -> ((f32, f32, f32, f32), (f32, f32, f32, f32), (f32, f32, f32, f32), (f32, f32, f32, f32), (f32, f32, f32, f32)) {
    let (rx, ry, rw, rh) = self.rect;
    let panel_h = if self.collapsed { 0.0 } else { self.panel_h };
    let code_h = (rh - PANEL_DIV_H - panel_h).max(40.0);
    let code = (rx, ry, rw, code_h);
    let divider = (rx, ry + code_h, rw, PANEL_DIV_H);
    let panel = (rx, ry + code_h + PANEL_DIV_H, rw, panel_h);
    let split = (rw * PANEL_STATS_SPLIT).max(0.0);
    let stats = (rx, panel.1, split, panel_h);
    let logs = (rx + split, panel.1, (rw - split).max(0.0), panel_h);
    (code, divider, panel, stats, logs)
  }

  /// Code viewport height (the code scrollbar model uses this).
  fn code_h(&self) -> f32 {
    self.zones().0.3
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
    self.lines().len() as f32 * self.pitch
  }

  /// Measure the real row pitch from the laid-out rows: the stack
  /// total holds `n` rows plus `n-1` gaps, so the pitch is exact
  /// instead of a px guess that drifts further down the page.
  fn sync_pitch(&mut self, fonts: &mut FontSystem) {
    let n = self.rows.len().max(1) as f32;
    let (_, h) = self.rows.measure(fonts);
    let pitch = (h + CODE_ROW_SPACING) / n;
    if pitch > 0.0 && pitch.is_finite() {
      self.pitch = pitch;
    }
  }

  /// True while the pointer is over the code overlay bar (same
  /// generous grab area the `Scrollbar` itself uses). Presses there
  /// drive the bar and never reach the text underneath.
  fn over_bar(&self, x: f32, y: f32) -> bool {
    let code = self.zones().0;
    if !self.bar.scrollable() || code.2 <= 0.0 || code.3 <= 0.0 {
      return false;
    }
    let bx = code.0 + code.2 - CODE_BAR_W;
    x >= bx - 2.0
      && x <= code.0 + code.2 + 2.0
      && y >= code.1
      && y <= code.1 + code.3
  }

  /// Sync the bar model with the content: `set_content` flashes the
  /// bar, so it only runs when the model changed (every-frame calls
  /// would keep the bar awake forever). The bar stays the single
  /// source of truth for the offset.
  fn sync_bar(&mut self, fonts: &mut FontSystem) {
    let code = self.zones().0;
    let model = (self.content_h(), code.3);
    if model != self.last_model {
      self.last_model = model;
      self.bar.set_content(model.0, model.1);
    }
    let bw = CODE_BAR_W.min(code.2).max(0.0);
    self.bar.set_rect(
      code.0 + (code.2 - bw).max(0.0),
      code.1,
      bw,
      code.3,
    );
    let content_h = model.0.max(code.3);
    self.rows.place(
      fonts,
      code.0,
      code.1 - self.bar.offset(),
      code.2,
      content_h,
    );
  }

  /// Keep the caret visible inside the code viewport.
  fn track_caret(&mut self) {
    let pitch = self.pitch;
    let (line, _) = self.line_col();
    let top = line as f32 * pitch;
    let offset = self.bar.offset();
    let visible = self.code_h();
    if top - offset < 0.0 {
      self.bar.set_offset(top.max(0.0));
    } else if top + pitch - offset > visible {
      self.bar.set_offset((top + pitch - visible).max(0.0));
    }
  }

  /// Wheel scroll in logical px (down positive, like the shell).
  /// The bar clamps and flashes; rows re-place on the next draw.
  pub fn scroll(&mut self, dx: f64, dy: f64) {
    self.bar.mouse_wheel(dx, dy);
  }

  pub(crate) fn line_col(&self) -> (usize, usize) {
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
    let line = ((rel / self.pitch) as usize).min(lines.len() - 1);
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

  /// Jump the caret to the start of a 1-based line (warning
  /// navigation): focuses, clears the highlight and scrolls the line
  /// into view. Memory only, no fonts needed.
  pub fn goto_line(&mut self, line: usize) {
    let count = self.lines().len().max(1);
    let line = line.max(1).min(count) - 1;
    self.caret = self.caret_from_line_col(line, 0);
    self.anchor = self.caret;
    self.sel = false;
    self.selected = true;
    self.track_caret();
  }

  /// Replace the diagnostic markers (1-based line plus severity).
  /// Sorted and deduplicated with error winning; drawing reads them
  /// every frame, so no rebuild is needed.
  pub fn set_diagnostics(&mut self, markers: &[(usize, bool)]) {
    let mut sorted: Vec<(usize, bool)> = markers
      .iter()
      .filter(|(line, _)| *line >= 1)
      .copied()
      .collect();
    sorted.sort();
    let mut out = Vec::with_capacity(sorted.len());
    for (line, error) in sorted {
      if let Some(last) = out.last_mut() {
        let prev: &mut (usize, bool) = last;
        if prev.0 == line {
          prev.1 = prev.1 || error;
          continue;
        }
      }
      out.push((line, error));
    }
    self.markers = out;
  }

  /// Severity at a 1-based line (`Some(true)` error, `Some(false)`
  /// warning, `None` clean).
  pub(crate) fn marker_at(&self, line: usize) -> Option<bool> {
    self.markers.iter().find(|(no, _)| *no == line).map(|(_, e)| *e)
  }

  /// Press handling: presses over a scrollbar drive that bar and
  /// never reach the text; presses on the divider arm a resize (a
  /// tap toggles the panel like a sidebar); clicks inside the code
  /// arm a focus plus caret resolve on the next draw (caret mapping
  /// needs fonts); anywhere else unfocuses at once.
  pub fn press(&mut self, x: f64, y: f64) {
    self.bar.mouse_down(x, y);
    self.log_bar.mouse_down(x, y);
    let (x, y) = (x as f32, y as f32);
    let (code, divider, _, _, logs) = self.zones();
    if self.over_bar(x, y) || point_in(logs, x, y) {
      return;
    }
    if point_in(divider, x, y) {
      self.divider_drag = true;
      self.divider_moved = false;
      self.divider_start = (y, self.panel_h);
      return;
    }
    if point_in(code, x, y) {
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

  /// Base text color (AGENTS.md palette, no secondary colors).
  fn ink(&self) -> Color {
    if self.dark {
      Color::from_rgb8(0xd8, 0xd9, 0xd9)
    } else {
      Color::from_rgb8(0x27, 0x27, 0x27)
    }
  }

  fn dim(&self) -> Color {
    if self.dark {
      TEXTFIELD_PLACEHOLDER_DARK
    } else {
      TEXTFIELD_PLACEHOLDER_LIGHT
    }
  }

  fn rule(&self) -> Color {
    if self.dark {
      DIVIDER_DARK
    } else {
      DIVIDER_LIGHT
    }
  }

  /// Fake performance sample in 0..100 (deterministic drift plus
  /// jitter from the tick counter, no IO).
  fn fake_sample(tick: u64, phase: f32) -> f32 {
    let t = tick as f32;
    let jitter = ((tick.wrapping_mul(37)) % 13) as f32 - 6.0;
    (48.0 + 26.0 * (t * 0.5 + phase).sin() + jitter).clamp(4.0, 98.0)
  }

  fn push_sample(series: &mut Vec<f32>, value: f32) {
    series.push(value);
    if series.len() > STATS_CAP {
      series.remove(0);
    }
  }

  /// RAM label in MB for a 16 GB machine (0..100 maps to the full
  /// range), instead of a bare percent.
  fn ram_text(value: f32) -> String {
    format!("RAM {:.0} MB", value * 163.84)
  }

  /// One fake cargo log line per tick (build loop, example only).
  fn fake_log_line(tick: u64) -> String {
    match tick % 6 {
      0 => "   Compiling xcode v27.0.0".to_string(),
      1 => "   Compiling sdk v27.0.0 (/Library/System/sdk)".to_string(),
      2 => "    Finished `dev` profile [unoptimized + debuginfo]".to_string(),
      3 => "     Running `target/debug/xcode`".to_string(),
      4 => "warning: unused variable `offset`".to_string(),
      _ => "    Finished in 1.24s".to_string(),
    }
  }

  /// Advance fake stats and logs. Called with the frame time by the
  /// app (see `EditorUi`); appends one sample plus one log line per
  /// elapsed second. Edits stay in memory only, nothing runs.
  pub fn tick(&mut self, now_secs: f64) {
    if now_secs - self.last_tick < TICK_SECONDS {
      return;
    }
    self.last_tick = now_secs;
    self.tick_count += 1;
    Self::push_sample(&mut self.samples_cpu, Self::fake_sample(self.tick_count, 0.0));
    Self::push_sample(&mut self.samples_gpu, Self::fake_sample(self.tick_count, 1.3));
    Self::push_sample(&mut self.samples_ram, Self::fake_sample(self.tick_count, 2.1));
    Self::push_sample(&mut self.samples_disk, Self::fake_sample(self.tick_count, 2.8));
    Self::push_sample(&mut self.samples_net, Self::fake_sample(self.tick_count, 3.6));
    self.logs.push(Self::fake_log_line(self.tick_count));
    if self.logs.len() > LOGS_CAP {
      let drop = self.logs.len() - LOGS_CAP;
      self.logs.drain(..drop);
    }
    // Stay stuck to the bottom while the user never scrolled up.
    if self.stick_logs {
      let list_h = self.zones().4.3 - PANEL_HEADER_H;
      let max = (self.logs.len() as f32 * LOG_LINE_H - list_h).max(0.0);
      self.log_bar.set_offset(max);
    }
  }

  /// Sync the logs bar model (guarded like the code bar so the
  /// overlay keeps fading out while idle).
  fn sync_log_bar(&mut self) {
    let logs = self.zones().4;
    let list_h = (logs.3 - PANEL_HEADER_H).max(0.0);
    let model = (self.logs.len() as f32 * LOG_LINE_H, list_h);
    if model != self.log_model {
      self.log_model = model;
      self.log_bar.set_content(model.0, model.1);
    }
    let bw = CODE_BAR_W.min(logs.2).max(0.0);
    self.log_bar.set_rect(
      logs.0 + (logs.2 - bw).max(0.0),
      logs.1 + PANEL_HEADER_H,
      bw,
      list_h,
    );
  }

  fn draw_label(
    scene: &mut Scene,
    fonts: &mut FontSystem,
    text: &str,
    size: f32,
    color: Color,
    x: f32,
    y: f32,
  ) {
    let layout = fonts.layout_text(text, size, color, None);
    draw_layout(scene, &layout, x, y, fonts.scale);
  }

  /// One sparkline polyline for `samples` (oldest left, newest
  /// right) in custom Vello paths (user-approved: TontooUI ships no
  /// line chart, only gauges).
  fn draw_spark(
    scene: &mut Scene,
    scale: f32,
    samples: &[f32],
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: Color,
  ) {
    if w <= 0.0 || h <= 0.0 || samples.len() < 2 {
      return;
    }
    let step = 4.0;
    let max_n = ((w / step) as usize).max(2);
    let skip = samples.len().saturating_sub(max_n);
    let data = &samples[skip..];
    let mut path = BezPath::new();
    for (i, v) in data.iter().enumerate() {
      let px = x + i as f32 * step;
      let py = y + h - (v.clamp(0.0, 100.0) / 100.0) * h;
      if i == 0 {
        path.move_to((px as f64, py as f64));
      } else {
        path.line_to((px as f64, py as f64));
      }
    }
    let mut stroke = Stroke::new(1.5 * scale as f64);
    stroke.start_cap = Cap::Round;
    stroke.end_cap = Cap::Round;
    stroke.join = Join::Round;
    scene.stroke(&stroke, Affine::IDENTITY, &Brush::Solid(color), None, &path);
  }

  /// Left panel stats: `Performance` header plus a divider-free 2x2
  /// grid (CPU top left, GPU top right, RAM bottom left, bottom right
  /// split into Disk and Network) with one fixed-color sparkline per
  /// metric (fake in-memory data, example only).
  fn draw_stats(
    &self,
    scene: &mut Scene,
    fonts: &mut FontSystem,
    stats: (f32, f32, f32, f32),
  ) {
    if stats.2 <= 0.0 || stats.3 <= 0.0 {
      return;
    }
    let pad = 10.0;
    let gap = 8.0;
    let dim = self.dim();
    Self::draw_label(
      scene,
      fonts,
      "Performance",
      PANEL_LABEL_SIZE,
      dim,
      stats.0 + pad,
      stats.1 + 4.0,
    );
    let top = stats.1 + PANEL_HEADER_H;
    let grid_w = (stats.2 - pad * 2.0).max(0.0);
    let grid_h = (stats.3 - PANEL_HEADER_H).max(0.0);
    let col_w = ((grid_w - gap) / 2.0).max(0.0);
    let row_h = (grid_h / 2.0).max(0.0);
    let sub_w = ((col_w - gap) / 2.0).max(0.0);
    let x0 = stats.0 + pad;
    let x1 = x0 + col_w + gap;
    let cpu = self.samples_cpu.last().copied().unwrap_or(0.0);
    let gpu = self.samples_gpu.last().copied().unwrap_or(0.0);
    let ram = self.samples_ram.last().copied().unwrap_or(0.0);
    let disk = self.samples_disk.last().copied().unwrap_or(0.0);
    let net = self.samples_net.last().copied().unwrap_or(0.0);
    let cells = [
      (x0, top, col_w, row_h, format!("CPU {:.0}%", cpu), &self.samples_cpu, STAT_CPU),
      (x1, top, col_w, row_h, format!("GPU {:.0}%", gpu), &self.samples_gpu, STAT_GPU),
      (x0, top + row_h, col_w, row_h, Self::ram_text(ram), &self.samples_ram, STAT_RAM),
      (x1, top + row_h, sub_w, row_h, format!("Disk {:.0} MB/s", disk * 3.2), &self.samples_disk, STAT_DISK),
      (x1 + sub_w + gap, top + row_h, sub_w, row_h, format!("Network {:.0} Mb/s", net * 1.8), &self.samples_net, STAT_NET),
    ];
    for (cx, cy, cw, ch, label, samples, color) in cells {
      Self::draw_metric(scene, fonts, dim, cx, cy, cw, ch, &label, samples, color);
    }
  }

  /// One stats cell: dim label on top, colorful sparkline below.
  /// Cells are spaced, never divided by rules.
  fn draw_metric(
    scene: &mut Scene,
    fonts: &mut FontSystem,
    dim: Color,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    label: &str,
    samples: &[f32],
    color: Color,
  ) {
    if w <= 0.0 || h <= 0.0 {
      return;
    }
    Self::draw_label(scene, fonts, label, PANEL_LABEL_SIZE, dim, x, y + 2.0);
    let gy = y + 16.0;
    let gh = (h - 18.0).max(0.0);
    Self::draw_spark(scene, fonts.scale, samples, x, gy, w, gh, color);
  }

  /// Right panel logs: `Logs` header plus the fake cargo lines on
  /// the standard overlay bar (wheel, thumb drag, track jump,
  /// bottom stick while the user never scrolled up).
  fn draw_logs(
    &mut self,
    scene: &mut Scene,
    fonts: &mut FontSystem,
    logs: (f32, f32, f32, f32),
  ) {
    if logs.2 <= 0.0 || logs.3 <= 0.0 {
      return;
    }
    let pad = 10.0;
    let x = logs.0 + pad;
    Self::draw_label(
      scene,
      fonts,
      "Logs",
      PANEL_LABEL_SIZE,
      self.dim(),
      x,
      logs.1 + 4.0,
    );
    let list_y = logs.1 + PANEL_HEADER_H;
    let list_h = (logs.3 - PANEL_HEADER_H).max(0.0);
    let list_w = (logs.2 - pad).max(0.0);
    if list_w <= 0.0 || list_h <= 0.0 {
      return;
    }
    self.sync_log_bar();
    let scale = fonts.scale as f64;
    let px = |v: f32| v as f64 * scale;
    let offset = self.log_bar.offset();
    let clip = Rect::new(px(x), px(list_y), px(x + list_w), px(list_y + list_h));
    scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &clip);
    let first = (offset / LOG_LINE_H) as usize;
    let mut y = list_y - (offset % LOG_LINE_H);
    for line in self.logs.iter().skip(first) {
      if y > list_y + list_h {
        break;
      }
      let color = if line.contains("warning") {
        self.accent
      } else {
        self.ink()
      };
      Self::draw_label(scene, fonts, line, LOG_FONT_SIZE, color, x, y);
      y += LOG_LINE_H;
    }
    scene.pop_layer();
  }
}

impl View for CodeEditor {
  fn measure(&mut self, fonts: &mut FontSystem) -> (f32, f32) {
    self.sync_pitch(fonts);
    let (w, h) = self.rows.measure(fonts);
    (w.max(300.0), h.max(self.lines().len() as f32 * self.pitch))
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
    self.sync_pitch(fonts);
    self.sync_bar(fonts);
    self.sync_log_bar();
  }

  fn draw(
    &mut self,
    scene: &mut Scene,
    fonts: &mut FontSystem,
    images: &mut ImageLoader<'_>,
  ) {
    // Refresh the measured pitch first: every caret, wash and
    // click below must use the real row positions.
    self.sync_pitch(fonts);
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
      let mark = self.marker_at(index + 1);
      if let Some(row) = self.rows.child_mut::<HStack>(index) {
        if let Some(gutter) = row.child_mut::<BasicText>(0) {
          // Marked lines keep a tinted number in both themes.
          match mark {
            Some(true) => gutter.set_foreground(TextForeground::Color(DIAG_ERROR)),
            Some(false) => gutter.set_foreground(TextForeground::Color(DIAG_WARN)),
            None => gutter.set_foreground(TextForeground::Secondary),
          }
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
    let (code, divider, panel, stats, logs) = self.zones();
    if code.2 <= 0.0 || code.3 <= 0.0 {
      return;
    }
    let clip = Rect::new(px(code.0), px(code.1), px(code.0 + code.2), px(code.1 + code.3));
    scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &clip);
    // Diagnostic line washes under the text (visible marked lines
    // only, full code width).
    let offset = self.bar.offset();
    let pitch = self.pitch;
    for (line, error) in self.markers.clone() {
      let cy = code.1 + (line as f32 - 1.0) * pitch - offset;
      if cy + pitch < code.1 || cy > code.1 + code.3 {
        continue;
      }
      let base = if error { DIAG_ERROR } else { DIAG_WARN };
      let alpha = if error { DIAG_ERROR_WASH } else { DIAG_WARN_WASH };
      let c = base.to_rgba8();
      scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        &Brush::Solid(Color::from_rgba8(c.r, c.g, c.b, (c.a as f32 * alpha).round() as u8)),
        None,
        &Rect::new(px(code.0), px(cy), px(code.0 + code.2), px(cy + pitch)),
      );
    }
    self.rows.draw(scene, fonts, images);
    // Gutter badges over the numbers (left of the digits, so they
    // never overlap).
    for (line, error) in self.markers.clone() {
      let cy = code.1 + (line as f32 - 1.0) * pitch - offset;
      if cy + pitch < code.1 || cy > code.1 + code.3 {
        continue;
      }
      let color = if error { DIAG_ERROR } else { DIAG_WARN };
      scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        &Brush::Solid(color),
        None,
        &Circle::new(
          (px(code.0 + CODE_GUTTER_W - 23.0), px(cy + pitch / 2.0)),
          px(DIAG_DOT_R),
        ),
      );
    }
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
            self.rect.1 + index as f32 * pitch - offset;
          if cy + pitch >= code.1 && cy <= code.1 + code.3 {
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
                px(cy + pitch - 3.0),
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
        self.rect.1 + line as f32 * pitch - self.bar.offset();
      if cy + pitch >= code.1 && cy <= code.1 + code.3 {
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
            px(cy + pitch - 3.0),
          ),
        );
      }
    }
    scene.pop_layer();
    // Divider rule across the strip: accent plus wider while the
    // strip hovers or drags, like the sidebar edge.
    let grabbing = self.divider_drag || self.divider_hover;
    let (dw, dc) = if grabbing {
      (
        2.0,
        if self.focused {
          self.accent
        } else {
          desaturate(self.accent)
        },
      )
    } else {
      (1.0, self.rule())
    };
    let dy = divider.1 + divider.3 / 2.0;
    scene.stroke(
      &Stroke::new(dw as f64 * scale),
      Affine::IDENTITY,
      &Brush::Solid(dc),
      None,
      &Line::new((px(divider.0), px(dy)), (px(divider.0 + divider.2), px(dy))),
    );
    // Bottom panel: vertical split plus stats left, logs right.
    if !self.collapsed && panel.3 > 0.0 {
      let rule = self.rule();
      let sx = stats.0 + stats.2;
      scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        &Brush::Solid(rule),
        None,
        &Rect::new(px(sx), px(panel.1), px(sx + 1.0), px(panel.1 + panel.3)),
      );
      self.draw_stats(scene, fonts, stats);
      self.draw_logs(scene, fonts, logs);
    }
    self.bar.draw(scene, fonts, images);
    if !self.collapsed {
      self.log_bar.draw(scene, fonts, images);
    }
  }

  fn mouse_down(&mut self, x: f64, y: f64) {
    self.press(x, y);
  }

  fn mouse_up(&mut self, x: f64, y: f64) {
    self.bar.mouse_up(x, y);
    self.log_bar.mouse_up(x, y);
    // Tap on the divider folds the panel away like a sidebar; a
    // drag resizes it instead (see `set_hover`).
    if self.divider_drag {
      if !self.divider_moved {
        self.collapsed = !self.collapsed;
      }
      self.divider_drag = false;
      self.divider_moved = false;
    }
    self.divider_hover = point_in(self.zones().1, x as f32, y as f32);
    self.pressing = false;
    self.pending = None;
    self.drag = None;
  }

  fn set_hover(&mut self, x: f32, y: f32) {
    self.last_hover = (x, y);
    self.bar.mouse_move(x as f64, y as f64);
    self.log_bar.mouse_move(x as f64, y as f64);
    self.divider_hover = self.divider_drag || point_in(self.zones().1, x, y);
    if self.divider_drag {
      let dy = self.divider_start.0 - y;
      if dy.abs() > PANEL_DRAG_SLOP {
        self.divider_moved = true;
      }
      if self.divider_moved {
        self.collapsed = false;
        self.panel_h =
          (self.divider_start.1 + dy).clamp(PANEL_MIN_H, PANEL_MAX_H);
      }
      return;
    }
    // The divider keeps the resize cursor, never the I-beam.
    let (rx, ry, rw, rh) = self.rect;
    self.hovered = !self.divider_hover
      && x >= rx
      && x <= rx + rw
      && y >= ry
      && y <= ry + rh;
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
    // Route by last hover: logs zone drives the logs bar (and
    // leaves the bottom stick once the user scrolls up), anywhere
    // else drives the code bar.
    let (_, _, _, _, logs) = self.zones();
    let (hx, hy) = self.last_hover;
    if !self.collapsed && point_in(logs, hx, hy) {
      self.log_bar.mouse_wheel(dx, dy);
      self.stick_logs =
        self.log_bar.offset() >= self.log_bar.max_offset() - 1.0;
    } else {
      self.bar.mouse_wheel(dx, dy);
    }
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
  fn goto_line_lands_on_line_start() {
    let mut ed = CodeEditor::new("ab\ncde\nf".to_string());
    ed.rect = (0.0, 0.0, 300.0, 200.0);
    ed.goto_line(2);
    assert_eq!(ed.text_value(), "ab\ncde\nf");
    let (line, col) = ed.line_col();
    assert_eq!((line, col), (1, 0));
    assert_eq!(ed.caret, 3);
    // Out of range clamps to the first and last line.
    ed.goto_line(0);
    assert_eq!(ed.caret, 0);
    ed.goto_line(99);
    let (last, _) = ed.line_col();
    assert_eq!(last, 2);
  }

  #[test]
  fn pitch_matches_measured_rows() {
    let mut fonts = FontSystem::new();
    let mut ed = CodeEditor::new("a\nb\nc".to_string());
    ed.sync_pitch(&mut fonts);
    let (_, h) = ed.rows.measure(&mut fonts);
    let expected = (h + CODE_ROW_SPACING) / 3.0;
    assert!((ed.pitch - expected).abs() < 0.001);
    assert!((ed.content_h() - 3.0 * ed.pitch).abs() < 0.001);
    // Stable across syncs (no drift of its own).
    ed.sync_pitch(&mut fonts);
    assert!((ed.pitch - expected).abs() < 0.001);
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
  fn panel_starts_expanded() {
    let ed = CodeEditor::new("hi".to_string());
    assert!(!ed.panel_collapsed());
    assert_eq!(ed.panel_height(), PANEL_DEFAULT_H);
  }

  #[test]
  fn diagnostics_sort_dedupe_and_resolve() {
    let mut ed = CodeEditor::new("a\nb\nc".to_string());
    assert_eq!(ed.marker_at(1), None);
    ed.set_diagnostics(&[(3, false), (0, true), (2, false), (2, true)]);
    // Line 0 filtered out, line 2 keeps error wins.
    assert_eq!(ed.marker_at(1), None);
    assert_eq!(ed.marker_at(2), Some(true));
    assert_eq!(ed.marker_at(3), Some(false));
    ed.set_diagnostics(&[]);
    assert_eq!(ed.marker_at(2), None);
  }

  #[test]
  fn ram_label_shows_megabytes() {
    assert_eq!(CodeEditor::ram_text(50.0), "RAM 8192 MB");
    assert!(CodeEditor::ram_text(0.0).ends_with("MB"));
  }

  #[test]
  fn divider_click_collapses_panel() {
    let mut ed = CodeEditor::new("hi".to_string());
    ed.rect = (0.0, 0.0, 400.0, 600.0);
    // Expanded: code 410px, divider 410..420.
    ed.press(200.0, 415.0);
    assert!(ed.divider_drag);
    <CodeEditor as View>::mouse_up(&mut ed, 200.0, 415.0);
    assert!(ed.panel_collapsed());
    // Collapsed: code 590px, divider 590..600; tap reopens.
    ed.press(200.0, 595.0);
    <CodeEditor as View>::mouse_up(&mut ed, 200.0, 595.0);
    assert!(!ed.panel_collapsed());
  }

  #[test]
  fn divider_drag_resizes_panel() {
    let mut ed = CodeEditor::new("hi".to_string());
    ed.rect = (0.0, 0.0, 400.0, 600.0);
    ed.press(200.0, 415.0);
    <CodeEditor as View>::set_hover(&mut ed, 200.0, 375.0);
    assert!((ed.panel_height() - 220.0).abs() < 0.01);
    <CodeEditor as View>::mouse_up(&mut ed, 200.0, 375.0);
    assert!(!ed.panel_collapsed());
  }

  #[test]
  fn divider_hover_requests_resize_cursor() {
    let mut ed = CodeEditor::new("hi".to_string());
    ed.rect = (0.0, 0.0, 400.0, 600.0);
    assert!(!ed.wants_divider_cursor());
    // Divider strip 410..420.
    <CodeEditor as View>::set_hover(&mut ed, 200.0, 415.0);
    assert!(ed.wants_divider_cursor());
    // Code area keeps the I-beam zone free of the resize cursor.
    <CodeEditor as View>::set_hover(&mut ed, 200.0, 100.0);
    assert!(!ed.wants_divider_cursor());
  }

  #[test]
  fn tick_samples_and_logs_each_second() {
    let mut ed = CodeEditor::new("hi".to_string());
    ed.rect = (0.0, 0.0, 400.0, 600.0);
    ed.tick(0.0);
    assert_eq!(ed.sample_count(), 0);
    assert!(ed.log_lines().is_empty());
    ed.tick(1.0);
    assert_eq!(ed.sample_count(), 1);
    assert_eq!(ed.log_lines().len(), 1);
    ed.tick(1.5);
    assert_eq!(ed.sample_count(), 1);
    ed.tick(2.0);
    assert_eq!(ed.sample_count(), 2);
    assert_eq!(ed.log_lines().len(), 2);
    // All five metric series sample together.
    assert_eq!(ed.samples_gpu.len(), 2);
    assert_eq!(ed.samples_ram.len(), 2);
    assert_eq!(ed.samples_disk.len(), 2);
    assert_eq!(ed.samples_net.len(), 2);
  }

  #[test]
  fn logs_and_samples_cap() {
    let mut ed = CodeEditor::new("hi".to_string());
    ed.rect = (0.0, 0.0, 400.0, 600.0);
    for i in 1..=400u64 {
      ed.tick(i as f64);
    }
    assert_eq!(ed.log_lines().len(), LOGS_CAP);
    assert_eq!(ed.sample_count(), STATS_CAP);
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
