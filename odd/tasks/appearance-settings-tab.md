# Appearance settings tab

## Objective

Theme, contrast, zoom, typography and language move out of the TopBar into a
new Configuración tab, "Apariencia". The TopBar keeps the search, the section
icons (Colecciones, Chat, Investigación, Escritura, Base de datos, Configuración)
and the window controls.

## Problem

Five of the TopBar's icons are preferences, not places. They crowd the bar and
sit beside navigation as if they were sections. (On 2026-09-18 the typography
picker went into a TopBar popover instead of a tab; the user now wants all
five together in Configuración.)

## Decisions (user, 2026-09-24)

- All five controls move: Tema, Contraste, Zoom, Tipografía, Idioma.
- Tab name "Apariencia", placed last in the Configuración tab list, so the
  default tab does not change.
- Startup must not depend on the TopBar: saved theme, contrast, font preset,
  zoom and locale are applied at app start by a `$lib` module, exactly as today.
- Zoom keyboard shortcuts (Ctrl +, Ctrl −, Ctrl 0) keep working in every view.
- Storage keys stay (`entropia-theme`, `entropia-contrast`, `entropia-font`,
  zoom, locale), so existing choices survive the update.

## Tasks

- [x] T1 Move the five preferences into Configuración → Apariencia and remove them from the TopBar (delegated; interrupted once by an API rate limit and resumed from its uncommitted work)
  - Commits `a42bbfb0` (`lib/theme.ts`, `lib/contrast.ts` apply/restore, `lib/appearance.ts` `initializeAppearance()` called from App.svelte onMount — theme/contrast used to be restored in TopBar's onMount and the font in TypographyMenu's), `a865a490` (`AppearanceTab.svelte`: Tema/Contraste/Idioma as ToolbarMenu radios, zoom stepper with Restablecer and the shortcut hint, `FontPresetGrid.svelte` extracted from TypographyMenu; tab last after Logs), `1abf1fc8` (TopBar keeps search, section icons and window controls; TypographyMenu and TopBar.zoom tests removed, coverage moved to AppearanceTab tests).
  - Zoom and locale already started in App.svelte; Ctrl +/−/0 were already global (`lib/keyboard.ts`). Storage keys unchanged.
  - GREEN desktop 165 files / 2163; typecheck Pro+Lite, lint, format:check clean. Parent spot check: 7 suites, 120 passed.
- [x] T2 Polish (user request 2026-09-24, inline): the "Ctrl + / Ctrl − / Ctrl 0" hint removed (key `topbar.zoomHint` deleted); Tipografía is a ToolbarMenu dropdown like Tema — trigger names the current preset, the menu's `children` holds `FontPresetGrid` (now `bind:current` + `onchoose`, which closes the menu). RED 3; GREEN desktop 165 files / 2165; typecheck Pro+Lite, lint, format:check clean.

## Checks

TDD strict. `pnpm --filter @entropia-pro/desktop test`, typecheck (Pro and
`VITE_LOCAL_ML=0`), lint, `pnpm format:check`; `@entropia/ui` suites if touched.
Visual check by the user. Delivery: commits on `main`; the user decides the push.

## Progress

- T1 done 2026-09-24. Not pushed; waiting for the user's visual check.
