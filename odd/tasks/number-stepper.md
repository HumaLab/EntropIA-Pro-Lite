# Numeric fields: one reusable stepper across the app

Branch `feat/number-stepper`. This replaces every native `<input type="number">` spin button with one `NumberField` component from `packages/ui`. The component has custom − and + buttons and keeps each field's existing min, max, step and handlers.

## Specs

- **S1. The user's request, verbatim:** «Revisá y mejorá transversalmente todos los campos numéricos de EntropIA, no solo los de la sección Exploración. Identificá todos los *inputs* numéricos y sus *spin buttons* en las distintas ventanas, módulos y configuraciones de la aplicación. Corregí los problemas de alineación, dimensiones y espaciado, y reemplazá los controles nativos de HTML por un componente reutilizable con botones personalizados de incremento (+) y decremento (−), integrados visualmente con los campos y acordes con la estética oscura, sobria y minimalista de la aplicación. El incremento y decremento debe ser de 1 unidad por clic, salvo en aquellos parámetros que requieran otros pasos por su naturaleza. Respetá los valores mínimos, máximos y las validaciones existentes. Garantizá consistencia visual, accesibilidad y comportamiento responsive en toda la aplicación, sin alterar la lógica ni la funcionalidad de los parámetros.»
- **S2. Inventory (8 Nov 2026), seven native numeric inputs:**
  - `CollectionAnalysisPanel.svelte:497`: cloud term count. Min and max come from constants; `onchange` handler.
  - `GraphSettingsForm.svelte:242`: graph parameters. Per-control step: 10, 50, 1, 0.1, 0.05, 5; `onchange` handler.
  - `InternetArchiveDialog.svelte:948` and `:958`: year from and year to. Range 0–9999; `bind:value`.
  - `InvestigationView.svelte:1069` and `:1081`: budget calls (min 1, step 1) and maximum cost (min 0, step 0.01); `bind:value`.
  - `SyncSettingsCard.svelte:666`: auto-sync interval. Min 1; `bind:value` plus `onchange`.
  - Out of scope: `WritingZoteroTab.svelte:490` is a Zotero library ID (an identifier, not a quantity), so it stays a text field.
- **S3. Component behavior.**
  - Each click moves the value by the field's `step`, which defaults to 1. Fields that already declare another step keep it.
  - The result is clamped to min and max and rounded to the step's decimals (no 0.30000000000000004).
  - A button is disabled at its bound and when the field is disabled.
  - An empty field starts from min, or from 0 when there is no min.
  - A click updates the input and fires the same `input` and `change` events typing does, so every existing handler, binding and validation runs unchanged.
  - Keyboard ArrowUp and ArrowDown, and typing, keep working.
- **S4. Accessibility.**
  - The input keeps role `spinbutton` with its existing label.
  - The buttons have translated accessible names («Disminuir», «Aumentar») and are not tab stops; the input is, and the keyboard arrows step it.
  - The native spin buttons are hidden.
- **S5. Visual consistency.**
  - The field is one control at the input height token: sunken background, hairline border, input radius, and the accent focus ring on the whole group.
  - Its buttons sit inside the border.
  - It fills its container's width and does not overflow on narrow layouts.
  - Each call site loses its spin-button-specific CSS hacks.
- **S6. Rows stay aligned.** The user's request, verbatim: «Corregí la alineación de todos los campos numéricos de la aplicación. Los controles (−, valor, +) deben quedar siempre alineados horizontalmente dentro de cada fila, independientemente de la longitud de sus etiquetas. Para lograrlo, establecé una altura uniforme para el espacio reservado a las etiquetas, aplicando el mismo criterio a todos los campos: salto de línea con una cantidad máxima de líneas o truncamiento con puntos suspensivos (…). No debe haber controles desplazados verticalmente respecto de los demás. Garantizá este comportamiento en todos los tamaños de ventana y diseños responsive.»
  - Decision: one global rule, `.number-field-label` in `app.css`. Every numeric field's label gets a fixed two-line slot: the text wraps to at most two lines and then ends in «…».

## Tasks

| ID | Specs | Route | Commit |
| --- | --- | --- | --- |
| T1 | S3, S4, S5 | `NumberField` in packages/ui plus the `subtract` ActionIcon, with tests | 6733f0b3 |
| T2 | S2 | Replace the seven inputs; i18n labels; existing view tests stay green | 9ab12825 |
| T3 | S6 | Shared two-line label rule plus a source test that every NumberField label uses it | 9ab12825 |

## Log

- L1 (user, 8 Nov 2026): the request quoted in S1.
- L2: explorer and worker subagents were unavailable (quota), so the inventory and the work are done inline.
- L3 (user): «fiajte qeu salta de 10 en 10 y no de 1 en 1», then «igualmente está bien porque se peude mofificar a mano, no es probelam de 10 en 10 ok así». The graph fields keep their declared steps (10, 50, 5, 0.1, 0.05).
- L4 (defect found by tests): inside a wrapping `<label>`, the first labelable descendant was the − `<button>`, so the label named and clicked the decrement. The buttons became `role="button"` spans, which are not labelable; a click on them is not forwarded to the input. The NumberField test pins this.
- L5 (user): the alignment request quoted in S6.
