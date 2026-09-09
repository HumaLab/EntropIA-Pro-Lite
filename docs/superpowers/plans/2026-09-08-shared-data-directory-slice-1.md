# Shared Data Directory — Slice 1: Path Helpers and Characterization Tests

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract the asset-path helpers that sync already owns into a shared module, add a resolver that accepts either a relative or an absolute stored path, and pin the current behavior of the two untested legacy-migration functions — all without changing where any file lives or what any column holds.

**Architecture:** `sync::blobs::derive_rel_path` and `RelPathError` move to `path_utils`, which already hosts `ensure_within_dir`, `validate_existing_file`, and `canonicalize_allowing_missing_tail`. `sync::blobs` re-exports them so the sync module and its tests keep compiling unchanged. A new `resolve_asset_path` generalizes the absolute-or-relative pattern that `llm/mod.rs:2139-2171` already implements privately. `migrate_legacy_app_dir` and `migrate_legacy_asset_paths` gain characterization tests that describe what they do today, so slice 4 can generalize them against a safety net.

**Tech Stack:** Rust, Tauri v2, rusqlite, tempfile (already a dependency at `Cargo.toml:33`).

**Spec:** `docs/superpowers/specs/2026-09-08-shared-data-directory-design.md`

## Global Constraints

- Nothing in this slice changes an on-disk location, a stored path value, or any caller's behavior. It is additive plus one mechanical move.
- Do **not** install the `assets` storage-guard trigger in this slice. All 2475 stored rows and every writer are still absolute; an absolute-rejecting guard here would abort the first import. It belongs at the end of slice 2.
- The canonical relative form is `assets/<collection>/<item>/<file>` with forward slashes.
- Test names describe behavior, not implementation. Match the existing style in `lib.rs` `mod tests` (line 1200) and `sync/apply/tests.rs`.
- Rust artifacts — identifiers, comments, doc comments, test names — are written in English, matching the surrounding code.
- Run Rust tests from `apps/desktop/src-tauri`.

---

### Task 1: Characterization tests for `migrate_legacy_app_dir`

`migrate_legacy_app_dir` (`lib.rs:686`) has no tests, and slice 4 generalizes it from one legacy identifier to eight. These tests pin what it does today.

The function is currently hardcoded to `LEGACY_APP_IDENTIFIER` (`lib.rs:45`) and derives the legacy directory as `app_dir.parent().join(LEGACY_APP_IDENTIFIER)`, so a test drives it by creating a temp parent holding both `com.entropia.app` and a target directory.

**Files:**
- Modify: `apps/desktop/src-tauri/src/lib.rs` — add tests inside the existing `#[cfg(test)] mod tests` block (starts line 1200)

**Interfaces:**
- Consumes: `migrate_legacy_app_dir(app_dir: &Path) -> Result<(), String>`, `LEGACY_APP_IDENTIFIER`, `LEGACY_MIGRATION_MARKER`, `SQLITE_BASENAME` — all already defined in `lib.rs`
- Produces: nothing new; this task only adds tests

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `apps/desktop/src-tauri/src/lib.rs`:

```rust
    /// Builds a temp parent directory containing a populated legacy app dir.
    /// Returns (parent, legacy_dir, target_dir) where target_dir does NOT exist yet.
    fn legacy_fixture() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let parent = tempfile::tempdir().expect("tempdir");
        let legacy_dir = parent.path().join(LEGACY_APP_IDENTIFIER);
        let target_dir = parent.path().join("com.entropia.target");
        fs::create_dir_all(legacy_dir.join("assets")).expect("legacy assets dir");
        fs::write(legacy_dir.join("assets").join("photo.jpg"), b"bytes").expect("legacy asset");
        (parent, legacy_dir, target_dir)
    }

    /// Creates a database at `path` with `rows` rows in `items`, so
    /// `sqlite_richness_score` has something to compare.
    fn seed_db(path: &std::path::Path, rows: usize) {
        let conn = Connection::open(path).expect("open db");
        conn.execute_batch("CREATE TABLE items (id TEXT PRIMARY KEY);")
            .expect("create items");
        for index in 0..rows {
            conn.execute(
                "INSERT INTO items(id) VALUES (?1)",
                rusqlite::params![format!("item-{index}")],
            )
            .expect("insert item");
        }
    }

    #[test]
    fn migrate_legacy_app_dir_renames_when_the_target_does_not_exist() {
        let (_parent, legacy_dir, target_dir) = legacy_fixture();

        migrate_legacy_app_dir(&target_dir).expect("migration succeeds");

        assert!(!legacy_dir.exists(), "legacy dir is consumed by the rename");
        assert!(
            target_dir.join("assets").join("photo.jpg").exists(),
            "legacy content is reachable at the target"
        );
        assert!(
            target_dir.join(LEGACY_MIGRATION_MARKER).exists(),
            "the marker records that the merge completed"
        );
    }

    #[test]
    fn migrate_legacy_app_dir_merges_only_missing_files_when_the_target_exists() {
        let (_parent, legacy_dir, target_dir) = legacy_fixture();
        fs::create_dir_all(target_dir.join("assets")).expect("target assets dir");
        fs::write(target_dir.join("assets").join("photo.jpg"), b"target wins")
            .expect("target asset");
        fs::write(legacy_dir.join("assets").join("only-legacy.jpg"), b"bytes")
            .expect("legacy-only asset");

        migrate_legacy_app_dir(&target_dir).expect("migration succeeds");

        assert_eq!(
            fs::read(target_dir.join("assets").join("photo.jpg")).expect("read"),
            b"target wins".to_vec(),
            "an existing target file is never overwritten"
        );
        assert!(
            target_dir.join("assets").join("only-legacy.jpg").exists(),
            "a file missing from the target is brought over"
        );
    }

    #[test]
    fn migrate_legacy_app_dir_skips_the_scan_once_the_marker_exists() {
        let (_parent, legacy_dir, target_dir) = legacy_fixture();
        fs::create_dir_all(&target_dir).expect("target dir");
        fs::write(target_dir.join(LEGACY_MIGRATION_MARKER), b"").expect("marker");
        fs::write(legacy_dir.join("assets").join("late-arrival.jpg"), b"bytes")
            .expect("late asset");

        migrate_legacy_app_dir(&target_dir).expect("migration succeeds");

        assert!(
            !target_dir.join("assets").join("late-arrival.jpg").exists(),
            "the marker short-circuits the merge entirely"
        );
    }

    #[test]
    fn migrate_legacy_app_dir_prefers_the_richer_legacy_database() {
        let (_parent, legacy_dir, target_dir) = legacy_fixture();
        fs::create_dir_all(&target_dir).expect("target dir");
        seed_db(&legacy_dir.join(SQLITE_BASENAME), 5);
        seed_db(&target_dir.join(SQLITE_BASENAME), 1);

        migrate_legacy_app_dir(&target_dir).expect("migration succeeds");

        let conn = Connection::open(target_dir.join(SQLITE_BASENAME)).expect("open target db");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM items", [], |row| row.get(0))
            .expect("count items");
        assert_eq!(count, 5, "the richer legacy database replaces the poorer target");
    }

    #[test]
    fn migrate_legacy_app_dir_keeps_the_richer_target_database() {
        let (_parent, legacy_dir, target_dir) = legacy_fixture();
        fs::create_dir_all(&target_dir).expect("target dir");
        seed_db(&legacy_dir.join(SQLITE_BASENAME), 1);
        seed_db(&target_dir.join(SQLITE_BASENAME), 5);

        migrate_legacy_app_dir(&target_dir).expect("migration succeeds");

        let conn = Connection::open(target_dir.join(SQLITE_BASENAME)).expect("open target db");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM items", [], |row| row.get(0))
            .expect("count items");
        assert_eq!(count, 5, "a target at least as rich as the legacy one is kept");
    }

    #[test]
    fn migrate_legacy_app_dir_is_a_no_op_without_a_legacy_directory() {
        let parent = tempfile::tempdir().expect("tempdir");
        let target_dir = parent.path().join("com.entropia.target");

        migrate_legacy_app_dir(&target_dir).expect("migration succeeds");

        assert!(!target_dir.exists(), "nothing is created when there is nothing to migrate");
    }

    #[test]
    fn migrate_legacy_app_dir_running_twice_changes_nothing() {
        let (_parent, _legacy_dir, target_dir) = legacy_fixture();

        migrate_legacy_app_dir(&target_dir).expect("first run");
        let after_first: Vec<_> = fs::read_dir(target_dir.join("assets"))
            .expect("read assets")
            .map(|entry| entry.expect("entry").file_name())
            .collect();

        migrate_legacy_app_dir(&target_dir).expect("second run");
        let after_second: Vec<_> = fs::read_dir(target_dir.join("assets"))
            .expect("read assets")
            .map(|entry| entry.expect("entry").file_name())
            .collect();

        assert_eq!(after_first, after_second, "the migration is idempotent");
    }
```

- [ ] **Step 2: Run the tests to see how they fail**

Run: `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml migrate_legacy_app_dir -- --nocapture`

Expected: compilation errors first (`fs` or `tempfile` not in scope inside `mod tests`). `mod tests` opens with `use super::*;`, and `lib.rs` already has `use std::fs;`, so `fs` resolves; `tempfile` is a plain dependency and resolves by crate name. If any name fails to resolve, add the missing `use` inside `mod tests` rather than at file scope.

These are characterization tests: they describe behavior that already exists, so once they compile they should PASS. A failure here is a real finding about the current implementation — record it, do not "fix" the test to match.

- [ ] **Step 3: Reconcile any surprise**

If a test fails, the current behavior differs from what the function's code suggests. Read the failure, correct the test to state what the code actually does, and add a one-line comment naming the surprise. Do not change `migrate_legacy_app_dir` in this slice — it is generalized in slice 4, and this net exists to catch that change.

- [ ] **Step 4: Run the full lib test suite**

Run: `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib`
Expected: PASS, with seven new tests.

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/src-tauri/src/lib.rs
git commit -m "test(data-dir): fijar el comportamiento actual de migrate_legacy_app_dir

Slice 4 generaliza esta función de un identificador legacy a ocho, y hoy
no tiene un solo test. Estos siete describen lo que hace ahora: renombra
cuando el destino no existe, fusiona solo lo faltante cuando existe, el
marcador corta el escaneo, la base más rica gana, y correrla dos veces no
cambia nada."
```

---

### Task 2: Characterization tests for `migrate_legacy_asset_paths`

`migrate_legacy_asset_paths` (`lib.rs:978`) rewrites absolute `assets.path` prefixes when the app data directory moves. Slice 2 changes it to strip the prefix instead of replacing it, and slice 4 runs it across eight identifiers. Pin it first.

**Files:**
- Modify: `apps/desktop/src-tauri/src/lib.rs` — add tests inside `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `migrate_legacy_asset_paths(db_path: &Path, app_dir: &Path) -> Result<(), String>`, `LEGACY_APP_IDENTIFIER`, and `seed_db` from Task 1
- Produces: nothing new; tests only

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `apps/desktop/src-tauri/src/lib.rs`:

```rust
    /// Creates a database with an `assets` table holding the given paths.
    fn seed_assets(db_path: &std::path::Path, paths: &[&str]) {
        let conn = Connection::open(db_path).expect("open db");
        conn.execute_batch("CREATE TABLE assets (id TEXT PRIMARY KEY, path TEXT NOT NULL);")
            .expect("create assets");
        for (index, path) in paths.iter().enumerate() {
            conn.execute(
                "INSERT INTO assets(id, path) VALUES (?1, ?2)",
                rusqlite::params![format!("asset-{index}"), path],
            )
            .expect("insert asset");
        }
    }

    fn asset_paths(db_path: &std::path::Path) -> Vec<String> {
        let conn = Connection::open(db_path).expect("open db");
        let mut stmt = conn
            .prepare("SELECT path FROM assets ORDER BY id")
            .expect("prepare");
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query");
        rows.map(|row| row.expect("row")).collect()
    }

    #[test]
    fn migrate_legacy_asset_paths_rewrites_the_legacy_prefix() {
        let parent = tempfile::tempdir().expect("tempdir");
        let app_dir = parent.path().join("com.entropia.target");
        let legacy_dir = parent.path().join(LEGACY_APP_IDENTIFIER);
        fs::create_dir_all(&app_dir).expect("app dir");
        let db_path = app_dir.join(SQLITE_BASENAME);
        let legacy_asset = format!("{}/assets/col/item/photo.jpg", legacy_dir.display());
        seed_assets(&db_path, &[&legacy_asset]);

        migrate_legacy_asset_paths(&db_path, &app_dir).expect("migration succeeds");

        let expected = format!("{}/assets/col/item/photo.jpg", app_dir.display());
        assert_eq!(asset_paths(&db_path), vec![expected]);
    }

    #[test]
    fn migrate_legacy_asset_paths_leaves_unrelated_paths_alone() {
        let parent = tempfile::tempdir().expect("tempdir");
        let app_dir = parent.path().join("com.entropia.target");
        fs::create_dir_all(&app_dir).expect("app dir");
        let db_path = app_dir.join(SQLITE_BASENAME);
        let foreign = "D:/somewhere/else/photo.jpg";
        seed_assets(&db_path, &[foreign]);

        migrate_legacy_asset_paths(&db_path, &app_dir).expect("migration succeeds");

        assert_eq!(
            asset_paths(&db_path),
            vec![foreign.to_string()],
            "a path outside the legacy dir is untouched"
        );
    }

    #[test]
    fn migrate_legacy_asset_paths_running_twice_changes_nothing() {
        let parent = tempfile::tempdir().expect("tempdir");
        let app_dir = parent.path().join("com.entropia.target");
        let legacy_dir = parent.path().join(LEGACY_APP_IDENTIFIER);
        fs::create_dir_all(&app_dir).expect("app dir");
        let db_path = app_dir.join(SQLITE_BASENAME);
        seed_assets(
            &db_path,
            &[&format!("{}/assets/col/item/photo.jpg", legacy_dir.display())],
        );

        migrate_legacy_asset_paths(&db_path, &app_dir).expect("first run");
        let after_first = asset_paths(&db_path);
        migrate_legacy_asset_paths(&db_path, &app_dir).expect("second run");

        assert_eq!(after_first, asset_paths(&db_path), "the rewrite is idempotent");
    }

    #[test]
    fn migrate_legacy_asset_paths_skips_a_database_without_an_assets_table() {
        let parent = tempfile::tempdir().expect("tempdir");
        let app_dir = parent.path().join("com.entropia.target");
        fs::create_dir_all(&app_dir).expect("app dir");
        let db_path = app_dir.join(SQLITE_BASENAME);
        seed_db(&db_path, 1);

        migrate_legacy_asset_paths(&db_path, &app_dir).expect("migration succeeds without assets");
    }
```

- [ ] **Step 2: Run the tests**

Run: `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml migrate_legacy_asset_paths -- --nocapture`

Expected: PASS. These describe existing behavior.

Note on separators: the test builds the legacy path with `{}` on a `Path`, which on Windows yields backslashes, matching what the function's `LIKE` and `REPLACE` compare against. Do not hand-write forward slashes into the prefix — the function does no separator normalization, and pretending otherwise would encode a false expectation.

- [ ] **Step 3: Run the full lib test suite**

Run: `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add apps/desktop/src-tauri/src/lib.rs
git commit -m "test(data-dir): fijar el comportamiento actual de migrate_legacy_asset_paths

Slice 2 la convierte de reemplazar un prefijo absoluto por otro a quitarlo,
y slice 4 la corre sobre ocho identificadores. Estos cuatro tests describen
lo que hace hoy: reescribe el prefijo legacy, no toca rutas ajenas, es
idempotente, y no falla contra una base sin tabla assets."
```

---

### Task 3: Move the asset-path helpers into `path_utils`

`derive_rel_path` and `RelPathError` live in `sync::blobs` (`blobs.rs:38-107`) but describe the storage format the whole application is adopting. Move them to `path_utils`, which already owns `ensure_within_dir` and the canonicalization helpers, and re-export from `sync::blobs` so sync and its tests compile untouched.

**Files:**
- Modify: `apps/desktop/src-tauri/src/path_utils.rs` — receive `RelPathError` and `derive_rel_path`
- Modify: `apps/desktop/src-tauri/src/sync/blobs.rs:38-107` — remove the definitions, add a re-export
- Test: `apps/desktop/src-tauri/src/path_utils.rs` — a `#[cfg(test)] mod tests` block

**Interfaces:**
- Consumes: nothing from earlier tasks
- Produces:
  - `path_utils::RelPathError` — `enum { OutsideAppData, NotUnderAssets, Empty }`, deriving `Debug, PartialEq, Eq` and implementing `Display`
  - `path_utils::derive_rel_path(abs_path: &str, data_dir: &Path) -> Result<String, RelPathError>`
  - `sync::blobs` continues to expose both names via `pub use crate::path_utils::{derive_rel_path, RelPathError};`

- [ ] **Step 1: Move the code**

Cut `RelPathError` (its enum, `Display` impl) and `derive_rel_path` verbatim from `apps/desktop/src-tauri/src/sync/blobs.rs` and paste them into `apps/desktop/src-tauri/src/path_utils.rs`. Change nothing about the bodies — this is a move, not a rewrite. Update the doc comment on `derive_rel_path` to say "data dir" instead of "app-data dir", and rename its second parameter from `app_data_dir` to `data_dir`, because the directory it anchors at is about to stop being Tauri's app-data dir.

In `sync/blobs.rs`, at the position the definitions occupied, add:

```rust
// The asset-path representation is shared storage vocabulary, not a sync
// concern: it now lives in `path_utils` and is re-exported here so this
// module and its tests keep their existing surface.
pub use crate::path_utils::{derive_rel_path, RelPathError};
```

- [ ] **Step 2: Run the existing sync tests to prove the move is behavior-neutral**

Run: `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib sync::blobs`

Expected: PASS. `derive_rel_path_strips_prefix_and_normalizes`, `derive_rel_path_handles_unicode_names`, `derive_rel_path_rejects_outside_app_data`, `derive_rel_path_rejects_non_assets_subtree`, and `derive_rel_path_rejects_empty` all still pass through the re-export. If any fails, the move was not verbatim — revert and redo it.

- [ ] **Step 3: Commit the move on its own**

```bash
git add apps/desktop/src-tauri/src/path_utils.rs apps/desktop/src-tauri/src/sync/blobs.rs
git commit -m "refactor(data-dir): mover derive_rel_path a path_utils

La representación relativa de una ruta de asset es vocabulario de
almacenamiento, no del protocolo de sync. Se muda a path_utils, junto a
ensure_within_dir, y sync la re-exporta para no cambiar su superficie.
Los tests existentes de sync::blobs pasan sin tocarse: el movimiento es
literal."
```

---

### Task 4: Add `resolve_asset_path`

`llm/mod.rs:2139-2171` already implements "absolute or relative, join if relative" privately. Slice 2 needs that behavior in `image_edit.rs` and elsewhere, so it becomes a named helper with its own tests.

**Files:**
- Modify: `apps/desktop/src-tauri/src/path_utils.rs`
- Test: `apps/desktop/src-tauri/src/path_utils.rs` — `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `path_utils::derive_rel_path` from Task 3
- Produces: `path_utils::resolve_asset_path(stored: &str, data_dir: &Path) -> PathBuf` — returns `stored` unchanged when it is already absolute, otherwise joins it under `data_dir` component-by-component after normalizing `\` to `/`. It does **not** validate: `validate_inbound_rel_path` remains the validating entry point for untrusted input.

- [ ] **Step 1: Write the failing tests**

Add to `apps/desktop/src-tauri/src/path_utils.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn data_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn resolve_asset_path_joins_a_relative_key_under_the_data_dir() {
        let dir = data_dir();
        let resolved = resolve_asset_path("assets/col-1/item-1/photo.jpg", dir.path());
        assert_eq!(resolved, dir.path().join("assets").join("col-1").join("item-1").join("photo.jpg"));
    }

    #[test]
    fn resolve_asset_path_normalizes_backslashes_in_a_relative_key() {
        let dir = data_dir();
        let resolved = resolve_asset_path(r"assets\col-1\item-1\photo.jpg", dir.path());
        assert_eq!(resolved, dir.path().join("assets").join("col-1").join("item-1").join("photo.jpg"));
    }

    #[test]
    fn resolve_asset_path_returns_an_absolute_path_unchanged() {
        let dir = data_dir();
        let absolute = dir.path().join("assets").join("photo.jpg");
        let resolved = resolve_asset_path(&absolute.to_string_lossy(), dir.path());
        assert_eq!(resolved, absolute);
    }

    #[test]
    fn resolve_asset_path_returns_a_foreign_absolute_path_unchanged() {
        let dir = data_dir();
        let foreign = if cfg!(windows) { r"D:\elsewhere\photo.jpg" } else { "/elsewhere/photo.jpg" };
        let resolved = resolve_asset_path(foreign, dir.path());
        assert_eq!(resolved, std::path::PathBuf::from(foreign));
    }

    #[test]
    fn derive_rel_path_and_resolve_asset_path_round_trip() {
        let dir = data_dir();
        let absolute = dir.path().join("assets").join("col-1").join("photo.jpg");
        let relative = derive_rel_path(&absolute.to_string_lossy(), dir.path()).expect("derive");
        assert_eq!(relative, "assets/col-1/photo.jpg");
        assert_eq!(resolve_asset_path(&relative, dir.path()), absolute);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib path_utils`
Expected: FAIL — `cannot find function 'resolve_asset_path' in this scope`.

- [ ] **Step 3: Write the implementation**

Add to `apps/desktop/src-tauri/src/path_utils.rs`:

```rust
/// Resolves a stored `assets.path` to a local filesystem path.
///
/// A relative key (the storage format this application is adopting) is joined
/// under `data_dir` component-by-component, after normalizing `\` to `/` so a
/// value written on Windows resolves the same way everywhere. An absolute path
/// — a row that predates the migration, or an external file that was never
/// copied in — is returned unchanged.
///
/// This helper does not validate. Untrusted input keeps going through
/// `crate::sync::apply::validate_inbound_rel_path`, which refuses traversal,
/// drive letters, and UNC paths before resolving.
pub fn resolve_asset_path(stored: &str, data_dir: &Path) -> PathBuf {
    let candidate = Path::new(stored);
    if candidate.is_absolute() {
        return candidate.to_path_buf();
    }

    let mut resolved = data_dir.to_path_buf();
    for component in stored.replace('\\', "/").split('/') {
        if component.is_empty() {
            continue;
        }
        resolved.push(component);
    }
    resolved
}
```

Note on Windows: `Path::new(r"D:\elsewhere\photo.jpg").is_absolute()` is true there and false elsewhere, which is why the foreign-path test picks its input by platform.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib path_utils`
Expected: PASS, five tests.

- [ ] **Step 5: Run the whole backend suite**

Run: `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml`
Expected: PASS. Nothing consumes `resolve_asset_path` yet, so no existing behavior can have changed. Any failure here is unrelated to this slice — investigate before continuing.

- [ ] **Step 6: Commit**

```bash
git add apps/desktop/src-tauri/src/path_utils.rs
git commit -m "feat(data-dir): resolve_asset_path acepta ruta relativa o absoluta

llm/mod.rs ya resolvía «absoluta o, si no, joineá contra el directorio de
datos» de forma privada. Se convierte en helper con nombre y tests para que
slice 2 lo use en image_edit.rs y en el resto de los consumidores.

No valida: la entrada no confiable sigue pasando por
validate_inbound_rel_path, que rechaza traversal, letra de unidad y UNC."
```

---

## Definition of done for this slice

- [ ] `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` passes.
- [ ] `cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets` reports no new findings.
- [ ] `cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml -- --check` is clean.
- [ ] No file moved on disk, no stored path changed, no caller's behavior changed.
- [ ] `git diff main --stat` touches exactly `lib.rs`, `path_utils.rs`, and `sync/blobs.rs`.

Slice 2 is planned only after this one is green.
