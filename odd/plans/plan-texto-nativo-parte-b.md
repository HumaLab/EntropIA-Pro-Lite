# Plan: texto nativo, parte B — reparar lo ya guardado

Estado: **propuesta para juicio. No hay nada implementado.**
Fecha: 2026-10-08. Rama `feat/native-text-reprocess`, desde main `49a5f42`.

Parte de `odd/plans/plan-texto-nativo.md` (revisión 3, secciones 3.2 a 3.4). La parte A ya está en main
(`odd/tasks/native-text-part-a.md`): PDFium lee las páginas con espacios de ahora en adelante y no cambió
ninguna regla de OCR.

## 0. Decisiones del owner

- **Ningún OCR pagado automático sobre lo ya guardado.** Cada cobro lo aprueba el owner, viendo el costo
  antes.
- **La vista previa muestra páginas y un monto estimado en USD**, marcado como estimado (2026-10-08).
- **La acción masiva va en la barra de herramientas de la Biblioteca** (`BibliotecaView`). La ficha de cada
  obra tiene la acción individual (2026-10-08).
- **Sin migración** sin preguntar, y homogeneidad visual estricta.

## 1. Estado actual (medido en el código)

- **Reencolado por contenido.** `extraction_is_settled` (`bibliography/repository.rs:2254`) da por no
  resuelta una extracción cuyo texto guardado marca `stored_extraction_is_garbled` (`:2205`). Lo usan:
  - la admisión del sync (`processing/repository.rs:2346`);
  - el atajo `alreadyCurrent` del ejecutor (`bibliography/processing.rs:2902`).
- **`unresolved_empty`.** Una extracción `empty` sigue sin resolver hasta que **alguna** tarea terminada
  diga `"ocrAttempted":true` (`repository.rs:2266-2274`). La consulta no está atada a la identidad del
  archivo, y una tarea que agotó los reintentos (`failed`) no cuenta.
- **Falta de clave.** El proveedor responde "not configured", la tarea queda `blocked`
  (`configuration_required_ocr`, `selective_ocr.rs:319-360`) y se reanuda al guardar la clave
  (`settings.rs:1335`). Es una tarea **viva**: nunca deja una extracción `empty` publicada sin intento.
- **Cola.** Hay una sola tarea viva por adjunto (`idx_processing_tasks_subject_active_unique`, migración
  0052). Las terminadas no bloquean una nueva y nunca se borran.
  - La admisión se suma a la tarea viva si existe (`admit_bibliography_attachment_extract_or_attach`,
    `processing/repository.rs:676`).
  - Una tarea guarda `kind`, `input_fingerprint` y `contract_hash`; no tiene columna de entrada.
  - `kind` tiene un CHECK, así que un tipo nuevo **necesita migración**.
- **Contrato.** `BIBLIOGRAPHY_EXTRACT_CONTRACT = "bibliography-extract-v1"` se compara por igualdad en tres
  lugares:
  - la validación del reclamo (`processing/repository.rs:4503`);
  - la verificación antes de publicar (`:5181`);
  - el ejecutor (`bibliography/processing.rs:2798`).

  Los puntos de control se guardan por `(task_id, unit_key, input_fingerprint, contract_hash)`.
- **OCR.** `ocr_candidate_pages` (`processing.rs:3559`) elige las páginas `sparse` y `empty`, y las
  `unreadable` solo si el documento es `native_blank`. `maybe_ocr_pages` prueba primero ventanas de PDF
  completo cuando hacen falta OCR la mitad de las páginas o más (`should_use_pdf_mode`,
  `selective_ocr.rs:288`).
- **Precio.** No hay ningún cálculo. GLM-OCR cobra USD 0,03 por millón de tokens, de entrada y de salida
  (docs.z.ai, *pricing*), y el cliente no lee el uso de tokens de la respuesta.
- **Detector propio.** No existe `is_garbled_bibliography_text`.

## 2. Diseño

### 2.1 Sin reencolado por contenido (plan 3.2)

- **`extraction_is_settled` deja de llamar a `stored_extraction_is_garbled`.** La función se borra si no
  queda otro uso. El texto ilegible ya guardado solo se repara con la acción del owner (2.3).
- **`unresolved_empty` pasa a ser un intento por identidad.** Una extracción `empty` que coincide con el
  archivo queda resuelta si existe **cualquier tarea terminal** (`succeeded`, `failed`, `skipped`,
  `cancelled`) de `bibliography_extract` para ese adjunto con el `input_fingerprint` **actual**. Cada caso
  queda así:

  | Caso | Qué pasa |
  |---|---|
  | Falta de clave | Tarea `blocked` y viva, no terminal: espera y se reanuda al guardar la clave, igual que hoy |
  | Fallo transitorio (429, timeout) | Lo reintenta la cola, con backoff y con los puntos de control de la misma tarea. Si se agotan los reintentos, la tarea queda `failed`: cuenta como intento y el sync no la repite. El owner puede reintentar con `processing_retry` o con la acción de reproceso |
  | Fallo definitivo en todas las páginas | La tarea termina `succeeded` con `ocrFailedPages`: cuenta como intento |
  | Cancelada por el owner | Cuenta como intento: un sync no reinicia un OCR cancelado |
  | Archivo nuevo o cambiado (cambia la huella) | Un intento nuevo, como siempre |

- **Se mantienen** las reglas del texto real después de convertir el markup y del HTML vacío como
  definitivo. Ya casi no hacen falta, pero no cuestan nada.
- **No cambia** `attachment_extraction_fingerprint` (`JD5-A-002`).
- **Se mantiene de `9b47a74`:** dentro de una extracción, una página que el detector marca va a OCR en esa
  misma extracción. Esto aplica a la primera extracción de un archivo nuevo o cambiado, nunca a un
  reencolado por contenido.

### 2.2 Detector de la bibliografía (plan 3.4)

- **Ubicación:** `is_garbled_bibliography_text(text) -> bool`, en `ocr/pdf.rs` junto a `is_garbled_text`,
  con `BIBLIOGRAPHY_DETECTOR_VERSION: u32 = 1`. `is_garbled_text` e `is_quality_text` no cambian.
- **Tokens elegibles.** Se descartan URL, DOI, correo y dominio (`algo.org`, `.com`, `.net`, `.edu`,
  `.gov`, TLD de dos letras).
- **Regla 1, palabras pegadas.**
  - Exige al menos 80 letras latinas (`char::is_alphabetic` con script latino: rangos ASCII, Latin-1 y
    Latin Extended).
  - Marca la página si el 50 % o más de esas letras está en tokens de más de 24 letras.
- **Regla 2, ruido de OCR viejo.**
  - Exige al menos 40 tokens elegibles de 4 letras o más.
  - Marca la página si el 8 % o más tiene `.`, `~` o `·` entre dos letras.
  - No cuentan `-`, `'`, el `l·l` catalán, las abreviaturas con punto entre mayúsculas (`U.S.A`) ni
    `e.g.`/`i.e.`.
- **Uso.** El detector entra en `ocr_candidate_pages`: una página marcada es candidata como una `sparse`.
  Se aplica sobre el texto nativo que la extracción publicaría, convertido el markup. No toca
  `extraction_quality` ni el grado guardado.
- **Validación de umbrales antes de mergear.**
  - Se mide sobre una **copia de solo lectura** de la base del perfil `prueba-sync`, nunca sobre el
    original.
  - Para cada página nativa `rich` se calcula el texto de PDFium y se aplica el detector.
  - Se informa cuántas páginas marca cada regla, y 30 páginas marcadas al azar quedan para que el owner las
    mire.
  - Criterio: menos del 0,5 % de falsos positivos sobre las páginas que el owner considera limpias. Si no
    se cumple, se ajustan los umbrales antes de seguir.

### 2.3 Reproceso explícito (plan 3.3)

#### Modo durable sin migración

- **Es la misma tarea `bibliography_extract`** con otro contrato:
  `contract_hash = "bibliography-extract-reprocess-v1|<planHash>"`.
- **Las tres comparaciones de contrato** aceptan `bibliography-extract-v1` o ese prefijo. Una función
  `parse_extract_contract(&str) -> Option<ExtractMode>` devuelve `Automatic` o `Reprocess { plan_hash }`.
  Cualquier otro valor sigue bloqueando como hoy.
- **El modo sobrevive a reinicios y reintentos** porque vive en la fila de la tarea. Los puntos de control
  de un reproceso no se mezclan con los de una tarea automática, porque la clave incluye el contrato.
- **No se suma a una tarea automática** (`JD5-A-005`, `JD5-B-001`). Si al confirmar el adjunto tiene una
  tarea viva, se rechaza con `busy` y la interfaz lo informa; nunca se encola detrás de ella.
  - Al revés, una admisión automática que encuentra un reproceso vivo se suma a él, como hoy. No cobra
    nada fuera de lo autorizado, porque el ejecutor sigue el plan del reproceso.

#### El plan, una sola función para la vista previa y el ejecutor

`plan_reprocess(input) -> ReprocessPlan` es pura y está en `bibliography/processing.rs`.

- **Entrada:**
  - las páginas nativas que lee el lector de la parte A sobre los bytes actuales (PDFium por página, con
    lopdf como alternativa);
  - `native_blank`, calculado como hoy;
  - las filas guardadas de `bibliographic_page_texts`, con la identidad guardada del archivo;
  - la identidad actual del archivo: `mtime` del catálogo y bytes.
- **Salida:**
  - `page_count`;
  - `ocr_pages`: las páginas que irían a GLM-OCR;
  - `reused_ocr_pages`, con el `text_hash` de cada una;
  - `fixed_without_ocr`: las páginas que hoy marca el detector y que PDFium ya arregla;
  - `plan_hash`.
- **Selección:**
  1. Si la extracción guardada coincide con el archivo (`extraction_matches_source`), cada página con fila
     `method = 'ocr'` se **reusa**: no va a GLM-OCR, aunque esté vacía (es una respuesta del proveedor).
  2. Del resto, van a OCR las que elige `ocr_candidate_pages`, ya ampliada con el detector (2.2).
- **`plan_hash`.** Es el SHA-256 del JSON canónico, con claves ordenadas, de:
  `attachmentId`, `sourceMtime`, `sourceBytes`, `pageCount`, `ocrPages`, `reusedOcr` (página y
  `textHash`), `detectorVersion` y `contract`.
- **El ejecutor no vuelve a elegir páginas.** En modo reproceso manda a OCR exactamente
  `plan.ocr_pages`. Así, la cantidad que mostró la vista previa es la que se cobra.

#### Comandos nuevos (Tauri, `bibliography/commands.rs`)

1. **`bibliography_reprocess_candidates()`**, de solo lectura y barato (SQL más el detector sobre el texto
   guardado).
   - Devuelve los adjuntos PDF con al menos una página guardada que marca `is_garbled_bibliography_text` o
     `is_garbled_text`, después de convertir el markup.
   - Excluye los que tienen un reproceso `succeeded` para la identidad actual (2.4).
2. **`bibliography_reprocess_preview(attachmentIds)`**, de solo lectura.
   - Lee cada PDF con PDFium, en tandas de 20 páginas como en la parte A, y corre `plan_reprocess`.
   - Corre en un hilo de bloqueo, informa el progreso por evento (`n de m adjuntos`) y se cancela con
     `bibliography_reprocess_preview_cancel`.
   - Por adjunto devuelve: obra, `pageCount`, `ocrPages.len()`, `reusedOcrPages.len()`,
     `fixedWithoutOcr.len()`, `planHash` y `busy`.
   - En total devuelve: adjuntos, páginas, páginas a GLM-OCR, OCR reusado y USD estimado.
   - Un adjunto que no se puede leer (archivo ausente, demasiado grande) aparece con su motivo y no se
     puede confirmar.
3. **`bibliography_reprocess_confirm(entries: [{attachmentId, planHash}])`**.
   - Admite cada entrada en el lote `manual`, que tiene prioridad interactiva 2, con el contrato de
     reproceso.
   - Devuelve, por entrada, `queued` o `busy`.
   - No vuelve a leer los PDF: la verificación del plan ocurre en el ejecutor.

#### Ejecutor en modo reproceso (`bibliography_extract`)

1. **No usa el atajo** `extraction_is_settled`/`alreadyCurrent`: el archivo coincide por definición.
2. **Recalcula el plan** con los bytes leídos. Si el hash no coincide con el del contrato, termina `Fatal`
   con `reprocess_authorization_stale` **antes de cualquier llamada a GLM-OCR** (`JD5-A-006`). Puede pasar
   porque cambió el archivo, cambiaron las filas de OCR guardadas o cambió la versión del detector. El
   owner vuelve a pedir la vista previa.
3. **Las páginas reusadas** toman el texto guardado convertido con `ocr_markup_to_text` (`JD4-B-006`),
   `method = 'ocr'` y un hash nuevo del texto convertido.
4. **OCR solo por página.** En modo reproceso no se usan ventanas de PDF completo: `maybe_ocr_pages` recibe
   la lista exacta y el modo, y salta la rama `should_use_pdf_mode`. Cada página guarda su punto de
   control `ocr-page:{n}`, como hoy.
5. **Publica como siempre.** El documento es la unión de las páginas, se borran las páginas que sobran y se
   vuelve a pedir el perfil si cambiaron las páginas (parte A).
6. **El recibo suma** `"reprocess": {"planHash", "detectorVersion"}`, `sourceMtime` y `sourceBytes`.

#### Monto estimado

- `estimated_usd = ocr_pages × ESTIMATED_TOKENS_PER_PAGE × 0,03 / 1.000.000`, con
  `ESTIMATED_TOKENS_PER_PAGE = 3000` (entrada más salida).
- Las constantes y la fuente del precio quedan documentadas en el código.
- La interfaz muestra "≈ USD 0,24 (estimado)" y, debajo, la tarifa y el supuesto de tokens por página.
- Con 0 páginas a OCR muestra "sin costo de OCR".

### 2.4 Una sola vez

- **La lista masiva** (`bibliography_reprocess_candidates`) excluye un adjunto si tiene un reproceso
  `succeeded` cuyo recibo coincide con la identidad actual (`sourceMtime`, `sourceBytes`) y con
  `detectorVersion`.
- **La acción de la ficha siempre está disponible:** es el "el owner lo pide otra vez".
- **El sync nunca mira esa marca:** no hay reencolado por contenido (2.1).

### 2.5 Interfaz

- **Componentes.** Solo los existentes de `@entropia/ui`: `Button`, `ActionIcon` (con un ícono nuevo en
  `ACTION_ICON_NAMES` si hace falta) y `ConfirmDialog`, con el patrón de `PassageReaderDialog`. Sin estilos
  nuevos, salvo la grilla de números, que usa los tokens existentes.
- **Biblioteca, barra de herramientas:** botón "Reprocesar texto".
  1. Llama a `candidates`.
  2. Abre el diálogo y corre `preview` con barra de progreso y botón Cancelar.
  3. Muestra el resumen: adjuntos, páginas, páginas a GLM-OCR, OCR reusado, páginas que se arreglan sin
     OCR y el USD estimado.
  4. Lista los adjuntos `busy` o ilegibles, que no se encolan.
  5. El botón de confirmar dice "Reprocesar (≈ USD x)".

  Sin candidatos, el diálogo lo dice y no ofrece confirmar.
- **Ficha de la obra:** el mismo botón en la barra de la ficha, con el mismo diálogo y los adjuntos PDF de
  la obra (no solo los candidatos).
- **Idioma.** Los textos van por `t(...)`, con claves en español y en inglés, como el resto de la
  Biblioteca.
- **Progreso del reproceso.** Lo informa el indicador de lotes existente (lote `manual`); no hace falta UI
  nueva.

## 3. Pruebas

Las pruebas van primero cuando se pueden escribir: rojo, verde y después refactor.

- **2.1:**
  - un adjunto con texto pegado guardado y la misma identidad **no** se reencola;
  - el ejecutor automático con texto ilegible guardado devuelve `alreadyCurrent`;
  - dos syncs con un OCR que falla de forma definitiva: el segundo no crea tareas;
  - dos syncs con un fallo transitorio que agota los reintentos: el segundo no crea tareas;
  - con la clave ausente, la tarea queda `blocked` y se reanuda al guardar la clave;
  - una tarea terminal con una huella **anterior** no resuelve el `empty` de un archivo nuevo.
- **2.2:**
  - verdadero para la p. 309 guardada (regla 1) y para su versión de PDFium (regla 2);
  - falso para ConflLlama con PDFium, para las páginas de referencias con enlaces y dominios, para prosa
    inglesa con compuestos y contracciones, para tablas en Markdown, para CJK y para el `l·l` catalán;
  - las pruebas del detector del corpus siguen iguales;
  - `ocr_candidate_pages` incluye una página `rich` marcada.
- **2.3:**
  - en un adjunto sintético con `FakeOcrProvider`, las páginas que la vista previa informa son exactamente
    las que el ejecutor manda al proveedor, y las de OCR guardado se descuentan;
  - una fila de OCR en HTML se reusa convertida a Markdown;
  - en modo reproceso no se piden ventanas aunque haga falta OCR en más de la mitad de las páginas;
  - con un hash que no coincide, termina `reprocess_authorization_stale` con cero llamadas al proveedor;
  - confirmar con una tarea viva devuelve `busy` y no crea ni modifica tareas;
  - el contrato de reproceso pasa el reclamo y la verificación antes de publicar;
  - un contrato desconocido sigue bloqueando;
  - interrumpir y reanudar un reproceso reusa sus puntos de control sin volver a pagar.
- **2.4:** después de un reproceso exitoso, el adjunto no aparece en `candidates`; si cambia el archivo,
  vuelve a aparecer.
- **2.5 (Vitest):**
  - el diálogo muestra los totales y el USD;
  - confirmar envía `{attachmentId, planHash}`;
  - Cancelar corta la vista previa;
  - sin candidatos no hay botón de confirmar.

## 4. Entregas (tareas)

| Tarea | Qué incluye |
|---|---|
| **B1** | Sin reencolado por contenido y `unresolved_empty` como un intento por identidad (2.1) |
| **B2** | Detector de la bibliografía, su uso en `ocr_candidate_pages` y la medición de umbrales sobre la copia de `prueba-sync` (2.2) |
| **B3** | `plan_reprocess`, `candidates`, `preview` con progreso y cancelación, y el monto estimado (2.3, 2.4) |
| **B4** | Contrato de reproceso, `confirm` y el modo reproceso del ejecutor: plan, reuso convertido y OCR solo por página (2.3) |
| **B5** | Interfaz en la Biblioteca y en la ficha (2.5) |
| **B6** | Verificación local, CI verde incluido Pro, juicio sobre el código y merge |

Cada tarea cierra con su commit en la rama.

## 5. Riesgos y preguntas abiertas

- **El monto es una estimación.** El owner la contrasta con la factura de z.ai después del primer
  reproceso. Si la respuesta de GLM-OCR trae el uso de tokens, leerlo y registrarlo es una mejora
  posterior, fuera de este plan.
- **La vista previa de los 82 adjuntos lee todas sus páginas con PDFium** (3–30 ms por página). Se espera
  menos de un par de minutos, con progreso y cancelación.
- **Una admisión automática que encuentra un reproceso vivo se suma a él** y publica el resultado del
  reproceso. Es aceptable porque no cobra fuera de lo autorizado.
- **Un plan que queda viejo entre la vista previa y la ejecución termina en `Fatal`, no en `blocked`.** Así
  no queda una tarea viva que bloquee una nueva confirmación. El owner ve el motivo y repite la vista
  previa.
- **El detector cambia la candidatura de las extracciones automáticas de archivos nuevos o cambiados.** Es
  el comportamiento que el plan general acepta (página nueva marcada → OCR en esa extracción).
