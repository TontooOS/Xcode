# Projects

Created projects persist across restarts in a CoreData store and appear
in the start page list. `Create` in the new-project sheet scaffolds a
real app skeleton on disk: `<chosen>/<en_name>/` with `Cargo.toml`,
`tontoo.proj`, `.gitignore`, `src/app.rs`, `src/content_view.rs`,
`lang/en_us.json`, `lang/de_de.json` and an empty `Resources/` folder.
There is no open/edit action yet: records are only listed.

## Store

`projects::load_projects` reads all `Project` entities, `save_project`
appends one record and flushes.

```rust
pub fn load_projects() -> Result<Vec<ProjectRecord>, String>
pub fn save_project(record: &ProjectRecord) -> Result<(), String>
```

### ProjectRecord

| Field | Type | Description |
|---|---|---|
| `name_en` | `String` | English app name (required, also the folder name) |
| `name_de` | `String` | German app name (optional, falls back to English) |
| `version` | `String` | Project version (required) |
| `bundle_id` | `String` | Organization identifier, defaults to `dev.<username>` (required) |
| `path` | `String` | Scaffolded project root on disk |

### Rules

- Store: per-app Fico container for bundle `com.tontoo.xcode`
  (`/Users/<user>/Library/Preferences/com.tontoo.xcode/`).
- `display_name()` returns the English name, German fallback when empty.
- A missing/unreadable store loads as an empty list (first launch);
  only failures inside an existing store produce `Err`.
- A store write failure after a successful scaffold only logs to
  stderr: the folder exists and the project still shows until quit.
- No serde usage in this crate.

## Scaffold

`scaffold::create_project` writes the skeleton and returns its root.

```rust
pub fn create_project(
  parent: &Path,
  en_name: &str,
  de_name: &str,
  version: &str,
  bundle_id: &str,
) -> Result<PathBuf, String>
```

### Generated files

| Path | Content |
|---|---|
| `Cargo.toml` | Package `sanitize_cargo_name(en_name)` at `version`, `[[bin]]` pointing at `src/app.rs`, SDK deps (`TontooUI`, `Foundation`, `Accessibility`, `CoreData`) plus `vello` |
| `tontoo.proj` | `bundle_id`, `name` (English string, build-safe), `version`, `icon: Resources/icon.tico` |
| `.gitignore` | `target/` and `Cargo.lock` |
| `src/app.rs` | Minimal TontooUI app shell (titlebar, theme watcher, close/minimize/maximize) hosting the content card |
| `src/content_view.rs` | Example card: `Hello World` (`Title2`) plus a capsule `Tap Me` button printing `tapped` (lowercase file: Rust module `content_view` only resolves to a lowercase file on Linux) |
| `lang/en_us.json` | `{"lang": "en_us", "translations": {"name": "<en>"}}` (build input: TBuild reads localized names from `<project>/lang`) |
| `lang/de_de.json` | Same shape with the German name (English fallback when empty) |
| `Resources/lang/en_us.json` | Runtime mirror of the English strings (TBuild stages `Resources/` into the bundle, where the running app looks them up) |
| `Resources/lang/de_de.json` | Runtime mirror of the German strings |
| `Resources/` | Empty folder for the future `icon.tico` |

### Rules

- `validate` rejects empty EN name/version/bundle ID
  (`sheet.err_required`), names with no usable characters
  (`sheet.err_name`) and non-numeric versions (`sheet.err_version`);
  an existing target folder fails with `sheet.err_exists`.
- `sanitize_cargo_name` lowercases and maps anything but ASCII
  alphanumeric to `-` (spaces included), trimmed of edge dashes.
- `normalize_version` pads short numeric versions to full Cargo semver
  (`1` and `1.0` become `1.0.0`); the normalized form lands in both
  `Cargo.toml` and `tontoo.proj`.
- `name` in `tontoo.proj` stays a plain string: TBuild parses it with
  `str_field` (`TBuild/src/main.rs`), so an object would fail the
  build. Both display names live in the generated `lang/` files, which
  TBuild reads via `bundle_names` into the bundle manifest.
- `Resources/lang/` mirrors `lang/` because TBuild stages the whole
  `Resources/` dir into the `.app` container, where the running app
  looks its strings up (same layout as AboutThisApp).
- `documents_dir()` resolves `$HOME/Documents` (current directory
  fallback) as the folder chooser root.
- `default_bundle_id()` builds `dev.<username>` from `USER` (then
  `USERNAME`, then `user`), lowercased to reverse-DNS
  (`dev.user` fallback); used as the org prefill and the preview
  fallback.

## Folder chooser flow

Step 2 of the sheet browses real directories starting at
`~/Documents`: subdirectory buttons plus `..` (hidden dotfiles
skipped, sorted, first 8 shown, no scrolling yet), a target preview
(`<dir>/<en_name>`, middle-truncated past 48 chars) and `Cancel` /
`Create`. `Create` scaffolds into the shown folder, saves the
CoreData record and closes the sheet; the project then appears as the
first (selected) list row.

### Rules

- Step 1 `Create` only advances after `validate` passes, otherwise a
  red `sheet.err_*` hint shows inside the form.
- The sheet card resizes per step (292px form, dynamic chooser height).
- An open sheet disables the background drag region; presses reach the
  fields and buttons directly.
- The list shows at most 4 rows (display only, first row selected) and
  the dim `project.empty` (`No Projects`) line while empty.

## Usage / Example

```bash
cargo run
```

Click `New Project`, fill EN name/version/bundle ID, press `Create`,
pick a folder, press `Create` again: `<folder>/<en_name>/` appears on
disk and the project shows in the list after the sheet closes. Quit
and restart: the list persists via CoreData.

## Cross References

- [StartPage.md](StartPage.md) – start window layout and sheet embedding
- [MAIN.md](MAIN.md) – project overview and quick start
- [RULE.md](RULE.md) – wiki design system and repo rules
