# Plan paso a paso de homogeneización estética — EntropIA

> Plan de ejecución derivado de [`AUDITORIA_ESTILOS.md`](./AUDITORIA_ESTILOS.md) (los códigos `C1`, `R4a`, `N6`… remiten a sus hallazgos) y regido por [`GUIA_DISENO.md`](./GUIA_DISENO.md).
> Rutas: `D/` = `apps/desktop/src/`, `U/` = `packages/ui/src/components/`.

## Cómo está optimizado este plan

La auditoría ordena los problemas por tipo. Este plan los ordena para **trabajar lo menos posible y no rehacer nada**:

1. **Medir antes de tocar.** Un script reproduce las métricas de la auditoría y marca el avance de cada paso con números.
2. **Cerrar la canilla primero.** Las guardas (lint y tests) entran antes de corregir, con una lista de excepciones que solo puede achicarse. La deuda deja de crecer desde el día 1.
3. **Arreglar lo visible sin refactor.** Los colores rotos y los tokens inexistentes son cambios de CSS de bajo riesgo y alto impacto: van antes que cualquier componente nuevo.
4. **Construir antes de migrar.** Cada componente base nuevo existe y tiene tests antes de que una vista lo use. Así nadie migra dos veces.
5. **Tocar cada archivo grande una sola vez.** `SettingsView`, `InternetArchiveDialog`, `DependenciasTab`, `InvestigationView` y `NoteEditor` concentran hallazgos de casi todos los tipos. Se migran **por archivo**, aplicando de una vez componentes, tokens, estados y escalas, en vez de volver a ellos en cada fase.
6. **Lo mecánico, con script.** Los reemplazos literal → token sin ambigüedad (`4px` → `var(--space-1)`, `font-weight: 600` → `var(--font-weight-semibold)`, fallbacks) se hacen con un codemod revisado, no a mano.
7. **PRs chicos y verificables.** Un paso = un PR (o dos), con criterio de hecho medible.

### Verificación estándar de cada paso

Salvo que el paso diga otra cosa, todo PR pasa por:

```bash
pnpm lint
```

```bash
pnpm typecheck
```

```bash
VITE_LOCAL_ML=0 pnpm --filter @entropia-pro/desktop typecheck
```

```bash
pnpm --filter @entropia/ui test
```

```bash
pnpm --filter @entropia-pro/desktop test
```

Además: el script de métricas del paso 1 tiene que bajar (nunca subir) y hay que hacer una **revisión visual en `dark`, `paper` y `vivid` con contraste `high`** de las pantallas tocadas. Si una vista usa un flujo Lite-específico, también hay que correr sus tests con `VITE_LOCAL_ML=0`. Nada de esto requiere builds de Tauri.

---

## Etapa A — Preparación (1–2 días)

### Paso 1 · Script de métricas de estilo

- **Objetivo:** tener una línea de base reproducible y un tablero de avance.
- **Acciones:** crear `scripts/style-metrics.mjs`, que recorra los `<style>` de `D/` y `U/` (sin comentarios ni `__tests__`) y cuente cada métrica del resumen de la auditoría: hex, `rgba`, `color-mix` con `black`/`white`, `var()` a tokens inexistentes, fallbacks, `font-size`/`font-weight`/`line-height`/`letter-spacing` literales, opacidades de `:disabled`, `z-index` y transiciones literales, overrides `:global` sobre componentes del sistema, y controles nativos en el markup (`<input>`, `<textarea>`, `<progress>`, `<details>`, radios). Que admita `--json` y que exponga un script `pnpm style:metrics`.
- **Hecho cuando:** su salida coincide con las cifras de la auditoría (±2 por diferencias de heurística) y su resultado se guarda como `scripts/style-metrics.baseline.json`.

### Paso 2 · Capturas de referencia

- **Objetivo:** poder comparar antes y después de cada migración.
- **Acciones:** elegir unas 12 pantallas representativas: Inicio, Colección, Ítem (paneles de texto y análisis), Escritura, Investigación, Chat, Ajustes (General, Apariencia, Dependencias, Sync), Internet Archive, Importar, Explorador. Capturarlas en `dark`, `paper` y `vivid`. Si las vistas cargan sin el backend de Tauri, con Playwright sobre el servidor de Vite (puerto 1420); si no, a mano desde un perfil de desarrollo (`ENTROPIA_DEV_PROFILE`, nunca sobre el archivo real).
- **Hecho cuando:** existe una carpeta local de capturas nombradas `pantalla__tema.png`.

### Paso 3 · Guardas con lista de excepciones decreciente

- **Objetivo:** que ningún PR nuevo agregue deuda.
- **Acciones:**
  1. En `U/__tests__/visual-contract.test.ts`:
     - validar también los `var()` **con** fallback (detecta T1–T4);
     - prohibir fallbacks sobre tokens publicados;
     - prohibir hex, `rgb[a]()` y `color-mix(…black|white…)` en `<style>`;
     - extender `keeps primary controls off the accent fill` a `D/**` y a cualquier selector con `background: var(--color-accent)` en un elemento interactivo.
  2. Cada test arranca con una **allowlist explícita** de los archivos que hoy fallan, escrita en el propio test, con un comentario que pida no agregar entradas.
  3. Agregar `progress` a `svelte/no-restricted-html-elements` en `eslint.config.js`, con allowlist por archivo.
- **Hecho cuando:** CI está verde con las allowlists y un archivo de prueba con `#fff` hace fallar el test.
- **Regla de aquí en adelante:** cada paso que arregla un archivo lo **quita** de la allowlist en el mismo PR.

---

## Etapa B — Arreglos visibles sin refactor (2–3 días)

Estos pasos tocan solo CSS y tokens. Tienen el mayor impacto visible al menor riesgo.

### Paso 4 · Tokens nuevos mínimos

- **Acciones:** agregar a `packages/ui/src/tokens/tokens.css`, en los 7 temas cuando corresponda:
  - `--color-on-danger` (calcularlo para ≥ 4.5:1 sobre `--color-danger` de cada tema);
  - `--opacity-disabled: 0.48` y `--opacity-disabled-primary: 0.72`;
  - `--space-0-5: 2px` y `--control-height-xs: 24px`;
  - `--line-height-reading: 1.6`;
  - `--letter-spacing-caps: 0.075em`;
  - la escala `--z-*`: `sticky` 20, `dropdown` 200, `menu` 210, `dialog` 1000, `dialog-nested` 1100, `dialog-top` 1200, `tooltip` 1300, `blocking` 2000 (los mismos valores de hoy, así el paso no cambia nada visual).

  Sumar el override de scrollbars de `lite` y `paper` en `apps/desktop/src/app.css` (C14) y sumar los tokens nuevos a `contrast-floor.test.ts` y `theme-completeness.test.ts`.
- **Hecho cuando:** los tests de contraste y completitud de temas pasan con los tokens nuevos. Todavía nadie los usa.

### Paso 5 · Colores de estado en Dependencias y banner de la cáscara

- **Hallazgos:** C1, C2, C3, C4, C5, I1, R4e, R7 (solo color).
- **Archivos:** `D/views/DependenciasTab.svelte`, `D/layout/AppShell.svelte`, `D/lib/deps-status.ts`.
- **Acciones:**
  - ámbar → `--color-warning`/`-soft` + `color-mix(… 30%, transparent)` para bordes;
  - índigo → `--color-accent`/`-soft`;
  - rojo/verde → `--color-danger*`/`--color-success*`;
  - quitar todos los fallbacks;
  - el color del ícono de estado pasa de `style=""` a clases `deps-row__icon--installed|missing|failed`.
- **Hecho cuando:** ambos archivos salen de la allowlist de colores.

### Paso 6 · Mezclas con negro y blanco

- **Hallazgos:** C9, C10.
- **Archivos:** `U/NoteEditor/NoteEditor.svelte` (5 reglas), `D/layout/DocumentExplorer.svelte` (4 reglas).
- **Acciones:**
  - en `NoteEditor`, los fondos más oscuros pasan a `--color-surface-sunken` (bloques `pre`, modal y su input) y el hover de botón a `--color-accent-faint`;
  - en `DocumentExplorer`, el texto del nodo activo pasa a `--color-text-primary` puro y el ícono activo a `--color-accent`.
- **Hecho cuando:** las capturas del paso 2 de Notas y Explorador en `paper` no muestran grises sucios y el texto activo no pierde contraste.

### Paso 7 · Tokens inexistentes y texto sobre rojo

- **Hallazgos:** T1–T4, C6, C7, C12.
- **Archivos:** `D/views/BibliographySearchTab.svelte`, `D/views/ItemSearchPanel.svelte`, `D/views/WritingHistoryPanel.svelte`, `D/layout/NotificationBell.svelte`.
- **Acciones:**
  - `--color-text-danger` → `--color-danger`;
  - `--color-text-tertiary` → `--color-text-muted`;
  - `--color-surface-hover` → `--color-accent-faint`;
  - el contador de notificaciones toma `--color-on-danger` y `--font-size-2xs`.
- **Hecho cuando:** la allowlist de tokens inexistentes queda solo con `InternetArchiveDialog` (se resuelve en el paso 16).

### Paso 8 · Brillos y sombras a mano

- **Hallazgos:** C11, H1, H2.
- **Archivos:** `U/Button/Button.svelte`, `U/Panel/Panel.svelte`, `D/views/ItemAssetPanel.svelte`, `D/views/ItemTextPanel.svelte`, `D/views/ItemAnalysisPanel.svelte`, `D/views/ItemSearchPanel.svelte`, `D/views/CollectionsView.svelte`, `U/NoteEditor/NoteEditor.svelte`, `U/MapViewer/MapViewer.svelte`, `D/layout/NotificationBell.svelte`. `SettingsView` se deja para el paso 15.
- **Acciones:**
  - `inset 0 1px 0 rgba(255,255,255,…)` → `var(--shadow-surface)`;
  - el *inset* negro de `Panel` se vuelve un token `--shadow-inset` (definido como `none` en los temas sin sombra);
  - quitar los fallbacks de `--shadow-*`.

### Paso 9 · Codemod de fallbacks y literales sin ambigüedad

- **Objetivo:** limpiar en un solo PR mecánico lo que no requiere criterio, para que las migraciones posteriores no lo arrastren.
- **Acciones:** escribir un script de un solo uso (que no se commitea) que reemplace en `<style>`:
  - `var(--token, fallback)` → `var(--token)` para todo token publicado (T5, T6);
  - `font-weight: 500|600` → `var(--font-weight-medium|semibold)` (F6);
  - `font-size: 11px` → `var(--font-size-2xs)`; `border-radius: 4px` → `var(--radius-xs)`; `999px`/`9999px` → `var(--radius-full)` (B2);
  - `padding|margin|gap` con `4|8|12|16|20|24|32px` → el `--space-*` exacto (S1);
  - `letter-spacing: 0.075em` → `var(--letter-spacing-caps)`.

  Revisar el diff completo antes de commitear. **No** tocar `2px`, `6px`, `0.9em` ni los `line-height`, que requieren criterio.
- **Hecho cuando:** el script de métricas baja en esas categorías y el diff es puramente de sustitución.

---

## Etapa C — Ampliar el sistema de diseño (1–2 semanas)

Un componente por PR, en `packages/ui`, con `*.types.ts`, tests y export en `src/index.ts`. **El orden importa:** va primero lo que desbloquea más migraciones.

| Paso | Componente | Desbloquea | Base a reutilizar |
| --- | --- | --- | --- |
| 10 | **`Input` ampliado**: acepta todos los `HTMLInputAttributes` (`...rest`), `class`, `bind:this`, `size="sm|md"` y snippets `leading`/`trailing`. La API actual sigue funcionando igual. | ~30 campos crudos (R1), G2 | `U/Input/Input.svelte`, `.search-field__icon` |
| 11 | **`Textarea`**: familia visual de `Input`, `rows`, autoaltura opcional, `label`/`hint`/`error`, `font="ui|reading|mono"`. | 22 textareas (R2) | `Input` |
| 12 | **`RadioGroup` + `RadioCard`**: radio oculto, tarjeta con `:has(:focus-visible)`, variantes `card` y `list`, y una opción con input embebido (“nueva colección”). | N1–N5, R5 (4 diálogos + Ajustes + 2 grillas) | `D/layout/FontPresetGrid.svelte`, `D/views/WritingExportTab.svelte` |
| 13 | **`SelectTrigger`** (y opcionalmente `Select` = trigger + `ToolbarMenu` radio): `size="sm|md"`, variantes `field` y `quiet`, chevron por `ActionIcon`, estado `open`. | 15 disparadores (R3) | `ToolbarMenu`, patrón `WritingZoteroTab` |
| 14 | **`ProgressBar`**, **`Disclosure`**, **`Badge`/`Chip`** (o `StatusBadge` con `size="xs"` y variante `count`), **`Switch`**, **`Dialog`** (la cáscara que luego compone `ConfirmDialog`) y la variante **`Button variant="link"`**. Un PR por componente; el orden interno puede seguir al de las migraciones de la etapa D. | N6–N11, R4f, R7–R10 | `.settings__download-progress-bar`, `AssistantsTab` (switch), `ConfirmDialog` |

En los pasos 10 a 14, adoptar dentro de cada componente nuevo (y de paso en `Button`, `IconButton`, `Input`, `Checkbox`, `NumberField` y `ToolbarMenuList`) `--opacity-disabled`, `--focus-ring` y `--transition-*` con `prefers-reduced-motion` (E1, E8). Así los componentes base quedan alineados entre sí antes de que las vistas los usen.

- **Hecho cuando (por componente):** tiene tests de comportamiento y de contrato visual (sin literales, alturas por token como en `control-block-size.test.ts`), está exportado y se ve bien en los 7 temas.

---

## Etapa D — Migración por archivo (3–5 semanas)

Cada paso toma **un archivo o un grupo pequeño** y lo deja terminado: componentes base, tokens, estados, escalas tipográficas y de espaciado, capas `--z-*` y salida de **todas** las allowlists. El orden va de lo más visible y concentrado a lo más disperso.

**Checklist común a cada archivo:**

- [ ] Controles nativos → componentes (radios, `progress`, `details`, `textarea`, `input`).
- [ ] Botones e inputs ad hoc → `Button`/`IconButton`/`Input`/`SelectTrigger`, salvo las excepciones de la guía (filas, nodos, tarjetas, edición inline).
- [ ] Sin `:global` que cambie la apariencia de un componente del sistema.
- [ ] `font-size` ≥ 11px y por token; `line-height` → `tight|base|reading`.
- [ ] `2px` → `--space-0-5`; resto del espaciado y de las alturas por token (`28px`/`26px`/`22px` → `xs` o `sm`).
- [ ] Radios con la escala semántica (`control`, `input`, `surface`, `dialog`, `full`).
- [ ] Foco con `--focus-ring` (no con `outline: 2px` ni con el mismo fondo del hover); disabled con `--opacity-disabled`.
- [ ] `z-index` global → `--z-*`; transiciones → tokens.
- [ ] El archivo sale de todas las allowlists del paso 3.
- [ ] Comparado contra las capturas del paso 2.

| Paso | Archivo(s) | Hallazgos que cierra |
| --- | --- | --- |
| 15 | `D/views/SettingsView.svelte`, `D/views/SyncSettingsCard.svelte` | G1, G2, N5 (16 radios), R2, R3, R4c, R7, F2, R8; los `:global` de botón e input desaparecen |
| 16 | `D/views/InternetArchiveDialog.svelte` | N4, N7, R1, R4a (primario con acento + `--color-text-on-accent`), R4b, R5, R6, R8, T2 |
| 17 | `D/views/NavegadorCopyDialog.svelte`, `D/views/ImportSourcesDialog.svelte`, `D/views/NavegadorZoteroDialog.svelte` | N1, N2, N3, N6, R1, R5: los tres pasan al mismo `RadioGroup` con opción “nueva colección” |
| 18 | `D/views/DependenciasTab.svelte` (lo que no cerró el paso 5) | R1, R7, R10 (confirmación de reset → `Dialog`), F2, F3 |
| 19 | `D/views/AppearanceTab.svelte`, `D/views/BackupTab.svelte`, `D/views/AssistantsTab.svelte`, `D/components/settings/EntityAliasesTab.svelte`, `D/components/settings/AgentParametersTab.svelte`, `D/components/OpenrouterModelList.svelte` | R3, R4d, R4f, R4i, R9 (`Switch`) |
| 20 | `D/views/InvestigationView.svelte`, `D/views/ResearchView.svelte`, `D/views/RagChatView.svelte`, `D/views/CollectionAnalysisPanel.svelte`, `D/views/GraphSettingsForm.svelte` | R2 (10 textareas de Investigación), N11, N12, R3, F1, F5, F6, E6 |
| 21 | `D/views/WritingView.svelte`, `D/views/WritingCitationEditor.svelte`, `D/views/WritingZoteroTab.svelte`, `D/views/WritingZoteroDetails.svelte`, `D/views/WritingHistoryPanel.svelte`, `D/views/WritingSharePanel.svelte`, `U/WritingEditor/WritingEditor.svelte` (solo inputs del popover de enlace) | R1, R3, R8, F5. **No tocar la serialización**: los tests dorados de `WritingEditor` y `NoteEditor` tienen que seguir sin cambios |
| 22 | `D/views/BatchProcessingTab.svelte`, `D/views/BatchFlowPicker.svelte`, `D/views/BatchSchemaPanel.svelte` | R3 (`menu-select` duplicado), N12, F5 |
| 23 | `D/views/ItemTextPanel.svelte`, `D/views/ItemAssetPanel.svelte`, `D/views/ItemAnalysisPanel.svelte`, `D/views/ItemSearchPanel.svelte`, `D/views/ItemLayoutPanel.svelte`, `D/views/OccurrenceStepper.svelte`, `D/views/ItemView.svelte` | N8, N11, R1, R2, R4d, R4h, R8, R11, scrollbars locales (§2.11), I2 |
| 24 | `D/views/CollectionView.svelte`, `D/views/CollectionsView.svelte`, `D/views/BibliotecaView.svelte`, `D/views/BibliographyWorkView.svelte`, `D/components/BibliographyAnnotationsPanel.svelte`, `D/views/DbBrowserView.svelte` | N9, R2, R3, R8, R10, R11, I2 |
| 25 | `D/views/HomeView.svelte` | F4 (antes, decidir si hacen falta `--font-size-2xl`/`3xl` para la portada y agregarlos en el paso 4 o aquí), E3 |
| 26 | Diálogos sueltos: `D/components/AgentApprovalDialog.svelte`, `D/components/PassageOriginalViewer.svelte`, `D/views/SimilarAssetPreviewDialog.svelte`, `U/NoteEditor/NoteEditor.svelte` (modal) | R10, R4f, R4i |
| 27 | Cáscara: `D/layout/AppShell.svelte`, `D/layout/TopBar.svelte`, `D/layout/NotificationBell.svelte`, `D/layout/DocumentExplorer.svelte`, `D/views/NavegadorView.svelte`, `D/views/NavegadorSources.svelte` | R1, R4e, R4f, R8, B2, E4, S1, scrollbars locales |
| 28 | Paquete UI: `U/AudioPlayer/AudioPlayer.svelte` (botones + slider estilizado, N10), `U/MetadataEditor/MetadataEditor.svelte`, `U/MapViewer/MapViewer.svelte`, `U/EntityViewer/EntityViewer.svelte`, `U/TopicEditor/TopicEditor.svelte` (C13), `U/ItemCard/ItemCard.svelte`, `U/CollectionCard/CollectionCard.svelte` | R4g, R4h, R8, C13, E2 |

Después del paso 15 conviene revisar el script de métricas: si alguna categoría no bajó lo esperado, ajustar el componente base antes de seguir migrando (es más barato corregirlo con una vista migrada que con diez).

---

## Etapa E — Cierre (2–3 días)

### Paso 29 · Escalas residuales

- Barrido final con el script de métricas para lo que quedó fuera de los archivos de la etapa D: `line-height`, `letter-spacing`, radios `sm/md` usados en controles (B1) y `@media` que deberían ser `@container pane` (§2.14).
- Decidir el destino de `--font-weight-bold` (F9): alias explícito de `semibold` documentado, o eliminarlo y reemplazar sus 14 usos.

### Paso 30 · Retirar allowlists y tokens sin uso

- Borrar las allowlists del paso 3. Solo quedan las excepciones permanentes documentadas en la guía (overlays de visor, `PdfMarkup`, `EntropicConstellation`, colores de impresión).
- Activar la prohibición de `textarea` crudo en `eslint.config.js`, con excepción para `U/Textarea`.
- Eliminar o empezar a usar `--radius-xl`, `--shadow-glow`, `--space-7` y `--color-primary-subtle`.

### Paso 31 · Documentación final

- En `GUIA_DISENO.md`, pasar todo lo marcado **(propuesto)** a vigente y quitar las alternativas temporales.
- Re-correr `pnpm style:metrics`, comparar contra la línea de base y anexar la tabla “antes / después” a `AUDITORIA_ESTILOS.md`.
- Repetir las capturas del paso 2 y compararlas.

---

## Calendario orientativo y dependencias

```
A  1 Métricas ─ 2 Capturas ─ 3 Guardas
                               │
B          4 Tokens ─┬─ 5 Deps/banner
                     ├─ 6 Negro/blanco
                     ├─ 7 Tokens rotos
                     ├─ 8 Brillos
                     └─ 9 Codemod
                               │
C   10 Input ─ 11 Textarea ─ 12 Radio ─ 13 SelectTrigger ─ 14 Resto
                               │
D   15 Ajustes ─ 16 IA ─ 17 Diálogos destino ─ 18…28 (en paralelo entre áreas)
                               │
E                   29 Escalas ─ 30 Allowlists ─ 31 Docs
```

| Etapa | Duración estimada (1 persona) | Paralelizable |
| --- | --- | --- |
| A | 1–2 días | No |
| B | 2–3 días | Pasos 5 a 9 entre sí, después del 4 |
| C | 1–2 semanas | Pasos 11 a 14 entre sí, después del 10 |
| D | 3–5 semanas | Pasos 18 a 28 entre sí, después del 15 (que valida los componentes) |
| E | 2–3 días | No |

## Riesgos y cómo mitigarlos

| Riesgo | Mitigación |
| --- | --- |
| Cambiar la API de `Input` rompe sus 50 usos | El paso 10 es aditivo: las props actuales no cambian; los tests existentes de las vistas tienen que seguir verdes sin modificaciones. |
| Tests de vistas que buscan `input[type=radio]` o `progress` | Mantener la semántica nativa: el radio oculto sigue siendo un `input[type=radio]` y `ProgressBar` expone `role="progressbar"` con `aria-valuenow`. Ajustar los selectores de los tests en el mismo PR. |
| Cambios que se cuelan en el contenido guardado del manuscrito o las notas | En los pasos 6, 21 y 26 correr explícitamente los tests dorados de serialización; ningún paso toca extensiones de Tiptap. |
| Diferencias Pro/Lite | Correr los tests de desktop con `VITE_LOCAL_ML=0` en los pasos que tocan Ajustes y Dependencias (15, 18, 19). |
| Regresiones de contraste | Todo token nuevo entra en `contrast-floor.test.ts`; revisar en contraste `high` y `soft`. |
| Deriva durante la migración (PRs ajenos que agregan deuda) | Las allowlists del paso 3 solo se achican; un PR que agrega una entrada se rechaza. |
| Migraciones largas que quedan a medias | Un archivo por PR; el archivo sale de las allowlists al terminar, así el estado parcial queda registrado. |
