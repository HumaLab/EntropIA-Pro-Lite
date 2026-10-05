# AGENTS.md

## Repo shape

- pnpm/Turbo monorepo: `apps/*` and `packages/*`; use pnpm 9.x with Node 22+.
- Main app is `apps/desktop`: Svelte 5 + Vite frontend, Tauri 2/Rust backend in `apps/desktop/src-tauri`.
- Shared packages: `packages/ui` exports Svelte UI/tokens, `packages/store` owns Drizzle/SQLite store logic, `packages/config-ts` exports shared TS configs.

## Commands that are easy to get wrong

- Install exactly from the lockfile: `pnpm install --frozen-lockfile`.
- Workspace checks from repo root: `pnpm lint`, `pnpm typecheck`, `pnpm test`.
- Focus a package: `pnpm --filter @entropia-pro/desktop test`, `pnpm --filter @entropia/ui typecheck`, `pnpm --filter @entropia/store test`.
- Focus one Vitest file by passing args through the package script, e.g. `pnpm --filter @entropia-pro/desktop test -- src/lib/ocr.test.ts`.
- Frontend Lite typecheck needs the variant env: `VITE_LOCAL_ML=0 pnpm --filter @entropia-pro/desktop typecheck`.
- Rust quality helper exists at root: `pnpm rust:quality:report`.

## Dev profile (never run `tauri dev` on the real archive with a new migration)

- `tauri dev` opens the same archive as the installed app (`%APPDATA%\com.entropia.shared`) and finds its sync session in the OS keyring. A new migration applied there is irreversible, and one sync raises the account's `schema_tag` for every device.
- Debug builds only: set `ENTROPIA_DEV_PROFILE=<name>` (`A-Z a-z 0-9 - _`, max 32) before `pnpm exec tauri dev`. Data goes to `<data>/com.entropia.shared/dev-profiles/<name>`, cache likewise under `%LOCALAPPDATA%`; sync is off (no engine, no keyring, sync commands answer `sync_disabled_in_dev_profile`). Release builds never read the variable. See `src-tauri/src/dev_profile.rs`.
- Check the startup line first: `[setup] profile=dev:<name> data_dir=... sync=disabled`. If it says `profile=shared`, close the app.

## Pro vs Lite variant rules

- Pro = Rust feature `local-ml` plus `VITE_LOCAL_ML=1`; Lite = no Cargo features plus `VITE_LOCAL_ML=0` and `src-tauri/tauri.lite.conf.json`.
- Default frontend/Rust configs are Pro-ish for local dev (`VITE_LOCAL_ML` defaults to `1`; Cargo default is lean/Lite), so set both sides explicitly when variant correctness matters.
- Run Tauri commands from `apps/desktop` or use the workspace filter; root `pnpm exec tauri` will not find the desktop-local CLI.
- Use `pnpm exec tauri ...`, not `pnpm tauri ... -- ...`; pnpm can eat the first `--` and break Cargo arg forwarding.
- PowerShell keeps `$env:VITE_LOCAL_ML` for the whole session; reset it when switching variants.
- Pro dev/build uses `--features local-ml` and may compile MNN from source on first Windows build (~30 min). Do not trigger full Tauri builds casually.
- Lite Windows build command shape: `pnpm exec tauri build --config src-tauri/tauri.lite.conf.json --config src-tauri/tauri.lite.windows.conf.json --bundles nsis,msi`; do not add `--features local-ml`. The second overlay replaces the shared Windows resource list so Pro-only payload (uv, models, runtime pack, scripts) stays out.

## Tauri/Rust gotchas

- `apps/desktop/src-tauri/src/lib.rs` swaps the whole `deps` module by feature: `deps/mod.rs` for `local-ml`, `deps/mod_lite.rs` otherwise. Keep the command/struct surface aligned across variants.
- Release builds can fail closed in `build.rs` if a fixture runtime-pack is bundled without `ENTROPIA_RUNTIME_BOOTSTRAP_MANIFEST_URL` and `ENTROPIA_RUNTIME_BOOTSTRAP_PUBLIC_KEY_BASE64`.
- Windows release builds stage VC runtime DLLs from `ENTROPIA_VC_RUNTIME_DIR` or `%WINDIR%\System32`; missing required DLLs panic in `build.rs`.

## Frontend/test quirks

- Root Vitest is a multi-project config for `packages/store`, `packages/ui`, and `apps/desktop`; package configs define environments (`node` for store, `happy-dom` for UI/desktop).
- Desktop tests mirror `VITE_LOCAL_ML`; default tests exercise Pro UI. Set `VITE_LOCAL_ML=0` to cover Lite-specific UI paths.
- Vite dev server is fixed to port `1420` for Tauri and ignores `src-tauri/**` watches.
- Desktop Vite pins dependency prebundling and `noDiscovery` to avoid stale optimized chunks in Tauri WebView; be careful when adding new bare runtime imports from linked workspace packages.

## Style constraints already encoded in config

- ESLint warnings allow `_`-prefixed unused args/vars, rest-sibling stripping, and `any`; do not “fix” those patterns blindly.
- Svelte runes dependency expressions intentionally disable `@typescript-eslint/no-unused-expressions` in `.svelte` files.
- Empty catches are allowed only for best-effort localStorage-style access in `.svelte` files.
- Icons go through `ActionIcon`, never around it. `no-restricted-imports` fails the build on a direct `@tabler/icons-svelte-runes` import anywhere under `apps/**` or `packages/**`; to add an icon, add its name to `ACTION_ICON_NAMES` and map it in `ActionIcon.svelte`. Genuinely non-icon SVG — charts, viewer overlays, third-party brand marks — is allowlisted in `eslint.config.js` rather than exempted by hand.

## Task board (hlab.com.ar)

- EntropIA tasks live at https://hlab.com.ar/admin/tablero-entropia (admins only). Each card has a code: `T-1`, `T-2`… (tasks) and `P-1`… (publications).
- When you **start** a task that has a card, move it to `En curso`. When it is **done** (committed, tests green), move it to `Hecho`. Find the code with the list call; never guess it. Create a card only when the user asks.
- The key is in the user environment variable `HLAB_TABLERO_CLAVE`; never print it or write it into the repo. Every call sends `-H "X-Tablero-Clave: $HLAB_TABLERO_CLAVE" -H "Accept: application/json"`.

```bash
B=https://hlab.com.ar/api/tablero/tarjetas
H=(-H "X-Tablero-Clave: $HLAB_TABLERO_CLAVE" -H "Accept: application/json" -H "Content-Type: application/json")
curl -s "${H[@]}" "$B?tablero=tareas&columna=En%20curso"           # list (filters: tablero, columna, responsable)
curl -s "${H[@]}" -X PATCH "$B/T-9" -d '{"columna": "Hecho"}'      # move or edit: send only the fields to change
curl -s "${H[@]}" -X POST "$B" -d '{"titulo": "…", "area": "…", "origen": "…"}'   # create one
curl -s "${H[@]}" -X POST "$B" -d '{"tarjetas": [{"titulo": "…"}, {"titulo": "…"}]}'  # create several: all or none
```

- Fields: `tablero` (`tareas` default, or `publicaciones`), `titulo`, `detalle`, `area`, `origen`, `responsable` (`Sin asignar`, `Rodrigo`, `Agustín`, `Ambos`), `columna`; publications also `tipo` (`Post`, `Artículo`, `Video`, `Reseña`, `Traducción`) and `estado` (`Idea`, `Borrador`, `Publicado`).
- Columns: tasks `Por hacer` (default), `En curso`, `Hecho`; publications `Ideas sin fecha` (default) or a month written like `Noviembre 2026`. Cards cannot be deleted through the API, only in the panel.
- Responses: 200/201 return the card(s) with their code; 401 = missing or wrong key (tell the user); 404 = unknown code; 422 = invalid field (the response lists the valid values).
