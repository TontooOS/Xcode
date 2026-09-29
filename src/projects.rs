//! Project persistence for Xcode, built on CoreData.
//!
//! Created projects are stored per app (`com.tontoo.xcode`, Fico store)
//! as `Project` entities with `name_en`, `name_de`, `version`,
//! `bundle_id` and `path` attributes, so the recents box survives
//! restarts. There is no open/edit API yet: records are only listed.

use crate::CoreData::{PersistentContainer, StoreType};

/// Bundle id isolating the Xcode store from other apps.
pub const XCODE_BUNDLE_ID: &str = "com.tontoo.xcode";
/// Entity name holding one record per created project.
pub const PROJECT_ENTITY: &str = "Project";

/// One created project, as shown in the recents box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRecord {
  pub name_en: String,
  pub name_de: String,
  pub version: String,
  pub bundle_id: String,
  pub path: String,
}

impl ProjectRecord {
  pub fn new(
    name_en: String,
    name_de: String,
    version: String,
    bundle_id: String,
    path: String,
  ) -> Self {
    Self { name_en, name_de, version, bundle_id, path }
  }

  /// Display name: English name, German fallback when empty.
  pub fn display_name(&self) -> &str {
    if self.name_en.trim().is_empty() {
      &self.name_de
    } else {
      &self.name_en
    }
  }
}

fn container() -> Result<PersistentContainer, String> {
  PersistentContainer::new_with_bundle(XCODE_BUNDLE_ID.to_string(), StoreType::Fico)
    .map_err(|e| format!("coredata: {e}"))
}

/// Load all saved projects. Returns an empty list when the store is
/// missing or unreadable (first launch); only I/O and decode failures
/// inside an existing store produce `Err`.
pub fn load_projects() -> Result<Vec<ProjectRecord>, String> {
  let mut container = container()?;
  let ctx = container.view_context();
  let objects = ctx
    .fetch_all(PROJECT_ENTITY)
    .map_err(|e| format!("coredata fetch: {e}"))?;
  Ok(
    objects
      .iter()
      .map(|o| ProjectRecord {
        name_en: o.get_str("name_en").unwrap_or("").to_string(),
        name_de: o.get_str("name_de").unwrap_or("").to_string(),
        version: o.get_str("version").unwrap_or("").to_string(),
        bundle_id: o.get_str("bundle_id").unwrap_or("").to_string(),
        path: o.get_str("path").unwrap_or("").to_string(),
      })
      .collect(),
  )
}

/// Append one project record and flush the store.
pub fn save_project(record: &ProjectRecord) -> Result<(), String> {
  let mut container = container()?;
  let mut ctx = container.view_context();
  let mut obj = ctx.create(PROJECT_ENTITY);
  obj.set("name_en", record.name_en.clone());
  obj.set("name_de", record.name_de.clone());
  obj.set("version", record.version.clone());
  obj.set("bundle_id", record.bundle_id.clone());
  obj.set("path", record.path.clone());
  ctx
    .save_object(obj)
    .map_err(|e| format!("coredata save: {e}"))?;
  ctx.save().map_err(|e| format!("coredata flush: {e}"))?;
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  /// Isolated store: never touches the real user preferences.
  fn test_env(bundle: &str) -> PathGuard {
    let dir = std::env::temp_dir().join(format!(
      "xcode-coredata-test-{}",
      std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("TONTOO_PREFERENCES_ROOT", dir.to_string_lossy().to_string());
    std::env::set_var("TONTOO_COREDATA_ALLOW_FOREIGN", "1");
    std::env::set_var(
      "TONTOO_COREDATA_KEY_FILE",
      dir.join("keyfile").to_string_lossy().to_string(),
    );
    std::env::set_var("TONTOO_APP_BUNDLE_ID", bundle);
    PathGuard(dir)
  }

  struct PathGuard(std::path::PathBuf);

  impl Drop for PathGuard {
    fn drop(&mut self) {
      std::env::remove_var("TONTOO_PREFERENCES_ROOT");
      std::env::remove_var("TONTOO_COREDATA_ALLOW_FOREIGN");
      std::env::remove_var("TONTOO_COREDATA_KEY_FILE");
      std::env::remove_var("TONTOO_APP_BUNDLE_ID");
      let _ = std::fs::remove_dir_all(&self.0);
    }
  }

  #[test]
  fn roundtrip_survives_reload() {
    // Single CoreData test: env vars are process-global, so all store
    // I/O stays in this one test.
    let _guard = test_env("com.tontoo.xcode.test");
    assert_eq!(load_projects().unwrap_or_default().len(), 0);
    let record = ProjectRecord::new(
      "MyApp".to_string(),
      "MeineApp".to_string(),
      "1.0".to_string(),
      "de.arlomu.myapp".to_string(),
      "/tmp/MyApp".to_string(),
    );
    save_project(&record).expect("save project");
    let loaded = load_projects().expect("load projects");
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0], record);
    assert_eq!(loaded[0].display_name(), "MyApp");
  }

  #[test]
  fn display_name_falls_back_to_german() {
    let record = ProjectRecord::new(
      String::new(),
      "MeineApp".to_string(),
      "1.0".to_string(),
      "de.arlomu.x".to_string(),
      "/tmp/x".to_string(),
    );
    assert_eq!(record.display_name(), "MeineApp");
  }
}
