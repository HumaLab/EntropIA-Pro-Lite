# Plan: texto nativo de los PDF de la Biblioteca

Estado: **propuesta para revisión. No hay nada implementado.**
Fecha: 2026-10-08 (revisión 3: enfoque simple, decidido por el owner después del segundo juicio).

## 0. Historial

- **Revisión 1.** Juicio: sin críticos, 6 altos. El diagnóstico apuntaba al extractor equivocado y no
  tenía condición de corte.
- **Revisión 2.** Juicio: sin críticos, 5 altos. Cada ronda encontró una nueva forma de que el
  **reproceso automático** volviera a cobrar OCR: la regla `unresolved_empty`, los recibos sin versión,
  los puntos de control sin los bytes, las páginas casi vacías y el arranque de PDFium.
- **Revisión 3 (este documento).** El owner eligió simplificar:
  - **corregir la extracción de ahora en adelante;**
  - **reprocesar lo ya guardado solo con una acción explícita del owner**, con el costo a la vista antes
    de correr;
  - **eliminar el reencolado automático por contenido.**

  Esta revisión no fue juzgada.

## 1. Problema y medición

- **El problema.** La bibliografía guarda texto nativo sin espacios entre palabras. Ejemplo: Abulafia
  1950, p. 309, `ArrozElcultivodelarrozaligualqueeldelgí.r'asoL…`. Ese texto se corta en pasajes y se
  embebe.
- **La magnitud** (perfil `prueba-sync`): 2693 de 19.244 páginas nativas `rich` (14 %), en 82 adjuntos,
  incluidos PDFs nacidos digitales (Meher y Brandt 2025).
- **La causa:**
  - las filas por página salen de **lopdf** (`read_native_page_texts`, `bibliography/processing.rs:3421+`),
    que pega las palabras;
  - el texto del documento sale de `pdf-extract`, y `richer_native_text` (`processing.rs:2912`) conserva el
    texto pegado cuando empata en caracteres alfanuméricos.
- **PDFium resuelve casi todo.** Extrae las mismas páginas con espacios en 3–30 ms. Con su texto, solo 4
  de las 2693 páginas siguen marcadas por una regla de palabras pegadas, y son páginas de referencias con
  enlaces.

## 2. Principio de diseño

**Ninguna regla automática dispara OCR pagado sobre lo ya guardado.** Una extracción se rehace sola
únicamente cuando cambia el archivo, como antes de `9b47a74`. Repararla por su contenido es una acción
explícita del owner, que ve el costo antes de confirmar y corre una sola vez.

## 3. Propuesta

### 3.1 Extracción correcta de ahora en adelante

- **Lector por página con PDFium.** `read_native_page_texts` obtiene el texto con PDFium
  (`page.text().all()`), y usa lopdf para la página donde PDFium no está disponible o falla.
  - **Marcas de guion de corte.** `\u{2}` (texto acotado) y `\u{FFFE}` se eliminan, junto con el salto de
    línea que las siga, y la palabra queda unida.
  - **Tope de tamaño** (`JD4-B-008`): el mismo límite por página que hoy aplica lopdf
    (`BIBLIOGRAPHY_PAGE_CONTENT_LIMIT_BYTES`) se aplica al texto de PDFium.
  - **Candado de PDFium:** una instancia por tanda de hasta 20 páginas, liberada entre tandas y nunca viva
    dentro de `maybe_ocr_pages`, porque el mutex no es reentrante. Esto evita la reentrada y que una lectura
    larga acapare el candado. **No promete** una espera máxima para el visor (`JD4-A-005`, `JD4-B-007`): el
    mutex de `pdfium-render` no tiene orden ni prioridad, y otros usos, como `render_pdf_pages`, también lo
    retienen por todo un documento.
- **Arranque de PDFium sin el runtime de ML** (`JD4-B-001`). Hace falta una función nueva, por ejemplo
  `ensure_pdfium_path_without_runtime(app)`, que resuelva solo la DLL incluida en el instalador (recursos
  de la app) y llene `PDFIUM_PATH` **sin** llamar a `RuntimeManager::ensure_ready_or_bootstrap`. La llaman:
  - el ejecutor de `bibliography_extract`, antes de leer páginas;
  - el arranque de la app, en Lite y en Pro.

  Si la DLL incluida no existe, se usa lopdf y queda registrado en el log. Nunca se arranca el runtime de
  ML desde un sync de fondo. Pruebas: con el caché frío, la primera extracción usa PDFium; en Pro no se
  llama al arranque del runtime.
- **El texto del documento sale de las páginas.** Para PDFs, `bibliographic_extractions.text_content` pasa
  a ser la unión de las páginas publicadas, en orden. La ficha y los pasajes quedan coherentes, y la trampa
  del empate desaparece.
- **Se borran las páginas que ya no existen** (`JD4-A-004`). Al publicar una extracción, las filas de
  `bibliographic_page_texts` con número de página mayor que el `page_count` nuevo se eliminan, y eso cuenta
  como cambio de páginas: se vuelve a pedir el perfil y se retiran los pasajes viejos. Prueba: un PDF de 3
  páginas reemplazado por uno de 2 con el mismo texto en las primeras dos.
- **Puntos de control atados a los bytes** (`JD4-A-002`). La huella que identifica los puntos de control
  del OCR (`attachment_extraction_fingerprint`, `processing/repository.rs:785`) incluye `source_bytes` y la
  fecha de modificación del archivo leído. Así, un OCR de una versión anterior del archivo nunca se publica
  con la nueva. Prueba: interrumpir, reemplazar el archivo y continuar.

### 3.2 Se elimina el reencolado automático por contenido

- `extraction_is_settled` deja de mirar si el texto guardado es ilegible: se quita la llamada a
  `stored_extraction_is_garbled`. Una extracción se rehace sola solo si cambia la identidad del archivo.
- **La regla `unresolved_empty` también se acota** (`JD4-A-001`). Una extracción `empty` se reencola para
  intentar OCR **una sola vez** por identidad de archivo: si ya existe una tarea terminada para esa
  identidad, haya fallado el OCR o no, queda resuelta. Los fallos por **configuración** (falta la clave de
  GLM-OCR) no cuentan como intento: siguen esperando, como hoy, y se reanudan al guardar la clave. Los
  fallos **transitorios** (429, timeout) tampoco cuentan: se reintentan con el mecanismo de reintentos de la
  cola, no con un sync nuevo (`JD4-B-005`). Prueba: dos syncs seguidos, con un OCR que falla de forma
  definitiva en todas las páginas; el segundo sync no crea tareas.
- **Lo que se mantiene de `9b47a74`:** una página **nueva** que el detector marca como ilegible durante
  una extracción sigue yendo a OCR en esa misma extracción. Lo que se elimina es solo el reencolado
  posterior por contenido.

### 3.3 Reproceso explícito de lo ya guardado

- **Una acción del owner:** "Reprocesar texto". Está en la ficha de cada obra, y en la pestaña Zotero para
  "todas las obras con texto dañado".
- **Primero, la vista previa del costo** (`JD4-A-003`, `JD4-B-003`). Un comando de solo lectura calcula,
  para los adjuntos elegidos, **exactamente las páginas que el ejecutor mandaría a OCR**. Usa la misma
  función de selección que el ejecutor (`ocr_candidate_pages`, ampliada con el detector) sobre el texto que
  PDFium extrae ahora. Incluye las páginas `sparse`, `empty` y, cuando corresponde, `unreadable`, y descuenta
  las que ya tienen OCR guardado. Muestra: adjuntos, páginas totales, páginas que van a GLM-OCR y páginas de
  OCR que se reusan.
- **El owner confirma.** Recién ahí se encolan las extracciones, con prioridad de trabajo manual.
- **Durante el reproceso:**
  - **se reusa el OCR guardado:** una página con fila `method = 'ocr'` para la misma identidad de archivo
    se conserva sin llamar a GLM-OCR;
  - **antes de reusarlas,** esas filas de OCR viejas pasan por la conversión de tablas HTML a Markdown de
    `fix/ocr-html-tables` (`JD4-B-006`), así todo el documento queda en un solo formato;
  - **solo páginas sueltas:** el OCR se pide por página, nunca en las ventanas de PDF completo que incluyen
    páginas limpias.
- **Una sola vez.** Una vez reprocesado con éxito, un adjunto no vuelve a aparecer como candidato hasta que
  cambie el archivo o el owner lo pida otra vez.

### 3.4 Detector de la bibliografía (solo para elegir candidatos)

El detector solo se usa en dos momentos: dentro de una extracción, para decidir qué páginas van a OCR, y en
la vista previa del reproceso. Nunca dispara un reencolado.

- **Función aparte:** `is_garbled_bibliography_text`. `is_garbled_text` e `is_quality_text` del corpus no
  cambian.
- **Regla 1, palabras pegadas:**
  - con al menos 80 letras **latinas**, para que no se aplique a escrituras sin espacios como el chino o el
    tailandés (`JD4-B-009`);
  - sin contar los tokens que son URL, DOI, correo o **dominio** (`algo.org`, `jstor.org`, `.com`, `.net`,
    `.edu`, `.gov` y TLD de dos letras, `JD4-B-004`);
  - se marca si el 50 % o más de las letras está en tokens de más de 24 letras.
- **Regla 2, ruido de OCR viejo:**
  - con al menos 40 tokens elegibles, de 4 letras o más, sin contar URL, DOI, correo ni dominio;
  - se marca si el 8 % o más tiene `.`, `~` o `·` entre dos letras. No cuentan `-` ni `'`, ni las
    abreviaturas con punto entre mayúsculas.
  - **Antes de mergear,** los umbrales se validan sobre las páginas limpias del perfil: menos de 0,5 % de
    falsos positivos.
- **`ocr_candidate_pages`** (`processing.rs:3383`) incluye, además de las `sparse` y `empty` de hoy, las
  páginas que marca este detector (`JD4-B-003`).

## 4. Pruebas

- **Lector:**
  - lopdf pegado → PDFium con espacios;
  - las marcas de guion se eliminan;
  - se respeta el tope por página;
  - si PDFium falla, se usa lopdf;
  - con el caché frío se usa PDFium sin arrancar el runtime de ML.
- **Documento:** `text_content` es la unión de las páginas, y tiene espacios aunque `pdf-extract` también
  pegue palabras.
- **Páginas eliminadas:** se borran y se vuelve a pedir el perfil.
- **Puntos de control:** interrumpir, reemplazar el archivo y continuar no publica OCR del archivo anterior.
- **Sin reencolado por contenido:**
  - un adjunto guardado con texto pegado y la misma identidad de archivo **no** se reencola;
  - la prueba de dos syncs de la sección 3.2 (OCR que falla de forma definitiva): el segundo no crea
    tareas;
  - un fallo por configuración espera y se reanuda al guardar la clave.
- **Vista previa:** sobre un adjunto sintético, la cantidad de páginas que informa es exactamente la
  cantidad de páginas que el ejecutor manda a OCR al confirmar, y las de OCR guardado se descuentan.
- **Reproceso:**
  - reusa las filas de OCR, convertidas a Markdown;
  - pide OCR solo página por página;
  - corre una vez y no vuelve a aparecer como candidato.
- **Detector:**
  - verdadero para la p. 309 guardada (regla 1) y para la versión de PDFium de esa p. 309 (regla 2);
  - falso para ConflLlama con PDFium, para las páginas de referencias con enlaces y dominios citadas por
    los jueces, para prosa inglesa con compuestos y contracciones, para tablas en Markdown y para texto CJK;
  - las pruebas del detector del corpus siguen iguales.

## 5. Riesgos

- **Orden del texto en columnas:** PDFium puede ordenar distinto el texto de documentos a varias columnas.
  Los pasajes nuevos cambian de posición; los citados en informes de Investigación guardan su propio texto.
- **Los 82 adjuntos siguen con texto pegado hasta que el owner los reprocese.** Es una decisión
  explícita: no se cobra nada sin su confirmación.
- **Si la DLL incluida de PDFium falta**, las extracciones nuevas usan lopdf, como hoy, y queda en el log.

## 6. Entregas

1. Arranque de PDFium sin el runtime de ML y lector por página con PDFium (guiones, tope y tandas).
2. Texto del documento desde las páginas, borrado de páginas eliminadas y puntos de control con bytes.
3. Sin reencolado por contenido, y `unresolved_empty` acotado.
4. Detector de la bibliografía y su uso en `ocr_candidate_pages`, con la medición de los umbrales.
5. Vista previa del costo, la acción "Reprocesar texto", el reuso del OCR guardado (convertido) y el OCR
   página por página.

Ninguna entrega necesita migración. La marca "ya reprocesado" queda en el recibo JSON de la tarea.

## 7. Dónde quedó cada hallazgo del segundo juicio

| Hallazgo | Severidad | Dónde se trata |
|---|---|---|
| `JD4-A-001`: `unresolved_empty` repite OCR si falla siempre | Alto | 3.2: un intento por identidad; configuración y fallos transitorios aparte |
| `JD4-A-002`: los puntos de control no incluyen los bytes | Alto | 3.1: huella con bytes |
| `JD4-A-003`, `JD4-B-003`: el costo y los candidatos no coinciden con el ejecutor | Alto / aviso | 3.3: la vista previa usa la selección del ejecutor; 3.4: `ocr_candidate_pages` ampliada |
| `JD4-A-004`: las páginas eliminadas no se borran | Alto | 3.1: se borran y se vuelve a pedir el perfil |
| `JD4-B-001`: el arranque de PDFium no estaba especificado | Alto | 3.1: `ensure_pdfium_path_without_runtime` |
| `JD4-A-005`, `JD4-B-007`: la espera del visor no está acotada | Aviso / sugerencia | 3.1: no se promete; solo se evitan la reentrada y el acaparamiento |
| `JD4-A-006`, `JD4-B-002`: selección del recibo y versión ausente | Aviso | Desaparece: ya no hay reencolado por versión del detector |
| `JD4-B-004`: dominios sin `http` | Aviso | 3.4: se excluyen los dominios |
| `JD4-B-005`: el tipo de fallo en la prueba de dos syncs | Aviso | 3.2: definitivo; configuración y transitorio documentados |
| `JD4-B-006`: OCR guardado en HTML mezclado con Markdown | Aviso | 3.3: se convierte antes de reusar |
| `JD4-B-008`: sin tope para el texto de PDFium | Sugerencia | 3.1: mismo tope que lopdf |
| `JD4-B-009`: escrituras sin espacios | Sugerencia | 3.4: la regla 1 solo para letras latinas |
