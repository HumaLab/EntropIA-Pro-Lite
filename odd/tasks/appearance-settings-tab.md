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

- [ ] T1 Move the five preferences into Configuración → Apariencia and remove them from the TopBar (delegated)

## Checks

TDD strict. `pnpm --filter @entropia-pro/desktop test`, typecheck (Pro and
`VITE_LOCAL_ML=0`), lint, `pnpm format:check`; `@entropia/ui` suites if touched.
Visual check by the user. Delivery: commits on `main`; the user decides the push.

## Progress

- Created 2026-09-24. T1 delegated.
