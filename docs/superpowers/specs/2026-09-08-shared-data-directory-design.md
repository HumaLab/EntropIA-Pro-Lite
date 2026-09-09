# Shared Data Directory Design

## Decision

Lite and Pro stop keeping separate archives. Both variants — and the dev build — read and write one canonical SQLite database and one canonical asset tree, split by the nature of the data rather than by the variant:

| Role | Path | Contents |
|---|---|---|
| Data | `%APPDATA%\Roaming\com.entropia.shared` | `entropia.sqlite`, `assets/`, `research/` |
| Cache | `%LOCALAPPDATA%\com.entropia.shared` | `models/`, `hf_cache/`, `paddlex_cache/`, `runtime-dev/`, `thumbnails/`, `audio-previews/`, `temp/`, `logs/` |

`assets.path` changes from an absolute, variant-scoped path to a relative key anchored at the data directory, reusing the representation the sync protocol already defines for the wire.

Neither variant's `identifier` changes. Lite and Pro must remain separately installable side by side, and the identifier is what separates their installers, their uninstall entries, and their install directories. That constraint is what rules out electing either variant's `app_data_dir()` as canonical.

## Review path

Reviewers should evaluate this design in this order:

1. Confirm the canonical directories are correct and that the security scope is narrowed, not widened.
2. Confirm the relative-path model reuses the existing, tested sync helpers instead of introducing a parallel implementation.
3. Challenge the migration's crash-safety and idempotency claims against the measured directory inventory.
4. Confirm every absolute-path store was found, and that out-of-scope absolute paths are explicitly excluded.
5. Confirm the delivery slices are independently revertible and that the slice touching real user data comes last.

## Current evidence

All figures below were measured on the development workstation before this design was written.

### Directory inventory

| Directory | Size | Database | Asset tree |
|---|---|---|---|
| `com.entropia.lite` | 14 G | 17 col / 2398 items / 2475 assets | 5069 files, 2475 rows resolve, 0 missing |
| `com.entropia.pro.desktop` | 11 G | 17 col / 2398 items / 2475 assets | 2653 files, 2475 rows resolve, 0 missing |
| `com.entropia.pro.desktop.dev` | 52 M | 17 col / 2398 items / 2475 assets | no `assets/` directory; rows point at Lite's tree |
| `com.entropia.desktop` | 4.9 G | 0 rows | dead weight |
| `com.entropia.lite.dev` | 91 M | 7 col / 24 items / 24 assets | development scratch |
| `com.entropia.lite.dev2` | 141 M | 7 col / 24 items / 24 assets | development scratch |
| `app.entropia.lite` | 64 K | none | empty |
| `com.entropia.app` | absent | — | the current `LEGACY_APP_IDENTIFIER` |

### Per-subdirectory sizes

| Subdirectory | Lite | Pro | Nature |
|---|---|---|---|
| `assets` | 3.4 G | 2.1 G | irreplaceable user data |
| `thumbnails` | 1.1 G | 260 M | derived, regenerable |
| `audio-previews` | 577 M | 126 M | derived, regenerable |
| `models` | 5.1 G | 5.1 G | downloadable, duplicated byte-for-byte |
| `paddlex_cache` | 2.0 G | 2.0 G | downloadable, duplicated byte-for-byte |
| `runtime-dev` | 1.3 G | 1.3 G | downloadable, duplicated byte-for-byte |
| `hf_cache` | 142 M | 142 M | downloadable, duplicated byte-for-byte |

Roughly 8.5 G of downloadable runtime is duplicated per variant. Together with the 4.9 G dead directory, about 13 G is recoverable — out of roughly 30 G that these eight directories currently occupy to hold a single working archive.

### The two databases are already converged

| Key | `com.entropia.lite` | `com.entropia.pro.desktop` |
|---|---|---|
| `account_id` | `5b599252-…` | `5b599252-…` (same) |
| `server_epoch` | `1cb97727-…` | `1cb97727-…` (same) |
| `last_pull_seq` | 24210 | 24210 (same) |
| `sync_row_versions` | 12982 | 12982 (same) |
| `capture_enabled` | 1 | 1 |
| `device_id` | `5b7f7951-…` | `6a7cded4-…` (different) |
| `last_sync_at` | 1788908502799 | 1788757813595 |

The identical row counts are not a coincidence and not a merge problem: these are two devices of the same sync account that converged through the server. There is no divergent content to reconcile. One machine is currently consuming two device slots.

### Code evidence

| Area | Verified evidence | Design consequence |
|---|---|---|
| Backend re-resolution | 21 call sites of `app_data_dir()`; everything else receives it as a threaded `&Path` parameter | Only the re-resolution sites change; the threaded chain is already correct |
| Non-Tauri derivation | `nlp/embeddings.rs:1458`, `python_discovery.rs:241`, `nlp/commands.rs:58` infer the directory from the SQLite file's parent | These break under the data/cache split and are invisible to a lint on `app_data_dir` |
| Frontend resolution | `file-import.ts:122`, `transcription.ts:58`, `CollectionView.svelte:677` | Only three sites resolve the directory |
| Frontend asset URLs | `getAssetUrl` (`file-import.ts:252-254`) is the single seam feeding `convertFileSrc`, with 7 call sites passing `assets.path` verbatim | Joining inside `getAssetUrl` fixes every consumer at once |
| Path comparison | `openSourcePath` (`InvestigationView.svelte:362-398`) and `resolverAsset` (`:547-556`) compare `normalizePath(assets.path)` against a path produced by `entropia_agent`, whose `mostrar_fuente` (`puerta_lectura.rs`) runs `SELECT path, page_number FROM assets WHERE item_id = ?1` with no join, normalization, or base-directory resolution | The engine is a pass-through of the same column; both sides read the same rows, so the shape is ours to fix and needs no canonicalization layer |
| Stored path shape | 2475 of 2475 rows absolute, 0 relative, Windows backslash separators, in both `com.entropia.lite` and `com.entropia.pro.desktop` | No mixed shapes to handle; the migration also normalizes separators |
| `page_number` | Populated on 20 of 2475 rows | Untouched by this change; the UI already renders the page only when present |
| Relative tolerance | `llm/mod.rs:2139-2171` already accepts absolute or relative `assets.path` | The precedent pattern to copy |
| Missing tolerance | `image_edit.rs:86-90` `validate_source_image_path` assumes absolute | Needs the same fallback |
| Second path store | `app_settings` keys `deps_venv_python_path`, `python.paddle_vl.path`, `python.faster_whisper.path`, `python.spacy_ner_es.path` (`deps/install.rs:845-860`, read at `deps/checks.rs:330-333`) hold variant-scoped absolute paths | Must be migrated |
| Existing wire format | `derive_rel_path` (`blobs.rs:72-107`), `validate_inbound_rel_path` (`apply.rs:386-426`), `blob_local_path` (`blobs.rs:587-593`), `sync_pending_blobs.rel_path` (`schema.rs:53-61`) | The target representation already exists and is tested |
| Existing migration shim | `migrate_legacy_app_dir` (`lib.rs:686`), `migrate_legacy_asset_paths` (`lib.rs:978`), keyed on one `LEGACY_APP_IDENTIFIER` (`lib.rs:45`) | Generalize rather than replace |
| Test coverage gap | Neither migration function has a single test | Cover current behavior before generalizing |
| Sync convergence assertion | `sync_e2e.rs:359-385` `canonical_table` excludes `assets.path`, commented "volatile (absolute, device-local)" | Relative paths make it device-independent and assertable |

`items.metadata` carries `__entropia_file_metadata.originalPath`, an absolute path to the external pre-import source file (for example `F:\OneDrive\...`). It is provenance, it lives outside the app data directory, and it must not be rewritten.

## Goals

- One database and one asset tree shared by Lite, Pro, and dev, regardless of which variant opens the app.
- `assets.path` stored as a relative key, making the archive portable between variants and between machines.
- No image fails because of asset-protocol scope in any of the three configurations.
- A user with data in `com.entropia.lite` loses nothing, and the migration is idempotent: running it twice changes nothing.
- Security scope narrowed relative to today, never widened, in the builds that ship.
- Around 13 G of duplicated runtime and dead directories made recoverable.

## Non-goals

- Changing any variant's `identifier`.
- Deleting anything. Legacy directories are reported, never removed.
- Copying asset trees. Movement is `rename` within the same volume, with a copy fallback only for a cross-volume failure.
- Cleaning the ~2400 unreferenced orphan files in Lite's asset tree. Measured and recorded; a separate change with its own verification.
- Revoking the stale sync device registration automatically. It is an action against a remote server; it is detected and surfaced.
- Rewriting `items.metadata.originalPath` or any other path outside the app data directory.
- Changing `EntropIA-Agent`. The engine passes the column through and keeps doing so.
- Merging the two databases. They are already converged; there is nothing to merge.

## Design

### Canonical directories

Two helpers replace every re-resolution:

- `entropia_data_dir()` resolves `path().data_dir()` joined with `com.entropia.shared`.
- `entropia_cache_dir()` resolves `path().data_local_dir()` joined with `com.entropia.shared`.

Neither calls `app_data_dir()`. The split follows the nature of the data: `Roaming` is designed for content that follows a user between machines in a domain, and 8.5 G of downloadable runtime does not belong there. The practical consequence is that a backup of `Roaming` drops from 14 G to roughly 3.5 G of genuinely irreplaceable content.

The split is done now rather than later because this change touches all 21 re-resolution sites exactly once. Deferring it means touching them all again, and choosing which helper each site calls is a decision this refactor has to make anyway.

### Guarding the wrong path

A `clippy.toml` `disallowed-methods` entry on `app_data_dir` makes the mistake fail to compile, with the two helpers as the only documented exception. Without it, the design degrades into "each variant points at Lite's directory" within months, silently.

The lint does not cover the three sites that derive the directory from `db_path.parent()`. Under a single directory those would survive by accident; under the data/cache split they resolve the cache root from the data root and break. They are converted by hand and called out in review as the most fragile part of the refactor.

### Scope

| File | Change |
|---|---|
| `tauri.conf.json:57` | `assetProtocol.scope` → `["$DATA/com.entropia.shared/**/*", "$LOCALDATA/com.entropia.shared/**/*"]` |
| `tauri.lite.conf.json` | Same `assetProtocol` block added |
| `tauri.dev.conf.json` | Same block; the two-identifier workaround is removed |
| `capabilities/default.json` | `fs:scope` uses the same two paths; `fs:allow-appdata-read-recursive` and `fs:allow-appdata-write-recursive` are removed |

`$APPDATA` is `Roaming\<identifier>`; `$DATA` is `Roaming`. A fixed sibling directory scoped as `$DATA/com.entropia.shared/**/*` is exactly as narrow as today's `$APPDATA/**/*`. Removing the recursive app-data grants narrows the surface further, because they point at a per-variant directory that no longer holds anything.

The migration runs in Rust, which the `fs` plugin scope does not govern — that scope constrains the frontend. Reading the eight legacy directories therefore requires no additional permission.

### Path model

`assets.path` stores `assets/<collection>/<item>/<uuid>_<name>`: a relative key with forward slashes, anchored at the data directory. This is exactly what `derive_rel_path` already emits on every push.

All 2475 stored rows are currently absolute with Windows backslashes, so the migration normalizes the separator as well as stripping the prefix. There are no mixed shapes to reconcile.

`derive_rel_path`, `validate_inbound_rel_path`, and `blob_local_path` move from `sync::blobs` into a shared path module. Sync continues to use them unchanged; the rest of the backend adopts them. Their existing validation already rejects absolute paths, drive letters, UNC paths, and `..` traversal — that check belongs at the storage boundary, not only at the sync boundary.

A side effect worth naming: `derive_rel_path` currently does conversion work on every push. When the storage format *is* the wire format, that work becomes identity.

`image_edit.rs:86-90` gains the absolute-or-join fallback that `llm/mod.rs:2139-2171` already implements. `getAssetUrl` joins the relative key against the data directory before calling `convertFileSrc`, which covers all seven frontend consumers.

### Comparing paths against the research engine

`entropia_agent` does not canonicalize anything. `mostrar_fuente` (`puerta_lectura.rs`) selects `path` straight from `assets` and the `source` operation wraps it in `{path, page}` after checking that the item belongs to the investigation. The path's shape is not a property of the engine; it is a property of the column this application writes. Storing relative paths makes the engine return relative paths the same day, with no change to `EntropIA-Agent`.

That makes the two sides of the comparison the same data. `researchSource` (`InvestigationView.svelte:341`) asks the engine, which reads `assets.path` for an item; `openSourcePath` then reads `store.assets.findByItem(item.id)` — the same column, the same database, the same rows. The match is exact by construction, before and after this change, so no canonicalization layer is needed and the engine needs no modification.

The filename-label fallback is therefore removed. It cannot fire for a shape mismatch, because a shape mismatch between a column and itself is not reachable; what it can do is silently substitute a wrong asset when a filename repeats across pages or versions. A fallback that covers no evidenced case while hiding a real inconsistency is worse than its absence. Removing it means a future inconsistency surfaces as a visible failure instead of a wrong document.

This is deliberately separate from the fix in `dbd506e`, which resolves a citation's item *by title* when an older report carries no `item_id`. That mechanism is unrelated to path shape and stays.

The alternative — having `mostrar_fuente` take the data directory and always return absolute paths — was offered and is declined. Resolution belongs at one explicit point in this application, in `getAssetUrl`. A second resolver in the engine would be a third notion of canonical form.

### Storage guard

A `BEFORE INSERT OR UPDATE` trigger on `assets` raises `ABORT` when `path` looks absolute.

A `CHECK` constraint is rejected deliberately: adding one to an existing SQLite table requires rebuilding it, and `assets` carries three indexes, a partial unique index, a self-referential foreign key with `ON DELETE CASCADE`, and the sync capture triggers. Rebuilding it to gain one constraint is not worth the risk to the only real database.

### Migration

Lite is elected as the seed. Both databases converged to the same server sequence, so the election preserves everything either way; Lite's `last_sync_at` is the more recent of the two.

`migrate_legacy_app_dir` is generalized from a single `LEGACY_APP_IDENTIFIER` to an ordered list: `com.entropia.lite`, `com.entropia.pro.desktop`, `com.entropia.pro.desktop.dev`, `com.entropia.lite.dev`, `com.entropia.lite.dev2`, `com.entropia.desktop`, `app.entropia.lite`, `com.entropia.app`. The first entry holding a database wins as the seed; from the rest, only what is missing is taken.

`copy_missing_recursive` currently copies. It becomes `fs::rename` per top-level subdirectory, falling back to a copy only when the rename fails because of a volume boundary. `Roaming` and `Local` are both under `C:\Users\<user>\AppData\`, so the 14 G moves in milliseconds and the asset trees are never copied.

The migration begins with `backup_sqlite_bundle` (`lib.rs:804`), which already produced the `.bak-presync` and `.bak-preclean` files present on disk.

The four `app_settings` python paths are cleared rather than rewritten, letting the normal discovery path repopulate them. A stale path to an interpreter that no longer exists is worse than no path at all.

Idempotency comes from three independent properties, not one: the existing `.legacy-app-dir-merged` marker, a `rename` whose source is already gone being a skip, and a prefix-stripping `UPDATE` that does not match rows already relative. An interrupted run is completed by the next launch.

Nothing is deleted. The legacy directories are left in place and reported, including the 4.9 G dead one. Deletion is an explicit user action taken after verifying the migration.

### Sync device identity

One database means one `device_id` — Lite's. Pro's registration becomes an orphan occupying a device slot on the account. It is detected and surfaced so the user can revoke it from Settings. It is not revoked automatically, because that is an outward-facing action against a remote server.

## Testing

Tests come first. `migrate_legacy_app_dir` and `migrate_legacy_asset_paths` — the two functions this change generalizes — currently have no tests at all, so their present behavior is covered before either is touched.

| Group | Coverage |
|---|---|
| Path helpers | Relative/absolute round trip; rejection of absolute, UNC, drive-letter, and `..` traversal. Extends the existing `blobs.rs` and `apply/tests.rs` suites rather than duplicating them |
| Migration | Seed election; move rather than copy; running twice changes nothing; interrupting between the move and the rewrite leaves the next launch able to finish |
| Storage guard | Inserting an absolute path into `assets` aborts |
| Configuration | The three configs' `assetProtocol` scope is exactly the two shared paths, so a future widening fails the build rather than an audit |
| Sync convergence | `sync_e2e.rs` `canonical_table` stops excluding `assets.path` and starts asserting it, because a relative key is device-independent |
| Source resolution | A citation whose asset filename repeats across pages or versions resolves to the correct asset, and an unresolvable one fails visibly rather than substituting a neighbour |

Frontend tests that assert Windows-style absolute paths (`file-import.test.ts`, the `CollectionView` suites, and the global `appDataDir` mock in `test-setup.ts:7-11`) are updated to the relative model.

### Verification that requires the user

The central acceptance criterion — install Lite, import a document, open Pro, see the document and its image, then the reverse — runs in a native Tauri window and cannot be verified from the agent side. It is handed to the user with exact steps, and no part of this change is reported as complete before that result comes back.

## Delivery

Four slices. Each compiles, passes its tests, and is revertible on its own.

1. Path helpers, the storage guard, and tests covering the current behavior of both migration functions. Nothing changes location; nothing yet consumes the helpers.
2. Relative paths: row migration including separator normalization, the `getAssetUrl` seam, the `image_edit.rs` fallback, and removal of the filename-label fallback in `InvestigationView`.
3. The two canonical directories: helpers, the 21 re-resolution sites, the three `db_path.parent()` derivations, the scopes, and the configs.
4. Convergence migration across the eight legacy directories.

Slice 2 lands before the directories move, and that is safe rather than accidental: a relative key resolved against whatever data directory the running variant currently uses reproduces exactly the absolute path it replaced. Slice 2 strips only the running variant's own prefix; the legacy prefixes belong to slice 4.

Slice 4 is the only one that touches real user data, and it lands only once the first three are green.
