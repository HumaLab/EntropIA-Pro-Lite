# Shared Data Directory — Slice 2: Relative Asset Paths

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move `assets.path` from an absolute, variant-scoped path to a relative key anchored at the data directory, without a window in which the application cannot resolve its own assets.

**Architecture:** The slice is split in two halves and the order is the whole point. **2a** makes every reader shape-agnostic — absolute in, absolute out; relative in, joined under the data directory. Today every stored path is absolute, so 2a changes no observable behavior and can ship on its own. **2b** then flips the writers, migrates the 2475 rows, and installs the guard, landing on readers that already accept the new shape. Doing it the other way round would mean flipping the data and then racing to fix readers.

**Tech Stack:** Rust, Tauri v2, rusqlite, Svelte 5, Vitest.

**Spec:** `docs/superpowers/specs/2026-09-08-shared-data-directory-design.md`

**Depends on:** slice 1 (`docs/superpowers/plans/2026-09-08-shared-data-directory-slice-1.md`), which added `path_utils::resolve_asset_path`.

## Global Constraints

- The canonical relative form is `assets/<collection>/<item>/<file>` with forward slashes.
- 2a must not change observable behavior. Every stored path is absolute today, so every reader change must be a no-op against absolute input.
- The storage guard trigger is the **last** step of 2b. Installing it earlier aborts the first import.
- `resolve_asset_path` does not validate. Untrusted input keeps going through `validate_inbound_rel_path`.
- Do not rewrite `items.metadata.__entropia_file_metadata.originalPath` — it points at the external pre-import source, outside the data directory.
- Rust and TypeScript artifacts are written in English, matching the surrounding code.
- A cold `cargo test` here takes ~28 minutes and competes for the `target/` lock. Batch the Rust edits of a task and run once.

---

## Part 2a — Readers stop assuming a shape

### Task 1: `llm/mod.rs` adopts the shared resolver

`resolve_ocr_correction_visual_source_path` (`llm/mod.rs:2139-2171`) already implements absolute-or-join privately. It becomes the first consumer of the shared helper, which also retires the `#[allow(dead_code)]` slice 1 attached to it.

**Files:**
- Modify: `apps/desktop/src-tauri/src/llm/mod.rs:2156-2160`
- Modify: `apps/desktop/src-tauri/src/path_utils.rs` — remove `#[allow(dead_code)]` from `resolve_asset_path`

**Interfaces:**
- Consumes: `path_utils::resolve_asset_path(stored: &str, data_dir: &Path) -> PathBuf`
- Produces: nothing new

- [ ] **Step 1: Replace the private logic**

In `llm/mod.rs`, replace:

```rust
    let path = std::path::PathBuf::from(path);
    let candidate = if path.is_absolute() {
        path
    } else {
        app_data_dir.join(path)
    };
```

with:

```rust
    let candidate = crate::path_utils::resolve_asset_path(&path, app_data_dir);
```

The behavior is identical for both shapes, and the shared version additionally normalizes `\` to `/` in a relative key.

- [ ] **Step 2: Drop the dead-code attribute**

In `path_utils.rs`, delete the `#[allow(dead_code)]` line above `resolve_asset_path` and the paragraph of its doc comment that begins "Not wired up yet". The function now has a real consumer.

- [ ] **Step 3: Run the tests that cover this path**

Run: `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib llm::`
Expected: PASS, including `remote_ocr_asset_request_attaches_its_image_and_visual_instruction` (`llm/mod.rs:3074`) and `remote_ocr_asset_request_retains_complete_single_page_text_with_production_budget` (`:3127`), both of which insert an absolute `assets.path` into a temp database.

- [ ] **Step 4: Commit**

```bash
git add apps/desktop/src-tauri/src/llm/mod.rs apps/desktop/src-tauri/src/path_utils.rs
git commit -m "refactor(data-dir): llm/mod.rs usa el resolvedor compartido

La lógica «absoluta o, si no, joineá» dejó de estar duplicada en privado.
Con un consumidor real, resolve_asset_path pierde el allow(dead_code)."
```

---

### Task 2: `image_edit.rs` resolves before it validates

`validate_source_image_path` (`image_edit.rs:86`) canonicalizes the string it is handed and scope-checks it, then its four callers pass the **original** string on to the file operation. A relative key would fail `validate_existing_file` (it canonicalizes against the process working directory, not the data directory). Resolving first fixes that, and the resolved path must be what flows onward — validating one path and then operating on another is the bug this task exists to avoid.

**Files:**
- Modify: `apps/desktop/src-tauri/src/image_edit.rs:86-91` (the function), `:126`, `:181`, `:227`, `:347` (the callers), `:716-736` (its tests)

**Interfaces:**
- Consumes: `path_utils::resolve_asset_path`
- Produces: `resolve_source_image_path(path: &str, data_dir: &Path) -> Result<PathBuf, String>` — replaces `validate_source_image_path`, returning the canonicalized path instead of `()`

- [ ] **Step 1: Change the function to return the resolved path**

Replace `validate_source_image_path` with:

```rust
/// Resolve an image-edit source path at the IPC boundary and scope-check it.
///
/// The incoming string may be an absolute path (rows written before the
/// relative-path migration) or a key relative to the data directory. Either is
/// resolved first, so the path that is validated is the same one the caller
/// operates on.
fn resolve_source_image_path(path: &str, data_dir: &Path) -> Result<PathBuf, String> {
    if path.trim().is_empty() {
        return Err("Path must not be empty".to_string());
    }
    let resolved = crate::path_utils::resolve_asset_path(path, data_dir);
    let canonical = validate_existing_file(&resolved.to_string_lossy())?;
    ensure_within_dir(&canonical, data_dir)?;
    Ok(canonical)
}
```

The empty-string guard is kept explicitly: `resolve_asset_path("")` would otherwise return the data directory itself, which exists and is a directory, and `validate_existing_file` would reject it with a less precise message.

- [ ] **Step 2: Thread the resolved path through each of the four callers**

At `:126`, `:181`, `:227`, and `:347`, replace

```rust
    validate_source_image_path(&path, &app_data_dir)?;
```

with

```rust
    let path = resolve_source_image_path(&path, &app_data_dir)?
        .to_string_lossy()
        .into_owned();
```

Each caller then passes that `path` into its `spawn_blocking` closure exactly as before. Shadowing keeps the rest of every caller untouched.

- [ ] **Step 3: Update the existing tests to the new name and return type**

In `image_edit.rs`, rename the two tests and assert on the returned path:

```rust
    #[test]
    fn resolve_source_image_path_accepts_files_inside_app_data_dir() {
        let app_data = tempfile::tempdir().expect("tempdir");
        let assets = app_data.path().join("assets");
        std::fs::create_dir_all(&assets).expect("assets dir");
        let file_path = assets.join("photo.jpg");
        std::fs::write(&file_path, b"bytes").expect("write file");

        let resolved =
            resolve_source_image_path(&file_path.to_string_lossy(), app_data.path()).expect("ok");
        assert!(resolved.ends_with("photo.jpg"));
    }

    #[test]
    fn resolve_source_image_path_accepts_a_relative_key() {
        let app_data = tempfile::tempdir().expect("tempdir");
        let assets = app_data.path().join("assets");
        std::fs::create_dir_all(&assets).expect("assets dir");
        std::fs::write(assets.join("photo.jpg"), b"bytes").expect("write file");

        let resolved = resolve_source_image_path("assets/photo.jpg", app_data.path()).expect("ok");
        assert!(resolved.ends_with("photo.jpg"));
    }

    #[test]
    fn resolve_source_image_path_rejects_missing_outside_and_directories() {
        let app_data = tempfile::tempdir().expect("tempdir");
        assert!(resolve_source_image_path("", app_data.path()).is_err());
        assert!(resolve_source_image_path("assets/ghost.jpg", app_data.path()).is_err());
        assert!(resolve_source_image_path(
            &app_data.path().to_string_lossy(),
            app_data.path()
        )
        .is_err());
    }
```

Keep the original body of `resolve_source_image_path_rejects_missing_outside_and_directories` for any case it covered that is not listed here — read `:727-745` before replacing it and carry every assertion over.

- [ ] **Step 4: Run the image-edit tests**

Run: `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib image_edit`
Expected: PASS, including the untouched `delete_asset_file_family_*` tests.

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/src-tauri/src/image_edit.rs
git commit -m "fix(data-dir): image_edit resuelve la ruta antes de validarla

validate_source_image_path canonizaba el string recibido y los cuatro
llamadores seguían operando sobre el original. Ahora la función devuelve la
ruta resuelta y es esa la que fluye: se valida y se opera sobre la misma.

Acepta clave relativa además de absoluta, sin cambiar nada para las
absolutas que hay hoy."
```

---

### Task 3: `prepare_asset_push` resolves before deriving

`prepare_asset_push` (`sync/blobs.rs`) reads `payload["path"]`, calls `derive_rel_path` on it, and then does `Path::new(&abs_path).is_file()`. Against a relative key, `derive_rel_path` returns `OutsideAppData` and the row is silently skipped, while `is_file()` resolves against the process working directory. Both are fixed by resolving first.

**Files:**
- Modify: `apps/desktop/src-tauri/src/sync/blobs.rs` — inside `prepare_asset_push`

**Interfaces:**
- Consumes: `path_utils::resolve_asset_path`
- Produces: nothing new

- [ ] **Step 1: Resolve the stored value first**

Replace

```rust
    let rel_path = match derive_rel_path(&abs_path, app_data_dir) {
        Ok(rel) => rel,
        Err(err) => return Ok(AssetPushOutcome::Skip(err.to_string())),
    };

    let local_path = Path::new(&abs_path);
```

with

```rust
    // The stored value may be an absolute path (rows written before the
    // relative-path migration) or a relative key. Resolve first so both the
    // wire derivation and the file probe below see a real local path.
    let resolved = crate::path_utils::resolve_asset_path(&abs_path, app_data_dir);
    let rel_path = match derive_rel_path(&resolved.to_string_lossy(), app_data_dir) {
        Ok(rel) => rel,
        Err(err) => return Ok(AssetPushOutcome::Skip(err.to_string())),
    };

    let local_path = resolved.as_path();
```

Rename the `abs_path` binding to `stored_path` in the same function so the name stops claiming a shape it no longer guarantees.

- [ ] **Step 2: Add a test for the relative shape**

In `sync/blobs.rs` `mod tests`, beside the existing `derive_rel_path_*` tests:

```rust
    #[test]
    fn prepare_asset_push_derives_the_same_rel_path_from_either_shape() {
        let dir = app_dir();
        let absolute = dir.path().join("assets").join("col-1").join("photo.jpg");
        let from_absolute =
            derive_rel_path(&absolute.to_string_lossy(), dir.path()).expect("absolute");
        let resolved = crate::path_utils::resolve_asset_path("assets/col-1/photo.jpg", dir.path());
        let from_relative =
            derive_rel_path(&resolved.to_string_lossy(), dir.path()).expect("relative");
        assert_eq!(from_absolute, from_relative);
    }
```

- [ ] **Step 3: Run the sync tests**

Run: `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib sync::`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add apps/desktop/src-tauri/src/sync/blobs.rs
git commit -m "fix(data-dir): prepare_asset_push resuelve antes de derivar

Contra una clave relativa, derive_rel_path devolvía OutsideAppData y la fila
se salteaba en silencio, y is_file() resolvía contra el directorio de trabajo
del proceso. Resolver primero arregla las dos cosas y no cambia nada para las
rutas absolutas de hoy."
```

---

### Task 4: the frontend resolves relative keys in one place

`getAssetUrl` (`lib/file-import.ts:252`) is the single seam that turns a stored path into a webview URL, and its seven call sites invoke it **synchronously inside markup** (`ItemSearchPanel.svelte:202`, `SimilarAssetPreviewDialog.svelte:164/184/197`, `CollectionView.svelte:358`, `InvestigationView.svelte:571`, `ItemView.svelte:1051`). It therefore cannot become `async`. The data directory is resolved once at startup and cached.

**Files:**
- Modify: `apps/desktop/src/lib/file-import.ts` — add the cache, change `getAssetUrl`
- Modify: `apps/desktop/src/App.svelte` — prime the cache before the first render that needs it
- Test: `apps/desktop/src/lib/file-import.test.ts`

**Interfaces:**
- Produces:
  - `primeDataDir(): Promise<void>` — resolves `appDataDir()` once and caches it
  - `getAssetUrl(storedPath: string): string` — unchanged signature; joins a relative key against the cached directory before `convertFileSrc`

- [ ] **Step 1: Write the failing tests**

In `apps/desktop/src/lib/file-import.test.ts`:

```typescript
describe('getAssetUrl', () => {
  it('passes an absolute path straight through', async () => {
    await primeDataDir()
    expect(getAssetUrl('C:\\app-data\\assets\\col\\item\\photo.jpg')).toBe(
      'https://asset.localhost/C:\\app-data\\assets\\col\\item\\photo.jpg'
    )
  })

  it('joins a relative key against the data directory', async () => {
    await primeDataDir()
    expect(getAssetUrl('assets/col/item/photo.jpg')).toBe(
      'https://asset.localhost//mock/app-data/assets/col/item/photo.jpg'
    )
  })
})
```

The global mock in `apps/desktop/src/test-setup.ts:7-11` returns `/mock/app-data` for `appDataDir()` and `'https://asset.localhost/' + path` for `convertFileSrc`, so the expected strings follow from it. Adjust them to whatever that mock actually produces rather than changing the mock.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `npm test -- file-import` from `apps/desktop`
Expected: FAIL — `primeDataDir` is not exported.

- [ ] **Step 3: Implement**

In `apps/desktop/src/lib/file-import.ts`:

```typescript
/**
 * The data directory, resolved once. `getAssetUrl` is called synchronously from
 * markup, so it cannot await; priming happens at startup instead.
 */
let cachedDataDir: string | null = null

export async function primeDataDir(): Promise<void> {
  cachedDataDir = await appDataDir()
}

/**
 * Convert a stored asset path to a URL usable in the webview.
 *
 * A stored path may be absolute (rows written before the relative-path
 * migration) or a key relative to the data directory.
 */
export function getAssetUrl(storedPath: string): string {
  const isAbsolute = /^([a-zA-Z]:[\\/]|\\\\|\/)/.test(storedPath)
  if (isAbsolute || cachedDataDir === null) {
    return convertFileSrc(storedPath)
  }
  const separator = cachedDataDir.endsWith('/') || cachedDataDir.endsWith('\\') ? '' : '/'
  return convertFileSrc(`${cachedDataDir}${separator}${storedPath}`)
}
```

Falling back to `convertFileSrc(storedPath)` when the cache is empty preserves today's behavior exactly for any call that beats the priming — which, for absolute paths, is every call.

- [ ] **Step 4: Prime at startup**

In `apps/desktop/src/App.svelte`, call `await primeDataDir()` in the same place the app performs its other startup resolution, before the first view renders.

- [ ] **Step 5: Run the frontend tests**

Run: `npm test` from `apps/desktop`
Expected: PASS. `file-import.test.ts` asserts Windows-style absolute paths at lines 83-89, 109-162, 218-222 and 298-318 — all absolute, so all unchanged.

- [ ] **Step 6: Commit**

```bash
git add apps/desktop/src/lib/file-import.ts apps/desktop/src/lib/file-import.test.ts apps/desktop/src/App.svelte
git commit -m "feat(data-dir): getAssetUrl resuelve claves relativas

getAssetUrl se llama de forma sincrónica desde el markup en siete lugares,
así que no puede volverse async: el directorio de datos se resuelve una vez
al arrancar y queda cacheado.

Una ruta absoluta pasa igual que siempre, y si el caché todavía no está
armado se cae al comportamiento actual."
```

---

### 2a is done when

- [ ] `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` passes.
- [ ] `npm test` passes from `apps/desktop`.
- [ ] `cargo clippy --all-targets` reports no new findings.
- [ ] No stored path has changed. `SELECT COUNT(*) FROM assets WHERE path NOT LIKE '_:\%'` is still 0 on the real database.

**2a is shippable on its own.** Every reader now accepts either shape while every stored path is still absolute.

---

## Part 2b — The shape flips

Planned in detail once 2a is green. Its shape is fixed by the spec:

1. **Writers emit relative keys** — `copyFileToItem` (`file-import.ts:117-130`), `splitPdfIntoPageAssets` (`CollectionView.svelte:670-702`), `duplicateAssetFile` (`file-import.ts:347-359`, which slices the source path's own directory and therefore follows automatically once its input is relative), and the image-edit result paths returned from `image_edit.rs` and written via `store.assets.updatePath` (`ItemView.svelte:1370,1616,1691`).
2. **Migrate the rows** — `migrate_legacy_asset_paths` changes from replacing one absolute prefix with another to stripping it, and normalizes `\` to `/`. Idempotent because a row already relative does not match the prefix. Covered by the four characterization tests slice 1 added.
3. **Remove the filename-label fallback** — `openSourcePath` (`InvestigationView.svelte:378-380`) and `resolverAsset` (`:547-556`). Both sides of that comparison read the same column of the same database for the same item, so the exact match cannot fail for a shape reason; the fallback can only substitute a wrong asset when a filename repeats.
4. **`sync_e2e.rs` asserts `assets.path`** — `canonical_table` (`:359-385`) stops excluding it, because a relative key is device-independent.
5. **The guard trigger, last** — `BEFORE INSERT OR UPDATE ON assets`, `RAISE(ABORT)` when the path looks absolute.

Step 5 lands only after steps 1 and 2, and only after a manual check on the real database confirms every row is relative.
