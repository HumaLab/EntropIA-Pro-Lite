# Auditoría integral de estilos — EntropIA

> Diagnóstico de solo lectura del frontend (`apps/desktop/src` y `packages/ui/src`) y plan de homogeneización.
> Base auditada: rama `claude/zealous-hamilton-980mls` sobre `319282d` (noviembre 2026).
> Reglas que se derivan de este informe: [`GUIA_DISENO.md`](./GUIA_DISENO.md).

---

## 0. Alcance, método y escala de severidad

**Alcance.** 144 componentes `.svelte` (~74 800 líneas, sin contar `__tests__`/`__mocks__`) y las tres hojas CSS globales:

| Archivo | Rol |
| --- | --- |
| `packages/ui/src/tokens/tokens.css` | Tokens: 6 temas (`dark` por defecto, `dim`, `light`, `lite`, `paper`, `forest`, `vivid`), 3 niveles de contraste, 4 presets tipográficos |
| `apps/desktop/src/app.css` | Reset, scrollbars globales, tipografía de encabezados, patrones de página (`.page-shell`, `.page-header`, `.surface-message`, `.search-field__icon`, `.number-field-label`) |
| `apps/desktop/src/fonts.css` | `@font-face` de las familias empaquetadas |

**Método.** Lectura completa de tokens, `app.css` y los componentes base de `packages/ui`; barrido estático de los bloques `<style>` (comentarios excluidos) y del markup con búsquedas por patrón: controles nativos, valores literales (hex, `rgb[a]`, `px`, `em`, pesos, `line-height`, `z-index`, duraciones), `var()` con tokens inexistentes, `outline: none` sin reemplazo, opacidades de `:disabled`, overrides `:global(...)` sobre componentes del sistema y estilos inline. Se cotejaron también las guardas existentes (`eslint.config.js`, `visual-contract.test.ts`, `contrast-floor.test.ts`, `control-block-size.test.ts`, `theme-completeness.test.ts`). No se ejecutaron builds ni la app: los hallazgos sobre el aspecto renderizado se infieren del CSS.

**Severidad.**

| Nivel | Criterio |
| --- | --- |
| **Alta** | Visible para el usuario: control con aspecto del navegador/SO, color que no sigue el tema (o rompe el contraste en algún tema), token inexistente que deja un estado sin estilo, componente del sistema reestilizado localmente. |
| **Media** | Inconsistencia perceptible entre pantallas (alturas, fondos, radios, foco, opacidades distintas para el mismo control) o duplicación que va a seguir divergiendo. |
| **Baja** | Deuda interna sin efecto visual hoy (fallbacks engañosos, literales equivalentes a un token, tokens sin uso). |

---

## 1. Resumen ejecutivo

El sistema de diseño **es sólido en su base**: los tokens cubren color, superficie, espaciado, tipografía, radios, sombras, alturas de control y foco para siete temas, con contraste medido por tests; las familias tipográficas pasan siempre por `--font-ui/--font-reading/--font-mono` (0 familias literales en todo el código); `<select>` y `<svg>` crudos ya están prohibidos por lint; las scrollbars son globales; la mayoría de los `outline: none` tienen un anillo de foco de reemplazo.

Los problemas se concentran en **cuatro causas raíz**:

1. **Faltan componentes base** para controles muy usados: no hay `Textarea`, `Select` (disparador de `ToolbarMenu`), `RadioGroup`/`RadioCard`, `Switch`, `ProgressBar`, `Badge`/`Chip`, `Disclosure` ni un `Dialog` genérico. Cada vista los reimplementa con su propia clase BEM y sus propios valores.
2. **`Input` es demasiado estrecho**: `Input.types.ts` solo acepta `value/type/placeholder/disabled/label/error/hint`; no reenvía `class`, `id`, `name`, `autocomplete`, `onkeydown`, `oninput`, `bind:this`, `maxlength`, `required`… Por eso 30+ campos usan `<input>` crudo con estilos copiados.
3. **Colores fuera de tema** en un puñado de archivos (sobre todo `DependenciasTab.svelte` y el banner de dependencias de `AppShell.svelte`), escritos con la paleta de Tailwind (`#6b7280`, `rgba(245,158,11,…)`, `rgba(99,102,241,…)`) y mezclas con `black`/`white` que no se adaptan a los temas claros.
4. **Tokens inexistentes** usados con fallback (`--color-text-tertiary`, `--color-text-danger`, `--color-text-on-accent`, `--color-surface-hover`): el test de contrato solo verifica los `var()` *sin* fallback, así que estos pasan.

### Métricas del barrido

| Métrica | Valor |
| --- | --- |
| `<button>` crudos / `<Button>` / `<IconButton>` | 250 / 255 / 107 |
| `<input>` crudos, radios incluidos (sin contar los internos de `packages/ui`) / `<Input>` | 63 (27 radios, 33 `text`, 1 `password`, 2 sin `type`) / 50 |
| `<textarea>` crudos (no existe componente) | 22 |
| `<input type="radio">` nativos visibles sin `appearance: none` | 25 (16 con `accent-color`, 9 sin ningún estilo) |
| `<progress>` nativos | 5 (3 sin ningún estilo) |
| `<details>/<summary>` | 8, cada uno estilizado por separado |
| Clases locales que imitan un disparador de select | 15 |
| Clases locales de badge/chip/pill/tag (fuera de `StatusBadge`) | 22 |
| Hex literales en `<style>` | 30 (20 en `DependenciasTab.svelte`) |
| `rgb[a]()` literales en `<style>` | 42 (17 en `DependenciasTab.svelte`) |
| `var(--token-inexistente, …)` | 6 usos de 4 tokens |
| `font-size` literales | 21 (incluye 9px y 10px, bajo el mínimo `--font-size-2xs` = 11px) |
| `line-height` literales | 48 (10 valores distintos) |
| `letter-spacing` distintos | 12 valores |
| Opacidades distintas para `:disabled` | 8 valores (0.4 → 0.72) |
| `z-index` literales | 33 (18 valores distintos, de 0 a 2000) |
| Overrides `:global(.btn|.input-field|.icon-button…)` | 37 en 11 archivos (14 en `SettingsView.svelte`) |

---

## 2. Hallazgos

Rutas abreviadas: `D/` = `apps/desktop/src/`, `U/` = `packages/ui/src/components/`. Las líneas apuntan a la apertura del elemento o de la regla CSS.

### 2.1 Controles nativos con apariencia del navegador

| # | Archivo:línea | Elemento | Problema | Sev. |
| --- | --- | --- | --- | --- |
| N1 | `D/views/NavegadorCopyDialog.svelte:205`, `:211` | `input[type=radio]` | Radio nativo sin ningún estilo (ni `accent-color`): círculo azul del sistema en WebView2/WebKit, fuera del tema. | Alta |
| N2 | `D/views/ImportSourcesDialog.svelte:291`, `:306` | `input[type=radio]` | Ídem N1 (selector de colección destino). | Alta |
| N3 | `D/views/NavegadorZoteroDialog.svelte:296` | `input[type=radio]` | Ídem N1 (biblioteca de Zotero). | Alta |
| N4 | `D/views/InternetArchiveDialog.svelte:1545`, `:1555`, `:1629`, `:1638` | `input[type=radio]` | Ídem N1 (destino y elección de formato). | Alta |
| N5 | `D/views/SettingsView.svelte:1934`, `1957`, `1967`, `2092`, `2107`, `2453`, `2463`, `2473`, `2488`, `2502`, `2677`, `2687`, `2697`, `2718`, `2733`, `2748` (regla `:3811`) | `input[type=radio]` | Solo `accent-color: var(--color-accent)` + `margin-top: 3px`: el control sigue siendo el del SO (tamaño, borde, anillo de foco nativo); en temas claros el anillo y el relleno no coinciden con `Checkbox`. | Media |
| N6 | `D/views/ImportSourcesDialog.svelte:223` | `<progress>` | Sin ninguna regla: barra verde/azul nativa. | Alta |
| N7 | `D/views/InternetArchiveDialog.svelte:1684` | `<progress>` | Ídem N6. | Alta |
| N8 | `D/views/ItemTextPanel.svelte:266`, `:388` (regla `:643`) | `<progress class="ocr-progress">` | `appearance: none` sin `::-webkit-progress-bar/-value`: queda el gris/verde por defecto de Chromium, y la barra de valor no toma el radio. | Alta |
| N9 | `D/views/CollectionView.svelte:1244` (regla `:1728`) | `<progress>` | Solo `accent-color`: track nativo; distinto de N8 y de las barras `div` de §2.2-R7. | Media |
| N10 | `U/AudioPlayer/AudioPlayer.svelte:398` (regla `:521`) | `input[type=range]` | Solo `accent-color`: thumb y track del SO. Es el único slider de la app. | Media |
| N11 | `D/views/ItemTextPanel.svelte:285`, `309`, `405`; `D/views/ResearchView.svelte:594`, `656`; `D/views/ItemSearchPanel.svelte:140`, `213`; `D/views/InvestigationView.svelte:1650` | `<details>/<summary>` | Marcador triangular nativo en 7 de 8 (en `ItemTextPanel` desaparece por `display: flex`); ninguno define `:focus-visible` en `summary` (queda el outline del navegador); tipografía y color distintos en cada uno (`D/views/ResearchView.svelte:1058`, `:1132`; `D/views/ItemSearchPanel.svelte:326`, `:493`; `D/views/InvestigationView.svelte:2113`; `D/views/ItemTextPanel.svelte:668`). | Media |
| N12 | `D/views/InvestigationView.svelte:1416`, `D/views/DbBrowserView.svelte:756`, `D/views/BatchSchemaPanel.svelte:261` | `<table>` | Tres tablas con tres estilos distintos (bordes, padding, cabecera); la de `BatchSchemaPanel` no tiene clase y usa `var(--color-border, rgba(127,127,127,.3))` (`:304`, `:328`). | Baja |

> Sin hallazgos: `<select>` nativo (prohibido por lint y efectivamente ausente), `type="color"`, `type="file"`, `<dialog>`, `<meter>`. Las scrollbars son globales (§2.10). Los `type="search"` ya tienen el botón nativo anulado en `app.css:292`.

### 2.2 Componentes reimplementados localmente

**R1 — Campos de texto (`<input>` crudos). Severidad: media (alta donde se indica).** Al menos cinco “familias” de fondo/borde coexisten para el mismo control, con alturas de 20 a 40px:

| Familia (fondo + borde) | Dónde (regla CSS) | Altura |
| --- | --- | --- |
| A · `color-mix(surface-sunken 88%)` + `hairline` (la de `Input`) | `U/Input/Input.svelte` (base); `D/components/OpenrouterModelList.svelte:365`; `D/views/InternetArchiveDialog.svelte:2372`, `:2672`; `D/views/NavegadorCopyDialog.svelte:303`; `D/views/ImportSourcesDialog.svelte:454`; `D/views/CollectionsView.svelte:390` | md / sm |
| B · `--surface-input` + `--border-subtle` | `D/layout/TopBar.svelte:875`; `D/views/RagChatView.svelte:1691`; `D/views/WritingCitationEditor.svelte:388`; `D/views/WritingZoteroTab.svelte:781`; `D/views/GraphSettingsForm.svelte:333`; `U/WritingEditor/WritingEditor.svelte:1557`; `D/layout/AppShell.svelte:1022` | md / sm / **28px** / **26px** |
| C · `color-mix(surface-glass 78%)` + `hairline 78%` | `D/views/SettingsView.svelte:4020`; `D/views/SyncSettingsCard.svelte:1059` | md |
| D · `--color-surface` + `--color-border` | `U/MetadataEditor/MetadataEditor.svelte:200`; `D/views/DependenciasTab.svelte:1355`; `D/views/ItemAnalysisPanel.svelte:785` | auto |
| E · `color-mix(surface 82%, black 18%)` | `U/NoteEditor/NoteEditor.svelte:979` (ver §2.3-C9) | auto |

Además: `D/views/NavegadorView.svelte:812` (`surface-sunken` puro), `D/views/RagChatView.svelte:1261` (siempre con anillo de foco, aun sin foco), `D/views/WritingView.svelte:1197`, `:1402`, `D/views/DependenciasTab.svelte:948`, `D/views/InvestigationView.svelte:1486`.

**R2 — Áreas de texto (`<textarea>`; no hay componente). Media.** 22 textareas con 9 clases distintas: `D/views/SettingsView.svelte:3947` (familia C), `D/views/ResearchView.svelte:1002` (B), `D/views/CollectionAnalysisPanel.svelte:727` (B), `D/views/CollectionsView.svelte:390` (A), `D/views/ItemTextPanel.svelte:686` (B, `radius-md`), `D/components/BibliographyAnnotationsPanel.svelte:652` (`surface-sunken`), `D/views/InvestigationView.svelte:2286` (`--color-border, currentColor`, `--radius-sm, 4px`, 8 textareas), `D/views/RagChatView.svelte:1691` (B), `D/views/SyncSettingsCard.svelte:938` (reutiliza la clase `sync-card__number-input` en un textarea). Paddings: `space-1/2`, `space-2`, `space-2/3`, `space-3`.

**R3 — Disparadores de select (botón que abre `ToolbarMenu`). Media.** 15 clases para el mismo patrón, en tres variantes visuales y tres radios distintos:

| Variante | Reglas |
| --- | --- |
| Transparente, sin borde, `radius-sm`, sm | `D/views/AppearanceTab.svelte:306`, `D/views/BackupTab.svelte:213` |
| `surface-input` + `border-subtle`, `radius-control`, sm, `font-size-xs` | `D/views/BatchProcessingTab.svelte:2229`, `D/views/BatchFlowPicker.svelte:146`, `D/views/BatchSchemaPanel.svelte:335` (clase `menu-select` duplicada en dos archivos), `D/views/RagChatView.svelte:1632`, `D/views/ResearchView.svelte:886`, `D/views/WritingZoteroTab.svelte:796` |
| Ídem pero `radius-input` y 28px | `D/views/WritingCitationEditor.svelte:413` |
| `surface-glass` + `hairline`, md | `D/views/BibliotecaView.svelte:407`, `D/views/DbBrowserView.svelte:1011` |
| `surface-glass 78%` + `hairline 78%`, `radius-input`/`radius-control` | `D/views/SettingsView.svelte:3704`, `D/views/SyncSettingsCard.svelte:1227` |
| Otras | `D/views/BibliographyWorkView.svelte:553`, `U/EntityViewer/EntityViewer.svelte:523` |

**R4 — Botones que reimplementan `Button`/`IconButton`. Media (alta en R4a).**

| # | Regla | Observación |
| --- | --- | --- |
| R4a | `D/views/InternetArchiveDialog.svelte:2390` `.ia-submit` (×5, template `:1411`) | **Alta.** Botón primario relleno con `--color-accent`: contradice la escalera `--control-primary-*` (“el primario se distingue por luminosidad, nunca por tono”) y escapa al test `keeps primary controls off the accent fill`, que solo revisa 4 archivos de `packages/ui`. Usa además `--color-text-on-accent`, inexistente. |
| R4b | `D/views/InternetArchiveDialog.svelte:2487` `.ia-results__action` (×8), `:2413` `.ia-back`, `:1903`/`:1912` paginación sin clase | Botones secundarios/ghost ad hoc. |
| R4c | `D/views/SettingsView.svelte:4044` `.settings__icon-btn` (×8) | Reimplementa `IconButton` con fondo C. |
| R4d | `D/views/ItemAnalysisPanel.svelte:832` `.triple-action` (×6, 24px), `D/components/settings/EntityAliasesTab.svelte:434` (28px) y `:480`, `U/EntityViewer/EntityViewer.svelte:834` (20px) | Botones de icono con alturas fuera de la escala `IconButton` (24/28/32/40). |
| R4e | `D/layout/AppShell.svelte:1271` `.deps-banner__btn` (×5) | Borde `rgba(245,158,11,.5)` literal (§2.3). |
| R4f | `D/layout/NotificationBell.svelte:319`, `D/components/AgentApprovalDialog.svelte:353`, `D/components/OpenrouterModelList.svelte:260` | Botón-enlace (`color: accent`, sin fondo): tres implementaciones del mismo patrón. |
| R4g | `U/NoteEditor/NoteEditor.svelte:699` `.note-editor__btn`, `U/AudioPlayer/AudioPlayer.svelte:430` `.audio-player__btn`, `U/MetadataEditor/MetadataEditor.svelte:235` (borde *dashed*, `transition: 0.15s` literal) | Botones propios dentro del propio paquete de UI. |
| R4h | `D/views/OccurrenceStepper.svelte:52`, `:60` (regla `:103`), `U/MapViewer/MapViewer.svelte:372`, `:385`, `:388` (regla `:521`), `D/views/ItemAssetPanel.svelte:598`–`:614` (regla `:849`) | Botones sin clase estilizados por selector de descendiente (`button {…}`, `.x button`). |
| R4i | `D/views/SimilarAssetPreviewDialog.svelte:315` (36px literal), `D/views/SyncSettingsCard.svelte:1022`, `D/views/AppearanceTab.svelte:334` | Cierre / mostrar contraseña / pasos de zoom: equivalentes a `IconButton`. |

**R5 — Radios en tarjeta / grupos de destino. Media.** El patrón “lista de destinos con opción ‘nueva colección’ + input” está duplicado 4 veces con nombres propios: `copy-dialog__option` (`D/views/NavegadorCopyDialog.svelte:277`), `import-dialog__option` (`D/views/ImportSourcesDialog.svelte:421`), `zotero-dialog__option` (`D/views/NavegadorZoteroDialog.svelte:400`), `ia-destination__option` (`D/views/InternetArchiveDialog.svelte:2646`). El patrón correcto (radio visualmente oculto + tarjeta con `:has(:focus-visible)`) ya existe dos veces, también duplicado: `D/layout/FontPresetGrid.svelte:157` y `D/views/WritingExportTab.svelte:188`. La clase `.visually-hidden` se redefine localmente (p. ej. `D/views/CollectionView.svelte:1733`).

**R6 — Pestañas y segmentados. Baja/Media.** `D/views/InternetArchiveDialog.svelte:2121` (`ia-tabs__tab`) y `:2264` (`ia-segmented__option`) reimplementan `TabButton`/`TabList`.

**R7 — Barras de progreso `div`. Media.** Tres implementaciones además de los `<progress>` nativos: `D/views/DependenciasTab.svelte:1133`–`:1150` (indigo literal, `transition: width 0.3s`), `D/views/SettingsView.svelte:4225` (×5, `transition: width 0.2s`), `U/AudioPlayer/AudioPlayer.svelte:494` (`0.1s linear`).

**R8 — Badges, chips, pills y tags. Media.** 22 clases locales además de `StatusBadge`: `D/components/BibliographyAnnotationsPanel.svelte:622`, `D/views/DependenciasTab.svelte:1261`, `D/views/ItemLayoutPanel.svelte:576`/`635`/`774`, `D/views/WritingZoteroDetails.svelte:379`, `D/views/BibliographyWorkView.svelte:882`, `D/views/AssistantsTab.svelte:804`, `D/views/SyncSettingsCard.svelte:1161`, `D/views/NavegadorSources.svelte:768`, `D/views/InternetArchiveDialog.svelte:2506`, `D/views/NavegadorView.svelte:905`, `D/views/SettingsView.svelte:3577`/`3833` (`padding: 2px 8px`, `font-size: 10px`), `D/layout/NotificationBell.svelte:261`, `D/layout/TopBar.svelte:851`, `U/CollectionCard/CollectionCard.svelte:153`, `U/EntityViewer/EntityViewer.svelte:651`/`673`/`934`, `U/TopicEditor/TopicEditor.svelte:160`, `U/ItemCard/ItemCard.svelte:211`.

**R9 — Interruptor. Baja.** Único `role="switch"`: `D/views/AssistantsTab.svelte:390` (regla `:746`, 20px de alto, `--radius-full, 9999px`). Candidato a componente `Switch` antes de que aparezca el segundo.

**R10 — Cáscaras de diálogo fuera de `ConfirmDialog`. Media.** Fondos de overlay, superficies y capas distintas:

| Regla | Overlay | Superficie | `z-index` |
| --- | --- | --- | --- |
| `U/ConfirmDialog/ConfirmDialog.svelte:208` (referencia) | `--color-overlay` | `--color-surface-glass` | 1000 |
| `D/views/DbBrowserView.svelte:1238` | `--surface-overlay` | `--color-surface-glass` | 1000 |
| `D/components/AgentApprovalDialog.svelte:340` | — | `--color-surface-elevated` | 1100 |
| `D/components/PassageOriginalViewer.svelte:152` | `color-mix(overlay 88%)` | — | 1100 |
| `D/views/SimilarAssetPreviewDialog.svelte:223` | `color-mix(overlay 88%)` | — | 1100 |
| `U/NoteEditor/NoteEditor.svelte:902` | `--color-overlay` | `surface` + `black 8%` | 1200 |
| `D/views/DependenciasTab.svelte:1316` (`role="alertdialog"`, `:931`) | en línea, sin overlay | `--color-surface` | — |

**R11 — Botón de panel derecho duplicado. Baja.** `.right-panel-toggle` se reestiliza idéntico en `D/views/ItemView.svelte:4142` y `D/views/CollectionView.svelte:1500`.

### 2.3 Colores hardcodeados o que no siguen el tema

| # | Archivo:línea | Valor | Problema | Sev. |
| --- | --- | --- | --- | --- |
| C1 | `D/views/DependenciasTab.svelte:986`, `988`, `1040`, `1079`, `1178`, `1179` | `rgba(245, 158, 11, …)` (ámbar Tailwind) | No es `--color-warning` de ningún tema (en `paper` el warning es `#84520f`): bordes y fondos ámbar sobre papel sepia o verde bosque. | Alta |
| C2 | `D/views/DependenciasTab.svelte:997`, `999`, `1271` y `:1012`/`1103`/`1149` (`#4f46e5`, `#6366f1`) | `rgba(99, 102, 241, …)` (indigo) | Indigo fijo: en `forest` (acento jade) o `vivid` (fucsia) el panel “preparar” y la barra de progreso quedan de otro sistema. | Alta |
| C3 | `D/views/DependenciasTab.svelte:1067`, `1304`, `1306`, `1321`, `1323`, `1345`; `:1073`, `1276` | `rgba(239,68,68,…)`, `rgba(127,29,29,.12)`, `rgba(34,197,94,…)` | Rojo/verde Tailwind en lugar de `--color-danger*`/`--color-success*` (los fondos *sí* usan `-soft`: borde y fondo de distinto origen). | Alta |
| C4 | `D/views/DependenciasTab.svelte:1094`…`1359` (20 líneas) | `var(--token, #6b7280 / #e5e7eb / #f59e0b / #d1d5db / #f3f4f6)` | Fallbacks de paleta Tailwind claros: hoy inertes, pero documentan colores ajenos y se copian. | Baja |
| C5 | `D/layout/AppShell.svelte:1225`, `1227`, `1254`, `1278`, `1288` | `rgba(245, 158, 11, …)` | Banner de dependencias y su botón (R4e): mismo problema que C1, visible en la cáscara principal. | Alta |
| C6 | `D/layout/NotificationBell.svelte:273` | `color: #fff` | Texto del contador (9px) sobre `--color-danger`. En los temas claros el danger es oscuro y el blanco contrasta; en los oscuros no: ≈3.1:1 en `dark` (`#e26f7f`) y ≈2.5:1 en `vivid` (`#ff7a90`), bajo el 4.5:1 que el sistema exige. Falta un token `--color-on-danger`. | Alta |
| C7 | `D/layout/NotificationBell.svelte:387` | `var(--color-warning, #d99a00)` | Fallback distinto del resto (`#f59e0b`): tres “ámbares” en el código. | Baja |
| C8 | `U/DocumentViewer/DocumentViewer.svelte:2163` | `stroke: rgb(245, 158, 11)` | Overlay del visor con ámbar fijo. Aceptable si es una marca de anotación independiente del tema, pero debería ser un token (`--viewer-mark-*`). | Baja |
| C9 | `U/NoteEditor/NoteEditor.svelte:727`, `876`, `921`, `985`, `1003` | `color-mix(… var(--color-surface) N%, black M%)` | Oscurecer con negro funciona en temas oscuros; en `light`/`lite`/`paper` produce grises sucios (hover de botones, bloques `pre`, modal y su input). | Alta |
| C10 | `D/layout/DocumentExplorer.svelte:1258`, `1282`, `1310`, `1328` | `color-mix(…, white N%)` | Aclarar con blanco el texto/acento del nodo activo: en temas claros *baja* el contraste del texto (`text-primary` + 8% blanco, `text-secondary` + 12%), justo lo que `contrast-floor` protege en los tokens. | Alta |
| C11 | `U/Button/Button.svelte:103`; `U/Panel/Panel.svelte:54`, `64`; `D/views/ItemAssetPanel.svelte:779`; `D/views/ItemTextPanel.svelte:558`; `D/views/ItemAnalysisPanel.svelte:689`; `D/views/ItemSearchPanel.svelte:279`; `D/views/CollectionsView.svelte:460`; `D/views/SettingsView.svelte:3565`, `4087`; `U/NoteEditor/NoteEditor.svelte:942` | `inset 0 1px 0 rgba(255,255,255,.025–.05)` y variantes | El “brillo superior” (y en `Panel.svelte:64` una sombra interna negra) está escrito a mano unas 10 veces con 5 valores (`.022`, `.025`, `2%`, `.04`, `.05`) en vez de usar `--shadow-surface`, que ya lo contiene. Un brillo blanco no tiene sentido en los temas claros y estos literales no se anulan allí. | Media |
| C12 | `D/views/BibliographySearchTab.svelte:301` | `var(--color-text-danger, inherit)` | Token inexistente: el mensaje de error se pinta con el color heredado, **no en rojo**. | Alta |
| C13 | `U/TopicEditor/TopicEditor.svelte:165`, `:245` | `background: var(--color-accent)` | Chip y hover de sugerencia rellenos de acento; el resto de la app marca la selección por contraste (`Checkbox`: “Selected reads as contrast, never as saturation”). | Media |
| C14 | `D/app.css:15`–`20` | override de scrollbars solo para `light` | `lite` y `paper` heredan el track `rgba(255,255,255,.014)` del tema oscuro. El thumb sí se adapta (usa `color-mix`), así que el efecto es menor. | Baja |

Fuera de `<style>` (TS): `D/lib/ocr-pdf.ts` y `U/WritingEditor/writing-colors.ts` (`PRINT_COLORS`), `U/PdfMarkup/types.ts` (paleta de anotación de Zotero) usan hex **a propósito** (exportación a PDF y compatibilidad con Zotero): no son hallazgos.

### 2.4 Tokens inexistentes y fallbacks engañosos

| # | Archivo:línea | Token | Efecto | Sev. |
| --- | --- | --- | --- | --- |
| T1 | `D/views/BibliographySearchTab.svelte:301` | `--color-text-danger` | Ver C12. | Alta |
| T2 | `D/views/InternetArchiveDialog.svelte:2398` | `--color-text-on-accent` | Cae en `--color-surface-raised`; ver R4a. | Media |
| T3 | `D/views/ItemSearchPanel.svelte:437`, `483`, `489` | `--color-text-tertiary` | Cae en `--color-text-secondary`: quien lo escribió quería un tercer nivel que no existe. | Media |
| T4 | `D/views/WritingHistoryPanel.svelte:338` | `--color-surface-hover` | Cae en `--color-surface`: hover y seleccionado del historial casi no se distinguen del fondo. | Media |
| T5 | 13 usos de `var(--radius-sm, 4px)` (`D/views/DependenciasTab.svelte:1039`, `1246`, `1344`; `D/views/AssistantsTab.svelte:876`; `D/views/InvestigationView.svelte:2066`, `2159`, `2292`, `2382`; `D/views/WritingHistoryPanel.svelte:328`, `346`; `U/MapViewer/MapViewer.svelte:405`, `441`, `525`) | `--radius-sm` vale **6px** | El fallback miente sobre el valor del token. | Baja |
| T6 | `D/views/InvestigationView.svelte` (42 `var(--x, …)`), `D/views/DependenciasTab.svelte` (28), `U/MapViewer/MapViewer.svelte` (12) | fallbacks en general | Ruido: todos los tokens se publican desde `:root`; un fallback solo sirve para esconder un nombre mal escrito. | Baja |

### 2.5 Tipografía

| # | Archivo:línea | Valor | Problema | Sev. |
| --- | --- | --- | --- | --- |
| F1 | `D/layout/NotificationBell.svelte:275`; `D/views/CollectionAnalysisPanel.svelte:684`, `695` | `font-size: 9px` | Por debajo del mínimo del sistema (`--font-size-2xs` = 11px). | Media |
| F2 | `D/views/SettingsView.svelte:3838`; `D/views/DependenciasTab.svelte:1265` | `font-size: 10px` | Ídem F1. | Media |
| F3 | `D/views/DependenciasTab.svelte:1242` | `font-size: 11px` | Igual a `--font-size-2xs`: usar el token. | Baja |
| F4 | `D/views/HomeView.svelte:742` (30px), `930` (26px), `990` (22px), `900`/`1051`/`1200` (17px), `1046` (15px) | literales | La portada define su propia escala: 17px y 15px no existen en la escala (16/18, 14/16); 22px duplica `--font-size-xl`. Si la portada necesita tamaños de display, faltan tokens (`--font-size-2xl`, `--font-size-3xl`). | Media |
| F5 | `D/views/InvestigationView.svelte:1969`; `D/views/RagChatView.svelte:1458`; `D/views/WritingHistoryPanel.svelte:351`; `D/views/BatchSchemaPanel.svelte:311`, `322`; `D/views/WritingSharePanel.svelte:256`; `D/views/ItemNotesPanel.svelte:405`; `U/NoteEditor/NoteEditor.svelte:872` | `0.9em` / `0.95em` | Tamaños relativos ad hoc (código en línea, mensajes de error). | Baja |
| F6 | `D/views/InvestigationView.svelte:1974`, `2010`, `2046`, `2051`, `2087`, `2199`, `2249`; `D/views/WritingZoteroDetails.svelte:376`, `402` | `font-weight: 500/600` | Pesos literales; existen `--font-weight-medium/semibold`. | Baja |
| F7 | 48 `line-height` literales (top: `D/views/SettingsView.svelte` ×6, `D/views/ItemLayoutPanel.svelte` ×4, `D/views/SyncSettingsCard.svelte` ×4, `D/views/BibliographyWorkView.svelte` ×3) | 1.5 ×11, 1.6 ×10, 1.4 ×8, 1 ×5, 1.25 ×4, 1.3, 1.2, 1.35, 1.55, 0 | Solo hay dos tokens (`tight` 1.22, `base` 1.46); el texto de lectura (1.55–1.6) no tiene token y cada vista elige el suyo. | Media |
| F8 | 67 `letter-spacing` con 12 valores (`0.075em` ×27, `0.08em` ×11, `0.04em` ×10, `0.02em` ×7, `0.12em`, `0.15em`, `-0.01em`…) | literales | Las etiquetas en versalitas usan `0.075em` (como `.page-header__eyebrow`) o `0.08em` indistintamente. | Baja |
| F9 | `packages/ui/src/tokens/tokens.css:79` | `--font-weight-bold: 600` | Alias igual a `semibold`: “bold” no es bold. Confunde al elegir peso (14 usos). | Baja |

> Sin hallazgos: `font-family` (0 literales; todo pasa por `--font-ui`, `--font-reading`, `--font-mono`).

### 2.6 Espaciado y tamaños

| # | Dónde | Problema | Sev. |
| --- | --- | --- | --- |
| S1 | 117 `padding/margin/gap` con literales: `2px` ×60, `1px` ×15, `4px` ×9, `3px` ×8, `6px` ×6, `5px` ×5, `1.5rem` ×4… Concentración: `D/views/ItemLayoutPanel.svelte` (9), `D/layout/DocumentExplorer.svelte` (9; p. ej. `padding: 36px 5px 10px 6px` en `:1082`), `U/EntityViewer/EntityViewer.svelte` (7), `D/views/ItemSearchPanel.svelte` (6), `U/MapViewer/MapViewer.svelte` (6) | `4px` = `--space-1`, `8px` = `--space-2`, `12px` = `--space-3`; `2px`/`6px` no tienen token (falta medio paso `--space-0-5` = 2px). `5px`, `7px`, `10px`, `1.5rem` están fuera de la escala. | Media |
| S2 | Alturas de control fuera de `--control-height-*` (30/36/40): `28px` ×15 (p. ej. `D/views/WritingCitationEditor.svelte:388`, `:413`, `U/WritingEditor/WritingEditor.svelte:1557`, `D/components/settings/EntityAliasesTab.svelte:434`), `26px` (`D/layout/AppShell.svelte:1022`), `24px` (`D/views/ItemAnalysisPanel.svelte:832`), `22px` (`D/layout/NotificationBell.svelte:231`), `20px` (`U/EntityViewer/EntityViewer.svelte:834`, `D/views/AssistantsTab.svelte:746`), `36px` literal (`D/views/SimilarAssetPreviewDialog.svelte:315`) | Controles densos sin escalón definido: falta `--control-height-xs` (24 o 28). | Media |
| S3 | `--space-7` (28px) | Token sin un solo uso; en cambio 28px aparece 18 veces como altura literal. | Baja |

### 2.7 Bordes y radios

| # | Dónde | Problema | Sev. |
| --- | --- | --- | --- |
| B1 | Dos escalas paralelas con los mismos valores: `--radius-sm` (6px, 70 usos) vs `--radius-control` (6px, 78); `--radius-md` (8px, 43) vs `--radius-input` (8px, 38) | Un mismo control usa una u otra según el archivo (p. ej. disparadores de select con `radius-sm`, `radius-control` o `radius-input` en R3). | Media |
| B2 | `border-radius: 2px` en `D/views/ItemSearchPanel.svelte:315`, `D/layout/TopBar.svelte:830`, `U/WritingEditor/WritingEditor.svelte:1842`; `4px` en `D/layout/DocumentExplorer.svelte:1322`; `999px` en `D/layout/NotificationBell.svelte:271` | Literales: `4px` = `--radius-xs`, `999px` ≈ `--radius-full`; `2px` no tiene token. | Baja |
| B3 | `border-radius: 50%` en `D/components/OcrRichText.svelte:145`, `D/views/AssistantsTab.svelte:776`, `D/views/SettingsView.svelte:3581`, `D/layout/TopBar.svelte:857`, `U/EntityViewer/EntityViewer.svelte:785`, `U/PdfMarkup/PdfMarkupLayer.svelte:670` | Aceptable para círculos; conviene `--radius-full` por uniformidad. | Baja |
| B4 | Bordes de campos: `--color-hairline` vs `--border-subtle` vs `--color-border` vs `color-mix(hairline 78%)` (familias de R1) | Mismo control, cuatro intensidades de borde. | Media |

### 2.8 Sombras

| # | Dónde | Problema | Sev. |
| --- | --- | --- | --- |
| H1 | Brillos `inset` a mano (C11) | Ya existe `--shadow-surface`. | Media |
| H2 | `D/layout/NotificationBell.svelte:292` `var(--shadow-lg, 0 12px 32px …)`, `U/MapViewer/MapViewer.svelte:443` `var(--shadow-md, 0 8px 24px …)` | Fallbacks con valores distintos a los tokens (`--shadow-lg` = `0 18px 42px`). | Baja |
| H3 | `U/PdfMarkup/PdfMarkupLayer.svelte:595`–`602` (`drop-shadow`, strokes negros) | Overlay de visor: aceptable, pero no se anula en temas sin sombra (`light`, `lite`, `paper` declaran `--shadow-*: none`). | Baja |
| H4 | `--shadow-glow` | Token sin uso. | Baja |

### 2.9 Estados (hover / focus / disabled)

| # | Dónde | Problema | Sev. |
| --- | --- | --- | --- |
| E1 | Opacidad de `:disabled`: `0.48` (`Button`, `IconButton`, `TabButton`, `ConfirmDialog`, 4 vistas), `0.4` (`D/components/settings/EntityAliasesTab.svelte:500`, `D/views/AppearanceTab.svelte:357`, `D/views/ItemAnalysisPanel.svelte:863`, `D/views/BatchProcessingTab.svelte:2158`, `U/EntityViewer/EntityViewer.svelte:856`, `U/ToolbarMenu/ToolbarMenuList.svelte:93`), `0.56` (`Input`, `NumberField`, 3 diálogos), `0.5`, `0.55` (`Checkbox`, `MapViewer`, `DbBrowserView`, `SyncSettingsCard`), `0.6`, `0.72` (primario), `1` | Ocho valores para “deshabilitado”, incluso entre componentes base (`Button` 0.48, `Input` 0.56, `Checkbox` 0.55, `ToolbarMenuList` 0.4). Falta `--opacity-disabled`. | Media |
| E2 | Foco con `outline: 2px solid var(--color-accent)` en `U/ItemCard/ItemCard.svelte:109`, `:276`; `U/EntityViewer/EntityViewer.svelte:693`, `:891`; `U/PdfMarkup/PdfMarkupLayer.svelte` | El resto usa `box-shadow: var(--focus-ring)` (2px, acento al 22–30%): dos lenguajes de foco. | Media |
| E3 | `D/views/HomeView.svelte:869`–`876` (`continuar-row`), `:1253` (`recent-row`) | Foco = mismo fondo que el hover (`accent-faint`) con `outline: none`: con teclado no se distingue hover de foco y el contraste del fondo es mínimo. | Media |
| E4 | `D/views/NavegadorSources.svelte:744` (solo cambia el borde), `D/layout/SplitDivider.svelte:146` (rellena de acento) | Foco sin `--focus-ring`; aceptable en el divisor, no en la fila. | Baja |
| E5 | `U/VirtualGrid/VirtualGrid.svelte:262` | `:focus { outline: none }` sobre un contenedor `tabindex="-1"`: correcto (foco programático), pero conviene documentarlo. | Baja |
| E6 | `D/views/RagChatView.svelte:1261` | El input de renombrar declara `--focus-ring` y borde de acento en su estado base, no en `:focus-visible`. | Baja |
| E7 | Hover: fondos `accent-faint` / `accent-soft` / `surface-elevated` / `surface-toolbar` según el archivo para filas de lista equivalentes (p. ej. `D/views/WritingHistoryPanel.svelte:338`, `D/views/HomeView.svelte:869`, opciones de R5) | Sin un token `--state-hover-*`. | Baja |
| E8 | Transiciones literales: `0.3s` (`D/views/DependenciasTab.svelte:1151`), `0.2s` (`D/views/SettingsView.svelte:4227`), `0.15s` (`U/EntityViewer/EntityViewer.svelte:666`, `683`; `U/TopicEditor/TopicEditor.svelte:185`; `U/ItemCard/ItemCard.svelte:171`; `U/MetadataEditor/MetadataEditor.svelte:235`), `0.1s` (`U/TopicEditor/TopicEditor.svelte:241`, `U/AudioPlayer/AudioPlayer.svelte:496`), `120ms` (`D/views/ItemAnalysisPanel.svelte:819`, `U/ResizeHandle/ResizeHandle.svelte:156`), `0.08s` (`U/DocumentViewer/DocumentViewer.svelte:2135`, `2211`) | Existen `--transition-base` (0.14s) y `--transition-smooth` (0.18s). Conviene revisar `prefers-reduced-motion` al migrarlas (hoy solo algunos componentes, como `Checkbox`, lo declaran). | Baja |

### 2.10 Capas (`z-index`)

18 valores sin escala: `0, 1, 2, 3, 4, 5, 20, 25, 30, 60, 100, 200, 210, 1000, 1100, 1200, 1300, 2000`. Los locales (0–5) son razonables; los globales no tienen tokens y se coordinan por comentarios (“the highest is 1200”, `D/App.svelte:184`). **Media.**

| Capa | Valor | Dónde |
| --- | --- | --- |
| Cabecera fija de página | 20 | `D/app.css:168` |
| Banda/cáscara | 25 | `D/layout/AppShell.svelte:1158` |
| Desplegable de modelos | 30 | `D/components/OpenrouterModelList.svelte:407` |
| Panel de notificaciones | 60 | `D/layout/NotificationBell.svelte:284` |
| Sugerencias de temas | 100 | `U/TopicEditor/TopicEditor.svelte:218` |
| Búsqueda global | 200 | `D/layout/TopBar.svelte:916` |
| Menús (`ToolbarMenu`) | 210 | `U/ToolbarMenu/ToolbarMenu.svelte:236` |
| Diálogos | 1000 / 1100 / 1200 | `ConfirmDialog`, `DbBrowserView`, `MapViewer:432`; Agent/Passage/SimilarAsset; `NoteEditor` |
| Tooltip | 1300 | `U/Tooltip/TooltipLayer.svelte:60` |
| Compactación | 2000 | `D/App.svelte:185` |

`EntityViewer` usa 20 (`U/EntityViewer/EntityViewer.svelte:738`) para un popover, el mismo valor que la cabecera fija de página (`D/app.css:168`): si llegan a superponerse, decide el orden del DOM. Con una escala de capas esto no dependería del azar.

### 2.11 Scrollbars

Globales y bien resueltas en `D/app.css:7`–`94` (7px, radio completo, colores por `color-mix`). Dos overrides locales divergen: `D/views/ItemAssetPanel.svelte:884`–`912` y `D/layout/DocumentExplorer.svelte:1084`–`1100` (8px, `radius-xs`, `text-muted 52–58%`, código duplicado entre ambos). **Baja.** Ver también C14.

### 2.12 Estilos inline

15 atributos `style="…"` y 96 directivas `style:`; casi todos son geometría dinámica legítima (anchos de progreso, columnas de grilla, posición de tooltip, alturas de `VirtualGrid`). Dos casos a revisar:

| # | Archivo:línea | Problema | Sev. |
| --- | --- | --- | --- |
| I1 | `D/views/DependenciasTab.svelte:832` | `style="color: {dependencyStatusColor(...)}"`: el color de estado se decide en TS (`D/lib/deps-status.ts:35`) y se inyecta inline. Usa tokens, pero debería ser una clase modificadora (`deps-row__icon--installed`…). | Baja |
| I2 | `D/views/ItemView.svelte:3537`, `D/views/CollectionView.svelte:1110` | `grid-template-columns` con `6px` literal del divisor, duplicado en dos vistas. | Baja |

### 2.13 Componentes del sistema reestilizados con `:global`

| # | Archivo:línea | Override | Sev. |
| --- | --- | --- | --- |
| G1 | `D/views/SettingsView.svelte:4074`–`4092` (y `3906`, `3943`, `4012`; 14 en total) | Reescribe `.input-field__input` (fondo `surface-glass 88%`) y `.btn--secondary` (degradado `rgba(255,255,255,.04)` + fondo C) para toda la vista de ajustes: los botones e inputs de Ajustes no se ven como en el resto de la app. | Alta |
| G2 | `D/views/SyncSettingsCard.svelte:1019` | Padding de `.input-field__input` para alojar el botón de contraseña: síntoma de que `Input` no tiene slot de acción final. | Media |
| G3 | `D/views/CollectionView.svelte:1500`, `:1685`; `D/views/ItemView.svelte:4142`; `D/views/CollectionsView.svelte:562`; `D/views/ResearchView.svelte:1030`, `:1120` | Ajustes de ancho/alineación de `.btn` e `.icon-button` (la mayoría de layout, aceptables) y el `right-panel-toggle` duplicado (R11). | Baja |

### 2.14 Breakpoints

8 `@media (max-width)` distintos (420, 520, 640, 720, 760, 820, 860, 900px) y 12 `@container` con otros tantos umbrales. Sin tokens ni convención. **Baja.**

---

## 3. Plan de homogeneización por fases

Principios: cada fase es mergeable sola y deja los tests verdes; primero lo que el usuario ve mal, después la duplicación, al final la deuda; toda migración de control se acompaña de una guarda (lint o test) para que no vuelva. Ninguna fase cambia el contenido guardado/sincronizado (los tests dorados de serialización no se tocan).

### Fase 0 — Guardas (antes de tocar estilos) · prioridad alta · esfuerzo S

Objetivo: que la deuda deje de crecer mientras se paga.

- Extender `U/__tests__/visual-contract.test.ts` (`names only published tokens…`) para que **también** valide los `var()` *con* fallback (detecta T1–T4) y prohíba fallbacks en tokens publicados (T5, T6, C4, H2).
- Ampliar `keeps primary controls off the accent fill` a `apps/desktop/src/**` y a selectores que no contengan `btn|button` (detecta R4a, C13).
- Nuevo test de contrato: prohibido `color-mix(…, black|white …)` y `rgb[a]()`/hex literales en `<style>` salvo una lista blanca (overlays de visor, `PdfMarkup`, `EntropicConstellation`).
- Añadir a `svelte/no-restricted-html-elements` (`eslint.config.js:80`) `progress` y `textarea` (con mensaje que apunte al componente nuevo), activándolo por archivo a medida que se migra (allowlist temporal).

Archivos: `packages/ui/src/components/__tests__/visual-contract.test.ts`, `eslint.config.js`.

### Fase 1 — Colores y tokens rotos · prioridad alta · esfuerzo S/M

Objetivo: que todos los temas y contrastes se vean bien hoy, sin refactor de componentes.

- Reemplazar la paleta Tailwind por tokens de estado (`--color-warning/-soft`, `--color-danger/-soft`, `--color-success/-soft`, `--color-accent/-soft`, `color-mix(in srgb, var(--color-*) N%, transparent)` para bordes) — C1–C5, C7.
- Reemplazar mezclas con `black`/`white` por tokens de superficie (`--color-surface-sunken`, `--color-surface-elevated`) o `color-mix` contra `--color-bg` — C9, C10.
- Corregir tokens inexistentes: `--color-text-danger` → `--color-danger` (C12/T1); `--color-text-tertiary` → `--color-text-muted` (T3); `--color-surface-hover` → `--color-accent-faint` o un token nuevo de hover (T4); `--color-text-on-accent` → resolver con R4a.
- Nuevos tokens mínimos en `tokens.css` (los 7 temas): `--color-on-danger` (C6), y override de scrollbars para `lite`/`paper` en `app.css` (C14).
- Sustituir los brillos `inset` manuales por `--shadow-surface` (C11/H1).
- Eliminar fallbacks (T5, T6, C4, H2).

Archivos: `D/views/DependenciasTab.svelte`, `D/layout/AppShell.svelte`, `D/layout/NotificationBell.svelte`, `D/layout/DocumentExplorer.svelte`, `U/NoteEditor/NoteEditor.svelte`, `D/views/BibliographySearchTab.svelte`, `D/views/ItemSearchPanel.svelte`, `D/views/WritingHistoryPanel.svelte`, `D/views/InternetArchiveDialog.svelte` (solo color de `.ia-submit`), `D/views/ItemAssetPanel.svelte`, `D/views/ItemTextPanel.svelte`, `D/views/ItemAnalysisPanel.svelte`, `D/views/CollectionsView.svelte`, `D/views/SettingsView.svelte` (brillos), `U/Button/Button.svelte`, `U/Panel/Panel.svelte`, `D/views/AssistantsTab.svelte`, `D/views/InvestigationView.svelte`, `D/views/BatchSchemaPanel.svelte`, `D/views/WritingSharePanel.svelte`, `D/views/BatchFlowPicker.svelte`, `U/MapViewer/MapViewer.svelte`, `U/DocumentViewer/DocumentViewer.svelte`, `packages/ui/src/tokens/tokens.css`, `apps/desktop/src/app.css`.

Criterio de hecho: Fase 0 en verde sin allowlist para estos archivos; `contrast-floor` y `theme-completeness` verdes; revisión visual de Dependencias, banner de dependencias, notas y explorador en `dark`, `paper` y `vivid`.

### Fase 2 — Ampliar el sistema de diseño · prioridad alta · esfuerzo M/L

Objetivo: que exista un componente base para cada control que hoy se reimplementa. Todos en `packages/ui`, con tipos, tests y exportados desde `index.ts`.

| Componente | Reemplaza | Notas |
| --- | --- | --- |
| `Input` ampliado | R1, G2 | Reenviar `...rest` (`HTMLInputAttributes`), `class`, `bind:this`; tamaños `sm`/`md`; slots `leading`/`trailing` (ícono de búsqueda, botón de contraseña). Compatibilidad hacia atrás con la API actual. |
| `Textarea` | R2 | Misma familia visual que `Input` (familia A), `rows`/autoaltura, `label/hint/error`. |
| `SelectTrigger` (o `Select` = trigger + `ToolbarMenu` radio) | R3 | Formaliza el “patrón ToolbarMenu radio” que el lint ya recomienda; tamaños `sm`/`md`, variante `quiet` (transparente) y `field`. |
| `RadioGroup` + `RadioCard` | N1–N5, R5 | Radio visualmente oculto + tarjeta con `:has(:focus-visible)` (patrón de `FontPresetGrid`); variante lista compacta; opción “nueva colección con input”. |
| `ProgressBar` | N6–N9, R7 | `div` con `role="progressbar"`, tamaños 4/6px, `--color-accent`, transición tokenizada y `prefers-reduced-motion`. |
| `Badge` / `Chip` | R8 | Unificar con `StatusBadge` (o extenderlo con `size="xs"` y variante `count`), chip removible. |
| `Switch` | R9 | Extraído de `AssistantsTab`. |
| `Disclosure` | N11 | `details/summary` con chevron `ActionIcon`, foco y tipografía únicos. |
| `Dialog` (cáscara) | R10 | Overlay, superficie, foco atrapado, capa; `ConfirmDialog` pasa a componerlo. |
| `LinkButton` (variante `link` de `Button`) | R4f | Botón con aspecto de enlace. |
| Escalón `xs` | S2, R4d | `--control-height-xs` (24px) y `Button size="xs"`/`IconButton size="xs"` ya existente (24). |

Tokens nuevos en la misma fase: `--opacity-disabled` (E1), `--line-height-reading` (F7), `--letter-spacing-caps` (F8), `--space-0-5` (2px, S1), `--z-*` (§2.10), `--duration-fast/base/slow` (E8), y decidir `--font-size-2xl/3xl` para la portada (F4).

Archivos: `packages/ui/src/components/{Input,Textarea,Select,Radio,ProgressBar,Badge,Switch,Disclosure,Dialog,Button}/**`, `packages/ui/src/index.ts`, `packages/ui/src/tokens/tokens.css`, `apps/desktop/src/app.css`, `apps/desktop/vite.config.ts` (solo si algún componente trae una dependencia nueva).

### Fase 3 — Migrar controles nativos visibles · prioridad alta · esfuerzo M

Objetivo: cero controles con aspecto del navegador.

- Radios nativos → `RadioGroup`/`RadioCard`: `D/views/NavegadorCopyDialog.svelte`, `D/views/ImportSourcesDialog.svelte`, `D/views/NavegadorZoteroDialog.svelte`, `D/views/InternetArchiveDialog.svelte`, `D/views/SettingsView.svelte` (16 radios), `D/layout/FontPresetGrid.svelte` y `D/views/WritingExportTab.svelte` (ya correctos: pasar al componente para eliminar duplicación).
- `<progress>` y barras `div` → `ProgressBar`: `D/views/ImportSourcesDialog.svelte`, `D/views/InternetArchiveDialog.svelte`, `D/views/ItemTextPanel.svelte`, `D/views/CollectionView.svelte`, `D/views/DependenciasTab.svelte`, `D/views/SettingsView.svelte`.
- `<details>` → `Disclosure`: `D/views/ItemTextPanel.svelte`, `D/views/ResearchView.svelte`, `D/views/ItemSearchPanel.svelte`, `D/views/InvestigationView.svelte`.
- Slider de volumen: estilizar `::-webkit-slider-*` con tokens dentro de `U/AudioPlayer/AudioPlayer.svelte` (o extraer `Slider` si aparece un segundo).
- Activar en lint la prohibición de `progress` sin allowlist.

### Fase 4 — Migrar reimplementaciones · prioridad media · esfuerzo L

Por áreas, de mayor a menor superficie visible:

1. **Ajustes** (G1, R4c, R3, R2): `D/views/SettingsView.svelte`, `D/views/SyncSettingsCard.svelte`, `D/views/AppearanceTab.svelte`, `D/views/BackupTab.svelte`, `D/views/AssistantsTab.svelte` (→ `Switch`), `D/views/DependenciasTab.svelte`, `D/components/settings/EntityAliasesTab.svelte`, `D/components/settings/AgentParametersTab.svelte`, `D/components/OpenrouterModelList.svelte`. Quitar todos los `:global(.btn…)`/`:global(.input-field…)` de `SettingsView`.
2. **Diálogos** (R10, R4a/b, R6): `D/views/InternetArchiveDialog.svelte`, `D/views/DbBrowserView.svelte`, `D/components/AgentApprovalDialog.svelte`, `D/components/PassageOriginalViewer.svelte`, `D/views/SimilarAssetPreviewDialog.svelte`, `U/NoteEditor/NoteEditor.svelte` (modal de enlace), `D/views/DependenciasTab.svelte` (confirmación de reset).
3. **Escritura y lotes** (R1, R3): `D/views/WritingCitationEditor.svelte`, `D/views/WritingZoteroTab.svelte`, `D/views/WritingView.svelte`, `U/WritingEditor/WritingEditor.svelte` (input de enlace), `D/views/BatchProcessingTab.svelte`, `D/views/BatchFlowPicker.svelte`, `D/views/BatchSchemaPanel.svelte` (unificar `menu-select`).
4. **Investigación y chat** (R2): `D/views/InvestigationView.svelte`, `D/views/ResearchView.svelte`, `D/views/RagChatView.svelte`, `D/views/CollectionAnalysisPanel.svelte`, `D/views/GraphSettingsForm.svelte`.
5. **Ítem y biblioteca** (R1, R4d, R4h, R8): `D/views/ItemAnalysisPanel.svelte`, `D/views/ItemAssetPanel.svelte`, `D/views/ItemLayoutPanel.svelte`, `D/views/OccurrenceStepper.svelte`, `D/components/BibliographyAnnotationsPanel.svelte`, `D/views/BibliotecaView.svelte`, `D/views/BibliographyWorkView.svelte`, `D/views/WritingZoteroDetails.svelte`, `D/views/NavegadorView.svelte`, `D/views/NavegadorSources.svelte`.
6. **Cáscara** (R1, R4e/f, R8, R11): `D/layout/AppShell.svelte` (filtro lateral → `Input size="sm"` con ícono), `D/layout/TopBar.svelte`, `D/layout/NotificationBell.svelte`, `D/views/ItemView.svelte` + `D/views/CollectionView.svelte` (extraer `right-panel-toggle` a una variante de `IconButton`).
7. **Paquete UI** (R4g, R8, C13): `U/AudioPlayer/AudioPlayer.svelte`, `U/MetadataEditor/MetadataEditor.svelte`, `U/MapViewer/MapViewer.svelte`, `U/EntityViewer/EntityViewer.svelte`, `U/TopicEditor/TopicEditor.svelte`, `U/ItemCard/ItemCard.svelte`, `U/CollectionCard/CollectionCard.svelte`.

Criterio de hecho por área: sin `<input>`/`<textarea>`/`<button>` crudos salvo los listados como excepción en la guía (filas de lista, nodos de árbol, tarjetas clicables, celdas editables inline), sin overrides `:global` de apariencia, tests de la vista verdes.

### Fase 5 — Escalas: tipografía, espaciado, radios, estados · prioridad media · esfuerzo M

- Tipografía: eliminar 9px/10px (F1, F2) a `--font-size-2xs`; portada a la escala decidida en Fase 2 (F4); pesos y `line-height` a tokens (F6, F7); `letter-spacing` de versalitas a `--letter-spacing-caps` (F8); renombrar o retirar `--font-weight-bold` (F9) con búsqueda y reemplazo de sus 14 usos.
- Espaciado: literales `4/8/12/16px` → `--space-*`; `2px` → `--space-0-5`; revisar `5/7/10px` y `rem` (S1). Concentrar en `ItemLayoutPanel`, `DocumentExplorer`, `EntityViewer`, `ItemSearchPanel`, `MapViewer`, `AppShell`, `NoteEditor`.
- Radios: decidir **una** escala para controles (recomendado: semántica `--radius-control/-input/-surface/-dialog` para componentes; primitiva `xs…xl` solo para decoraciones) y migrar B1–B3.
- Estados: `--opacity-disabled` en todos los componentes base y vistas (E1); foco único con `--focus-ring` (E2–E4, E6); hover de filas con un token (E7).

### Fase 6 — Capas, movimiento y breakpoints · prioridad baja · esfuerzo S

- `z-index` globales → `--z-sticky`, `--z-dropdown`, `--z-popover`, `--z-dialog`, `--z-dialog-nested`, `--z-tooltip`, `--z-blocking` (§2.10); corregir el popover de `EntityViewer`.
- Transiciones → `--transition-*`/`--duration-*` y `prefers-reduced-motion` en todas (E8).
- Scrollbars locales → variables globales o una clase utilitaria `.scroll-area--inset` en `app.css` (§2.11).
- Breakpoints: documentar 3–4 umbrales de contenedor (`pane`) y migrar los `@media` que en realidad dependen del ancho del panel (§2.14).
- Estilo inline: clases modificadoras para el color de estado de dependencias (I1); extraer el ancho del divisor a una variable (I2).

### Fase 7 — Cierre · prioridad baja · esfuerzo S

- Retirar las allowlists temporales de Fase 0; hacer obligatorios los tests de contrato para `apps/desktop`.
- Eliminar tokens sin uso o darles uso (`--radius-xl`, `--shadow-glow`, `--space-7`, `--color-primary-subtle`).
- Actualizar `GUIA_DISENO.md` con los componentes y tokens nuevos (mover lo “propuesto” a “vigente”).

### Resumen de priorización

| Fase | Prioridad | Hallazgos que cierra | Riesgo |
| --- | --- | --- | --- |
| 0 Guardas | Alta | (previene) | Bajo |
| 1 Colores/tokens | Alta | C1–C14, T1–T6, H1–H2 | Bajo (solo CSS) |
| 2 Sistema de diseño | Alta | base para R1–R10, S2, E1, F7, F8 | Medio (API de `Input`) |
| 3 Controles nativos | Alta | N1–N11 | Medio (tests de vistas con radios/progress) |
| 4 Reimplementaciones | Media | R1–R11, G1–G3 | Medio/alto (volumen) |
| 5 Escalas | Media | F1–F9, S1–S3, B1–B4, E1–E7 | Bajo |
| 6 Capas/movimiento | Baja | §2.10, E8, §2.11, §2.12, §2.14 | Bajo |
| 7 Cierre | Baja | H4, S3, tokens sin uso | Bajo |
