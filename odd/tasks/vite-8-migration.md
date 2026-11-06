# Vite 8 migration

**Status:** pending, not started. Deferred by the owner on 2026-11-06, when Dependabot #239 (vite 6.4.4 → 8.3.5) was
closed.

- [ ] Migrate the frontend build to Vite 8 (Rolldown and Oxc instead of esbuild and Rollup).

## Why it was not a plain bump

#239's `Build` job failed:

```
[plugin vite:esbuild-transpile]
Error: Failed to load `transformWithEsbuild`. It is deprecated and it now requires esbuild to be installed
separately. If you are a package author, please migrate to `transformWithOxc` instead.
```

A transform still goes through esbuild, from our config or from a plugin. Vite 8 no longer ships esbuild.

## What the migration must keep

`apps/desktop/vite.config.ts` is the delicate part (see AGENTS.md, "Frontend/test quirks" and "Dependencies"):

- The pinned dependency prebundling (`optimizeDeps.include`) and `noDiscovery`. Without them the Tauri WebView loads
  stale optimized chunks.
- A single `prosemirror-state`. The editor packages are prebundled from the app's own resolution root.
- The `chrome105` build target, and the pdf.js legacy build (`pdfjs-dist/legacy/build/…`, macOS Safari 16.4+).
- The dev server fixed to port 1420, and the `src-tauri/**` watch ignore.

## Done when

- Lint, the Pro and Lite typechecks, the full desktop and UI suites, and `vite build` pass on Vite 8.
- `pnpm exec tauri dev` runs in a dev profile (`ENTROPIA_DEV_PROFILE=<name>`), and the editor, a PDF and the batch
  tab open.
- The `vite` major ignore is removed from `.github/dependabot.yml`.
