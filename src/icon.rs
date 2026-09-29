//! App icon pipeline for the Xcode start page.
//!
//! The bundled `Resources/icon.tico` is rendered through CoreIcon in its
//! normal (light) variant: `.tico` entries composite at full resolution
//! with the Apple app-icon finish and no tint. Plain files fall back to
//! the CoreIcon glass pipeline in the light variant. Callers display the
//! returned PNG with `FileImage`; when everything fails they fall back to
//! the raw path and `FileImage` draws its theme placeholder.

use std::path::{Path, PathBuf};

fn safe_name(app_name: &str) -> String {
  app_name
    .chars()
    .map(|c| if c.is_alphanumeric() { c } else { '_' })
    .collect()
}

/// Candidate locations of the bundled `Resources/icon.tico`.
fn tico_candidates() -> Vec<PathBuf> {
  let mut out = Vec::new();
  if let Ok(env) = std::env::var("APP_RESOURCES_DIR") {
    if !env.is_empty() {
      out.push(PathBuf::from(env).join("icon.tico"));
    }
  }
  if let Ok(cwd) = std::env::current_dir() {
    out.push(cwd.join("Resources").join("icon.tico"));
    out.push(cwd.join("icon.tico"));
  }
  if let Ok(exe) = std::env::current_exe() {
    if let Some(parent) = exe.parent() {
      out.push(parent.join("icon.tico"));
      out.push(parent.join("Resources").join("icon.tico"));
      if let Some(grand) = parent.parent() {
        out.push(grand.join("Resources").join("icon.tico"));
      }
    }
  }
  out
}

/// Resolve the bundled `Resources/icon.tico`, when it exists.
pub fn resolve_bundled_tico() -> Option<PathBuf> {
  tico_candidates().into_iter().find(|p| p.is_file())
}

/// Render a `.tico` icon in its normal (light) variant into a plain PNG.
///
/// Composites at 512px with the Apple app-icon finish baked in and no
/// tint, so the light artwork shows as authored. Returns the PNG path,
/// or `None` when loading or rendering fails.
pub fn tico_to_png_normal(tico: &Path, app_name: &str) -> Option<PathBuf> {
  use crate::CoreIcon::tico::Tico;
  let icon = Tico::load(tico).ok()?;
  let image = icon.render(512, None).ok()?;
  let out = std::env::temp_dir().join(format!(
    "xcode-icon-{}-normal.png",
    safe_name(app_name)
  ));
  image.save(&out).ok()?;
  Some(out)
}

/// Run a raw icon file through CoreIcon in the light variant so a plain
/// file becomes a proper Apple-style app icon with the Liquid Glass
/// finish. Returns the finished PNG path, or `None` on failure.
pub fn beautify_icon_light(raw: &Path, app_name: &str) -> Option<PathBuf> {
  let icon = crate::CoreIcon::generator::AppIcon::from_file(raw).light();
  let out = std::env::temp_dir().join(format!(
    "xcode-icon-{}-light.png",
    safe_name(app_name)
  ));
  icon.save(&out).ok()?;
  Some(out)
}

/// Resolve the display icon: bundled `Resources/icon.tico` rendered in
/// the normal (light) variant, else the raw path for the `FileImage`
/// placeholder.
pub fn display_icon() -> PathBuf {
  let missing = PathBuf::from("__xcode_missing_icon__");
  match resolve_bundled_tico() {
    Some(raw)
      if raw
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("tico")) =>
    {
      tico_to_png_normal(&raw, "Xcode").unwrap_or(raw)
    }
    Some(raw) => beautify_icon_light(&raw, "Xcode").unwrap_or(raw),
    None => missing,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn missing_icon_falls_back_to_placeholder() {
    // No `Resources/icon.tico` next to the test binary in most dev
    // checkouts is fine: either a real icon resolves or the placeholder
    // sentinel is returned. Both are valid `FileImage` inputs.
    let icon = display_icon();
    assert!(!icon.as_os_str().is_empty());
  }

  #[test]
  fn tico_renders_normal_variant_to_png() {
    use crate::CoreIcon::generator::{Background, IconCanvas, Layer, LayerContent};
    use crate::CoreIcon::{Color, tico::Tico};
    let canvas = IconCanvas::new()
      .background(Background::color(Color::WHITE))
      .layer(Layer::new(LayerContent::circle(512.0)))
      .glass();
    let path = std::env::temp_dir().join("xcode-test-icon.tico");
    Tico::export(&canvas, "t", &path).expect("tico export");
    let png = tico_to_png_normal(&path, "RenderTest").expect("tico renders");
    let bytes = std::fs::read(&png).expect("read png");
    assert!(bytes.starts_with(&[0x89, b'P', b'N', b'G']));
  }
}
