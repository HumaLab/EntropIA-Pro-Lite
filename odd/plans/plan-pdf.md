# Plan: carga de PDF en los visores de la Biblioteca

Estado: **propuesta para revisión. No hay nada implementado.**
Fecha: 2026-10-08 (revisión 4: alcance ampliado a todos los visores de PDF).

## 0. Historial de esta propuesta

- **Revisión 1.** Proponía dibujar las páginas con PDFium en Rust y guardarlas en un caché en disco.
- **Juicio del día final** (dos jueces a ciegas): `JUDGMENT: APPROVED` tras una ronda de corrección.
  - **Los dos hallazgos críticos** (`JD-A-001` y `JD-B-001`) eran el mismo problema: retener una instancia de
    PDFium colgaba el OCR, las miniaturas y las ediciones. Se corrigieron y los dos jueces los verificaron.
  - **Hallazgos informativos.** Hubo 3 altos y 9 avisos. Esta revisión los incorpora; la tabla de la
    sección 9 dice dónde.
- **Revisión 2 (este documento).**
  - **Nueva medición** (sección 2): activando una opción de pdf.js, la página más lenta pasa de 21 s a
    1,1 s.
  - **Decisión de alcance del owner** (sección 3): solo los visores conectados a Zotero y a la Biblioteca.
  - **Consecuencia:** el plan se reordena. La fase 1 es ajustar pdf.js, y PDFium queda como contingencia
    (fase 3), con el diseño ya corregido en el juicio.
  - **Fuera del juicio:** esta revisión no fue juzgada. El juicio aprobó la revisión 1 corregida.
- **Segundo juicio, sobre la revisión 2** (pedido por el owner, con foco en el alcance):
  `JUDGMENT: APPROVED`, sin hallazgos críticos. Hubo 1 hallazgo alto, 6 avisos y 3 sugerencias.
- **Revisión 3 (este documento).** Incorpora los hallazgos del segundo juicio (tabla de la sección 9b) y
  la investigación del hallazgo alto:
  - **los dos bugs de Chromium** por los que pdf.js apagaba `ImageDecoder` se corrigieron alrededor de
    Chrome 132–133;
  - **pdf.js lo volvió a activar por defecto en Chrome** el 1/5/2026 (mozilla/pdf.js#20961);
  - **el WebView2 del owner es la versión 154.**

  Los cambios de la revisión 3 no fueron juzgados.

## 1. Problema

En la vista de obra de la Biblioteca, algunas páginas tardan una eternidad en aparecer o se quedan en
"Cargando PDF…". Al volver a una página ya vista, vuelve a tardar lo mismo. Las páginas afectadas son
las que tienen imágenes.

## 2. Medición

Las mediciones se hicieron en la máquina del owner con el PDF real: *Comparative Performance of YOLOv8…*,
16 páginas, **128 MB**. Edge headless usa el mismo motor Chromium que el WebView2 de la app, con
pdfjs-dist 4.10.38.

| Página | Imágenes embebidas |
|---|---|
| 6 | 3 JPEG; la mayor es de 7050×9300 (65 megapíxeles) |
| 11 | Más de 10 JPEG de 2650×3950 a 3244×4606 (10–15 megapíxeles cada una) |
| 1, 4, 7, 9 | Sin imágenes |

Tiempo de dibujo al alto del visor (~700 px):

| Caso | pdf.js como hoy | pdf.js con `isImageDecoderSupported: true` | PDFium (referencia) |
|---|---|---|---|
| Abrir el documento | 1,5 s | 1,5 s | 1 ms |
| Página 6, primera vez | 7,0 s | **0,6 s** | 0,6 s |
| Página 11, primera vez | 21,1 s | **1,1 s** | 1,8 s |
| Página 11, re-dibujo inmediato | 0,5 s | 0,8 s | — |
| Página 12 (texto) | 65 ms | 119 ms | — |
| Página 11, al volver 7 s después | 20,7 s | **0,8 s** | ~0 con caché |

Qué distingue cada caso (esto corrige `JD-A-003`):

- **Decodificación en frío.** Es lo que domina hoy: 7–21 s.
- **Reuso inmediato.** pdf.js conserva las imágenes decodificadas unos segundos: 0,5 s.
- **Reuso diferido.** Pasados unos 5 s, pdf.js libera esas imágenes y vuelve a decodificar en frío: 20,7 s.
- **Configuración del decodificador.** Es la palanca principal. En pdf.js 4.10 el valor por defecto es
  `isImageDecoderSupported = !isNode && (isFirefox || !globalThis.chrome)`, es decir, **falso en
  Chromium/WebView2**. La app hoy decodifica los JPEG con el decodificador en JavaScript de pdf.js.
  Activando la opción, usa el decodificador nativo del navegador (`ImageDecoder`).

Los scripts para reproducir la medición están en `G:\EntropIA-Stack\agent-scratch\pdfbench-scripts\` y en
`agent-scratch\pdfbench-web\` (`bench.html`, `bench2.html`). Los resultados quedaron en
`agent-scratch\pdfbench\pdfjs-result*.txt`.

## 3. Alcance (decisión del owner, revisión 4)

El owner amplió el alcance después del segundo juicio. Los jueces habían señalado (`JD2-A-004`,
`JD2-B-003`) que el Navegador puede copiar un mismo PDF a una colección y a Zotero, y que con el alcance
anterior ese archivo se vería rápido en la Biblioteca y lento en Colecciones.

**La decodificación rápida (fase 1) se aplica a todos los lugares donde la app dibuja un PDF con pdf.js:**

| Visor | `ImageDecoder` (fase 1) | Pestaña montada, sin dibujos ocultos, caché (fase 2) |
|---|---|---|
| Biblioteca: pestaña Original (`BibliographyWorkView`) | sí | sí |
| Biblioteca: "Abrir original" de pasajes (`PassageOriginalViewer`) | sí | caché de la fase 2, si se hace |
| Colecciones: vista de documento (`ItemAssetPanel`) | sí | no; ya está montada |
| Navegador (`NavegadorPdfViewer`) | sí | no |
| Vista previa de similares (`SimilarAssetPreviewDialog`) | sí | no |
| Dibujo de páginas para el texto enriquecido del OCR (`lib/ocr-rich-text.ts`) | sí | no |

Por qué el riesgo de ampliar es bajo:

- la opción de pdf.js solo cambia **cómo se decodifican los JPEG y los BMP**;
- no toca la geometría, las anotaciones, los recortes, la rotación ni las herramientas de edición de
  Colecciones: la página sale igual, más rápido;
- queda sujeta a la condición de Chromium 134 o posterior.

Los cambios que dependen de cómo está armada cada vista (pestaña montada, sin dibujos con el contenedor
oculto, caché de la fase 2) quedan solo en la Biblioteca, como en la revisión 3.

El barrido de los jueces confirmó que estos son todos los puntos donde se dibuja un PDF con pdf.js. Los dos
lectores de pasajes (Chat y búsqueda bibliográfica) pasan por `PassageOriginalViewer`, y la ficha de
Zotero en Escritura solo lista nombres de adjuntos.

## 4. Causas

1. **El decodificador de JPEG en JavaScript.** pdf.js decodifica cada foto a resolución completa con su
   decodificador en JavaScript, porque en Chromium el nativo viene apagado por defecto. Es la causa
   dominante.
2. **El reuso diferido no sirve.** Volver a una página después de unos segundos vuelve a decodificar todo.
3. **Cambiar de pestaña reabre el documento.** El visor se desmonta y se monta de nuevo, y vuelve a leer los
   128 MB (el "Cargando PDF…").
4. **Una página lenta demora a las siguientes.** Los dibujos se hacen de a uno (`aacb702`), y una
   decodificación larga en curso demora el siguiente dibujo. Con decodificación rápida, este efecto se
   vuelve marginal.
5. **El archivo entero pasa por el protocolo de assets** (1,5 s para 128 MB).

## 5. Propuesta en fases

### Fase 1 — Ajustar pdf.js en todos los visores (principal)

- **Una sola función compartida** decide la opción: por ejemplo, `pdfDocumentOptions(url)` en
  `packages/ui`. Devuelve `{ url, isImageDecoderSupported: true }` si el motor es Chromium 134 o posterior y
  `{ url }` si no. La usan las dos llamadas a `getDocument` que existen hoy: `DocumentViewer.svelte` y
  `apps/desktop/src/lib/ocr-rich-text.ts`. Así todos los visores de la sección 3 quedan cubiertos sin
  tocar cada pantalla, y no puede quedar uno rápido y otro lento.
- **Revisión a ojo en Colecciones.** Como ahora también aplica ahí, la muestra de la salvaguarda incluye
  documentos del corpus con anotaciones, regiones de layout, recortes y rotación. La geometría no debería
  cambiar, pero se confirma viendo.
- **El motivo del bloqueo en Chromium, ya documentado** (`JD2-A-001`). Según la nota de
  `pdfjs-dist/types/src/display/api.d.ts:146–154`, son dos bugs de Chromium:
  - el decodificador **BMP cierra el proceso** con imágenes enormes (crbug 374807001, `issue6741.pdf`);
  - el decodificador **JPEG dibuja mal** imágenes con perfil de color propio (crbug 378869810).

  La opción no es solo para JPEG: también activa el decodificador nativo de BMP. Un cierre del proceso no
  pasa por la vía de error de pdf.js, así que no hay caída ordenada a la decodificación lenta. Lo que se
  sabe hoy:
  - los dos bugs se corrigieron alrededor de Chrome 132–133;
  - pdf.js volvió a activar `ImageDecoder` por defecto en Chrome el 1/5/2026 (mozilla/pdf.js#20961),
    cuando subió su mínimo soportado a Chrome 125;
  - el WebView2 del owner es la versión 154.
- **Condición de versión.** La opción se activa solo si el motor es Chromium **134 o posterior**. La
  versión se lee en el navegador con `navigator.userAgentData` y, si no está disponible, del user agent
  (`Edg/NNN` o `Chrome/NNN`). En versiones anteriores el visor sigue como hoy. Así un WebView2 viejo en otra
  máquina no queda expuesto a esos bugs.
- **Una salvaguarda de corrección.** Antes de mergear:
  - revisar a ojo, con y sin la opción, una muestra de PDFs reales de la biblioteca del owner: escaneos de
    archivo, revistas, libros, el PDF de 128 MB, **al menos un JPEG con perfil de color propio y, si
    aparece, un PDF con imágenes BMP grandes**;
  - si se ve un error de color o de dibujo, se descarta la fase.
- **Lo que la fase 1 no acelera** (`JD2-B-001`). pdf.js 4.10 manda a `ImageDecoder` solo los JPEG **sin**
  `/SMask`, `/Mask` ni arreglo `/Decode` (`pdf.worker.mjs:30024`), y además los BMP. Las fotos con máscara,
  las JPEG2000 (JPX) y las PNG siguen con el decodificador lento. La muestra tiene que incluir alguna página
  así, para saber cuánto de la biblioteca real queda afuera. Si es mucho, eso justifica la fase 3.
- **El visor queda montado al cambiar de pestaña.** En `BibliographyWorkView`, la pestaña Original se oculta
  en lugar de desmontarse, como ya hace `ItemAssetPanel` con su clase `is-hidden`. Así, volver a Original
  no reabre los 128 MB.
- **Sin dibujos mientras está oculto** (`JD2-A-002`, `JD2-B-002`). Al ocultarse, el contenedor mide 0×0.
  El `ResizeObserver` de `DocumentViewer` dispara entonces un redibujo, y `pdfFitScale` devuelve escala 1
  para un contenedor vacío: se dibuja una página entera a tamaño natural (multiplicado por el zoom) que
  nadie ve. Además, el redibujo al volver queda en cola detrás de ese. Con la opción de la Biblioteca
  activada, el visor no dibuja mientras su contenedor mide 0×0, y redibuja una sola vez al volver a tener
  tamaño. El comportamiento de los demás visores no cambia.
- **Sin cambios en Rust**, sin caché en disco y sin comandos nuevos.

### Fase 2 — Retener lo ya dibujado (si la fase 1 no alcanza para "volver a una página")

- **Opción A.** Un caché acotado en memoria, del lado del visor, con las últimas páginas dibujadas como
  `ImageBitmap` (`JD2-A-003`):
  - **Clave:** generación del documento (cambia al elegir otro adjunto de la misma obra), número de página y
    geometría del dibujo (ancho y alto en píxeles, que dependen del zoom y del tamaño del contenedor). Así no
    se muestra la página del adjunto anterior ni una imagen de otro tamaño.
  - **Límite en bytes**, no en cantidad de páginas (por ejemplo, 150 MB), porque el área crece con el
    cuadrado del zoom. Una entrada más grande que el límite no se guarda.
  - **Liberación explícita** con `ImageBitmap.close()` al reemplazar, al desalojar y al cerrar el visor.
  - Se libera todo al cerrar la obra.
- **Opción B.** Adelantar en segundo plano la página siguiente después de dibujar la actual. Solo se hace si
  no compite con la interacción.
- Las dos se activan solo en los visores de la Biblioteca.

### Fase 3 — Contingencia: dibujo nativo con PDFium (solo si las fases 1 y 2 no cumplen la sección 6)

Este es el diseño de la revisión 1 con las correcciones del juicio. Solo se implementa si las metas de la
sección 6 no se alcanzan con las fases 1 y 2.

- **Sin estado de PDFium entre pedidos** (corrige `JD-A-001` y `JD-B-001`). Cada pedido hace todo el
  ciclo: crea la instancia con `get_pdfium()`, abre el archivo con `load_pdf_from_file`, dibuja una página,
  guarda la imagen y libera todo.
  - **Por qué:** `pdfium-render` 0.8.37 se compila con `thread_safe` por defecto. `Pdfium::new` toma un
    mutex global del proceso y solo lo suelta al destruir la instancia. Retenerla bloquearía sin límite
    `generate_pdf_thumbnail`, `probe_pdf`, `crop_pdf`/`edit_pdf`, `render_pdf_pages`, `llm_correct_ocr` y
    el OCR de la bibliografía.
  - **Costo:** abrir el documento cuesta ~1 ms (medido con pypdfium2; hay que confirmarlo en Rust).
  - **Contención:** el candado se toma y se suelta en cada dibujo. Cada pedido espera, como máximo, un
    dibujo por cada otro pedido que tenga delante: 0,6–1,8 s en las páginas más pesadas medidas. Con
    páginas más pesadas el OCR podría tardar más, y eso no se midió.
- **Iniciar PDFium sin arrancar el runtime de ML** (`JD-A-005`). En Pro, `init_pdfium_path`
  (`ocr/pdf.rs:111–178`) prueba primero `managed_runtime_root_for_pdfium`, que puede descargar y activar el
  runtime de ML. Para el visor, la resolución de PDFium tiene que usar solo la DLL incluida en el instalador
  y nunca esperar ese arranque. Hay que verificarlo en Pro y en Lite, sin conexión y con un perfil nuevo.
- **Identidad del documento y seguridad** (corrige `JD-A-002` y `JD-B-006`). El único identificador que
  aceptan los comandos es el **id del adjunto de la bibliografía**, o el **id del fragmento** del pasaje,
  que se resuelve a su adjunto. Se resuelve con la cadena existente: `resolve_attachment_file` +
  `validate_pdf_original`. No hay rama para el corpus: `resolve_asset_path_at_boundary` no es una frontera
  de acceso (`path_utils.rs:425`) y queda fuera del alcance. Los comandos se registran en `lib.rs`, en
  `APP_COMMANDS` de `build.rs` y en la capability, con sus tests de guarda.
- **Presupuesto de píxeles antes de reservar memoria** (corrige `JD-A-004`).
  - El tamaño se ajusta al alto **y** al ancho disponibles, como `pdfFitScale`.
  - Se fija un máximo por dimensión (por ejemplo, 4096 px) y un máximo total (por ejemplo, 16 MP). Una
    página de 72×14400 puntos nunca reserva gigas.
  - El escalón de ancho se elige después de aplicar ese presupuesto.
- **Rotación de página** (`JD-B-002`). El tamaño que se informa y el dibujo respetan `/Rotate`, como ya hace
  `ocr/pdf.rs:1250`, que intercambia ancho y alto cuando la página está rotada 90° o 270°.
- **Zoom profundo** (`JD-B-003`). El escalón máximo crece con el zoom hasta el presupuesto de píxeles, y se
  dibuja el recorte visible a escala completa cuando el zoom pasa ese máximo. Si no, el zoom al 500% queda
  borroso donde hoy se ve nítido.
- **Caché en disco**, en `<cache_dir>/pdf-pages/<clave>/<página>-<ancho>.<formato>`:
  - **Clave** (`JD-A-006`, `JD-B-005`): SHA-256 de la ruta canónica, el tamaño, la fecha de modificación, el
    `md5` de Zotero cuando existe y una **versión del dibujante**. Con la versión, cambiar PDFium o la
    calidad de imagen invalida lo guardado.
  - **Desalojo** con un índice propio de último uso (`JD-B-005`). En Windows/NTFS la fecha de último acceso
    viene desactivada, así que no sirve para saber qué se usó.
  - **Límite total:** propuesta de 1 GB.
  - El directorio ya está dentro del alcance del protocolo de assets (`$LOCALDATA/com.entropia.shared/**`).
- **Unión con el frontend** (`JD-B-004`). `packages/ui` no importa Tauri. El modo nativo recibe por
  propiedad una función inyectada (`renderNativePage(identidad, página, ancho)`) y la señal de
  disponibilidad, como ya se hace con `audioFallbackBlobLoader`. Si la función falla, el visor vuelve a
  pdf.js.
- **Capas superpuestas** (`JD-A-007`). En la Biblioteca el visor es de solo lectura, sin anotaciones,
  regiones de layout ni edición. Si en el futuro se extiende a otros visores, la paridad de geometría con
  pdf.js tiene que probarse aparte, porque las regiones de layout usan píxeles de referencia, no
  coordenadas normalizadas.

## 6. Metas (criterios de aceptación con el PDF de 128 MB, en la app)

| Caso | Hoy | Meta |
|---|---|---|
| Página 11, primera vez | 21–23 s | ≤ 2,5 s |
| Página 6, primera vez | 7–9 s | ≤ 1,5 s |
| Página de texto | 0,1–0,4 s | ≤ 0,3 s |
| Volver a una página vista | igual que en frío | ≤ 1 s (fase 1); ≤ 0,2 s si se hace la fase 2 |
| Volver a la pestaña Original | se reabre todo | sin recarga |
| Colecciones, Navegador y similares con el PDF de 128 MB | igual de lento que la Biblioteca | las mismas metas de primera vez que la Biblioteca; anotaciones, regiones, recortes y rotación sin cambios visibles |
| Memoria con la pestaña montada (`JD2-B-005`) | — | se mide el proceso de la app y el worker de pdf.js con el PDF de 128 MB abierto, cambiando de pestaña varias veces; no debe crecer en cada cambio |

Estas metas se miden en la app del owner. En otra máquina con un WebView2 anterior a 134 la fase 1 no se
activa y el tiempo sigue como hoy (`JD2-B-004`): `pdf.worker.mjs:55324` combina la opción con la
disponibilidad real de `ImageDecoder`, sin avisar. La medición de la sección 8 dice si funciona en esta
máquina, no en todas.

## 7. Pruebas

- **Fase 1:**
  - `pdfDocumentOptions` devuelve `isImageDecoderSupported: true` con Chromium 134 o posterior y no la
    incluye con Chromium anterior a 134 (user agent simulado) ni en motores que no son Chromium;
  - `DocumentViewer` y `ocr-rich-text.ts` pasan por esa función (un test por cada uno, sobre el mock de
    pdf.js); un test de guarda falla si aparece otra llamada a `getDocument` que no la use;
  - con el visor oculto (contenedor de 0×0), un aviso del `ResizeObserver` no provoca un dibujo, y al
    volver a tener tamaño se dibuja una sola vez;
  - sin la opción, la llamada no cambia (se fija con un test sobre el mock de pdf.js);
  - el visor de la Biblioteca sigue montado al cambiar de pestaña y no vuelve a llamar a `getDocument`;
  - revisión visual manual de la muestra de PDFs (sección 5, fase 1), con capturas antes y después.
- **Fase 2:** con el caché en memoria:
  - volver a una página no llama a `render`;
  - el uso de memoria queda dentro del límite en bytes;
  - cambiar del adjunto A al B en el mismo número de página no muestra la página de A;
  - cambiar el zoom o el tamaño redibuja en lugar de reusar una imagen de otro tamaño;
  - una entrada más grande que el límite no se guarda;
  - `close()` se llama al desalojar, al reemplazar y al cerrar.
- **Fase 3**, solo si se hace:
  - clave de caché (incluida la versión del dibujante) y desalojo con el índice propio;
  - presupuesto de píxeles con relaciones de aspecto extremas;
  - páginas con `/Rotate` de 90° y 270°;
  - cola con prioridad y descarte de pedidos viejos;
  - acceso solo por id de adjunto o de fragmento; se rechaza una ruta libre, `../` o un id inexistente;
  - guardas de ACL;
  - con el visor inactivo después de mostrar una página, una miniatura no cacheada y una página de OCR
    terminan dentro de un tiempo acotado;
  - PDFium del visor no arranca el runtime de ML en Pro, verificado sin conexión.
- **Manual:** la tabla de la sección 6, medida en `tauri dev` con el perfil de desarrollo.

## 8. Entregas (commits revisables)

1. **Fase 1:** la opción de `DocumentViewer`, su activación en los dos visores de la Biblioteca y la pestaña
   Original montada, con sus tests. Antes de mergear, la revisión visual de la muestra.
2. **Medición en la app contra la sección 6.** Con eso se decide si hacen falta las fases 2 y 3.
3. **Fase 2**, solo si hace falta.
4. **Fase 3**, solo si hace falta y con una aprobación nueva del owner.

Ninguna entrega necesita migración de base de datos.

## 9. Dónde quedó cada hallazgo del juicio

| Hallazgo | Severidad | Dónde se trata |
|---|---|---|
| `JD-A-001`, `JD-B-001`: retener PDFium bloquea a los demás usos | Crítico (corregido y verificado) | Fase 3, "Sin estado de PDFium" |
| `JD-A-002`, `JD-B-006`: la rama del corpus no tiene frontera de acceso | Alto / sugerencia | Fase 3, "Identidad del documento y seguridad"; el corpus queda fuera del alcance |
| `JD-A-003`: la afirmación sobre pdf.js no estaba respaldada | Alto | Sección 2 (medición separada por caso) y nueva fase 1 |
| `JD-A-004`: sin tope de píxeles | Alto | Fase 3, "Presupuesto de píxeles" |
| `JD-A-005`: el arranque de PDFium puede disparar el runtime de ML | Aviso | Fase 3, "Iniciar PDFium sin arrancar el runtime de ML" |
| `JD-A-006`: la clave de caché no detecta todos los cambios | Aviso | Fase 3, "Caché en disco" (md5 y versión) |
| `JD-A-007`: la geometría de las capas no es normalizada | Aviso | Fase 3, "Capas superpuestas"; la Biblioteca es de solo lectura |
| `JD-A-008`: el Navegador no tiene identidad de origen | Aviso | Fuera del alcance (sección 3) |
| `JD-B-002`: `/Rotate` | Aviso | Fase 3, "Rotación de página" |
| `JD-B-003`: el zoom profundo queda borroso | Aviso | Fase 3, "Zoom profundo" |
| `JD-B-004`: la unión con Tauri desde `packages/ui` no estaba especificada | Sugerencia | Fase 3, "Unión con el frontend" |
| `JD-B-005`: el desalojo en NTFS y la versión del caché | Sugerencia | Fase 3, "Caché en disco" |

## 9b. Dónde quedó cada hallazgo del segundo juicio

| Hallazgo | Severidad | Dónde se trata |
|---|---|---|
| `JD2-A-001`: cierre del proceso y JPEG mal dibujados por bugs de Chromium | Alto | Fase 1: motivo documentado, condición de Chromium 134 o posterior, muestra con perfil de color y BMP |
| `JD2-A-002`, `JD2-B-002`: dibujos invisibles con la pestaña oculta | Aviso | Fase 1, "Sin dibujos mientras está oculto"; test en la sección 7 |
| `JD2-A-003`: el caché de la fase 2 no tenía clave ni límite en bytes | Aviso | Fase 2, opción A; tests en la sección 7 |
| `JD2-A-004`, `JD2-B-003`: el mismo PDF, rápido en la Biblioteca y lento en Colecciones | Aviso / sugerencia | Resuelto en la revisión 4: el owner amplió la fase 1 a todos los visores (sección 3) |
| `JD2-B-001`: las fotos con máscara, JPX y PNG no se aceleran | Aviso | Fase 1, "Lo que la fase 1 no acelera" |
| `JD2-B-004`: un WebView2 viejo cae al camino lento sin avisar | Sugerencia | Sección 6 |
| `JD2-B-005`: falta una meta de memoria para la fase 1 | Sugerencia | Sección 6 |

## 10. Decisiones para el owner

1. ¿Aprobás la fase 1 (opción de pdf.js y pestaña montada) con la salvaguarda de revisión visual?
2. Si la fase 1 no alcanza la meta de "volver a una página", ¿preferís el caché en memoria (fase 2) antes que
   PDFium (fase 3)? Es más simple y no toca Rust.
3. La fase 3 queda congelada hasta que haya una medición que la justifique. ¿De acuerdo?
