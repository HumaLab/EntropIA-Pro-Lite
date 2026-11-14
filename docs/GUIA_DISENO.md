# Guía de diseño — EntropIA

> Reglas de estilo para todo el frontend (`apps/desktop/src`, `packages/ui/src`).
> Fuente de verdad de los valores: `packages/ui/src/tokens/tokens.css` y `apps/desktop/src/app.css`.
> Diagnóstico y plan que la motivan: [`AUDITORIA_ESTILOS.md`](./AUDITORIA_ESTILOS.md).
>
> Las marcas **(propuesto)** señalan tokens o componentes que el plan crea y que todavía **no existen**: hasta entonces, seguí la alternativa indicada al lado.

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
| Acento | `--color-accent`, `--color-accent-hover`, `--color-accent-soft`, `--color-accent-faint` | `soft` para seleccionado/hover marcado, `faint` para hover suave. |
| Estados | `--color-danger`/`-hover`/`-soft`, `--color-success`/`-soft`, `--color-warning`/`-soft`, `--color-info`/`-soft`; `--state-ai*`, `--state-evidence*` | Texto e ícono con el color pleno, fondo con `-soft`, borde con `color-mix(… 28–34%, transparent)` del mismo color. |
| Texto sobre relleno de estado | `--color-on-danger` **(propuesto)**; mientras tanto, `--color-text-primary` sobre `--color-danger-soft` | Nunca `#fff` fijo. |
| Primario | `--control-primary-bg`, `-bg-hover`, `-bg-active`, `-border`, `-text` | Solo a través de `Button variant="primary"`. |
| Espaciado | `--space-1` (4) … `--space-8` (32); `--space-0-5` (2) **(propuesto)** | Todo `padding`, `margin` y `gap`. Hasta que exista `--space-0-5`, `2px` es el único literal tolerado (ajustes ópticos). |
| Tipografía | `--font-ui`, `--font-reading`, `--font-mono`; `--font-size-2xs` (11) … `--font-size-xl` (22); `--font-weight-normal/medium/semibold`; `--line-height-tight`, `--line-height-base` | La familia se elige por **función** (operar la app / leer / identificadores), nunca por nombre. 11px es el mínimo. |
| Lectura | `--line-height-reading` **(propuesto)**; mientras tanto `--line-height-base` | Para manuscrito, notas, OCR, respuestas largas. |
| Versalitas | `--letter-spacing-caps` **(propuesto)**; mientras tanto `0.075em` (el de `.page-header__eyebrow`) | Etiquetas en mayúsculas. |
| Radios | `--radius-control` (botones, triggers), `--radius-input` (campos), `--radius-surface` (tarjetas, paneles), `--radius-dialog` (diálogos), `--radius-full` (píldoras, círculos); `--radius-xs` solo para decoraciones pequeñas | Para controles usá la escala **semántica**; `--radius-sm/md/lg/xl` quedan para casos que no son un control. |
| Sombras | `--shadow-surface` (brillo superior de una superficie), `--shadow-sm/md/lg`, `--shadow-glow` | Los temas claros las anulan: no las repitas a mano. |
| Alturas de control | `--control-height-sm` (30), `-md` (36), `-lg` (40); `--control-height-xs` (24) **(propuesto)** | Toda altura de botón, campo o trigger. |
| Foco | `--focus-ring`, `--focus-ring-danger` | `box-shadow: var(--focus-ring)` en `:focus-visible`. |
| Movimiento | `--transition-base` (0.14s), `--transition-smooth` (0.18s); `--duration-*` **(propuesto)** | Toda transición; respetar `prefers-reduced-motion`. |
| Deshabilitado | `--opacity-disabled` **(propuesto)**; mientras tanto `0.48` (el de `Button`), y `0.72` solo para el primario | Un único valor en toda la app. |
| Capas | `--z-sticky`, `--z-dropdown`, `--z-popover`, `--z-dialog`, `--z-tooltip`, `--z-blocking` **(propuesto)**; mientras tanto respetar el orden de la auditoría §2.10 (menús 210, diálogos 1000–1200, tooltip 1300) | `z-index` local (0–5) libre dentro de un componente con su propio contexto de apilamiento. |
| Paleta del manuscrito | `--writing-text-*`, `--writing-highlight-*`, `--writing-blend-highlight` | Solo en `WritingEditor`/`ColorPalette`. |

### Componentes base (`@entropia/ui`)

| Necesidad | Usá | Notas |
| --- | --- | --- |
| Acción con texto | `Button` (`primary`, `secondary`, `outline`, `ghost`, `subtle`, `danger`; `sm`/`md`/`lg`; `loading`) | Un solo `primary` por zona. `title` se convierte en tooltip del sistema. |
| Acción con ícono | `IconButton` (`xs` 24 · `sm` 28 · `md` 32 · `lg` 40; `label` obligatorio) | El contenedor sale del tamaño del ícono, nunca al revés. |
| Ícono | `ActionIcon` | Única puerta a los íconos; para uno nuevo, agregalo a `ACTION_ICON_NAMES`. |
| Campo de texto | `Input` (`label`, `hint`, `error`) | Tras la Fase 2 acepta todos los atributos de `<input>`, tamaños y slots de ícono/acción. |
| Búsqueda | `SearchBar` (+ `SearchClearButton`) | O el patrón `.search-field__icon` de `app.css` si el campo ya tiene markup propio. |
| Número | `NumberField` (+ clase global `.number-field-label` en la etiqueta) | Nunca `<input type="number">` suelto. |
| Casilla | `Checkbox` | Selección por contraste, foco en la fila. |
| Elección de una opción en un desplegable | `ToolbarMenu` con ítems de tipo radio + un trigger | Patrón de referencia: `WritingZoteroTab`. `SelectTrigger` **(propuesto)** lo empaqueta. |
| Elección visible entre pocas opciones | Radio visualmente oculto + tarjeta con `:has(:focus-visible)` (patrón `FontPresetGrid`, `WritingExportTab`); `RadioGroup`/`RadioCard` **(propuesto)** | |
| Pestañas | `TabList` + `TabButton` | |
| Menú de acciones | `ToolbarMenu` / `ToolbarMenuList` | |
| Estado o etiqueta | `StatusBadge` (`neutral`, `stored`, `success`, `warning`, `danger`, `info`, `ai`, `evidence`) | `Badge`/`Chip` **(propuesto)** para contadores y chips removibles. |
| Superficie | `Card` (`padding`, `hoverable`, `header`, `footer`), `Panel` (`default`, `raised`, `glass`, `sunken`) | |
| Confirmación / diálogo | `ConfirmDialog` (`size`, `variant`) | `Dialog` **(propuesto)** para diálogos con contenido libre. |
| Tooltip | `use:tooltip` / `TooltipLayer` | Nunca el atributo `title` nativo en elementos propios. |
| Capa flotante | `portal` (`OVERLAY_ROOT_ATTRIBUTE`) | |
| Divisor redimensionable | `ResizeHandle` / `SplitDivider` | |
| Progreso | `ProgressBar` **(propuesto)**; mientras tanto, la barra de `SettingsView` (`.settings__download-progress-bar`) como referencia visual | |
| Interruptor | `Switch` **(propuesto)**; mientras tanto, el de `AssistantsTab` | |
| Desplegable de contenido | `Disclosure` **(propuesto)** | |

### Patrones globales permitidos (`app.css`)

- **Página:** `.page-shell`, `.page-header` (+ `__content`, `__eyebrow`, `__meta`, `__pipeline`), `.page-toolbar` (`--split`).
- **Mensajes:** `.surface-message` (`--center`, `--error`, `--success`).
- **Búsqueda:** `.search-field__icon` y `--search-field-inset`.
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
