# Plan: texto nativo, parte B — reparar lo ya guardado

Estado: **revisión 2, para re-juicio acotado. No hay nada implementado.**
Fecha: 2026-10-08. Rama `feat/native-text-reprocess`, desde main `49a5f42`.

Parte de `odd/plans/plan-texto-nativo.md` (revisión 3, secciones 3.2 a 3.4). La parte A ya está en main
(`odd/tasks/native-text-part-a.md`): PDFium lee las páginas con espacios de ahora en adelante y no cambió
ninguna regla de OCR.

## 0. Historial y decisiones del owner

- **Revisión 1** (`e69242b`). Juicio 7: sin críticos.
  - 2 hallazgos graves confirmados por los dos jueces.
  - 4 graves de un solo juez.
  - Avisos y sugerencias.

  El owner pidió corregir todo (tabla en la sección 6).
- **Decisiones del owner:**
  - **Ningún OCR pagado automático sobre lo ya guardado.** Cada cobro lo aprueba el owner, viendo el costo
    antes.
  - **La vista previa muestra páginas y un monto en USD**, marcado como estimado.
  - **La acción masiva va en la barra de herramientas de la Biblioteca** (`BibliotecaView`); la ficha de
    cada obra tiene la acción individual.
  - **Sin migración** sin preguntar, y homogeneidad visual estricta.

## 1. Estado actual (medido en el código)

- **Resolución de una extracción.** `extraction_is_settled` (`bibliography/repository.rs:2254`) la decide así:
  1. Si la extracción guardada no coincide con el archivo (`extraction_matches_source`, `:2172`: `mtime`
     del catálogo y tamaño), no está resuelta.
  2. Si el texto guardado es ilegible (`stored_extraction_is_garbled`, `:2205`), no está resuelta.
  3. Si es `empty`, no está resuelta hasta que **alguna** tarea `succeeded` diga `"ocrAttempted":true`. La
     consulta no está atada al archivo.

  La usan la admisión del sync (`processing/repository.rs:2346`) y el atajo `alreadyCurrent` del ejecutor
  (`bibliography/processing.rs:2902`).
- **Falla definitiva de una tarea** (`processing/repository.rs:5408-5417`). Pasa a `failed` y **borra sus
  puntos de control**: las páginas ya pagadas se pierden.
  - Si era la primera extracción del archivo, no queda ninguna extracción publicada, y el sync siguiente la
    vuelve a encolar y a pagar (`JD7-A-001`).
  - `processing_retry` reabre la misma tarea sin puntos de control y vuelve a pagar todo (`JD7-B`).
- **Reintentos.**
  - Timeout y 5xx se reintentan hasta `MAX_ATTEMPTS_PER_CYCLE = 3` por ciclo.
  - El 429 (`RATE_LIMITED_CODE`) **nunca agota** el ciclo (`:5387`): espera con backoff, en la misma tarea y
    con sus puntos de control (`JD7-A-006`).
- **Falta de clave.** El proveedor responde "not configured" y la tarea queda `blocked`
  (`configuration_required_ocr`), viva, hasta que se guarda la clave (`settings.rs`).
- **Huella de la tarea.** `attachment_extraction_fingerprint` (`processing/repository.rs:784`) es
  `attachment|<id>|mtime:<m>|version:<v>`. La versión del catálogo cambia también con una edición de
  metadatos, sin que cambie el archivo (`JD7-A-002`).
- **Cola.**
  - Hay una sola tarea viva por adjunto (`idx_processing_tasks_subject_active_unique`).
  - Las tareas terminales nunca se borran.
  - La admisión existente se suma a la tarea viva, también cuando pierde la carrera del `INSERT OR IGNORE`
    (`:701-753`).
  - `kind` tiene un CHECK, así que un tipo nuevo necesita migración.
- **Lotes.**
  - `processing_batches.origin` admite `user`, `manual`, `repair` y `bibliography`.
  - `list_batches` solo lista `user` (`:3383`), así que el lote `manual` es invisible (`JD7-A-005`,
    `JD7-B`).
  - El indicador reinicia su seguimiento cuando cambian los IDs de los lotes de usuario
    (`BatchStatusIndicator.svelte:47-75`).
  - Pausar y cancelar se hacen por lote (`processing_control`).
- **Contrato.** `BIBLIOGRAPHY_EXTRACT_CONTRACT = "bibliography-extract-v1"` se compara por igualdad en el
  reclamo (`:4503`), antes de publicar (`:5181`) y en el ejecutor (`bibliography/processing.rs:2798`). Los
  puntos de control se guardan por `(task_id, unit_key, input_fingerprint, contract_hash)`.
- **Respuesta vacía de GLM-OCR.** `ocr/mod.rs:769` la devuelve como `Err` (`GLM_OCR_EMPTY_RESPONSE_MESSAGE`).
  El error sale de `ctx.unit` sin punto de control, y la página se vuelve a pagar al reanudar
  (`JD7-A-004`).
- **OCR.** `ocr_candidate_pages` (`processing.rs:3559`) elige `sparse`, `empty` y, si el documento es
  `native_blank`, `unreadable`. `maybe_ocr_pages` prueba ventanas de PDF completo cuando la mitad de las
  páginas o más necesitan OCR (`should_use_pdf_mode`, `selective_ocr.rs:288`).
- **Sin precio ni detector propio.** No existe ningún cálculo de precio: GLM-OCR cobra USD 0,03 por millón
  de tokens, de entrada y de salida, y el cliente no lee el uso. Tampoco existe
  `is_garbled_bibliography_text`.
- **UI.** `@entropia/ui` no tiene un componente de barra de progreso (`JD7-B`).

## 2. Diseño

### 2.1 Ningún reencolado automático sobre lo ya guardado (plan 3.2)

- **`extraction_is_settled` pasa a ser solo "la extracción guardada coincide con el archivo".**
  - Se borra `stored_extraction_is_garbled`, que no tiene otro uso.
  - Se borran la regla `empty` sin OCR y las dos excepciones que la acompañaban (markup convertido y HTML
    vacío).
  - Una extracción publicada nunca se vuelve a pedir sola, sea ilegible, esté vacía o haya fallado su OCR.
    Repararla es la acción del owner (2.3), que lista también las extracciones vacías que nunca pasaron por
    OCR (2.4). Así se cierra también el caso del texto ilegible clasificado `empty` (`JD7-B`).
- **Un intento de OCR fallido sin extracción publicada tampoco se repite** (`JD7-A-001`).
  - La admisión del sync omite un adjunto si existe una tarea `bibliography_extract` terminal `failed` o
    `cancelled` para el **mismo archivo**.
  - "Mismo archivo" se compara con el prefijo `attachment|<id>|mtime:<m>|` de su `input_fingerprint`, sin
    mirar la versión del catálogo (`JD7-A-002`).
  - Solo cuentan las fallas del OCR (`provider_transient`, `rate_limited`, fatales del proveedor) y las
    cancelaciones. Una falla de lectura o de almacenamiento se sigue reintentando como hoy.
  - El owner recupera el adjunto con "Reprocesar texto" o con "Reintentar".
- **La versión del catálogo no dispara nada** (`JD7-A-002`). Con la primera regla, una edición de metadatos
  con el mismo archivo deja la extracción resuelta.
  - `attachment_extraction_fingerprint` **no cambia** (`JD5-A-002`). Sigue siendo la llave de los puntos de
    control y de la verificación antes de publicar.
- **Los puntos de control de las extracciones sobreviven a la falla** (`JD7-A-001`, `JD7-B`).
  - Para `bibliography_extract`, la falla definitiva no borra los puntos de control.
  - `processing_retry` reabre la misma tarea, con la misma huella y el mismo contrato, y reusa las páginas
    ya pagadas.
  - Se borran cuando la tarea termina con éxito, como hoy.
  - Si el archivo cambió, la huella ya no coincide y los puntos de control viejos se ignoran.
- **La respuesta vacía se guarda como resultado** (`JD7-A-004`).
  - Dentro de la función de `ctx.unit`, `GLM_OCR_EMPTY_RESPONSE_MESSAGE` se convierte en `Ok("")`.
  - `settled_page_row` ya trata un texto vacío como página en blanco.
  - Una respuesta vacía queda en el punto de control y nunca se vuelve a pagar.
- **Lo que sigue igual:**
  - **Falta de clave:** tarea viva `blocked` que se reanuda al guardar la clave.
  - **429:** espera indefinida en la misma tarea, sin perder puntos de control.
  - **Primera extracción de un archivo nuevo o cambiado:** puede llevar OCR automático de sus páginas
    candidatas, como hasta ahora (incluidas las que marca el detector, 2.2).

### 2.2 Detector de la bibliografía (plan 3.4)

- **Ubicación:** `is_garbled_bibliography_text(text) -> bool`, en `ocr/pdf.rs` junto a `is_garbled_text`,
  con `BIBLIOGRAPHY_DETECTOR_VERSION: u32 = 1`. `is_garbled_text` e `is_quality_text` no cambian.
- **Tokens elegibles.** Se descartan URL, DOI, correo y dominio (`algo.org`, `.com`, `.net`, `.edu`,
  `.gov`, TLD de dos letras).
- **Regla 1, palabras pegadas.**
  - Exige al menos 80 letras latinas (ASCII, Latin-1, Latin Extended).
  - Marca la página si el 50 % o más de esas letras está en tokens de más de 24 letras.
- **Regla 2, ruido de OCR viejo.**
  - Exige al menos 40 tokens elegibles de 4 letras o más.
  - Marca la página si el 8 % o más tiene `.`, `~` o `·` entre dos letras.
  - No cuentan `-`, `'`, el `l·l` catalán, las abreviaturas con punto entre mayúsculas ni `e.g.`/`i.e.`.
- **Uso.** El detector entra en `ocr_candidate_pages`: una página marcada es candidata como una `sparse`. Se
  aplica sobre el texto nativo que la extracción publicaría, convertido el markup. No toca
  `extraction_quality`.
- **Validación antes de mergear.**
  - Se mide sobre una **copia de solo lectura** de la base del perfil `prueba-sync`.
  - Para cada página nativa `rich` se calcula el texto de PDFium y se aplica el detector.
  - Se informa cuántas marca cada regla, y 30 páginas marcadas al azar quedan para que el owner las mire.
  - Criterio: menos del 0,5 % de falsos positivos sobre las páginas limpias. Si no se cumple, se ajustan los
    umbrales.

### 2.3 Reproceso explícito (plan 3.3)

#### Modo durable sin migración

- **Es la misma tarea `bibliography_extract`** con otro contrato:
  `contract_hash = "bibliography-extract-reprocess-v1|<planHash>"`.
- **Las tres comparaciones de contrato** pasan por `parse_extract_contract(&str) -> Option<ExtractMode>`,
  que devuelve `Automatic` o `Reprocess { plan_hash }`. Cualquier otro valor bloquea como hoy.
- **Durabilidad.** El modo vive en la fila de la tarea y sobrevive a reinicios, a reintentos y a
  `processing_retry`. Los puntos de control del reproceso no se mezclan con los automáticos, porque la clave
  incluye el contrato.

#### El plan, una sola función para la vista previa y el ejecutor

`plan_reprocess(input) -> ReprocessPlan` es pura y está en `bibliography/processing.rs`.

- **Entrada:**
  - las páginas nativas que lee el lector de la parte A sobre los bytes actuales;
  - `native_blank`, calculado como hoy;
  - las filas guardadas de `bibliographic_page_texts` y la identidad guardada de la extracción;
  - el SHA-256 de los bytes del archivo.
- **Salida:**
  - `page_count`;
  - `ocr_pages`: las páginas que van a GLM-OCR;
  - `reused_ocr_pages`, con el `text_hash` de cada una;
  - `fixed_without_ocr`: las páginas que hoy marca el detector y que PDFium arregla;
  - `plan_hash`.
- **Selección:**
  1. Si la extracción guardada coincide con el archivo, cada fila `method = 'ocr'` se **reusa** y no va a
     GLM-OCR, aunque esté vacía.
  2. Del resto, van a OCR las que elige `ocr_candidate_pages` ampliada con el detector (2.2).
- **`plan_hash`** (`JD7-A-003`, `JD7-B`). Es el SHA-256 del JSON canónico, con claves ordenadas, de:
  - `attachmentId` y `sourceSha256`;
  - `pageCount`, `ocrPages` y `reusedOcr` (página y `textHash`);
  - `detectorVersion`;
  - `planVersion`, el número constante `1`.

  El hash **no incluye** el contrato. Así, un PDF reemplazado por otro del mismo tamaño cambia el hash.
- **El ejecutor no vuelve a elegir páginas.** En modo reproceso manda a OCR exactamente `plan.ocr_pages`.

#### Lote visible y cancelable (`JD7-A-005`, `JD7-B`)

- **Cada confirmación crea un lote propio** con `origin = 'user'`, `planning_done = 1`, prioridad
  interactiva 2 y `request_id = "bibliography-reprocess-<uuid>"`.
- **Aparece** en la pestaña de lotes y en el indicador. El indicador reinicia su seguimiento porque cambian
  los IDs de los lotes de usuario.
- **Se pausa o se cancela** con los controles existentes. Una cancelación deja la tarea `cancelled`, que
  cuenta como intento (2.1).
- **Termina solo** cuando terminan sus tareas, como cualquier lote de usuario.
- **Verificación en B4.** La pestaña de lotes y el indicador muestran bien un lote de usuario cuyas tareas
  son de bibliografía, porque hoy esos lotes llevan tareas del corpus.
  - Hay que confirmar el nombre que muestran, sus operaciones (`operations`) y su finalización.
  - Si algo no encaja, el ajuste se hace en esos componentes existentes, sin un indicador nuevo.

#### Comandos nuevos (Tauri, `bibliography/commands.rs`)

1. **`bibliography_reprocess_candidates()`**, de solo lectura y barato. Devuelve los adjuntos PDF que
   cumplen alguna de estas condiciones:
   - al menos una página guardada marcada por `is_garbled_bibliography_text` o por `is_garbled_text`,
     convertido el markup;
   - una extracción `empty`, o páginas `empty`, sin ningún recibo `"ocrAttempted":true` para el `mtime`
     actual;
   - una tarea terminal fallida o cancelada por OCR para el archivo actual (2.1).

   Excluye los que tienen un reproceso exitoso para el archivo actual (2.4).
2. **`bibliography_reprocess_preview(attachmentIds)`**, de solo lectura.
   - Lee cada PDF (SHA-256 de los bytes y páginas con PDFium en tandas de 20) y corre `plan_reprocess`.
   - Corre en un hilo de bloqueo, informa el progreso por evento (`n de m adjuntos`) y se cancela con
     `bibliography_reprocess_preview_cancel`.
   - Por adjunto devuelve: obra, `pageCount`, cantidades de OCR, de reuso y de arreglo sin OCR,
     `planHash`, `busy` y un motivo si no se puede leer.
   - En total devuelve: adjuntos, páginas, páginas a GLM-OCR, OCR reusado y USD estimado.
3. **`bibliography_reprocess_confirm(entries: [{attachmentId, planHash}])`**, con admisión propia
   (`JD7-B-001`).
   - En **una transacción `IMMEDIATE`** crea el lote; por cada entrada, si hay una tarea viva para el
     adjunto, la marca `busy` y sigue.
   - Si no la hay, hace un `INSERT` **sin `OR IGNORE`**. Un choque con el índice único también da `busy`.
   - **Nunca** se suma a una tarea existente.
   - Un lote sin ninguna entrada encolada se borra dentro de la misma transacción.
   - Devuelve, por entrada, `queued` o `busy`.
   - Una admisión automática que encuentra un reproceso vivo se suma a él, como hoy; no cobra fuera de lo
     autorizado, porque el ejecutor sigue el plan.

#### Ejecutor en modo reproceso

1. **No usa el atajo** `extraction_is_settled`/`alreadyCurrent`.
2. **Recalcula el plan** con los bytes leídos. Si el hash no coincide con el del contrato, termina `Fatal`
   con `reprocess_authorization_stale` **antes de cualquier llamada a GLM-OCR** (`JD5-A-006`, `JD7-A-003`).
   El owner vuelve a pedir la vista previa.
3. **Las páginas reusadas** toman el texto guardado convertido con `ocr_markup_to_text` (`JD4-B-006`),
   `method = 'ocr'` y un hash nuevo.
4. **OCR solo por página.** `maybe_ocr_pages` recibe la lista exacta y el modo, y salta la rama de ventanas.
   Cada página guarda su punto de control `ocr-page:{n}`, también las respuestas vacías (2.1).
5. **Publica como siempre** (parte A): la unión de las páginas, el borrado de las páginas que sobran y el
   perfil pedido de nuevo si cambiaron.
6. **El recibo suma** `"reprocess": {"planHash", "detectorVersion"}`, `sourceMtime`, `sourceBytes` y
   `sourceSha256`.

#### Garantía de costo

- **El total de páginas enviadas a GLM-OCR** por un reproceso, sumando reinicios, reintentos del ciclo y
  `processing_retry`, es **como mucho** `ocr_pages` del plan.
- **Cómo se cumple:**
  - cada página tiene su punto de control;
  - los puntos de control sobreviven a la falla (2.1);
  - las respuestas vacías también quedan guardadas.
- **La excepción:** una página cuya llamada falla sin respuesta (timeout o 5xx) se vuelve a pedir. Si el
  proveedor la procesó igual, puede cobrarla dos veces (sección 5).

#### Monto estimado

- `estimated_usd = ocr_pages × ESTIMATED_TOKENS_PER_PAGE × 0,03 / 1.000.000`, con
  `ESTIMATED_TOKENS_PER_PAGE = 3000`, documentado en el código con su fuente.
- La interfaz muestra "≈ USD 0,24 (estimado)" y, debajo, la tarifa y el supuesto. Con 0 páginas a OCR
  muestra "sin costo de OCR".

### 2.4 Una sola vez

- **La lista masiva** excluye un adjunto si tiene un reproceso `succeeded` cuyo recibo trae el
  `sourceSha256` y el `detectorVersion` actuales. Se compara con `sourceMtime` y `sourceBytes`, sin leer el
  archivo en la lista; el SHA lo verifica la vista previa.
- **La acción de la ficha siempre está disponible.**
- **El sync nunca mira esa marca** (2.1).

### 2.5 Interfaz

- **Componentes.**
  - Solo los existentes de `@entropia/ui`: `Button`, `ActionIcon` (un ícono nuevo va en
    `ACTION_ICON_NAMES`) y `ConfirmDialog`, con el patrón de `PassageReaderDialog`.
  - **El progreso de la vista previa es texto** ("Leyendo 12 de 82 adjuntos"), sin barra (`JD7-B`).
  - El resumen es una lista de definición con las clases tipográficas existentes. Sin estilos nuevos.
- **Biblioteca, barra de herramientas:** botón "Reprocesar texto".
  1. Llama a `candidates`.
  2. Abre el diálogo y corre `preview`, con el progreso en texto y el botón Cancelar.
  3. Muestra adjuntos, páginas, páginas a GLM-OCR, OCR reusado, páginas que se arreglan sin OCR y el USD
     estimado.
  4. Lista los adjuntos `busy` o ilegibles, que no se encolan.
  5. El botón de confirmar dice "Reprocesar (≈ USD x)".

  Sin candidatos, el diálogo lo dice y no ofrece confirmar.
- **Ficha de la obra:** el mismo botón y el mismo diálogo, con todos los adjuntos PDF de la obra.
- **Después de confirmar,** el diálogo informa cuántos adjuntos quedaron encolados y cuántos `busy`. El
  avance, la pausa y la cancelación están en la pestaña de lotes (2.3).
- **Idioma.** Los textos van por `t(...)`, con claves en español y en inglés.

## 3. Pruebas

Las pruebas van primero cuando se pueden escribir: rojo, verde y después refactor.

- **2.1:**
  - una extracción guardada ilegible o vacía, con el mismo archivo, **no** se reencola, y el ejecutor
    automático devuelve `alreadyCurrent`;
  - cambiar solo la versión del catálogo (mismo archivo, mismo `mtime` y tamaño) **no** reencola
    (`JD7-A-002`);
  - dos syncs con un OCR que falla de forma definitiva: el segundo no crea tareas;
  - **primera extracción de un archivo nuevo** que agota los reintentos por timeout, **sin extracción
    publicada**: el segundo sync no crea tareas (`JD7-A-001`);
  - una falla de lectura sí se reencola;
  - `processing_retry` después de esa falla reusa los puntos de control: las llamadas al proveedor, sumadas
    entre intentos, no superan las páginas candidatas;
  - respuesta vacía en la página 1, timeout en la página 2 y reanudación: la página 1 se llama una sola vez
    (`JD7-A-004`);
  - el 429 espera sin agotar el ciclo;
  - con la clave ausente, la tarea queda `blocked` y se reanuda al guardar la clave.
- **2.2:**
  - verdadero para la p. 309 guardada (regla 1) y para su versión de PDFium (regla 2);
  - falso para ConflLlama con PDFium, para las referencias con enlaces y dominios, para prosa inglesa con
    compuestos y contracciones, para tablas en Markdown, para CJK y para el `l·l` catalán;
  - las pruebas del corpus siguen iguales;
  - `ocr_candidate_pages` incluye una página `rich` marcada.
- **2.3:**
  - en un adjunto sintético con `FakeOcrProvider`, las páginas que la vista previa informa son exactamente
    las que el ejecutor manda, y las de OCR guardado se descuentan;
  - el reuso convierte el HTML a Markdown;
  - en modo reproceso no se piden ventanas;
  - un PDF reemplazado por otro del **mismo tamaño** termina `reprocess_authorization_stale` con cero
    llamadas (`JD7-A-003`);
  - confirmar con una tarea viva da `busy`;
  - si una tarea automática se inserta entre la verificación y el `INSERT`, el choque da `busy` y nunca se
    suma a ella (`JD7-B-001`);
  - el contrato de reproceso pasa el reclamo y la verificación antes de publicar, y uno desconocido
    bloquea;
  - el lote de reproceso aparece en `list_batches` y se cancela con `processing_control`;
  - después de cancelar, el sync no lo reencola;
  - la garantía de costo, con interrupción, falla definitiva y `processing_retry`.
- **2.4:** después de un reproceso exitoso, el adjunto no aparece en `candidates`; si cambia el archivo,
  vuelve a aparecer.
- **2.5 (Vitest):**
  - el diálogo muestra los totales y el USD;
  - confirmar envía `{attachmentId, planHash}` y muestra encolados y `busy`;
  - Cancelar corta la vista previa;
  - sin candidatos no hay botón de confirmar.

## 4. Entregas (tareas)

| Tarea | Qué incluye |
|---|---|
| **B1** | Ningún reencolado automático sobre lo guardado, intento fallido por archivo, puntos de control que sobreviven a la falla y respuesta vacía guardada (2.1) |
| **B2** | Detector, su uso en `ocr_candidate_pages` y la medición sobre la copia de `prueba-sync` (2.2) |
| **B3** | `plan_reprocess`, `candidates`, `preview` con progreso y cancelación, y el monto estimado (2.3, 2.4) |
| **B4** | Contrato de reproceso, `confirm` con lote de usuario y admisión propia, y el modo reproceso del ejecutor (2.3) |
| **B5** | Interfaz en la Biblioteca y en la ficha (2.5) |
| **B6** | Verificación local, CI verde incluido Pro, juicio sobre el código y merge |

Ninguna entrega necesita migración. Cada tarea cierra con su commit en la rama.

## 5. Riesgos

- **El monto es una estimación.** El owner la contrasta con la factura de z.ai después del primer
  reproceso. Leer el uso de tokens de la respuesta queda fuera de este plan.
- **La vista previa de los 82 adjuntos** lee todas sus páginas con PDFium y calcula el SHA de cada archivo.
  Se espera menos de un par de minutos, con progreso y cancelación.
- **Un adjunto cuya primera extracción falló por OCR** queda sin texto hasta que el owner lo reprocese o lo
  reintente. Es el costo de no repetir cobros solos.
- **Un timeout puede cobrarse sin respuesta:** si GLM-OCR procesó la página pero la respuesta no llegó, el
  reintento la vuelve a pagar. Es inevitable desde el cliente y queda acotado por los 3 intentos por ciclo.
- **Los puntos de control de las extracciones fallidas quedan guardados.** Son textos de página, de tamaño
  acotado.
- **El detector cambia la candidatura de las extracciones automáticas** de archivos nuevos o cambiados, como
  el plan general acepta.

## 6. Dónde quedó cada hallazgo del juicio 7

| Hallazgo | Severidad | Dónde se trata |
|---|---|---|
| C1 (`A-005`, B): el lote `manual` es invisible | Alto (confirmado) | 2.3: lote de usuario propio, visible y cancelable |
| C2 (`A-001`, B): una falla definitiva borra los puntos de control y se vuelve a pagar | Alto (confirmado) | 2.1: intento fallido por archivo y puntos de control que sobreviven; 2.3: garantía de costo |
| `A-002`: la versión del catálogo dispara OCR | Alto (un juez) | 2.1: resuelta solo por el archivo; el intento se compara sin la versión |
| `A-003`: el plan liga el tamaño y no el contenido | Alto (un juez) | 2.3: `sourceSha256` en el hash |
| `A-004`: la respuesta vacía no queda en el punto de control | Alto (un juez) | 2.1: `Ok("")` dentro de `ctx.unit` |
| `B-001`: carrera en la confirmación | Alto (un juez) | 2.3: admisión propia, `INSERT` sin `OR IGNORE`, choque = `busy` |
| `A-006`: el 429 no agota | Aviso | 1 y 2.1 corregidos |
| B: hash circular | Aviso | 2.3: el hash no incluye el contrato |
| B: sin barra de progreso | Sugerencia | 2.5: progreso en texto |
| B: no se puede cancelar | Sugerencia | 2.3: lote propio cancelable |
| B: texto ilegible clasificado `empty` todavía se reencolaba | Sugerencia | 2.1: lo publicado nunca se reencola |
