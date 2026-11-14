# Plan paso a paso de homogeneización estética — EntropIA

> Plan de ejecución derivado de [`AUDITORIA_ESTILOS.md`](./AUDITORIA_ESTILOS.md) (los códigos `C1`, `R4a`, `N6`… remiten a sus hallazgos) y regido por [`GUIA_DISENO.md`](./GUIA_DISENO.md).
> Rutas: `D/` = `apps/desktop/src/`, `U/` = `packages/ui/src/components/`.

## Cómo está optimizado este plan

La auditoría ordena los problemas por tipo. Este plan los ordena para **trabajar lo menos posible y no rehacer nada**:

1. **Medir antes de tocar.** Un script reproduce las métricas de la auditoría y marca el avance de cada paso con números.
2. **Cerrar la canilla primero.** Las guardas (lint y tests) entran antes de corregir, con una lista de excepciones que solo puede achicarse. La deuda deja de crecer desde el día 1.
3. **Arreglar lo visible sin refactor.** Los colores rotos y los tokens inexistentes son cambios de CSS de bajo riesgo y alto impacto: van antes que cualquier componente nuevo.
4. **Construir antes de migrar.** Cada componente base nuevo existe y tiene tests antes de que una vista lo use. Así nadie migra dos veces.
5. **Migrar cada archivo grande una sola vez.** `SettingsView`, `InternetArchiveDialog`, `DependenciasTab`, `InvestigationView` y `NoteEditor` concentran hallazgos de casi todos los tipos. Se migran **por archivo** en la etapa D, aplicando de una vez componentes, tokens, estados y escalas. Las etapas B y E pueden pasar antes o después por esos archivos, pero con dos condiciones: solo tocan declaraciones que la etapa D no reescribe, y cada uno de esos pasos queda listado en la tabla [Archivos con más de un paso](#archivos-con-más-de-un-paso). Un arreglo que la etapa D va a tirar (por ejemplo, colorear un `<progress>` que luego pasa a `ProgressBar`) se hace directamente en la etapa D.
6. **Lo mecánico, con script.** Los reemplazos literal → token sin ambigüedad (`4px` → `var(--space-1)`, `font-weight: 600` → `var(--font-weight-semibold)`, fallbacks) se hacen con un codemod revisado, no a mano, y **antes** que cualquier arreglo a mano.
7. **PRs chicos y verificables.** Un paso = un PR, o varios de unas 400 líneas cuando el archivo es grande (pasos 14, 15, 20 y 23), con un criterio de hecho medible.

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
pnpm format:check
```

```bash
pnpm --filter @entropia/ui test
```

```bash
pnpm --filter @entropia-pro/desktop test
```

`format:check` va siempre: el gate de prettier bloquea CI y ni lint, ni typecheck, ni los tests lo detectan. Antes de commitear, correr `pnpm format` sobre lo tocado.

**Además, según lo que toque el paso:**

- **Variante Lite.** Los pasos que tocan archivos con ramas `VITE_LOCAL_ML` o componentes que consume Ajustes Lite (5, 6, 9, 10–14, 15, 18, 19, 22, 23 y 27) corren también los tests de desktop en Lite:

  ```bash
  VITE_LOCAL_ML=0 pnpm --filter @entropia-pro/desktop test
  ```

- **Serialización de Tiptap.** Los pasos que tocan `U/NoteEditor/**` o `U/WritingEditor/**` (5, 7, 21, 26 y 28) verifican que `U/WritingEditor/serialization-golden.test.ts` y `U/NoteEditor/__tests__/note-serialization-golden.test.ts` pasan **sin actualizar snapshots**. Ningún paso toca extensiones de Tiptap.
- **Métricas.** `pnpm style:metrics` tiene que bajar (nunca subir) frente a `scripts/style-metrics.baseline.json`.
- **Revisión visual.** En `dark`, `light`, `lite`, `paper` y `vivid`, con contraste `high`, de las pantallas tocadas. La hace la persona que revisa, en la app: la herramienta no ve la ventana de Tauri. Nada de esto requiere builds de Tauri, y nunca se abre `tauri dev` sobre el archivo real (ver el paso 2).

---

## Etapa A — Preparación (2–3 días)

### Paso 1 · Script de métricas de estilo

- **Objetivo:** tener una línea de base reproducible y un tablero de avance.
- **Acciones:** crear `scripts/style-metrics.mjs`, que recorra los `<style>` de `D/` y `U/` (sin comentarios ni `__tests__`) y cuente cada métrica del resumen de la auditoría: hex, `rgba`, `color-mix` con `black`/`white`, `var()` a tokens inexistentes, fallbacks, `font-size`/`font-weight`/`line-height`/`letter-spacing` literales, opacidades de `:disabled`, `z-index` y transiciones literales, overrides `:global` sobre componentes del sistema, y controles nativos en el markup (`<input>`, `<textarea>`, `<progress>`, `<details>`, radios). Que admita `--json` y `--file <ruta>` (métricas de un solo archivo, que usan los criterios de las etapas B y D) y que exponga un script `pnpm style:metrics`.
- **Hecho cuando:** su salida coincide con las cifras de la auditoría (±2 por diferencias de heurística), su resultado se guarda como `scripts/style-metrics.baseline.json` y `pnpm style:metrics --file <ruta>` devuelve las categorías de un archivo.

### Paso 2 · Capturas de referencia

- **Objetivo:** poder comparar antes y después de cada migración.
- **Acciones:** elegir unas 12 pantallas representativas: Inicio, Colección, Ítem (paneles de texto y análisis), Escritura, Investigación, Chat, Ajustes (General, Apariencia, Dependencias, Sync), Internet Archive, Importar, Explorador. Capturarlas en `dark`, `light`, `lite`, `paper` y `vivid`: C9, C10 y C11 solo se ven en los temas claros.
  - **Vía preferida:** la app de escritorio con un perfil de desarrollo (`ENTROPIA_DEV_PROFILE=capturas`, nunca sobre el archivo real; la línea de arranque tiene que decir `profile=dev:capturas`). El perfil arranca vacío: antes de capturar, cargar una colección de prueba con ítems, OCR y un manuscrito.
  - **Playwright sobre Vite (puerto 1420)** solo para las pantallas que rendericen sin el backend de Tauri: casi todas las vistas llaman a `invoke` al montar, así que hay que comprobarlo pantalla por pantalla y no darlo por hecho.
  - Las capturas las toma y las valida la persona que revisa.
- **Hecho cuando:** la carpeta local de capturas tiene 60 archivos `pantalla__tema.png` (12 × 5) y un `INDEX.md` que dice cómo se llega a cada pantalla y con qué datos de prueba.

### Paso 3 · Guardas con lista de excepciones decreciente

- **Objetivo:** que ningún PR nuevo agregue deuda.
- **Acciones:**
  1. Guardas de `U/**` en `U/__tests__/visual-contract.test.ts`, y guardas de `D/**` en `D/design-tokens.test.ts`, que ya contiene las equivalentes de desktop (`names only published tokens…`, `DECORATIVE_TINT`, `CONTROL_SCALE_PX`). No se hacen lecturas cruzadas entre paquetes. En las dos:
     - validar también los `var()` **con** fallback (detecta T1–T4);
     - prohibir fallbacks sobre tokens publicados;
     - prohibir hex, `rgb[a]()` y `color-mix(…black|white…)` en `<style>`;
     - en desktop, extender a `D/**` la regla `keeps primary controls off the accent fill` con el mismo filtro de selectores que ya usa (`/btn|button/i`). No se intenta detectar “cualquier elemento interactivo” por regex: los usos legítimos de `--color-accent` (`SplitDivider`, el switch) quedan fuera por construcción.
  2. **Las allowlists no se escriben en el test.** Hay un archivo de datos por paso de la etapa D (`D/__tests__/style-allowlists/paso-15.json` … `paso-28.json`, y `U/__tests__/style-allowlists/paso-28.json` para el paquete de UI), más `resto.json` para lo que va a los pasos 29–30. Cada archivo lista, por regla, los archivos que hoy fallan. Los tests leen todos los JSON de su paquete. Así, cada PR solo edita el JSON de su paso, y los pasos 18–28 en paralelo no se pisan.
  3. Un test de completitud verifica que cada archivo de las allowlists aparece en **un solo** JSON, y que ese JSON corresponde al paso que lo migra según este plan.
  4. Agregar `progress` a `svelte/no-restricted-html-elements` en `eslint.config.js`, con allowlist por archivo.
- **Hecho cuando:** CI está verde con las allowlists, un archivo de prueba con `#fff` hace fallar el test y una entrada duplicada entre dos JSON hace fallar el test de completitud.
- **Regla de aquí en adelante:** cada paso que arregla un archivo lo **quita** del JSON de su paso en el mismo PR.

---

## Etapa B — Arreglos visibles sin refactor (4–5 días)

Estos pasos tocan solo CSS y tokens. Tienen el mayor impacto visible al menor riesgo. **Orden:** primero el 4 (tokens), después el 5 (codemod, mecánico, sobre todo el árbol) y recién entonces los 6–9 a mano, que pueden ir en paralelo entre sí. Así, ningún arreglo a mano queda en conflicto con el diff del codemod.

### Paso 4 · Tokens nuevos mínimos y decisiones de escala

- **Acciones:** agregar a `packages/ui/src/tokens/tokens.css`, en los 7 temas cuando corresponda:
  - `--color-on-danger` (calcularlo para ≥ 4.5:1 sobre `--color-danger` de cada tema);
  - `--opacity-disabled: 0.48` y `--opacity-disabled-primary: 0.72`;
  - `--space-0-5: 2px` y `--control-height-xs`;
  - `--line-height-reading: 1.6`;
  - `--letter-spacing-caps: 0.075em`;
  - `--shadow-highlight`: solo el brillo superior (`0 1px 0 rgba(255,255,255,.025) inset`), con valor `none` en `light`, `lite` y `paper`. **No** se usa `--shadow-surface` para reemplazar los brillos a mano: además del brillo trae una sombra exterior (`0 1px 2px rgba(0,0,0,.28)`), y los temas claros no la anulan (`tokens.css:100`; solo `--shadow-sm/md/lg` pasan a `none`);
  - `--shadow-inset` (la sombra interna negra de `Panel`), con valor `none` en los temas sin sombra;
  - la escala `--z-*` con **todos** los valores globales de hoy, así el paso no cambia nada visual: `sticky` 20, `drawer` 25 (`D/layout/AppShell.svelte:1158`), `popover` 30 (`D/components/OpenrouterModelList.svelte:407`), `popover-raised` 60 (`D/layout/NotificationBell.svelte:284`), `floating` 100 (`U/TopicEditor/TopicEditor.svelte:218`), `dropdown` 200, `menu` 210, `dialog` 1000, `dialog-nested` 1100, `dialog-top` 1200, `tooltip` 1300 y `blocking` 2000. Si al migrar algún valor conviene fusionarlo con otro, se decide en el paso que migra ese archivo y se escribe en el PR.

  **Decisiones que se cierran en este paso**, para que ningún paso posterior vuelva a `tokens.css`:
  - **Valor de `--control-height-xs`: 24px o 28px.** Hay 15 usos de `28px` (S2) y el escalón `sm` es 30px. Con xs = 24px, los `28px` pasan a `sm` y crecen 2px; con xs = 28px, coinciden, pero `IconButton` xs (24px) queda fuera de la escala. Se decide acá y se anota la tabla “literal → token” que usará la etapa D: `28px`, `26px`, `24px`, `22px` y `20px`, cada uno con su destino y la diferencia en px.
  - **`--font-size-2xl` y `--font-size-3xl` para la portada (F4):** se agregan acá o se descartan. El paso 25 ya no toma esa decisión.

  Sumar el override de scrollbars de `lite` y `paper` en `apps/desktop/src/app.css` (C14) y sumar todos los tokens nuevos a `contrast-floor.test.ts` y `theme-completeness.test.ts`. Actualizar `CONTROL_SCALE_PX` en `D/design-tokens.test.ts` si cambia la escala de alturas.
- **Hecho cuando:** los tests de contraste y de completitud de temas pasan con los tokens nuevos, `pnpm style:metrics` da los mismos números que la línea de base (todavía nadie usa los tokens) y las dos decisiones quedan escritas en este documento.

### Paso 5 · Codemod de fallbacks y literales sin ambigüedad

- **Objetivo:** limpiar en un solo PR mecánico lo que no requiere criterio, antes que cualquier arreglo a mano, para que nada posterior arrastre esos literales ni entre en conflicto con este diff.
- **Acciones:** escribir `scripts/codemods/style-literals.mjs` (se commitea, para que el cambio sea reproducible y auditable) que reemplace en `<style>`:
  - `var(--token, fallback)` → `var(--token)` para todo token publicado (T5, T6);
  - `font-weight: 500|600` → `var(--font-weight-medium|semibold)` (F6);
  - `font-size: 11px` → `var(--font-size-2xs)`; `border-radius: 4px` → `var(--radius-xs)`; `999px`/`9999px` → `var(--radius-full)` (B2);
  - `padding|margin|gap` con `4|8|12|16|20|24|32px` → el `--space-*` exacto (S1);
  - `letter-spacing: 0.075em` → `var(--letter-spacing-caps)`.

  Correr `pnpm format` después del codemod. **No** tocar `2px`, `6px`, `0.9em` ni los `line-height`, que requieren criterio.
- **Hecho cuando:**
  - `pnpm style:metrics` llega a 0 en fallbacks sobre tokens publicados y en las categorías de literales listadas arriba;
  - el resto de las categorías no cambia frente a la línea de base;
  - correr el script otra vez no produce diff (idempotente);
  - `git diff --stat` solo muestra archivos `.svelte` y el propio script;
  - los tests dorados de serialización pasan sin cambios.

### Paso 6 · Colores de estado en Dependencias y banner de la cáscara

- **Hallazgos:** C1, C2, C3, C4, C5, I1.
- **Archivos:** `D/views/DependenciasTab.svelte`, `D/layout/AppShell.svelte`, `D/lib/deps-status.ts`, `D/lib/deps-status.test.ts`.
- **Fuera de este paso, porque la etapa D los reemplaza:**
  - la barra de progreso de Dependencias (R7) pasa a `ProgressBar` en el paso 18;
  - los botones `.deps-banner__btn` (R4e y su parte de C5: `AppShell.svelte:1271`, `1278`, `1288`) pasan a `Button` en el paso 27. Pintarles ahora el hover con `--color-warning` además rompería el test `DECORATIVE_TINT` (`D/design-tokens.test.ts:119`).
- **Acciones:**
  - ámbar → `--color-warning`/`-soft` + `color-mix(… 30%, transparent)` para bordes;
  - índigo → `--color-accent`/`-soft`;
  - rojo/verde → `--color-danger*`/`--color-success*`;
  - el color del ícono de estado pasa de `style=""` a clases con los **cuatro** estados que hoy devuelve `dependencyStatusColor` (`D/lib/deps-status.ts:35`): `deps-row__icon--installed` (`--color-success`), `--missing` (`--color-danger`), `--failed` (`--color-warning`, no danger) y `--neutral` (`--color-text-muted`, para `checking`, `installing` y el resto). En `deps-status.test.ts`, un test que mapee cada estado a su clase.
- **Hecho cuando:** `pnpm style:metrics --file` da 0 en hex y `rgba` en los dos archivos, salvo las declaraciones de la barra de progreso y de `.deps-banner__btn`, que quedan en los JSON de los pasos 18 y 27; `deps-status.test.ts` cubre los cuatro estados.

### Paso 7 · Mezclas con negro y blanco

- **Hallazgos:** C9 (bloques `pre`), C10.
- **Archivos:** `U/NoteEditor/NoteEditor.svelte` (solo las reglas de `pre`), `D/layout/DocumentExplorer.svelte` (4 reglas).
- **Fuera de este paso:** el modal de `NoteEditor`, su input y el hover de `.note-editor__btn` (el resto de C9, junto con R4g) van al paso 26, que reemplaza esos elementos por `Dialog`, `Input` y `Button`.
- **Acciones:**
  - en `NoteEditor`, el fondo de los bloques `pre` pasa a `--color-surface-sunken`;
  - en `DocumentExplorer`, cada una de las cuatro reglas citadas en C10 recibe su token:
    - `:1258`, la barra `::before` del nodo activo (acento + 12% de blanco) → `--color-accent`;
    - `:1282`, el texto del nodo activo → `--color-text-primary` puro;
    - `:1310`, el ícono en reposo (`text-secondary` + 12% de blanco) → `--color-text-secondary` puro;
    - `:1328`, el ícono activo → `--color-accent`.
- **Hecho cuando:** `DocumentExplorer.svelte` tiene 0 `color-mix` con `white` o `black` y los `pre` de `NoteEditor` no usan `black`; los tests dorados de serialización pasan sin cambios.

### Paso 8 · Tokens inexistentes y texto sobre rojo

- **Hallazgos:** T1, T3, T4, C6, C7, C12.
- **Archivos:** `D/views/BibliographySearchTab.svelte`, `D/views/ItemSearchPanel.svelte`, `D/views/WritingHistoryPanel.svelte`, `D/layout/NotificationBell.svelte`.
- **Acciones:**
  - `--color-text-danger` → `--color-danger`;
  - `--color-text-tertiary` → `--color-text-muted`;
  - `--color-surface-hover` → `--color-accent-faint`;
  - en el contador de notificaciones, solo `color: #fff` → `--color-on-danger` (C6, alta). El contador pasa a `Badge` en el paso 27, y ahí toma `--font-size-2xs` y la forma definitiva.
- **Hecho cuando:** `pnpm style:metrics` da 0 en `var()` a tokens inexistentes, salvo `InternetArchiveDialog` (T2, se resuelve en el paso 16), y `NotificationBell.svelte` no tiene hex.

### Paso 9 · Componentes base: brillos, sombras, deshabilitado, foco y transiciones

- **Hallazgos:** C11 y H1 en los componentes base, E1 y E8 en los componentes base.
- **Archivos:** `U/Button/Button.svelte`, `U/IconButton/IconButton.svelte`, `U/Input/Input.svelte`, `U/Checkbox/Checkbox.svelte`, `U/NumberField/NumberField.svelte`, `U/ToolbarMenu/ToolbarMenuList.svelte`, `U/Panel/Panel.svelte`.
- **Fuera de este paso:** C11, H1 y H2 en vistas y en otros componentes los cierra el paso de la etapa D que migra cada archivo. `ItemAssetPanel`, `ItemTextPanel`, `ItemAnalysisPanel` e `ItemSearchPanel` van al 23, `CollectionsView` al 24, `SettingsView` al 15, `NoteEditor:942` al 26, `NotificationBell` al 27 y `MapViewer` al 28.
- **Acciones:**
  - `inset 0 1px 0 rgba(255,255,255,…)` → `var(--shadow-highlight)`;
  - el *inset* negro de `Panel` → `var(--shadow-inset)`;
  - `:disabled` → `--opacity-disabled` (`--opacity-disabled-primary` en el primario); foco → `--focus-ring`; transiciones → `--transition-*` con `prefers-reduced-motion`.

  Este paso existe para que los pasos 10 a 14 no editen todos estos mismos seis archivos en paralelo.
- **Hecho cuando:**
  - los siete archivos tienen 0 `inset 0 1px 0 rgba`, 0 opacidades literales en `:disabled` y 0 transiciones literales;
  - un test de contrato en `visual-contract.test.ts` lo verifica sobre esos archivos;
  - `--shadow-highlight` vale `none` en los temas claros (lo comprueba `theme-completeness.test.ts`).

---

## Etapa C — Ampliar el sistema de diseño (2–3 semanas)

Un componente por PR, en `packages/ui`, con `*.types.ts`, tests y export en `src/index.ts`. **El orden importa:** va primero lo que desbloquea más migraciones. Los componentes nuevos nacen con `--opacity-disabled`, `--focus-ring` y `--transition-*` (E1, E8); los componentes base ya quedaron alineados en el paso 9.

| Paso | Componente | Desbloquea | Base a reutilizar |
| --- | --- | --- | --- |
| 10 | **`Input` ampliado**: acepta todos los `HTMLInputAttributes` (`...rest`), `class`, `bind:this`, `size="sm|md"` y snippets `leading`/`trailing`. La API actual sigue funcionando igual. **No admite `type="search"`**: las cajas de búsqueda son siempre `SearchBar`. | ~30 campos crudos (R1), G2 | `U/Input/Input.svelte`, `.search-field__icon` |
| 11 | **`Textarea`**: familia visual de `Input`, `rows`, autoaltura opcional, `label`/`hint`/`error`, `font="ui|reading|mono"`. | 22 textareas (R2) | `Input` |
| 12 | **`RadioGroup` + `RadioCard`**: radio oculto, tarjeta con `:has(:focus-visible)`, variantes `card` y `list`, y una opción con input embebido (“nueva colección”). | N1–N5, R5 (4 diálogos + Ajustes + 2 grillas) | `D/layout/FontPresetGrid.svelte`, `D/views/WritingExportTab.svelte` |
| 13 | **`SelectTrigger`** (y opcionalmente `Select` = trigger + `ToolbarMenu` radio): `size="sm|md"`, variantes `field` y `quiet`, chevron por `ActionIcon`, estado `open`. Nunca un `<select>` nativo. | 15 disparadores (R3) | `ToolbarMenu`, patrón `WritingZoteroTab` |
| 14a | **`ProgressBar`** (`role="progressbar"`, `aria-valuenow`) | N6, N7, N8, R7: pasos 15, 16, 17, 18 y 23 | `.settings__download-progress-bar` |
| 14b | **`Badge`/`Chip`** (o `StatusBadge` con `size="xs"` y variante `count`) | R8: pasos 15, 16, 21, 23, 24 y 27 | `StatusBadge` |
| 14c | **`Dialog`** (la cáscara que luego compone `ConfirmDialog`) | R10: pasos 18, 24 y 26 | `ConfirmDialog` |
| 14d | **`Switch`** | R9: paso 19 | `AssistantsTab` (switch) |
| 14e | **`Disclosure`** | N9, N11: pasos 20, 23 y 24 | — |
| 14f | **`Button variant="link"`** | R4f: pasos 19, 26 y 27 | `Button` |

Los pasos 14a–14f son PRs separados. Cada uno tiene que estar mergeado antes del primer paso de la etapa D que lo consume (columna “Desbloquea”). El 14a va primero porque cierra hallazgos altos. Que `ConfirmDialog` pase a componerse con `Dialog` (14c) afecta a toda la app: sus tests de uso tienen que seguir verdes sin modificaciones.

- **Hecho cuando (por componente):**
  - tiene tests de comportamiento;
  - tiene un test de contrato visual en `visual-contract.test.ts`: 0 hex, 0 `rgba` y 0 fallbacks en su `<style>`, y alturas por token como en `control-block-size.test.ts`;
  - un test recorre los 7 temas de `tokens.css` y verifica que todos los tokens que usa el componente están definidos en cada uno;
  - está exportado en `src/index.ts`.

---

## Etapa D — Migración por archivo (6–9 semanas)

Cada paso toma **un archivo o un grupo pequeño** y lo deja terminado: componentes base, tokens, estados, escalas tipográficas y de espaciado, capas `--z-*` y salida de **todas** las allowlists. El orden va de lo más visible y concentrado a lo más disperso. Los pasos 15, 20 y 23 se parten en PRs de unas 400 líneas (por ejemplo, `SettingsView` por pestaña). El archivo sale del JSON de su paso con el último de esos PRs.

**Checklist común a cada archivo:**

- [ ] Controles nativos → componentes (radios, `progress`, `details`, `textarea`, `input`).
- [ ] Botones e inputs ad hoc → `Button`/`IconButton`/`Input`/`SelectTrigger`, salvo las excepciones de la guía (filas, nodos, tarjetas, edición inline).
- [ ] **Campos de búsqueda → `SearchBar`, nunca `Input`** (el test `D/design-tokens.test.ts` exige que `type="search"` viva solo en `SearchBar`). Si un buscador necesita algo que `SearchBar` no tiene, se amplía `SearchBar`.
- [ ] Selects → `SelectTrigger`/`ToolbarMenu`, nunca `<select>`.
- [ ] Sin `:global` que cambie la apariencia de un componente del sistema.
- [ ] `font-size` ≥ 11px y por token; `line-height` → `tight|base|reading`.
- [ ] `2px` → `--space-0-5`; resto del espaciado y de las alturas por token, según la tabla “literal → token” del paso 4. Cuando un control cambia de altura, la diferencia en px se anota en el PR.
- [ ] Radios con la escala semántica (`control`, `input`, `surface`, `dialog`, `full`).
- [ ] Brillos y sombras: `--shadow-highlight`/`--shadow-inset`/`--shadow-*` sin fallback (C11, H1, H2 del archivo).
- [ ] Foco con `--focus-ring` (no con `outline: 2px` ni con el mismo fondo del hover); disabled con `--opacity-disabled`.
- [ ] `z-index` global → `--z-*`; transiciones → tokens.
- [ ] Los tests que buscan `input[type=radio]` o `progress` en ese archivo se ajustan en el mismo PR.

**Hecho cuando (cada paso de la etapa D):**

- el JSON de allowlist del paso queda vacío y se borra;
- `pnpm style:metrics --file <ruta>` da 0 en cada archivo del paso en hex, `rgba`, `color-mix` con `black`/`white`, fallbacks, tokens inexistentes, `<progress>`, radios nativos, `<textarea>` crudo y `:global` sobre componentes del sistema;
- la verificación estándar pasa, con los extras de Lite y de serialización que correspondan;
- la persona que revisa lo comparó contra las capturas del paso 2 en los cinco temas.

| Paso | Archivo(s) | Hallazgos que cierra |
| --- | --- | --- |
| 15 | `D/views/SettingsView.svelte`, `D/views/SyncSettingsCard.svelte` | G1, G2, N5 (16 radios), R2, R3, R4c, R7, F2, R8, C11 (`:3565`, `:4087`); los `:global` de botón e input desaparecen |
| 16 | `D/views/InternetArchiveDialog.svelte` | N4, N7, R1, R4a (primario con acento + `--color-text-on-accent`), R4b, R5, R6, R8, T2 |
| 17 | `D/views/NavegadorCopyDialog.svelte`, `D/views/ImportSourcesDialog.svelte`, `D/views/NavegadorZoteroDialog.svelte` | N1, N2, N3, N6, R1, R5: los tres pasan al mismo `RadioGroup` con opción “nueva colección” |
| 18 | `D/views/DependenciasTab.svelte` (lo que no cerró el paso 6) | R1, R7 (barra → `ProgressBar`), R10 (confirmación de reset → `Dialog`), F2, F3, E8 (`:1151`) |
| 19 | `D/views/AppearanceTab.svelte`, `D/views/BackupTab.svelte`, `D/views/AssistantsTab.svelte`, `D/components/settings/EntityAliasesTab.svelte`, `D/components/settings/AgentParametersTab.svelte`, `D/components/OpenrouterModelList.svelte` | R3, R4d, R4f, R4i, R9 (`Switch`), E1 |
| 20 | `D/views/InvestigationView.svelte`, `D/views/ResearchView.svelte`, `D/views/RagChatView.svelte`, `D/views/CollectionAnalysisPanel.svelte`, `D/views/GraphSettingsForm.svelte` | R2 (8 textareas de Investigación), N11, N12, R3, F1, F5, F6, E6; el buscador de `RagChatView` sigue siendo `SearchBar` |
| 21 | `D/views/WritingView.svelte`, `D/views/WritingCitationEditor.svelte`, `D/views/WritingZoteroTab.svelte`, `D/views/WritingZoteroDetails.svelte`, `D/views/WritingHistoryPanel.svelte`, `D/views/WritingSharePanel.svelte`, `U/WritingEditor/WritingEditor.svelte` (solo inputs del popover de enlace) | R1, R3, R8, F5. **No tocar la serialización**: los tests dorados de `WritingEditor` y `NoteEditor` tienen que seguir sin cambios |
| 22 | `D/views/BatchProcessingTab.svelte`, `D/views/BatchFlowPicker.svelte`, `D/views/BatchSchemaPanel.svelte` | R3 (`menu-select` duplicado), N12, F5, E1 |
| 23 | `D/views/ItemTextPanel.svelte`, `D/views/ItemAssetPanel.svelte`, `D/views/ItemAnalysisPanel.svelte`, `D/views/ItemSearchPanel.svelte`, `D/views/ItemLayoutPanel.svelte`, `D/views/OccurrenceStepper.svelte`, `D/views/ItemView.svelte` | N8 (alta), N11, R1, R2, R4d, R4h, R8, R11, C11, scrollbars locales (§2.11), I2, E1, E8 |
| 24 | `D/views/CollectionView.svelte`, `D/views/CollectionsView.svelte`, `D/views/BibliotecaView.svelte`, `D/views/BibliographyWorkView.svelte`, `D/components/BibliographyAnnotationsPanel.svelte`, `D/views/DbBrowserView.svelte` | N9, R2, R3, R8, R10, R11, C11, G3, I2, E1 |
| 25 | `D/views/HomeView.svelte` | F4 (con los tokens que decidió el paso 4), E3 |
| 26 | Diálogos sueltos: `D/components/AgentApprovalDialog.svelte`, `D/components/PassageOriginalViewer.svelte`, `D/views/SimilarAssetPreviewDialog.svelte`, `U/NoteEditor/NoteEditor.svelte` (modal, su input y `.note-editor__btn`) | R10, R4f, R4g (`NoteEditor`), R4i, C9 (modal, input y hover), C11 (`NoteEditor:942`) |
| 27 | Cáscara: `D/layout/AppShell.svelte`, `D/layout/TopBar.svelte`, `D/layout/NotificationBell.svelte`, `D/layout/DocumentExplorer.svelte`, `D/views/NavegadorView.svelte`, `D/views/NavegadorSources.svelte` | R1 (el buscador global de `TopBar` sigue siendo o pasa a ser `SearchBar`, nunca `Input`), R4e y C5 (`.deps-banner__btn` → `Button`), R4f, R8 (contador → `Badge`), H2 (`NotificationBell`), B2, E4, S1, scrollbars locales |
| 28 | Paquete UI: `U/AudioPlayer/AudioPlayer.svelte` (botones + slider estilizado, N10), `U/MetadataEditor/MetadataEditor.svelte`, `U/MapViewer/MapViewer.svelte`, `U/EntityViewer/EntityViewer.svelte`, `U/TopicEditor/TopicEditor.svelte` (C13), `U/ItemCard/ItemCard.svelte`, `U/CollectionCard/CollectionCard.svelte` | R4g (`AudioPlayer`, `MetadataEditor`), R4h, R8, C13, H2 (`MapViewer`), E1, E2, E8 |

**Prioridad dentro de los pasos 18–28:** el 23 va primero, apenas esté el 14a, porque cierra N8, el único hallazgo alto que queda abierto después del paso 17.

Después del paso 15 conviene revisar el script de métricas: si alguna categoría no bajó lo esperado, ajustar el componente base antes de seguir migrando (es más barato corregirlo con una vista migrada que con diez).

---

## Etapa E — Cierre (3–4 días)

### Paso 29 · Escalas y capas residuales

- **Acciones:**
  - barrido final con el script de métricas para lo que quedó fuera de los archivos de la etapa D: `line-height`, `letter-spacing`, radios `sm/md` usados en controles (B1) y `@media` que deberían ser `@container pane` (§2.14);
  - capas `z-index` de archivos que ningún paso de la etapa D migra: `U/Tooltip/TooltipLayer.svelte` (1300), `U/ToolbarMenu/ToolbarMenu.svelte` (210), `D/App.svelte:185` (2000) y `D/app.css:168` (20) → `--z-*`;
  - E2 en `PdfMarkupLayer` y E8 en `U/ResizeHandle/ResizeHandle.svelte` y `U/DocumentViewer/DocumentViewer.svelte`;
  - H3, E5 y E7, o su paso a excepción permanente documentada en la guía;
  - decidir el destino de `--font-weight-bold` (F9): alias explícito de `semibold` documentado, o eliminarlo y reemplazar sus 14 usos.
- **Hecho cuando:**
  - `resto.json` queda vacío;
  - `pnpm style:metrics` da 0 `z-index` numéricos de tres o más cifras en `D/` y `U/` (el grep `z-index: [0-9]{3,}` no encuentra nada);
  - da 0 transiciones literales y 0 `line-height` literales fuera de las excepciones de la guía;
  - F9 queda decidido en `GUIA_DISENO.md`.

### Paso 30 · Retirar allowlists y tokens sin uso

- **Acciones:**
  - borrar la carpeta `style-allowlists/` y el test de completitud del paso 3. Solo quedan las excepciones permanentes documentadas en la guía (overlays de visor, `PdfMarkup`, `EntropicConstellation`, colores de impresión), escritas en el propio test;
  - activar la prohibición de `textarea` crudo en `eslint.config.js`, con excepción para `U/Textarea`;
  - eliminar o empezar a usar `--radius-xl`, `--shadow-glow`, `--space-7` y `--color-primary-subtle`.
- **Hecho cuando:** no existe ningún archivo `style-allowlists/*.json`, `pnpm lint` falla con un `<textarea>` de prueba fuera de `U/Textarea`, y la búsqueda de cada uno de los cuatro tokens da 0 o ≥ 1 uso fuera de `tokens.css`.

### Paso 31 · Documentación final

- **Acciones:**
  - en `GUIA_DISENO.md`, pasar todo lo marcado **(propuesto)** a vigente y quitar las alternativas temporales;
  - re-correr `pnpm style:metrics`, comparar contra la línea de base y anexar la tabla “antes / después” a `AUDITORIA_ESTILOS.md`;
  - repetir las capturas del paso 2.
- **Hecho cuando:**
  - `GUIA_DISENO.md` no contiene la palabra “(propuesto)”;
  - la tabla “antes / después” está en la auditoría, con todas las categorías del paso 1 en 0 salvo las excepciones permanentes;
  - existen las 60 capturas nuevas, y la persona que revisa anotó en `INDEX.md` cada diferencia intencional.

---

## Archivos con más de un paso

Cada fila es un doble toque **declarado**: los pasos tocan declaraciones distintas, y el de la etapa D no rehace lo que hizo el anterior. Fuera de los pasos 5 (codemod mecánico sobre todo el árbol) y 29 (barrido final), ningún archivo de la app o del paquete de UI (`.svelte`, `.ts`, `.css`) aparece en más pasos que los que lista esta tabla. Los archivos de test y los JSON de allowlist no cuentan: cada paso agrega sus propios tests.

| Archivo | Pasos | Qué toca cada uno |
| --- | --- | --- |
| `packages/ui/src/tokens/tokens.css` | 4, 29, 30 | 4: todos los tokens nuevos y las decisiones de escala. 29: solo el destino de `--font-weight-bold`. 30: borrar tokens sin uso |
| `eslint.config.js` | 3, 30 | 3: prohibir `progress`. 30: prohibir `textarea` y quitar las allowlists |
| `D/views/DependenciasTab.svelte` | 6, 18 | 6: colores de estado y el ícono. 18: controles, `ProgressBar`, `Dialog`, tipografía |
| `D/layout/AppShell.svelte` | 6, 27 | 6: colores del banner (no sus botones). 27: botones del banner → `Button`, el resto de la cáscara |
| `U/NoteEditor/NoteEditor.svelte` | 7, 26 | 7: fondo de los `pre`. 26: modal, input, botones y C11 |
| `D/layout/DocumentExplorer.svelte` | 7, 27 | 7: los cuatro `color-mix` de C10. 27: scrollbars, botones, escalas |
| `D/layout/NotificationBell.svelte` | 8, 27 | 8: `#fff` → `--color-on-danger`. 27: contador → `Badge`, H2, capas |
| `D/views/ItemSearchPanel.svelte` | 8, 23 | 8: tokens inexistentes (T3). 23: la migración completa |
| `D/views/WritingHistoryPanel.svelte` | 8, 21 | 8: tokens inexistentes (T4). 21: la migración completa |
| `D/views/BibliographySearchTab.svelte` | 8 | Solo el 8 (C12/T1) |
| `U/Button/Button.svelte` | 9, 14f | 9: sombras, disabled, foco y transiciones. 14f: la variante `link` |
| `U/Input/Input.svelte` | 9, 10 | 9: disabled, foco y transiciones. 10: API ampliada |
| `U/ToolbarMenu/ToolbarMenuList.svelte` | 9, 13 | 9: disabled. 13: solo si `Select` lo compone, sin tocar su estilo |

---

## Calendario orientativo y dependencias

```
A  1 Métricas ─ 2 Capturas ─ 3 Guardas
                               │
B          4 Tokens ─ 5 Codemod ─┬─ 6 Deps/banner
                                 ├─ 7 Negro/blanco
                                 ├─ 8 Tokens rotos
                                 └─ 9 Componentes base
                               │
C   10 Input ─ 11 Textarea ─ 12 Radio ─ 13 SelectTrigger ─ 14a…14f
                               │
D   15 Ajustes ─ 16 IA ─ 17 Diálogos destino ─ 23 Ítem ─ 18…28 (en paralelo entre áreas)
                               │
E                   29 Escalas ─ 30 Allowlists ─ 31 Docs
```

| Etapa | Duración estimada (1 persona) | Paralelizable |
| --- | --- | --- |
| A | 2–3 días | No |
| B | 4–5 días | Pasos 6 a 9 entre sí, después del 5 |
| C | 2–3 semanas | Pasos 11 a 14f entre sí, después del 10 |
| D | 6–9 semanas | Pasos 18 a 28 entre sí, después del 15 (que valida los componentes) y del componente de la etapa C que consume cada uno; cada uno edita solo su JSON de allowlist |
| E | 3–4 días | No |

Las estimaciones suponen unos 2–3 días por archivo grande de la etapa D (`SettingsView` ≈ 4300 líneas, `InternetArchiveDialog` ≈ 2700, `InvestigationView` ≈ 2400), y suman la revisión visual en cinco temas, que hace a mano la persona que revisa.

## Riesgos y cómo mitigarlos

| Riesgo | Mitigación |
| --- | --- |
| Cambiar la API de `Input` rompe sus 50 usos | El paso 10 es aditivo: las props actuales no cambian; los tests existentes de las vistas tienen que seguir verdes sin modificaciones. |
| Tests de vistas que buscan `input[type=radio]` o `progress` (unos 50 usos en 10 archivos de test) | Mantener la semántica nativa: el radio oculto sigue siendo un `input[type=radio]` y `ProgressBar` expone `role="progressbar"` con `aria-valuenow`. Ajustar los selectores de los tests en el mismo PR. |
| Cambios que se cuelan en el contenido guardado del manuscrito o las notas | Los pasos 5, 7, 21, 26 y 28 corren los tests dorados de serialización sin actualizar snapshots; ningún paso toca extensiones de Tiptap. |
| Diferencias Pro/Lite | Los pasos 5, 6, 9, 10–14, 15, 18, 19, 22, 23 y 27 corren los tests de desktop con `VITE_LOCAL_ML=0` (ver la verificación estándar). |
| Abrir la app de desarrollo sobre el archivo real | Las capturas y las revisiones visuales usan siempre `ENTROPIA_DEV_PROFILE`; si la línea de arranque dice `profile=shared`, se cierra la app. |
| CI rojo por formato | `pnpm format:check` está en la verificación estándar, y el codemod del paso 5 termina con `pnpm format`. |
| Las capturas con Playwright sobre Vite no cargan las vistas (dependen de `invoke`) | El paso 2 usa como vía principal un perfil de desarrollo; Playwright solo para las pantallas que se comprueba que rinden sin backend. |
| La revisión visual no se puede automatizar | La hace la persona que revisa en cada PR; las estimaciones ya incluyen ese tiempo. |
| Una sustitución de sombra cambia el aspecto | Los brillos pasan a `--shadow-highlight` (solo brillo, `none` en temas claros), nunca a `--shadow-surface`. |
| Las alturas de control cambian unos px al pasar a la escala | La tabla “literal → token” del paso 4 fija cada destino; cada PR anota las diferencias en px. |
| Tests existentes que codifican reglas de estilo (`CONTROL_SCALE_PX`, `DECORATIVE_TINT`, “native checkbox in exactly one component”) | Se actualizan en el mismo PR que agrega el token o el componente; ningún arreglo se hace en contra de ellos (por eso los botones del banner esperan al paso 27). |
| Conflictos de merge en las allowlists durante 6–9 semanas | Un JSON por paso: cada PR edita solo el suyo, y el test de completitud detecta un archivo listado en dos lugares. |
| Regresiones de contraste | Todo token nuevo entra en `contrast-floor.test.ts`; revisar en contraste `high` y `soft`. |
| Deriva durante la migración (PRs ajenos que agregan deuda) | Las allowlists del paso 3 solo se achican; un PR que agrega una entrada se rechaza. |
| Migraciones largas que quedan a medias | PRs de unas 400 líneas; el archivo sale del JSON de su paso con el último, así el estado parcial queda registrado. |
