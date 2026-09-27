# Handoff: merge a `main` para el agente del checkout padre

> Ejecutar en `G:\EntropIA-Stack\EntropIA-Pro-Lite`, rama `main`.
> Estado al 2026-09-24: rama `feature/zotero-bibliografia-semantica` (148 commits)
> lista en el worktree `G:/EntropIA-Stack/EntropIA-Pro-Lite-worktrees/zotero-bibliografia-semantica`.
> No borrar el worktree hasta que el merge esté verificado.

Quiero mergear la rama `feature/zotero-bibliografia-semantica` (capa semántica
Zotero: catálogo, sync, perfiles, retrieval híbrido, ingesta, uploads, eval —
148 commits) a `main` con **merge true (`--no-ff`)**. Prohibido squash y rebase:
la documentación del proyecto (`odd/tasks/zotero-semantic-bibliography.md` y el
plan) cita decenas de SHAs de esa rama como evidencia; reescribir historia los
invalida.

## Precondiciones

1. `git status` limpio. Si hay `M apps/desktop/src-tauri/Cargo.lock`:
   inspecciona el diff; si es residuo de build, descártalo
   (`git checkout -- apps/desktop/src-tauri/Cargo.lock`); si es intencional,
   commitealo aparte antes del merge. No lo mezcles con el merge.
2. Backup de los data dirs (`com.entropia.shared`, `com.entropia.lite`) — el
   merge trae migraciones 0038–0054.
3. Lee `AGENTS.md` del checkout padre y
   `odd/tasks/zotero-semantic-bibliography.md` en el worktree (ahí están el
   estado completo, las decisiones y la evidencia).

## Merge

```
git fetch origin   # si hay remoto; si falla, sigue en local
git merge --no-ff feature/zotero-bibliografia-semantica -m "Merge feature/zotero-bibliografia-semantica into main"
```

Main avanzó ~328 commits desde el punto de bifurcación (`9c85295`): espera
conflictos en archivos compartidos.

- Zonas seguras (aditivas, no deberían conflictuar):
  `apps/desktop/src-tauri/src/bibliography/`, migraciones `0038–0054`,
  `tests/fixtures/zsb-eval-v1.json`.
- Zonas calientes (revisar con cuidado): `src/lib.rs` (la rama agrega
  2 re-exports para tests live), `src/lib/i18n.ts` (keys `bibliography.*`),
  `WritingResearchPanel.svelte`, `processing/*`,
  `packages/store/src/{schema,runner}.ts`.

Criterio ante conflicto: preserva ambas funcionalidades; ante duda, pregunta
antes de borrar código de cualquier lado.

## Verificación post-merge (obligatoria, en este orden)

1. `pnpm install --frozen-lockfile` (desde la raíz del checkout padre)
2. `pnpm lint`, `pnpm typecheck`, `pnpm test`
3. `VITE_LOCAL_ML=0 pnpm --filter @entropia-pro/desktop typecheck` (variante Lite)
4. En `apps/desktop/src-tauri`: `cargo test` (lib + integración; tarda varios minutos)

Todo debe quedar verde. Si algo falla por el merge, corrígelo en commit
separado `fix(merge): ...`, nunca reescribiendo historia.

## Prohibido sin autorización explícita del usuario

Tagear versión, pushear, borrar la rama feature o el worktree, tocar
`C:\Users\agusn\.zsb\` o cualquier credencial, y correr builds Tauri completos.

## Reporte al terminar

SHA del merge, lista de archivos con conflicto y cómo se resolvieron,
resultado de cada verificación, y estado final
(`git status` + `git log --oneline -3`).
