# Plan: sincronizar el trabajo derivado de la Bibliografía (no pagar dos veces)

Estado: **revisión 7, borrador para evaluación externa. Sin implementar.**
Fecha: 2026-10-27. Sin rama todavía.

Historial:

- **Revisión 1:** borrador inicial.
- **Revisión 2** (Judgment Day, ronda 1). Corrige:
  - `JD-A-001`: backfill de lo ya pagado (§2.3b, WU1b, §8).
  - `JD-A-002`: la respuesta de OCR vacía se guarda aparte de la fila publicada (§2.2, §2.3).
  - `JD-A-003`: la espera distingue capability desconocida de no soportada, y dónde se aplica (§2.6, WU5).
  - `JD-A-004`: las Colecciones sí re-embeben fragmentos; entran como slice (§0, §1.1, §2.8, WU2b).
  - `JD-A-005`: descripción correcta de `web-capture-v1` y dos opciones de transporte (§1.1, §2.5, D8).
  - `JD-B-001`: semántica de cuenta, logout y epoch (§2.9, §5, WU1).
- **Revisión 3** (Judgment Day, ronda 2). Corrige:
  - `JD-B-001`: las filas escritas sin sesión ya no las adopta cualquier cuenta que siembra, sino solo la
    primera cuenta que se vincula al archivo; después toman la última cuenta vinculada (§2.2, §2.3, §2.9, §5,
    WU1, D6).
- **Revisión 4** (avisos del Judgment Day). Corrige:
  - `JD-B-003`: dos citas apuntaban a líneas equivocadas (§5, S-d).
  - `JD-A-007`: el volumen estaba subestimado, porque "pasajes 1050" cuenta adjuntos y no vectores; y "N/N"
    no prueba éxito, porque cuenta tareas fallidas (§5, D4, §8).
- **Revisión 5:** volúmenes medidos en el archivo real ya procesado del owner (§1.5); reemplazan la
  estimación de §5 y D4.
- **Revisión 6** (Judgment Day 2, ronda 1). Corrige:
  - `JD-A-101`: el backfill solo corre para el dueño original del archivo, nunca para una cuenta posterior
    (§2.3b, §2.9, §5, WU1b, §8).
  - `JD-A-102`: los aciertos de la caché salen físicamente de las ventanas de PDF completo, no solo de
    `needing` (§2.4, §5, WU2).
  - `JD-A-103`: barrera de frescura antes de pagar un miss, porque haber terminado el catch-up no prueba que
    la caché esté al día (§2.6, §5, D5, WU5, §8).
  - `JD-A-104`: lo descargado o confirmado queda atado a la identidad de la sesión que hizo la petición; si
    cambió, se descarta (§2.9, §5, WU4, §8).
- **Revisión 7** (avisos del Judgment Day 2). Corrige:
  - `JD-A-107`: la opción (a) de transporte no exige una migración; se quitó ese argumento falso (§2.5, D8).
  - `JD-B-102`: un dispositivo sin sesión ahora lee la caché local del dueño del archivo en lugar de volver
    a pagar (§2.4, §2.6, §2.9, WU2, §8).

## 0. Problema

El owner usa EntropIA en varios dispositivos con la misma cuenta de EntropIA Cloud. En un dispositivo la
Bibliografía ya está procesada por completo; en otro, con "Sincronización al día", el pie dice:

> Bibliografía: fichas 1934/2810 · pasajes 0/1050

Cada dispositivo vuelve a pagar el mismo trabajo: embeddings de fichas y pasajes (OpenRouter) y OCR de
páginas (GLM-OCR). Las Colecciones también repiten el embedding de sus fragmentos (§1.1). Con N dispositivos
se paga N veces por el mismo resultado. Palabras del owner:

> es pagar dos veces o mas, si lo abro en un tercer dispositivo seria pagar tre veces pro lo mismo, todo
> tendria que viajar via sync una ves que en un dispositivo ya se proceso el amterial sea colecicones o
> bibliotecas

**Objetivo:** una vez que un dispositivo pagó un resultado derivado de la Bibliografía o de las Colecciones,
ningún otro dispositivo de la misma cuenta lo vuelve a pagar, siempre que tenga el mismo contenido y la misma
configuración. Incluye lo que ese dispositivo ya pagó antes de actualizar (§2.3b).

**No objetivos:**

- Sincronizar el catálogo (fichas, creadores, tags, colecciones de Zotero). Cada dispositivo lo arma gratis
  desde su Zotero, y Zotero ya lo sincroniza por su cuenta.
- Sincronizar los PDF de Zotero. Eso es trabajo de Zotero.
- Cambiar cómo se procesa, se fragmenta o se busca.

## 1. Estado actual (medido en el código)

### 1.1 Qué sincroniza Cloud hoy

- **Por filas** (oplog con triggers): una lista cerrada de 18 tablas, `SYNCED_TABLES`
  (`apps/desktop/src-tauri/src/sync/capture.rs:16-37`), que se valida en los dos extremos. Incluye el corpus de
  Colecciones: `extractions` y `transcriptions` (el OCR) y `vec_assets`, el vector agregado por activo, que
  viaja en base64 dentro de la fila (`sync/push.rs:251-275`, `sync/apply.rs:526-541`).
  - **El OCR de Colecciones no se paga dos veces:** `ocr_decision` devuelve `AlreadyDone` si existe cualquier
    fila de extracción (`processing/eligibility.rs:155,260`), y esas filas viajan.
  - **Los fragmentos de Colecciones sí se pagan dos veces:** `rag_chunks` **no** está en `SYNCED_TABLES`.
    `embedding_decision` exige que los fragmentos locales coincidan con el texto y devuelve `IncompleteChunks`
    si no (`processing/eligibility.rs:379-401`). El ejecutor entonces llama de nuevo al embedder por cada
    fragmento y por el agregado (`processing/embedding.rs:409`, `:440`). Un segundo dispositivo recibe
    `vec_assets` pero igual re-embebe todos los fragmentos.
- **Por canales con envelope propio:** `writing-envelope-v1` (`sync/writing_*.rs`) y `research-envelope-v1`
  (`sync/research_*.rs`). Cada uno tiene su outbox en `sync_meta`, sus métodos en `SyncApi` con su token, su
  cursor y un catch-up `since-zero` por epoch.
- **`web-capture-v1` no es un canal independiente.** Son tablas físicas (`web_sources`, `web_captures`) que
  viajan por los triggers, el oplog y el push/apply genéricos; solo el catch-up tiene un cursor aparte, y
  el resto es disciplina de capability (`sync/web_capture.rs:1-15`; `sync/http.rs:13-16` lo distingue de
  writing y research). No es la plantilla de un canal con outbox propio (ver §2.5).
- **Ninguna tabla `zotero_*` ni `bibliographic_*` viaja.** `SQLite.md:372` lo dice explícitamente:
  "`processing_*`, `zotero_*`/`bibliographic_*`, `extraction_*` son locales".

### 1.2 Qué cuesta y qué es gratis en la Bibliografía

| Resultado | Tabla | ¿Se paga? | ¿Es determinista localmente? |
| --- | --- | --- | --- |
| Catálogo | `bibliographic_items`, `zotero_*` | No (Zotero) | Sí, desde Zotero |
| Perfil de ficha (texto) | `bibliographic_semantic_profiles` | No | Sí: función pura de los metadatos (`bibliography/profile.rs:1-12`) |
| Vector de ficha | `bibliographic_item_embeddings` | **Sí** (OpenRouter) | No |
| Texto nativo por página | `bibliographic_page_texts` (`method='native'`) | No (PDFium) | Sí, a partir del archivo |
| Texto OCR por página | `bibliographic_page_texts` (`method='ocr'`) | **Sí** (GLM-OCR) | No |
| Pasajes (chunks) | `bibliographic_chunks`, `_spans` | No | Sí, a partir de las páginas más `chunking_contract` (supuesto a verificar, §6) |
| Vector de pasaje | `bibliographic_chunk_embeddings` | **Sí** (OpenRouter) | No |

Esquema: `packages/store/src/schema.ts:1160-1372`, migraciones `0048`-`0055`.

**Colecciones** (solo lo que hoy no viaja; esquema `packages/store/src/schema.ts:85-120`):

| Resultado | Tabla | ¿Se paga? | ¿Viaja hoy? |
| --- | --- | --- | --- |
| Texto OCR / transcripción | `extractions`, `transcriptions` | **Sí** | Sí (`SYNCED_TABLES`) |
| Vector agregado del activo | `vec_assets` | **Sí** | Sí |
| Fragmento y su vector | `rag_chunks` (`text_content`, `source_text_hash`, `embedding`, `embedding_model`, `embedding_contract`, `dimensions`) | **Sí** | **No** |

**Lo que hay que hacer viajar son cuatro cosas: vectores de fichas, vectores de pasajes, páginas OCR (incluida
la respuesta vacía, §2.3) y vectores de fragmentos de Colecciones.** Las tres de vectores comparten una
sola caché (§2.1).

### 1.3 Por qué no alcanza con agregar las tablas a `SYNCED_TABLES`

- Las claves primarias son UUID locales al azar: `bibliographic_items.id`, `zotero_attachments.id`,
  `bibliographic_chunks.id` y `zotero_libraries.id`. El mismo libro tiene IDs distintos en cada dispositivo.
- Lo que sí es estable entre dispositivos es la identidad de Zotero, `(libraryType, libraryId, itemKey)` y
  `attachmentKey`, y **los hashes de contenido**.
- Copiar filas obligaría a mapear identidades, a resolver conflictos con el catálogo que cada dispositivo
  arma por su lado y a que el catálogo también viaje. Es todo lo que el objetivo quiere evitar.

### 1.4 Lo que ya existe y hace viable el diseño

- **Los vectores se identifican por contenido.** Cada vector guarda `input_hash`, `embedding_model`,
  `embedding_contract` y `dimensions`. `reusable_chunk_embeddings` (`bibliography/repository.rs:2609`) ya
  reutiliza un vector solo si `input_hash = text_hash` y coinciden el modelo, el contrato y las dimensiones.
  El ejecutor lo consulta antes de pagar (`bibliography/processing.rs:2287`).
- **Los contratos son inmutables.** `bibliographic_embedding_contracts(contract_hash PK, provider, model,
  dimensions, chunking_contract)` y `bibliographic_index_generations` (`bibliography/generation.rs:21-98`).
  Un modelo distinto da otro `contract_hash`, así que nunca se mezclan espacios vectoriales.
- **El perfil de ficha es determinista.** Su `input_hash` cubre la versión de plantilla más el texto
  canónico (`bibliography/profile.rs`). Dos dispositivos con los mismos metadatos de Zotero producen el
  mismo hash.
- **El OCR se decide por página.** `maybe_ocr_pages` (`bibliography/processing.rs:3466`) elige qué páginas
  van a OCR; la fila se publica con `upsert_page_text_in_transaction` (`bibliography/repository.rs:2318`). El
  planificador de reproceso ya calcula `source_sha256` del archivo (`bibliography/reprocess.rs:85`).
- **Una página OCR pagada no siempre deja fila `method='ocr'`.** `settled_page_row`
  (`bibliography/processing.rs:3924-3944`) conserva la fila nativa cuando la respuesta es vacía y el texto
  nativo no está corrupto. El pago existió y no queda rastro en `bibliographic_page_texts`.
- **Hay tablas que no cambian el `schema_tag`.** Las tablas `sync_*` se crean con `ensure_sync_schema`
  (`sync/schema.rs`, `CREATE TABLE IF NOT EXISTS`), fuera de `_migrations`. El `schema_tag` sale de la
  cabeza de `_migrations` (`sync/engine.rs:633`).

### 1.5 Medición real (archivo del owner, 2026-10-27)

Medido con consultas de solo lectura (`mode=ro`) sobre el archivo de la PC Windows que ya completó la
Bibliografía (`entropia.sqlite`, 951 MB, Lite, una biblioteca de Zotero):

| Qué | Filas | Bytes del vector (f32) | Observación |
| --- | --- | --- | --- |
| Vectores de fichas | 2.810 | 11,0 MiB | 2.745 `input_hash` distintos |
| Vectores de pasajes | 100.477 | **392,5 MiB** | 98.659 `input_hash` distintos (1,8 % repetidos); 1.437 adjuntos, 1.144 obras |
| Texto de pasajes | 100.477 | — | 69,3 M caracteres; no viaja (se recalcula gratis) |
| Páginas nativas | 17.900 | — | 61,6 M caracteres; gratis |
| Páginas OCR | 579 (65 adjuntos) | — | 1,05 M caracteres; 5 con calidad `empty` |
| Fragmentos de Colecciones (`rag_chunks`) | 7.260 | 28,4 MiB | hoy no viajan |
| `vec_assets` (ya viaja) | 1.302 | 5,1 MiB | — |

Tareas: `bibliography_extract` 1.447 `succeeded` y 1 `failed` (`provider_transient`);
`bibliography_profile` 4.257 `succeeded`. Contrato único: `api` / `baai/bge-m3` / 1024 /
`rag-chunk-800-100-char-v1`.

Conclusiones para el diseño:

- **El costo está casi entero en los vectores de pasajes.** El OCR de esta biblioteca es chico (579 páginas):
  la caché de OCR importa para bibliotecas escaneadas, pero no es lo que hoy duplica el gasto.
- **El volumen por cuenta está en el orden de 400 MiB en f32** (unos 200 MiB en f16) para esta biblioteca.
  Es manejable, pero define D4 y D7: subirlo y bajarlo por cada dispositivo nuevo tiene un costo de
  almacenamiento y transferencia que hay que comparar con el precio de volver a embeber unos 69 M caracteres.
- **La deduplicación por contenido casi no ahorra dentro de una biblioteca** (1,8 %). Su valor está entre
  dispositivos, que es el objetivo.

## 2. Diseño propuesto: caché derivada direccionada por contenido

### 2.1 Idea

Un canal nuevo de Cloud, `bibliography-derived-v1`, que **no transporta filas de la Bibliografía sino
resultados pagados, indexados por lo que se calculó** y no por a qué ficha pertenecen:

- **Vector** = `f(contract_hash, input_hash)`. Sirve igual para fichas, pasajes y fragmentos de Colecciones:
  el texto, su hash y el modelo determinan el resultado. `input_hash` es el hash de perfil que ya guarda la
  ficha, o el sha256 del texto exacto enviado al embedder (pasaje, fragmento, o el texto completo del agregado
  de un activo).
- **Página OCR** = `f(source_hash, page_number, ocr_contract)`. El archivo, la página y la configuración de
  OCR determinan el resultado, **incluida la respuesta vacía** (§2.2).

Las entradas son **inmutables y de solo agregado**: la misma clave produce el mismo resultado. Por eso no
hay conflictos, no hay LWW, no hay borrados que propagar ni mapeo de identidades. Gana el primero que
escribe, y una segunda escritura con la misma clave se descarta. Cada fila pertenece a la cuenta que la
escribió (§2.9).

### 2.2 Tablas locales (sin migración)

Se crean en `ensure_sync_schema`, junto a las demás `sync_*`, así que no suben el `schema_tag`:

```sql
CREATE TABLE IF NOT EXISTS sync_derived_vectors (
  account_id    TEXT NOT NULL,          -- originating account; '' only before the archive's first-ever binding (see 2.9)
  contract_hash TEXT NOT NULL,
  input_hash    TEXT NOT NULL,          -- profile hash, or sha256 of the exact text sent to the embedder
  dimensions    INTEGER NOT NULL,
  embedding     BLOB NOT NULL,          -- f32 LE, dimensions*4 bytes
  origin        TEXT NOT NULL CHECK (origin IN ('local','remote')),
  pushed        INTEGER NOT NULL DEFAULT 0,
  created_at    INTEGER NOT NULL,
  PRIMARY KEY (account_id, contract_hash, input_hash)
);

CREATE TABLE IF NOT EXISTS sync_derived_ocr_pages (
  account_id    TEXT NOT NULL,          -- see 2.9
  source_hash   TEXT NOT NULL,          -- sha256 of the file, see open decision D2
  page_number   INTEGER NOT NULL,
  ocr_contract  TEXT NOT NULL,          -- provider + model + prompt/parser version
  outcome       TEXT NOT NULL CHECK (outcome IN ('text','empty')),  -- the provider's answer, not the published row
  text_content  TEXT NOT NULL,          -- answer as Markdown; '' when outcome = 'empty'
  text_hash     TEXT NOT NULL,
  origin        TEXT NOT NULL CHECK (origin IN ('local','remote')),
  pushed        INTEGER NOT NULL DEFAULT 0,
  created_at    INTEGER NOT NULL,
  PRIMARY KEY (account_id, source_hash, page_number, ocr_contract)
);

CREATE TABLE IF NOT EXISTS sync_derived_contracts (   -- mirror of the immutable contract row
  contract_hash     TEXT PRIMARY KEY,
  provider          TEXT NOT NULL,
  model             TEXT NOT NULL,
  dimensions        INTEGER NOT NULL,
  chunking_contract TEXT NOT NULL
);
```

`outcome` registra lo que **respondió el proveedor**, separado de la fila que la Bibliografía termina publicando.
Es necesario porque `settled_page_row` (`bibliography/processing.rs:3924-3944`) conserva la fila nativa cuando
la respuesta es vacía y el texto nativo no está corrupto: sin `outcome='empty'`, el segundo dispositivo
reenviaría a GLM-OCR una página que ya se pagó. No hay columna `quality`: el dispositivo que lee la deriva con
`settled_page_row` sobre su propia página nativa, así que la fila publicada sale igual que si hubiera llamado
al proveedor. Una falla definitiva del proveedor (`PageOcrOutcome::Failed`) **no** se guarda: no es un
resultado y puede no repetirse.

Si los expertos prefieren el namespace propio de la Bibliografía, la alternativa está en §3.3.

### 2.3 Escritura (el dispositivo que paga)

En los puntos donde hoy se persiste un resultado pagado, la misma transacción también escribe en la caché,
con `origin='local'`, `pushed=0` y el `account_id` que fija §2.9 (el de la sesión; sin sesión, el de la última cuenta vinculada):

- vector de ficha: `upsert_item_embedding_in_transaction` (llamado en `bibliography/processing.rs:2511`);
- vector de pasaje: `upsert_chunk_embedding_in_transaction` (`bibliography/processing.rs:2557`);
- vectores de fragmentos y agregado de una Colección: el publish del ejecutor de embeddings
  (`processing/embedding.rs`), junto con las filas de `rag_chunks` (§2.8);
- **respuesta de OCR de página.** El punto de captura no es `upsert_page_text_in_transaction`
  (`bibliography/processing.rs:4480`, que solo ve la fila publicada) sino la respuesta del proveedor en
  `maybe_ocr_pages`, donde `answer` pasa a `settled_page_row` (`bibliography/processing.rs:3674`). El cómputo
  agrega a su salida una lista `ocr_outcomes` (página, `outcome`, texto) y calcula una vez el sha256 del
  archivo, que ya tiene en memoria (`bytes`). Esa lista se escribe en la caché en la misma transacción que
  publica `output.pages`. Cubre las respuestas con texto (`'text'`) y las vacías (`'empty'`), también cuando la
  fila publicada queda como nativa.

Al hacerlo en la misma transacción, la caché nunca registra un resultado que la Bibliografía no confirmó.

### 2.3b Backfill de lo ya pagado

Sin una siembra inicial, actualizar el dispositivo A no exporta nada de lo que ya pagó. Los puntos de §2.3
solo corren cuando se paga algo nuevo, y el trabajo existente no se vuelve a ejecutar: una ficha con perfil
sin cambios no se readmite (`processing/repository.rs:2452`) y una extracción vigente termina por el atajo
`already_current` con `pages=[]` (`bibliography/processing.rs:3132-3170`). B pagaría todo de nuevo.

Una pasada de siembra, local y sin llamadas pagas, copia lo ya guardado a la caché con `origin='local'` y
`pushed=0`. Corre en segundo plano, una vez por contrato, y es idempotente (`INSERT OR IGNORE`).

**Atribución: solo el dueño original del archivo siembra.** Las tablas fuente no tienen `account_id`
(`packages/store/src/schema.ts:1182-1372`) y sobreviven al logout, así que sembrar bajo la cuenta de la sesión
le atribuiría a una cuenta posterior el OCR privado de la anterior. La siembra se rige por
`sync_meta['derived_owner_account']` (§2.9), la primera cuenta que se vinculó al archivo:

- corre solo mientras `account_id` de la sesión sea igual a `derived_owner_account`, y escribe sus filas con
  ese `account_id`;
- bajo cualquier otra cuenta no corre. Esa cuenta no exporta lo que ya estaba en el archivo, y solo cachea lo
  que pague desde su vinculación (§2.3);
- la marca de avance es `sync_meta['derived_backfill:<account_id>:<contract_hash>']`. Describe filas locales
  que el logout conserva, así que `clear_account_metadata` no la borra (§2.9). Un re-login del dueño no repite
  la lectura de los PDF.

Fuentes:

- `bibliographic_item_embeddings`: `inputHash`, `embedding` y `dimensions` ya están guardados. El
  `contract_hash` sale de `generation_id` → `bibliographic_index_generations.contract_hash`.
- `bibliographic_chunk_embeddings`: igual, pero solo las filas donde `input_hash = bibliographic_chunks.text_hash`
  (la condición de `reusable_chunk_embeddings`, `bibliography/repository.rs:2609`). El ancho del blob se valida.
- `rag_chunks` y `vec_assets` de Colecciones: solo los activos que `embedding_decision` da por `Fresh` bajo el
  contrato efectivo actual. El `input_hash` de cada fragmento es el sha256 de su `text_content`, y el del
  agregado el sha256 del texto completo de las fuentes del activo (§2.8).
- `bibliographic_page_texts` con `method='ocr'`: **el hash del archivo no está guardado.**
  `bibliographic_extractions` solo conserva `source_mtime` y `source_bytes` (`schema.ts:1252-1270`). La siembra
  lee el PDF local, comprueba que `mtime` y `bytes` siguen coincidiendo con la extracción vigente
  (`extraction_matches_source`) y calcula el sha256 como el planificador de reproceso
  (`bibliography/reprocess.rs:85`). Sin archivo, o si cambió, esas páginas se omiten y se registra. El costo es
  releer los PDF una vez, por lo que corre con prioridad baja y acotada. El `ocr_contract` que se estampa es el
  vigente (D3): la versión 1 pasa a ser, por definición, la línea base de todo lo pagado hasta ahora.
- **Respuestas de OCR vacías:** solo se pueden reconstruir en dos casos. Las páginas `method='ocr'` con
  `quality='empty'` (la rama de texto nativo corrupto de `settled_page_row`, `processing.rs:3930-3937`) entran
  como `outcome='empty'`. Y los checkpoints `ocr-page:N` que retienen las tareas `bibliography_extract`
  `failed` o `cancelled` (`PAID_EXTRACTION_RETENTION_SQL`, `processing/repository.rs:5108`), cuyo payload vacío
  es una respuesta vacía pagada. **Las demás no dejan evidencia:** una tarea exitosa borra sus checkpoints
  (`delete_task_checkpoints`, `processing/repository.rs:5685`), y la página que se quedó nativa tras una
  respuesta vacía no tiene fila `ocr`. Esas páginas, en archivos ya procesados, no se pueden sembrar: el
  primer dispositivo que las vuelva a necesitar las paga una vez y de ahí en más quedan en la caché. Es una
  pérdida acotada y está en §5.

### 2.4 Lectura (el dispositivo que no quiere pagar)

- **Vectores:** `reusable_chunk_embeddings` y el camino del vector de ficha también consultan
  `sync_derived_vectors` por `(contract_hash, input_hash)`, solo con filas de la **cuenta efectiva**: la de la
  sesión o, sin sesión, `derived_bound_account` (§2.9); nunca las de `''` ni las de otra cuenta. Un
  acierto se publica como si fuera un cálculo propio, en la misma generación y con los mismos sellos. La
  validación de ancho (`blob.len() == dimensions * 4`, `bibliography/processing.rs:2299`) se aplica igual. El
  ejecutor de embeddings de Colecciones hace lo mismo antes de cada `embed_text` (§2.8).
- **OCR:** `maybe_ocr_pages` consulta `sync_derived_ocr_pages` por cada página candidata **antes de planificar
  las ventanas de PDF completo**, y los aciertos salen de la petición al proveedor **físicamente**, no solo
  de `needing`. Quitarlos de `needing` no alcanza: `plan_pdf_windows` conserva la ventana completa que
  contenga cualquier página pendiente (`bibliography/selective_ocr.rs:288-313`) y `recognize_pdf_pages` recibe
  todo ese rango (`bibliography/processing.rs:3512-3532`). En un PDF de diez páginas con cuatro aciertos y seis
  pendientes se enviarían las diez. La regla:
  - `needing` pasa a ser solo los fallos, y `should_use_pdf_mode` se decide sobre esa cantidad.
  - `plan_pdf_windows` recibe además las páginas en caché y corta cada ventana en ellas: devuelve tramos
    contiguos que no contienen ninguna página en caché, y descarta los tramos sin páginas pendientes. Ningún
    rango enviado a `recognize_pdf_pages` incluye una página ya cacheada.
  - Vale igual durante el backfill y cuando los fallos son respuestas vacías históricas sin evidencia
    (§2.3b): las recuperables no se vuelven a pagar por compartir ventana con ellas.

  Una página encontrada pasa su respuesta por
  `settled_page_row` con la página nativa local: con `outcome='text'` queda `method='ocr'`; con
  `outcome='empty'` queda lo que hoy quedaría, nativa o vacía por corrupción. En ningún caso se llama a
  GLM-OCR. Una página en caché cuenta en el progreso igual que un checkpoint.
- **Las reglas de selección no cambian.** La caché solo responde "¿ya lo pagó alguien?". El detector, el
  plan de reproceso y la fragmentación siguen decidiendo igual que hoy.

### 2.5 Transporte

Hay dos formas de transportar la caché, y el repositorio ya tiene un ejemplo de cada una. `web-capture-v1` no
es un canal independiente, así que no sirve de plantilla para la segunda (§1.1).

- **(a) Tablas físicas por el oplog genérico, como `web-capture-v1`.** Las tablas de cable pasan a ser tablas
  físicas con triggers de captura. Eso exige agregarlas a `SYNCED_TABLES` y a `SYNCED_TABLES_FK_ORDER`
  (`sync/capture.rs`), subir `TRIGGERS_VERSION`, y que el push y el apply genéricos las acepten. Además:
  - No exige una migración: las tablas físicas pueden seguir creándose en `ensure_sync_schema`, como las
    `sync_*` de D1, y el `schema_tag` sale solo de la cabeza de `_migrations` (`sync/engine.rs:633`). Pero
    agregarlas a `SYNCED_TABLES` las hace parte del allowlist que el servidor valida en los dos extremos, y un
    cliente viejo o un servidor sin la capability tendrían que tolerarlas (la disciplina de `web-capture-v1`).
  - El oplog genérico lleva versiones por fila, tombstones y LWW que una caché inmutable de solo agregado no
    necesita.
  - `seed_account` (`sync/pull.rs:105-132`) las sembraría en cada cuenta nueva.
  - Obliga a cambiar `SYNCED_TABLES`, que §5 promete no tocar.
- **(b) Canal dedicado, como writing y research.** Outbox propio (el flag `pushed` de las tablas
  `sync_derived_*`, sin migración), métodos propios en `SyncApi` con el token exacto `bibliography-derived-v1`
  (las requests ordinarias de push y pull no lo envían), cursor propio que no mueve `last_pull_seq` y
  catch-up `since-zero` por epoch. No toca `SYNCED_TABLES` ni el esquema principal.

**Recomendado: (b).** Las dos se pueden hacer sin migración; (b) es la única que además no toca el corpus que
ya sincroniza ni carga con versiones por fila y LWW. Su costo es más código de cliente que (a), porque el outbox, el cursor y el
catch-up son propios. Queda como decisión abierta D8. El resto de esta sección y las demás asumen (b).

Disciplina común, la misma que usa `web-capture-v1` (`sync/web_capture.rs:1-20`):

- La capability solo se confía si la anuncia una respuesta ordinaria, y solo para ese epoch. Mientras no
  esté anunciada, las entradas `pushed=0` esperan. Un servidor sin la capability nunca recibe una llamada que
  no entiende.
- **Lotes acotados por bytes.** Reutiliza `DEFAULT_MAX_PUSH_BYTES` (`sync/push.rs`). Los vectores van en
  base64 dentro de la fila, como `vec_assets`.

### 2.6 Pull antes de pagar

Este es el punto crítico para que el objetivo se cumpla en la práctica. Hoy el backlog derivado arranca
apenas termina la sincronización con Zotero: `admit_stale_profile_demands` y
`admit_stale_extraction_demands` admiten trabajo de inmediato (`bibliography/processing.rs:870-878`). Si el
trabajo arranca antes de que llegue la caché, el dispositivo nuevo paga igual.

**La compuerta distingue tres estados de la capability,** no un booleano. `supports_web_capture` devuelve
`false` tanto para "epoch desconocido" y "capability sin descubrir" como para "anunciada: false"
(`sync/web_capture.rs:70-77`). Usar ese patrón dejaría sin protección el primer arranque de B, que es
justamente el caso del owner. El estado sale de `server_epoch`, del registro de capability y del catch-up del
epoch:

| Estado | Condición | Acción |
| --- | --- | --- |
| Sin sesión | no hay `account_id` en `sync_meta` | consultar la caché local con la cuenta efectiva (`derived_bound_account`, §2.9); sin esperar ni hacer pull, pagar solo los misses |
| **Desconocida** | hay sesión y no hay registro de capability para el epoch actual (o no hay epoch todavía) | **esperar, acotado** |
| **Soportada, catch-up pendiente** | anunciada en el epoch actual y el catch-up de ese epoch no terminó | **esperar, acotado** |
| Soportada y al día | anunciada y catch-up terminado | consultar la caché; antes de pagar un miss, **barrera de frescura** (abajo) |
| **No soportada (confirmada)** | registro del epoch actual con `advertised: false` | pagar como hoy |

Se procede sin esperar solo con ausencia **confirmada**, sin sesión, o al vencer el tiempo máximo (D5), también
si no hay red. La caché ahorra plata pero nunca bloquea al usuario.

**Dónde se aplica: antes de la primera llamada paga, no al reclamar la tarea.** El OCR de pago corre dentro de
tareas `bibliography_extract`, vía `maybe_ocr_pages`, y esas tareas también hacen la extracción nativa
(PDFium), que es gratis y no debe esperar. Por eso la espera va en:

1. `maybe_ocr_pages`, después de calcular `needing` y solo si no está vacío, antes de la primera llamada al
   proveedor (`recognize_pdf_pages` o `recognize_page`). Al terminar la espera se vuelve a consultar la caché.
2. El ejecutor de embeddings de la Bibliografía (perfil y `stage_chunks`), antes de la primera oleada.
3. El ejecutor de embeddings de Colecciones (`processing/embedding.rs`), antes del primer `embed_text`.

La espera es un helper que respeta el `StopFlag` y el lease como el resto del ejecutor, y recuerda su
veredicto por epoch para que las tareas siguientes no vuelvan a esperar. El motor de sync, no el ejecutor, es
quien descubre la capability y corre el catch-up; la espera solo observa su estado.

**Barrera de frescura.** El veredicto "soportada y al día" dice que el catch-up del epoch terminó alguna vez,
no que la caché local contenga lo que otro dispositivo subió después. Secuencia que lo rompe: B termina el
catch-up; A procesa y sube resultados nuevos en el mismo epoch; B recibe el catálogo de Zotero antes del
siguiente pull de Cloud y admite trabajo de inmediato (`bibliography/processing.rs:870-878`, independiente
del canal). Con solo el veredicto, B pagaría material que A ya terminó y subió. Por eso, en cada punto de
aplicación de la lista anterior:

1. Se consulta la caché. Los aciertos se usan; la barrera solo existe para los misses.
2. Antes de la primera llamada paga para una unidad con miss, el helper comprueba la frescura: el canal guarda
   `sync_meta['derived_last_pull_started_at']`, el instante en que empezó el último pull incremental exitoso
   del epoch. La caché es fresca para esa unidad si ese instante es posterior a la admisión del trabajo (la
   `created_at` de la tarea, que sigue a la sincronización de Zotero que lo creó) **y** no tiene más de `F`
   segundos (cota pequeña, D5).
3. Si no es fresca, el helper pide al motor un pull incremental del canal (un solo pull en vuelo, compartido
   por las unidades que esperan), lo espera acotado por D5 y vuelve a consultar la caché. Solo los misses
   que sobreviven pagan.
4. Con timeout, sin red o sin sesión se paga, como en el resto de la compuerta: la barrera ahorra plata pero
   nunca bloquea al usuario.

La barrera cubre la secuencia que el catch-up no puede ver. No elimina la carrera de dos dispositivos que
procesan a la vez (§5): un resultado que A sube después de que empezó el último pull de B sigue pudiendo
pagarse dos veces. También responde en parte a que un tope fijo puede quedar corto frente a un catch-up
grande: el catch-up inicial de una cuenta como la de §1.5 ronda los 400 MiB, así que D5 debe dimensionarse
con ese volumen. La barrera no reemplaza ese dimensionamiento: si vence el tope, se paga.

- El pie lo muestra: "Bibliografía: esperando resultados de tus otros dispositivos…".

### 2.7 Servidor

El servidor de EntropIA Cloud tiene que:

1. anunciar `bibliography-derived-v1` y exponer los métodos de push y pull del canal (opción (b), §2.5);
2. aceptar y servir dos tablas de cable, `derived_vectors` y `derived_ocr_pages`, más
   `derived_contracts`, con clave compuesta e inserción idempotente (`ON CONFLICT DO NOTHING`);
3. aplicar cuota por cuenta (ver §5).

**El código del servidor no está en este repositorio.** Esta parte la tiene que estimar quien lo mantiene.

### 2.8 Colecciones (slice propio)

El owner pidió explícitamente "sea colecciones o bibliotecas". El OCR de Colecciones ya viaja (§1.1), así que
el slice es solo el embedding de fragmentos, y entra en la misma caché de vectores en lugar de agregar una
tabla nueva.

- **Por qué cabe en la misma caché.** `plan_rag_chunks` produce `text_content` por fragmento
  (`nlp/embeddings.rs:1135`) y el ejecutor embebe exactamente ese texto (`processing/embedding.rs:409`). La
  clave es `(contract_hash, sha256(text_content))`. `contract_hash` es el `hash` del contrato efectivo
  (`resolve_effective_embedding_contract`, `processing/eligibility.rs`), el mismo que usa la Bibliografía
  (`bibliography/processing.rs:2156`). Las filas de `rag_chunks` guardan el nombre del contrato y no ese hash,
  pero el modelo, el contrato y el ancho se resuelven desde el mismo contrato efectivo.
- **Agregado de activo.** Se embebe el texto completo de las fuentes (`processing/embedding.rs:432-440`) y
  se guarda con `input_hash = sha256(texto completo)`. Aunque `vec_assets` ya viaja, el ejecutor no puede
  saber si ese vector sale del mismo texto sin esta clave.
- **Lectura.** Antes de cada `embed_text`, el ejecutor consulta la caché; en un acierto usa el blob. El
  publish de `rag_chunks` no cambia. La tarea del dispositivo B seguirá siendo `IncompleteChunks`
  (`eligibility.rs:379-401`), pero ahora se completa sin llamadas pagas.
- **Escritura.** Cada fragmento y el agregado que el ejecutor paga se escriben en la caché en la transacción
  de publish (§2.3).
- **Supuesto a verificar (S-e):** `embed_text` (Colecciones) y `embed_many` (pasajes) devuelven el mismo
  vector para el mismo texto, sin prefijo ni instrucción distinta. Si no, se separan los `input_hash` por
  prefijo de uso y se pierde solo el cruce entre dominios.

### 2.9 Cuenta, logout y epoch

Las filas de la caché dependen del contenido, no de la sesión, así que **se conservan al cerrar sesión**, igual
que `sync_blob_index` (`sync/session.rs`, comentario de `SESSION_STATE_TABLES`): borrarlas destruiría trabajo
ya pagado. Pero contienen el texto OCR de PDF privados, así que no pueden seguir al siguiente usuario de la
cuenta.

- **Cada fila guarda el `account_id` que la originó.** Con sesión, el de `sync_meta['account_id']`. La clave
  primaria lo incluye.
- **Marca durable de la última cuenta vinculada:** `sync_meta['derived_bound_account']`. La escribe
  `write_sync_session` (`sync/session.rs:340`) en la misma transacción que `account_id` (línea 352), y
  **no está en `SESSION_META_KEYS`** (`sync/session.rs:239`), así que `clear_sync_state` no la borra. Su
  ausencia significa que ninguna cuenta se vinculó jamás a este archivo. Al crear las tablas en
  `ensure_sync_schema`, si la marca falta pero hay una sesión activa, se inicializa con esa cuenta.
- **Filas escritas sin sesión (offline tras un logout):** toman `derived_bound_account` (la última cuenta, el
  dueño del archivo), nunca `''` y nunca la cuenta siguiente. Solo si la marca no existe se escriben con
  `''`.
- **Dueño original del archivo:** `sync_meta['derived_owner_account']` guarda la **primera** cuenta que se
  vinculó. Se escribe una sola vez, en la misma transacción de `write_sync_session` que adopta las filas `''`
  (abajo), no se sobrescribe nunca y, como la marca anterior, no está en `SESSION_META_KEYS`. Es la única
  cuenta para la que corre el backfill (§2.3b). Si falta pero hay una sesión activa al crear las tablas, se
  inicializa con esa cuenta, igual que `derived_bound_account`.
- **Las filas `''` las adopta únicamente la primera vinculación del archivo.** En esa misma transacción de
  `write_sync_session`, si `derived_bound_account` no existe, las filas con `account_id = ''` pasan a la
  cuenta que inicia sesión (lo que había antes del primer login pertenece a quien lo inicia; ante una colisión
  de clave gana la fila de la cuenta y se descarta la `''`), y recién entonces se escriben las dos marcas,
  `derived_bound_account` y `derived_owner_account`. A partir de
  ahí no se vuelve a crear ninguna fila `''`. No se usa `seed_account` para esto: su guarda es
  `sync_meta['seeded_account']` (`sync/pull.rs:106`), que `clear_sync_state` borra (`sync/session.rs:245`) y
  que por eso se vuelve a ejecutar con cada cuenta nueva (`sync/engine.rs:680`).
- **Solo se empuja y solo se consulta lo de la cuenta efectiva** (§2.4): la de la sesión o, sin sesión,
  `derived_bound_account`. Sin sesión no se empuja nada, pero sí se lee la caché local del dueño del archivo:
  es la misma regla que ya atribuye a esa cuenta lo escrito sin sesión, así que no expone nada nuevo y evita
  volver a pagar lo que el archivo ya tiene. Tras un cambio de cuenta, las filas de
  la cuenta anterior, incluidas las escritas offline después de su logout, no se suben a la nueva ni se
  sirven a la nueva para procesar. Quedan inertes hasta que la cuenta anterior vuelva a iniciar sesión.
  **Esto es deliberadamente más estricto que el corpus actual:** `seed_account` (`sync/pull.rs:105`, llamado en
  `sync/engine.rs:680`) vuelve a sembrar en cada cuenta nueva todo `SYNCED_TABLES`, incluido el OCR de las
  Colecciones en `extractions` y `transcriptions`. La caché derivada no hereda esa política; alinear ambas es
  parte de D6.
- **En logout o cambio de cuenta** (`clear_sync_state`, `sync/session.rs`): se pone `pushed = 0` en todas las
  filas (el mismo patrón que `UPDATE sync_blob_index SET uploaded = 0`) y se borra la metadata de cuenta del
  canal con una función `clear_account_metadata` propia, llamada desde allí: capability, epoch del
  catch-up, cursor, `derived_last_pull_started_at` y veredicto de la espera. Las filas y las marcas de backfill
  (`derived_backfill:<account_id>:<contract_hash>`, que describen filas locales) no se borran.
- **Re-login con la misma cuenta:** las filas siguen siendo suyas, se re-empujan (idempotente en el servidor) y
  el catch-up corre de nuevo en el epoch vigente.
- **Una petición en vuelo queda atada a la identidad que la hizo.** Limpiar la metadata en el logout no cancela
  una respuesta HTTP que ya salió: sin esta regla, un pull que A lanzó con su token y que vuelve después de
  que B inició sesión se guardaría con el `account_id` de B, B podría consultarlo y re-empujarlo. Los canales
  existentes ya protegen esta frontera: `sync/writing_pull.rs:13-30` exige cuenta, servidor, dispositivo,
  epoch e incarnation de sesión completos, y rechaza el staging obsoleto (`:204-211`); y
  `sync/writing_receive.rs:29-36` transporta ese alcance (`WritingReceiveScope`). El canal derivado hace lo
  mismo:
  - **Pull.** Al empezar se congela el alcance `(account_id, server_url, device_id, server_epoch,
    session_incarnation)` leído de `sync_meta`. La red corre fuera de toda transacción. Al volver, una
    única transacción relee el alcance y lo compara con el congelado y con el `server_epoch` de la respuesta.
    Si coincide, instala las filas (con `origin='remote'` y el `account_id` congelado), mueve el cursor y
    registra capability y catch-up. Si no coincide, **descarta la respuesta**: no instala filas y no toca ni
    cursor ni capability ni catch-up de la sesión actual.
  - **Push.** El mismo alcance congelado al armar el lote. La respuesta solo pone `pushed = 1` si el alcance
    sigue igual, y únicamente sobre las claves enviadas y con `account_id` igual al congelado; si cambió, no
    marca nada (las filas se reenvían bajo su cuenta, y es idempotente en el servidor).
  - Lo que baja del servidor se guarda siempre con `origin='remote'` y el `account_id` **congelado**, nunca
    el que haya en `sync_meta` al terminar la red.

## 3. Alternativas evaluadas

### 3.1 Sincronizar las tablas `bibliographic_*` por filas (descartada)

Exige que viaje el catálogo, mapear los UUID locales a la identidad de Zotero, resolver conflictos entre el
catálogo sincronizado y el que arma Zotero localmente, y migraciones en las 18+ tablas del allowlist. Es
mucho riesgo para transportar cuatro resultados.

### 3.2 Sincronizar por identidad de Zotero (descartada como base)

Claves `(libraryType, libraryId, itemKey/attachmentKey)`. Funciona solo si los dos dispositivos espejan la
misma cuenta de Zotero. Pierde el reuso entre bibliotecas distintas que contienen el mismo PDF, y obliga a
invalidar a mano cuando cambian los metadatos. Con el direccionamiento por contenido, la invalidación sale
sola: si cambia el texto, cambia el hash.

### 3.3 Tablas de caché con migración propia en lugar de `sync_*`

Semánticamente más limpio: la caché es un dato, no contabilidad de sync. El costo es que **sube el
`schema_tag` de la cuenta en todos los dispositivos** (ver `AGENTS.md`, "Dev profile"). Se recomienda
`sync_*` salvo que los expertos vean un motivo fuerte.

## 4. Decisiones abiertas para los expertos

- **D1. Dónde viven las tablas locales:** `sync_*` sin migración (recomendado) o migración propia (§3.3).
- **D2. Clave de la página OCR:**
  - (a) `sha256` del archivo, calculado localmente. Robusto, pero exige tener el PDF.
  - (b) el `md5` de Zotero (`zotero_attachments.md5`, `schema.ts:715`). Permitiría reusar sin el archivo,
    pero depende de que Zotero lo informe, y en modo local puede faltar.
  - (c) los dos: `sha256` cuando hay archivo y `md5` como respaldo.

  Recomendado: (a). Sin el PDF tampoco hay pasajes que construir.
- **D3. Composición de `ocr_contract`:** proveedor + modelo + versión del prompt/parser de GLM-OCR. Hoy no
  existe un contrato de OCR versionado como el de embeddings. Hay que definirlo, y fijar que un cambio de
  prompt o parser lo invalida.
- **D4. Formato del vector en el cable:** f32 exacto (4 KiB por vector de 1024 dimensiones) o int8 (1 KiB,
  ya existe para `vecscan` en `vector_index.rs`). Recomendado: f32. El int8 se deriva localmente, y
  cuantizar en el cable cambiaría resultados. **Con la medición de §1.5** (392,5 MiB de vectores de
  pasajes en f32 para 1.437 adjuntos), f16 los dejaría en unos 196 MiB, sin pérdida apreciable para la
  similitud coseno, a costa de dejar de reusar el vector byte a byte. Los expertos deberían decidir con
  ese número y con el precio real de embeber unos 69 M caracteres en el proveedor.
- **D5. Espera máxima del "pull antes de pagar"** (§2.6) y si el usuario puede saltearla. Debe cubrir el
  primer arranque de un dispositivo nuevo, donde la capability todavía es desconocida. Debe dimensionarse
  para el volumen del catch-up (unos 400 MiB en una cuenta como la de §1.5), no para un pull corto. También
  fija el tope de cada pull de la barrera de frescura y su cota `F` de antigüedad del último pull (§2.6).
- **D6. Privacidad y cuentas:** el texto OCR de los PDF pasa a guardarse en Cloud. Las Colecciones ya suben su
  OCR, así que el precedente existe. Falta confirmar que la política de privacidad lo cubre para la
  Bibliografía. Por defecto, una fila de otra cuenta nunca se usa ni se empuja (§2.9); si el owner quiere
  reusar resultados entre sus propias cuentas, hay que decidirlo aquí. Esa regla es más estricta que la del
  corpus actual, que `seed_account` (`sync/pull.rs:105`, `sync/engine.rs:680`) vuelve a sembrar en cada
  cuenta nueva, incluido el OCR de las Colecciones. Alinear ambas políticas (endurecer el corpus o relajar la
  caché) se decide aquí y no se toca en este plan.
- **D7. Recolección de basura:** la caché crece sin borrar. ¿TTL, cuota por cuenta, o borrar en el servidor
  las entradas de contratos retirados?
- **D8. Transporte (§2.5):** (a) tablas físicas por el oplog genérico, como `web-capture-v1`, o (b) canal
  dedicado con outbox, cursor y catch-up propios, como writing y research. Ninguna de las dos exige migración.
  Recomendado: (b), porque evita el LWW, las versiones por fila, la siembra por `seed_account` y cualquier
  cambio en `SYNCED_TABLES`, a costa de más código de cliente.

## 5. Riesgos

| Riesgo | Mitigación |
| --- | --- |
| Mezclar espacios vectoriales | La clave incluye `contract_hash`; el ancho se valida al leer; el contrato viaja con su fila inmutable. |
| Lite y Pro con configuraciones distintas | Solo hay acierto con el mismo contrato. En el canónico `baai/bge-m3`/1024 (`CANONICAL_EMBEDDING_MODEL` y `CANONICAL_EMBEDDING_DIMENSIONS`, `nlp/embeddings.rs:37-39`; el espacio de la Bibliografía se resuelve en `processing/eligibility.rs:105-131`) coinciden; si un dispositivo usa embeddings locales (`local-ml`), paga su propio espacio, y es correcto que lo haga. |
| Pagar igual por carrera al arrancar | Pull antes de pagar (§2.6), con tres estados de capability: se espera mientras es desconocida. Dos dispositivos que procesan a la vez pueden duplicar alguna unidad; se acepta y se documenta. |
| Catch-up terminado pero caché desactualizada | Haber terminado el catch-up del epoch no prueba que la caché contenga lo que otro dispositivo subió después: B puede recibir el catálogo de Zotero antes del siguiente pull de Cloud y pagar lo que A ya subió. Barrera de frescura (§2.6): antes de pagar un miss, el último pull debe ser posterior a la admisión del trabajo y reciente (`F`); si no, un pull incremental acotado por D5 y nueva consulta. Residuo: lo que A sube después de que empezó ese pull. |
| Respuesta en vuelo de una cuenta anterior | Un pull o push que salió con el token de A puede volver tras el logout y el login de B. Se congela el alcance (cuenta, servidor, dispositivo, epoch, incarnation) al empezar y se verifica en la transacción que instala filas o mueve cursor/capability/`pushed`; si cambió, la respuesta se descarta (§2.9, patrón de `sync/writing_pull.rs:13-30` y `sync/writing_receive.rs:29-36`). Test en WU4. |
| El trabajo ya pagado no se exporta | Backfill (§2.3b), solo para la cuenta dueña original del archivo. Pérdida acotada: las respuestas de OCR vacías de archivos ya procesados que dejaron la fila nativa no tienen evidencia y se pagan una vez más; las recuperables de un PDF mixto no se repagan porque salen de la ventana (§2.4). |
| OCR privado empujado a otra cuenta tras un cambio de cuenta | Cada fila lleva su `account_id`; solo se empuja y se consulta lo de la cuenta actual; en logout se resetean `pushed`, cursor, capability y catch-up y las filas se conservan. El OCR escrito offline tras el logout toma la última cuenta vinculada (`derived_bound_account`, que el logout no borra), y solo la primera vinculación del archivo adopta filas `''` (§2.9). El backfill no reatribuye derivados ajenos: las fuentes no tienen `account_id`, así que solo siembra la cuenta dueña original del archivo (`derived_owner_account`, §2.3b); una cuenta posterior no exporta lo que ya estaba en el archivo. Test en WU1, WU1b y WU4. Residuo: un archivo que ya tuvo cuentas y se actualiza estando sin sesión no tiene las marcas; si escribe OCR offline antes del siguiente login, la primera cuenta que entre lo adoptaría y sería su dueña. Si se actualiza con sesión, queda como dueña la cuenta de esa sesión, aunque no haya sido la primera. |
| Volumen | El denominador "pasajes 1050" del pie cuenta **adjuntos** (tareas `bibliography_extract`, `processing/repository.rs:1751-1790`), no vectores. Cada adjunto produce muchos pasajes: unos ~800 caracteres con 100 de solapamiento (`CHUNK_TARGET_CHARS`, `CHUNK_OVERLAP_CHARS`, `bibliography/chunks.rs:20-23`). **Medido (§1.5):** 2.810 vectores de fichas (11,0 MiB) y 100.477 vectores de pasajes (392,5 MiB en f32, unos 70 por adjunto); el OCR, 1,05 M caracteres. El push inicial de una cuenta como esta ronda los 400 MiB. Lotes acotados por bytes. Cuota en el servidor (D7). |
| Un dispositivo viejo rechaza el canal | Gating por capability y por epoch, como `web-capture-v1`. Un cliente viejo nunca ve las tablas nuevas. |
| Resultado corrupto en la caché | Se valida al leer: ancho del vector, y `text_hash` recalculado sobre `text_content` de la página OCR. Si no coincide, se descarta y se paga. |
| No determinismo del proveedor | El mismo texto puede dar vectores apenas distintos. Gana el primero que escribe y la entrada es inmutable. Es inocuo para la búsqueda. |
| Romper el sync actual | Con la opción (b) de §2.5: cursor propio, sin tocar `last_pull_seq` ni `SYNCED_TABLES` (con (a) habría que cambiar `SYNCED_TABLES`). Toda prueba manual con `ENTROPIA_DEV_PROFILE` y servidor mock, nunca contra el archivo real (`AGENTS.md`). |

Clasificación ODD: **riesgo alto** (sincronización, contrato con el servidor, datos guardados). Requiere
verificación independiente por slice.

## 6. Supuestos a verificar antes de implementar

- **S-a.** La fragmentación es determinista dadas las páginas y `chunking_contract`, es decir, el mismo
  `text_hash` por pasaje en dos dispositivos. Si no lo fuera, los vectores de pasaje no tendrían acierto
  (solo se perdería ahorro, no habría error).
- **S-b.** El perfil de ficha produce el mismo `input_hash` en dos dispositivos con la misma cuenta de
  Zotero: el orden de creadores y la normalización de tags ya son estables según `profile.rs`. Verificarlo
  con dos espejos reales, local y web.
- **S-c.** El texto nativo (PDFium) es igual en dos dispositivos con el mismo archivo. Influye en qué
  páginas se eligen para OCR, pero no en la clave de la caché.
- **S-e.** `embed_text` y `embed_many` dan el mismo vector para el mismo texto, de modo que fichas, pasajes y
  fragmentos de Colecciones comparten la clave `(contract_hash, input_hash)` (§2.8).
- **S-f.** `ocr_markup_to_text` es idempotente sobre su propia salida, de modo que el texto OCR guardado sirve
  como entrada del backfill y de la lectura (la reutilización de reproceso ya lo da por hecho,
  `bibliography/processing.rs:3382-3392`).
- **S-d.** El servidor admite tablas de cable nuevas con clave compuesta sin columna `id`. `vec_assets` ya
  usa un `id` sintético (`sync/push.rs:247-256`); se podría replicar.

## 7. Plan de trabajo (slices, cada uno con su commit)

1. **WU1. Caché local y escritura.** Tablas `sync_derived_*` en `ensure_sync_schema` (con `account_id` y
   `outcome`); escritura en la misma transacción en los puntos de §2.3, incluida la lista `ocr_outcomes`.
   Semántica de cuenta de §2.9 (sellado, adopción de filas `''`, reset en logout). Tests: un resultado pagado
   deja su entrada; un rollback no la deja; **una respuesta de OCR vacía con fila nativa conservada deja
   `outcome='empty'`**; tras logout las filas siguen, con `pushed=0` y sin cursor ni capability; **tras un
   cambio de cuenta, el OCR sin empujar de la cuenta anterior no se empuja a la nueva ni se usa para
   procesar con ella. Caso: A cierra sesión, se procesa OCR offline, B inicia sesión → B no recibe ni usa esas
   filas (llevan el `account_id` de A, no `''`), y si A vuelve las re-empuja. Caso: en un archivo sin ninguna
   vinculación previa, las filas escritas antes del primer login pasan a la cuenta que inicia sesión y se
   escribe `derived_bound_account`.**
   - **WU1b. Backfill** (§2.3b). Tests: un archivo ya procesado (ficha sin cambios, extracción vigente) deja
     sus vectores y su OCR en la caché sin llamadas pagas y sin readmitir tareas; el sha256 del archivo se
     obtiene del PDF local y una discrepancia de `mtime`/`bytes` omite la página; las páginas `quality='empty'`
     entran como `'empty'`; un segundo pase no duplica nada;
     **A cierra sesión con OCR sembrado, B inicia sesión y el backfill corre: B no recibe ninguna entrada
     nueva procedente de esas filas, y las de A siguen con `account_id` de A. Con A de vuelta, la siembra
     corre y no repite la lectura de los PDF (marca por cuenta conservada).**
2. **WU2. Lectura antes de pagar.** Consultar la caché en los vectores y en el OCR; validar al leer.
   Tests: con la entrada sembrada, el proveedor doble recibe **cero** llamadas y el resultado publicado es
   byte a byte igual; con un contrato distinto, el proveedor sí se llama; una entrada corrupta se descarta;
   **una página sembrada como `'empty'` no se reenvía a GLM-OCR y la fila publicada sigue siendo la nativa**;
   las páginas en caché no entran en las ventanas de PDF completo; **PDF de diez páginas con cuatro aciertos y
   seis fallos (también con los fallos como respuestas vacías históricas sin evidencia): ningún rango enviado a
   `recognize_pdf_pages` contiene una página en caché, y se prueba con aciertos contiguos y alternados**;
   **sin sesión, con filas de `derived_bound_account` sembradas, el proveedor recibe cero llamadas, y las filas
   de otra cuenta no se usan**.
   - **WU2b. Colecciones** (§2.8). Consultar y escribir la caché en el ejecutor de embeddings de
     `processing/embedding.rs`. Tests: con la caché sembrada, un activo `IncompleteChunks` se completa con
     **cero** llamadas a `embed_text` y las filas de `rag_chunks` publicadas son iguales; con otro contrato
     sí se llama.
3. **WU3. Contrato de OCR versionado** (D3). Tests: un cambio de versión invalida los aciertos.
4. **WU4. Transporte `bibliography-derived-v1`.** Push y pull con capability, cursor propio, catch-up por
   epoch y lotes por bytes, contra el mock de `SyncApi`, según la decisión D8. Tests: dos dispositivos simulados (patrón
   `tests/web_capture_sync_two_device.rs`), el segundo procesa la misma biblioteca con **cero** llamadas a
   los proveedores; un servidor sin la capability no recibe nada y el corpus sigue sincronizando; tras un
   logout y un login con otra cuenta no se empuja ninguna fila de la cuenta anterior, incluidas las escritas offline tras ese logout;
   **pull suspendido de A: se cambia a B y se libera la respuesta; no instala filas ni modifica el cursor, la
   capability ni el catch-up de B, y B no puede consultarla. Igual con un push en vuelo: no pone `pushed = 1`
   bajo la sesión de B.**
5. **WU5. Pull antes de pagar y pie** (§2.6, D5). Tests: con capability **desconocida** y sesión, el
   ejecutor espera hasta el tope y después procesa; con ausencia confirmada (`advertised:false` en el epoch
   actual) o sin sesión no espera; la espera va antes de la primera llamada paga y no bloquea la extracción
   nativa; si `needing` está vacío no espera; el veredicto se recuerda por epoch; **B termina el catch-up, A sube resultados nuevos en el mismo epoch, B
   recibe el catálogo de Zotero antes del siguiente pull: B hace un pull incremental antes de pagar, y el
   proveedor doble recibe cero llamadas por lo que A subió; un pull reciente y posterior a la admisión no
   se repite, y los misses de varias unidades comparten un solo pull**; el texto del pie aparece
   en `i18n.ts` y en `BatchStatusIndicator.test.ts`.
6. **WU6. Servidor** (fuera de este repo). Capability, tablas de cable, idempotencia y cuota.
7. **WU7. Verificación con dos perfiles de desarrollo** contra el servidor de staging: se procesa una
   biblioteca chica en A (y una Colección chica) y se mide en B que fichas, pasajes y fragmentos llegan al
   100 % sin gasto en OpenRouter ni en GLM-OCR. Incluye el caso de A actualizado sobre un archivo ya
   procesado (solo backfill) y el primer arranque de B con la capability todavía sin descubrir.

## 8. Criterio de aceptación

Con dos dispositivos en la misma cuenta, el mismo Zotero y la misma configuración de embeddings y OCR:
después de que A termina, B completa "fichas N/N · pasajes M/M" **sin ninguna llamada paga a OpenRouter ni
a GLM-OCR** para el material que A ya procesó. Un tercer dispositivo, C, se comporta igual que B.

El pie no alcanza como prueba: su contador da por terminadas también las tareas `failed` y `cancelled`
(`processing/repository.rs:1751-1790`). La aceptación se mide en filas: en B, la cantidad de
`bibliographic_item_embeddings` y `bibliographic_chunk_embeddings` de la generación activa y de páginas
`method='ocr'` es igual a la de A, sin tareas `failed`, y el proveedor doble registra cero llamadas. Además:

- **Material ya procesado antes de actualizar:** si A ya había procesado la biblioteca antes de tener este
  canal, tras actualizar y sembrar (§2.3b) B también la completa sin gasto, salvo las páginas de OCR con
  respuesta vacía que dejaron la fila nativa en archivos anteriores al canal, que no tienen evidencia
  y se pagan una vez.
- **Primer arranque de B:** con sesión y capability aún desconocida, B espera (hasta el tope) antes de la
  primera llamada paga, en vez de pagar de inmediato.
- **Frescura:** si A sube resultados después de que B terminó el catch-up y B recibe el catálogo de Zotero
  antes del siguiente pull, B los baja antes de pagar y no hace llamadas por ellos.
- **Colecciones:** B completa los fragmentos de las Colecciones de A sin llamadas a `embed_text`.
- **Cuentas:** tras un cambio de cuenta no se empuja ni se usa ninguna fila de la cuenta anterior, el logout
  no borra las filas y el backfill de la cuenta nueva no recrea bajo ella el OCR de la anterior. Una respuesta
  que salió con la sesión de la cuenta anterior no instala filas ni mueve cursor, capability o `pushed` de la
  nueva.
- **Sin sesión:** un dispositivo sin sesión que reprocesa material ya cacheado por el dueño del archivo no
  hace llamadas pagas por él.
