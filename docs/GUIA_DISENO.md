# Guía de diseño — EntropIA

> Reglas de estilo para todo el frontend (`apps/desktop/src`, `packages/ui/src`).
> Fuente de verdad de los valores: `packages/ui/src/tokens/tokens.css` y `apps/desktop/src/app.css`.
> Diagnóstico y plan que la motivan: [`AUDITORIA_ESTILOS.md`](./AUDITORIA_ESTILOS.md).

## Principios

1. **Una sola fuente.** Ningún valor visual se escribe dos veces: si un color, un tamaño o un radio no está en los tokens, se agrega al token (en los siete temas) antes de usarlo.
2. **Todos los temas, todos los contrastes.** Cada pantalla se tiene que ver bien en `dark`, `dim`, `light`, `lite`, `paper`, `forest` y `vivid`, en contraste `soft`/normal/`high` y con los cuatro presets tipográficos. Un valor que solo funciona en el tema oscuro es un error.
3. **La app dibuja sus controles.** Nada que el usuario toque puede verse como el control por defecto del navegador o del sistema operativo.
4. **Jerarquía por contraste, no por saturación.** El primario se distingue por luminosidad, borde y brillo del texto (`--control-primary-*`); la selección se marca con superficie y borde, no rellenando de acento. El acento queda para foco, enlaces, barras de progreso y detalles finos.
5. **Componer antes que copiar.** Si un control aparece en dos vistas, va a `packages/ui`.

---

## Se debe usar

### Tokens

| Categoría | Tokens | Regla |
| --- | --- | --- |
| Superficies | `--surface-app`, `--surface-panel`, `--surface-toolbar`, `--surface-card`, `--surface-input`, `--surface-overlay`, `--surface-glass`; primitivas `--color-bg`, `--color-surface`, `--color-surface-raised`, `--color-surface-elevated`, `--color-surface-sunken`, `--color-surface-glass` | Preferí el alias semántico (`--surface-input` para un campo) sobre la primitiva. |
| Bordes | `--color-hairline` (separaciones finas), `--border-subtle`, `--border-panel`, `--color-border-strong`, `--color-border-hover`, `--border-focus` | Un borde translúcido se arma con `color-mix(in srgb, var(--color-*) N%, transparent)`. |
| Texto | `--color-text-primary`, `--color-text-secondary`, `--color-text-muted` | Son los **tres únicos** niveles: no existe `--color-text-tertiary`. Todos cumplen 4.5:1 en cada tema y contraste. |
| Acento | `--color-accent`, `--color-accent-hover`, `--color-accent-soft`, `--color-accent-faint`; `--color-on-accent` (marca sobre un relleno de acento) | `soft` para seleccionado/hover marcado, `faint` para hover suave. |
| Hover de fila | `--state-hover-row` | El hover de una fila de lista (historial, recientes, resultados, filas de tabla). Las opciones de un menú usan el de `ToolbarMenuList` (`--surface-toolbar`). |
| Estados | `--color-danger`/`-hover`/`-soft`, `--color-success`/`-soft`, `--color-warning`/`-soft`, `--color-info`/`-soft`; `--state-ai*`, `--state-evidence*` | Texto e ícono con el color pleno, fondo con `-soft`, borde con `color-mix(… 28–34%, transparent)` del mismo color. |
| Texto sobre relleno de estado | `--color-on-danger` | Nunca `#fff` fijo: la tinta sigue al tema y cumple 4.5:1 sobre `--color-danger` y `-hover`. |
| Primario | `--control-primary-bg`, `-bg-hover`, `-bg-active`, `-border`, `-text` | Solo a través de `Button variant="primary"`. |
| Espaciado | `--space-0-5` (2), `--space-1` (4) … `--space-8` (32) | Todo `padding`, `margin` y `gap`; `--space-0-5` solo para ajustes ópticos. |
| Tipografía | `--font-ui`, `--font-reading`, `--font-mono`; `--font-size-2xs` (11) … `--font-size-xl` (22); `--font-weight-normal/medium/semibold`; `--line-height-none` (1), `--line-height-tight`, `--line-height-base`; `--font-size-code-inline` (0.9em) | La familia se elige por **función** (operar la app / leer / identificadores), nunca por nombre. 11px es el mínimo. **`semibold` es el peso más alto**: `--font-weight-bold` valía lo mismo (600) y se eliminó (F9). `--line-height-none` es para glifos de ícono y la capa de texto del PDF; `--font-size-code-inline` para código en línea dentro de un texto. |
| Lectura | `--line-height-reading` | Para manuscrito, notas, OCR, respuestas largas. |
| Espaciado de letras | `--letter-spacing-caps` (versalitas), `--letter-spacing-heading` (títulos), `--letter-spacing-tag` (etiquetas y contadores chicos en negrita) | Toda etiqueta en mayúsculas lleva `--letter-spacing-caps`. |
| Radios | `--radius-control` (botones, triggers), `--radius-input` (campos), `--radius-surface` (tarjetas, paneles), `--radius-dialog` (diálogos), `--radius-full` (píldoras, círculos); `--radius-xs` solo para decoraciones pequeñas | Para controles usá la escala **semántica**; `--radius-sm/md/lg` quedan para casos que no son un control. |
| Sombras | `--shadow-surface` (brillo superior de una superficie), `--shadow-sm/md/lg` | Los temas claros las anulan: no las repitas a mano. |
| Alturas de control | `--control-height-xs` (28), `-sm` (30), `-md` (36), `-lg` (40) | Toda altura de botón, campo o trigger; `xs` es la fila densa (pestañas, ítems de menú, contadores). Un cuadrado de solo ícono usa la escala de `IconButton` (24/28/32/40). |
| Foco | `--focus-ring`, `--focus-ring-danger` | `box-shadow: var(--focus-ring)` en `:focus-visible`. |
| Movimiento | `--transition-base` (0.14s), `--transition-smooth` (0.18s); `--duration-fast` (0.08s), `-base` (0.14s), `-smooth` (0.18s), `-slow` (0.3s); bucles `--duration-spin` (0.8s, una vuelta de spinner) y `--duration-pulse` (1.6s, un latido de estado) | Toda transición y animación; respetar `prefers-reduced-motion`. |
| Deshabilitado | `--opacity-disabled` (0.48); `--opacity-disabled-primary` (0.72) solo para el primario | Un único valor en toda la app. |
| Capas | `--z-sticky`, `--z-drawer`, `--z-popover`, `--z-popover-raised`, `--z-floating`, `--z-dropdown`, `--z-menu`, `--z-assistant`, `--z-dialog`, `--z-dialog-nested`, `--z-dialog-top`, `--z-tooltip`, `--z-blocking` | `z-index` local (0–5) libre dentro de un componente con su propio contexto de apilamiento. |
| Marcas sobre la página | `--annotation-ink`, `--annotation-ink-soft`, `--annotation-shadow`, `--viewer-layout-block` | Anotaciones del PDF y regiones de OCR: se dibujan sobre la página o el escaneo, no sobre el tema, así que su tinta es la misma en todos; `--annotation-shadow` se anula donde el tema anula las sombras. |
| Paleta del manuscrito | `--writing-text-*`, `--writing-highlight-*`, `--writing-blend-highlight` | Solo en `WritingEditor`/`ColorPalette`. |

### Componentes base (`@entropia/ui`)

| Necesidad | Usá | Notas |
| --- | --- | --- |
| Acción con texto | `Button` (`primary`, `secondary`, `outline`, `ghost`, `subtle`, `danger`, `link`; `sm`/`md`/`lg`; `loading`; `wrap` para una tarjeta de acción cuyo texto puede ocupar varias líneas) | Un solo `primary` por zona. `title` se convierte en tooltip del sistema. |
| Acción con ícono | `IconButton` (`xs` 24 · `sm` 28 · `md` 32 · `lg` 40; `label` obligatorio; variante `edge` para la tira que abre y cierra un panel; `hoverTone` `danger`/`success`) | El contenedor sale del tamaño del ícono, nunca al revés. `hoverTone` avisa en el hover qué hace la acción (cerrar o borrar, confirmar). |
| Ícono | `ActionIcon` | Única puerta a los íconos; para uno nuevo, agregalo a `ACTION_ICON_NAMES`. |
| Campo de texto | `Input` (`label`, `hint`, `error`; `sm`/`md`; `font` `ui`/`mono`; `leading`/`trailing` para ícono o acción) | Acepta los atributos de `<input>`. |
| Texto largo | `Textarea` (`rows`, `autosize`, `maxRows`; `font` `ui`/`reading`/`mono`) | Nunca `<textarea>` crudo (lint). |
| Búsqueda | `SearchBar` (`sm`/`md`; props de combobox para una búsqueda con resultados) + `SearchClearButton` | Siempre `SearchBar`, nunca un `type="search"` crudo. |
| Número | `NumberField` (+ clase global `.number-field-label` en la etiqueta) | Nunca `<input type="number">` suelto. |
| Casilla | `Checkbox` (`sm`/`md`; `row` o `card`) | Selección por contraste, foco en la fila. |
| Elección de una opción en un desplegable | `Select` (opciones + `value`) o `SelectTrigger` con un `ToolbarMenu` propio (`field`/`quiet`; `sm`/`md`; `block`) | Nunca `<select>` nativo (lint). |
| Elección visible entre pocas opciones | `RadioGroup` + `RadioCard` (`card`/`list`) | El radio nativo queda oculto y el foco se ve en la tarjeta. |
| Pestañas | `TabList` + `TabButton` | |
| Menú de acciones | `ToolbarMenu` / `ToolbarMenuList` | |
| Estado o etiqueta | `StatusBadge` (`neutral`, `stored`, `success`, `warning`, `danger`, `info`, `ai`, `evidence`; `xs`/`sm`/`md`); `variant="count"` con `tone`, `count` y `max` para contadores | Los chips de temas y etiquetas también son `StatusBadge`. |
| Superficie | `Card` (`padding`, `hoverable`, `header`, `footer`), `Panel` (`default`, `raised`, `glass`, `sunken`) | |
| Confirmación / diálogo | `ConfirmDialog` (`size`, `variant`) para confirmar; `Dialog` (`sm`/`md`/`lg`/`viewer`; `eyebrow`, `headerActions`, `footer`, `nested`) para contenido libre | Nunca un modal armado a mano. |
| Tooltip | `use:tooltip` / `TooltipLayer` | Nunca el atributo `title` nativo en elementos propios. |
| Capa flotante | `portal` (`OVERLAY_ROOT_ATTRIBUTE`) | |
| Divisor redimensionable | `ResizeHandle` / `SplitDivider` | |
| Progreso | `ProgressBar` (`tone`, `sm`/`md`, `indeterminate`) | Nunca `<progress>` nativo (lint). |
| Interruptor | `Switch` (`sm`/`md`, `description`) | |
| Desplegable de contenido | `Disclosure` (`plain`/`card`, `sm`/`md`) | Nunca `<details>` con el marcador del navegador. |

### Patrones globales permitidos (`app.css`)

- **Página:** `.page-shell`, `.page-header` (+ `__content`, `__eyebrow`, `__meta`, `__pipeline`), `.page-toolbar` (`--split`).
- **Mensajes:** `.surface-message` (`--center`, `--error`, `--success`).
- **Números:** `.number-field-label`.
- **Scrollbars:** las globales (`--scrollbar-*`); un área que necesite otra cosa redefine esas variables, no los pseudo-elementos.

### Prácticas obligatorias

- **Estilos en `<style>` del componente**, con clases BEM con prefijo del componente (`.history-panel__item--selected`).
- **Estados completos** en todo elemento interactivo propio: `:hover:not(:disabled)`, `:focus-visible` con `--focus-ring`, `:disabled` con la opacidad común y `cursor: not-allowed`, `:active` si aplica.
- **`outline: none` solo junto con su reemplazo** (`box-shadow: var(--focus-ring)` en el mismo selector o en el contenedor que lo envuelve con `:has(:focus-visible)`).
- **Colores derivados con `color-mix` contra `transparent` o contra otro token**: `color-mix(in srgb, var(--color-danger) 30%, transparent)`.
- **Estilos dinámicos solo para geometría**: `style:width={`${pct}%`}`, columnas de grilla, posiciones calculadas. El color o el estado se expresan con una clase modificadora.
- **Contenedores antes que viewport**: un panel que cambia de layout según su ancho usa `@container pane (…)`.
- **Movimiento reducido**: toda animación o transición no trivial tiene su bloque `@media (prefers-reduced-motion: reduce)`.
- **Contraste medido**: un token de texto o estado nuevo se agrega en los siete temas y en los bloques de contraste, y pasa `contrast-floor.test.ts`.
- **Renderers de Markdown**: salida de LLM por `lib/markdown.ts`; OCR por `markdown-it` + `sanitizeOcrHtml` (ver `AGENTS.md`).

### Excepciones aceptadas (elementos nativos con estilo propio)

Se permite `<button>` crudo, con estilos completos y tokens, cuando **no es un botón visual** sino una superficie clicable: filas de lista y de tabla, nodos del árbol del explorador, tarjetas clicables, celdas editables inline, swatches de color, pestañas de `TabStrip`. Se permite `<input>` crudo sin borde para la edición inline dentro de otro control (`TopicEditor`, `EntityViewer`, título del documento). Se permiten colores literales en datos que no dependen del tema: colores de impresión/exportación (`PRINT_COLORS`, `ocr-pdf.ts`) y la paleta de anotación compatible con Zotero (`PdfMarkup/types.ts`).

Excepciones permanentes, con su motivo:

- **Foco dentro de un SVG** (`PdfMarkupLayer`): una forma SVG no admite `box-shadow`, así que la marca enfocada muestra el anillo como su propio trazo en `--color-accent`. Los botones del mismo componente usan `--focus-ring`.
- **`outline: none` en un contenedor con foco programático** (`VirtualGrid`, `tabindex="-1"`): solo recibe el foco por código, nunca con Tab, cuando desaparece la tarjeta que lo tenía y no hay otra a la que pasárselo; así el foco no cae en `<body>`. No hay nada que marcar: el siguiente Tab lleva a una tarjeta, que sí muestra su anillo.
- **`line-height: 0`** en el contenedor de imagen de `WritingEditor`: impide que el contenedor agregue una línea propia alrededor de la imagen.
- **Tablas de datos** hechas a mano (`InvestigationView`, `DbBrowserView`) mientras no exista un componente `Table`: usan tokens para color, borde, tipografía y espaciado.
- **El botón punteado de agregar campo** de `MetadataEditor`: es una zona de alta, no un botón de acción; ninguna variante de `Button` dibuja un borde punteado.
- **Colores de las regiones en los overlays de `DocumentViewer`**, calculados en el script (relleno y trazo de las regiones de layout y del recorte): pintan sobre el escaneo, no sobre el tema, igual que `--viewer-layout-block`.
- **`@media` contra el viewport** solo en el cromo de la ventana (`App`, `TopBar`, `TabStrip`), en diálogos (anclados a la ventana) y en la barra de anotación (que reacciona a la altura de la ventana). Una vista dentro de un panel usa `@container pane`.
- **`z-index` local** de 0 a 5 dentro del contexto de apilamiento de un componente.
- **Fallback en un `var()` de un gancho propio** del componente (`--tree-level`, `--writing-indent`, `--number-field-max-width`, `--overlay-cursor`, `--investigation-header-height`): es el valor por defecto de una variable que el padre puede fijar o que el componente fija desde el markup, no un token publicado. Un fallback sobre un token publicado sigue prohibido.
- **El elemento nativo dentro de su propio componente**: el `<textarea>` de `Textarea` y el radio oculto de `RadioCard`.
- **Tamaños fuera de la escala** listados en `OFF_SCALE_BY_DESIGN` (`apps/desktop/src/design-tokens.test.ts`): el botón de reproducir de `AudioPlayer` (56px) y la acción de 20px de los chips de `EntityViewer`.

---

## No se debe usar

### Controles nativos sin estilizar

- **`<select>`** — prohibido por lint (`svelte/no-restricted-html-elements`): su popup lo dibuja el sistema operativo.
- **`<input type="radio">` o `type="checkbox">` visibles** con la apariencia del sistema, ni “arreglados” solo con `accent-color`. Usá `Checkbox` o el patrón de radio oculto + tarjeta.
- **`<progress>`** nativo, ni con `appearance: none` sin estilizar sus pseudo-elementos.
- **`<input type="range">`** con el thumb y el track del sistema.
- **`<input type="number">`** fuera de `NumberField`.
- **`<details>/<summary>`** con el marcador triangular y el foco por defecto del navegador.
- **`<textarea>` / `<input>` con clase propia** que copie la apariencia de `Input` (fondo, borde, radio, altura): extendé el componente.
- **El atributo `title`** como tooltip en elementos propios.
- **`<svg>` escrito a mano** para un ícono (lint); solo gráficos y overlays en la allowlist de `eslint.config.js`.
- **Importar `@tabler/icons-svelte-runes`** directamente (lint).

### Valores fuera de los tokens

- **Colores literales** en `<style>`: hex, `rgb()`/`rgba()`, `hsl()`, nombres (`white`, `black`), **y en particular la paleta de Tailwind** (`#6b7280`, `#e5e7eb`, `rgba(245,158,11,…)`, `rgba(99,102,241,…)`, `rgba(239,68,68,…)`).
- **`color-mix(…, black N%)` o `color-mix(…, white N%)`**: oscurece o aclara bien en un tema y mal en los otros.
- **Brillos y sombras escritos a mano** (`inset 0 1px 0 rgba(255,255,255,.03)`): usá `--shadow-surface` o `--shadow-*`.
- **Tokens inventados** (`--color-text-tertiary`, `--color-text-danger`, `--color-text-on-accent`, `--color-surface-hover`): si falta un token, se crea en `tokens.css`.
- **Fallbacks en `var()`** para tokens publicados (`var(--radius-sm, 4px)`, `var(--color-accent, #4f46e5)`): esconden errores de nombre y mienten sobre el valor.
- **`font-size` en `px`/`em` literales**, y nunca por debajo de 11px (`--font-size-2xs`).
- **`font-weight` numérico**, **`line-height`** y **`letter-spacing`** sueltos cuando hay token.
- **`font-family`** con un nombre de familia: siempre `--font-ui`, `--font-reading` o `--font-mono`.
- **Espaciados literales** (`padding: 5px 10px`, `gap: 6px`) y **alturas de control** fuera de `--control-height-*` (26px, 28px, 22px…).
- **Radios literales** (`2px`, `4px`, `999px`) en lugar de la escala.
- **Opacidades de deshabilitado propias** (0.4, 0.5, 0.55, 0.56…).
- **`z-index` globales improvisados** (`1100`, `60`, `25`…); ver la escala de capas.
- **Duraciones literales** (`0.15s`, `120ms`, `0.3s`).
- **`!important`**, salvo para el cursor global durante un arrastre (patrón de `ItemView`/`CollectionView`).

### Prácticas prohibidas

- **Estilos inline** (`style="…"`) para color, tipografía, borde o espaciado. `style:` solo para geometría calculada.
- **Reestilizar componentes del sistema desde una vista** con `:global(.btn…)`, `:global(.input-field__input)`, `:global(.icon-button…)` para cambiar su apariencia (fondo, borde, degradado). Se permite solo para layout (ancho, `flex`, alineación). Si una vista necesita otro aspecto, es una variante nueva del componente.
- **Rellenar un botón o un chip de `--color-accent`** (o de cualquier tono) para darle jerarquía: el primario es `Button variant="primary"`.
- **Botón primario hecho a mano** (`.x-submit { background: var(--color-accent) }`).
- **`outline: none` sin reemplazo visible**, o un foco que sea igual al hover.
- **Dos lenguajes de foco**: nada de `outline: 2px solid var(--color-accent)` donde el resto usa `--focus-ring`.
- **Duplicar un patrón entre vistas** (lista de destinos con “nueva colección”, disparador de menú, banner de estado, botón de panel derecho): se extrae a `packages/ui`.
- **Scrollbars locales** con otros tamaños o radios que las globales.
- **Media queries por viewport** para algo que depende del ancho del panel.
- **Colores de estado decididos en TS e inyectados inline** (`style="color: {statusColor}"`): clase modificadora.
- **Ruido de mayúsculas a mano**: `text-transform: uppercase` siempre con el `letter-spacing` de versalitas.

---

## Checklist de revisión (PR que toca estilos)

- [ ] ¿Cero hex/`rgba`/`black`/`white` nuevos en `<style>`?
- [ ] ¿Todos los `var()` nombran tokens que existen, sin fallback?
- [ ] ¿Usa `Button`/`IconButton`/`Input`/`Checkbox`/`NumberField`/`ToolbarMenu`/`StatusBadge` en vez de reimplementarlos?
- [ ] ¿Ningún control con apariencia nativa (radio, progress, range, details)?
- [ ] ¿Hover, foco (`--focus-ring`), disabled y active definidos?
- [ ] ¿Ningún `:global` que cambie la apariencia de un componente del sistema?
- [ ] ¿Revisado en `dark`, `paper` y `vivid`, y en contraste `high`?
- [ ] ¿`pnpm lint` y `pnpm typecheck` sin warnings; `visual-contract`, `contrast-floor` y `control-block-size` verdes?
