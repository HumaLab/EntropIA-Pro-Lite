# Vite 8 migration

**Status:** in progress (2026-11-12), branch `chore/vite-8`. Deferred by the owner on 2026-11-06, when Dependabot #239 (vite 6.4.4 → 8.3.5) was
closed; started on the owner's request on 2026-11-12.

- [x] Migrate the frontend build to Vite 8 (Rolldown and Oxc instead of esbuild and Rollup). PR #285.

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

## Log

- L1 (2026-11-12, owner, verbatim): «arranquemos con estas: - el PR #247 (ESLint 10); - la migración a Vite 8;» and, after the plan below, «avanzá».
- L2 (2026-11-12): recon. The `vite:esbuild-transpile` failure came from our own `build.minify: 'esbuild'` in `apps/desktop/vite.config.ts`, not a plugin. Only `vite` must move: `@sveltejs/vite-plugin-svelte@7.3.1` already requires vite ^8, `vitest@4.1.11` accepts ^6–^8. Plan: vite ^8 in apps/desktop and packages/ui; `minify: 'oxc'`; `rollupOptions` → `rolldownOptions` in both vite configs; drop the dead `hot` option from both vitest configs (removed in plugin 7); remove the vite major ignore from `.github/dependabot.yml`. optimizeDeps, target chrome105, port 1420 and the src-tauri watch ignore unchanged.
- L3 (2026-11-12): applied; `pnpm install` resolved vite 8.3.8, esbuild left the lockfile.
- L4 (2026-11-12): local checks on vite 8.3.8: format, lint, Pro and Lite typecheck clean; desktop Pro 4212 passed (a first full run had 3 load-related failures in binary-provenance.test.ts, which spawns git/node; 17/17 alone and green on the rerun), desktop Lite 4176, ui 961, store 500; `vite build` for ui and desktop pass and still emit `pdf.worker.min-*.mjs`, `index.html` and `splash.html`. Vite 8 warned that `__dirname` is unsupported by the future native config loader: replaced with `import.meta.dirname` in both vite configs and the desktop vitest config.
- L5 (2026-11-12, owner, verbatim): «todo anda» — checked in `tauri dev` (Lite, dev profile `ocrnativo`, Vite 8.3.8): editor with a footnote, item notes, a PDF, the batch tab and moving between views. The app log shows no optimizer, stale-chunk, prosemirror or pdf.worker errors; the only errors are dictation without an AssemblyAI key (the profile has none). CI on c3a21493: 5/5 applicable checks passed.
