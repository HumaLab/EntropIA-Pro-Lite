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

## Pro vs Lite variant rules

- Pro = Rust feature `local-ml` plus `VITE_LOCAL_ML=1`; Lite = no Cargo features plus `VITE_LOCAL_ML=0` and `src-tauri/tauri.lite.conf.json`.
- Default frontend/Rust configs are Pro-ish for local dev (`VITE_LOCAL_ML` defaults to `1`; Cargo default is lean/Lite), so set both sides explicitly when variant correctness matters.
- Run Tauri commands from `apps/desktop` or use the workspace filter; root `pnpm exec tauri` will not find the desktop-local CLI.
- Use `pnpm exec tauri ...`, not `pnpm tauri ... -- ...`; pnpm can eat the first `--` and break Cargo arg forwarding.
- PowerShell keeps `$env:VITE_LOCAL_ML` for the whole session; reset it when switching variants.
- Pro dev/build uses `--features local-ml` and may compile MNN from source on first Windows build (~30 min). Do not trigger full Tauri builds casually.
- Lite build command shape: `pnpm exec tauri build --config src-tauri/tauri.lite.conf.json --bundles nsis,msi`; do not add `--features local-ml`.

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
