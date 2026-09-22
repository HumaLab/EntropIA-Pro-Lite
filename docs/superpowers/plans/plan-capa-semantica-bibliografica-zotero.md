# Capa semántica bibliográfica de Zotero — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. No ejecutar etapas sin autorización del usuario para iniciar implementación.

**Goal:** Hacer buscables obras y pasajes académicos vinculados obligatoriamente a Zotero, verificarlos en su adjunto e incorporarlos al editor sin confundir bibliografía con fuentes documentales.

**Architecture:** Zotero conserva la autoridad bibliográfica; EntropIA mantiene una réplica derivada e índices bibliográficos separados. Biblioteca procesa el adjunto PDF mediante extracción nativa y análisis de layout, con OCR selectivo cuando el texto no es utilizable. Su recorrido de extracción es independiente del OCR por asset de Fuentes; comparte únicamente infraestructura de coordinación cuando corresponde y el motor CSL existente, sin otra cola ni otro sistema de citas.

**Tech Stack:** Svelte 5, TypeScript, Tauri 2, Rust, SQLite/Drizzle, FTS5, proveedores de embeddings existentes, Hayagriva/citationberg y API de Zotero.

**Estado:** especificación técnica reformulada y plan maestro por etapas para revisión. E0 registró las decisiones operativas y la matriz de evidencia; las capacidades Zotero en vivo siguen pendientes de ejercitar porque todavía no se seleccionaron las bibliotecas de prueba y sus conexiones concretas. Este documento no acredita funcionalidades implementadas, pruebas aprobadas ni compatibilidad todavía no ejercitada. Los contratos siguientes son objetivos de implementación, no descripciones de interfaces ya disponibles.

**Origen:** documento del usuario trasladado desde `S:/Descargas/plan-capa-semantica-bibliografica-zotero.md`. El commit `9d5fc99` conserva sus 590 líneas originales sin cambios. Esta revisión incorpora la auditoría y la aclaración definitiva del usuario: la regla de OCR por asset/página pertenece a Fuentes, no al nuevo recorrido de Biblioteca.

## Global Constraints

- Trabajar exclusivamente en `feature/zotero-bibliografia-semantica`. No modificar `main`, fusionar ni publicar sin autorización.
- Antes de crear la rama, comprobar árbol limpio y actualizar la referencia remota de `main`; ante cambios locales sin confirmar, detenerse sin descartarlos ni sobrescribirlos. Esa comprobación ya se realizó para el traslado documental.
- Entregar unidades de comportamiento verificables, con commits pequeños. Una etapa puede requerir varios commits, pero ninguno debe presentarse como una funcionalidad completa si solo contiene estructura vacía.
- El `Cargo.lock` reescrito por el patch local de `entropia-agent` no se incluye en los commits de este trabajo. No ejecutar Cargo para validar cambios exclusivamente documentales.
- Ninguna obra vectorizada puede carecer de una identidad Zotero previamente verificada. Una clave con formato válido no prueba que exista el ítem.
- No crear una biblioteca autónoma que compita con Zotero ni inferir referencias finales mediante un LLM.
- Fuentes y bibliografía mantienen catálogos, índices, filtros y tipos de evidencia separados. Compartir SQLite o infraestructura no significa compartir las tablas del corpus.
- No introducir bibliografía como assets ficticios en `assets/items`, `vec_assets` o `rag_chunks` para reutilizar consultas o Lotes.
- **Fuentes conserva su flujo actual: un asset por página y OCR sobre ese asset, incluido GLM-OCR. Esa regla no se traslada a Biblioteca. Biblioteca trabaja sobre adjuntos PDF, prioriza texto nativo y layout, y tiene extracción, elegibilidad y OCR propios, sin crear assets del corpus ni invocar su ejecutor de OCR.**
- No reemplazar Hayagriva, duplicar las citas del editor ni crear una segunda cola. Los cambios compartidos son parte de las etapas de Bibliografía, no una refactorización general previa.
- Preservar proyectos, manuscritos, exportaciones, citas históricas e índice documental. No reclasificar documentos automáticamente.
- No borrar ni modificar datos Zotero como efecto de limpiar derivados locales. Las escrituras explícitas de ingesta requieren autorización propia.
- Para E0, el acceso autorizado se limita a lectura y escritura sobre bibliotecas personal y grupal aisladas de prueba seleccionadas explícitamente. No acceder a bibliotecas privadas ni ejecutar escrituras hasta confirmar esos objetivos y su conexión.
- Desde E1 se aplica TDD estricto, con el ciclo RED → GREEN → REFACTOR y la verificación focalizada de cada unidad.
- La entrega se realizará directamente en este worktree y su rama `feature/zotero-bibliografia-semantica`, conservando un commit por unidad de trabajo; no se adopta Feature Branch Chain ni PR apiladas hacia `main` sin una decisión posterior.
- Conservar el uso manual del editor y de fuentes aunque Zotero o un proveedor de modelos no estén disponibles.
- Node 22+, pnpm 9.x. Pro: `VITE_LOCAL_ML=1` y feature Rust `local-ml`; Lite: `VITE_LOCAL_ML=0` sin features Rust.
- Reutilizar componentes, tokens y `ActionIcon`. No añadir dependencias sin justificar que las existentes no resuelven el requisito.

---

## 1. Lectura rápida y alcance completo

El recorrido final será:

```text
Zotero → catálogo verificado → perfil de obra / adjuntos PDF → Lotes
       → extracción nativa + layout → OCR selectivo solo si hace falta
       → índices bibliográficos → obras → pasajes → cita existente

Fuentes documentales → recuperación documental independiente
Bibliografía académica → recuperación bibliográfica independiente
                      → contexto con presupuestos y procedencia separados
```

La entrega completa incluye estas capacidades; el orden de las etapas no las elimina del alcance:

| Capacidad                                                                                    | Etapa responsable     |
| -------------------------------------------------------------------------------------------- | --------------------- |
| Elegir biblioteca personal, grupo y colecciones; sincronizar metadatos, etiquetas y adjuntos | E1                    |
| Abrir una obra en Zotero y gestionar vínculo, disponibilidad y derivados                     | E1, E4                |
| Procesamiento persistente, recuperación, cancelación y prioridad editorial                   | E2                    |
| Perfil semántico por obra, búsqueda híbrida y filtros                                        | E3                    |
| Texto nativo, layout, OCR selectivo, chunks y recuperación jerárquica                        | E4                    |
| Arrastrar un PDF, vincularlo o crear primero su ítem y adjunto en Zotero                     | E5                    |
| Buscar desde una selección, examinar evidencia e insertar/editar citas                       | E6                    |
| Consulta solo fuentes, solo bibliografía o combinada                                         | E7                    |
| Evaluación, privacidad y compatibilidad Pro/Lite y proyectos existentes                      | E0 y todas las etapas |

**Condición de arranque:** revisar este documento con el usuario antes de tocar código de aplicación. E0 verifica capacidades concretas del entorno y registra sus resultados aquí. Si contradicen una decisión del plan, se presenta la diferencia antes de ejecutar la etapa afectada; no se sustituye silenciosamente una función por otra más pequeña.

## 2. Lo que ya existe y qué se modifica

Las rutas enlazadas son el punto de entrada para el relevamiento; sus funciones deben volver a leerse al ejecutar cada etapa. Un módulo existente no garantiza que su contrato sirva sin cambios.

| Área                  | Evidencia existente                                                                                                                                                                                                                                      | Uso y límite para Bibliografía                                                                                                                                                            |
| --------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Zotero local          | [connector.rs](../../../apps/desktop/src-tauri/src/writing/zotero/connector.rs), [mirror.rs](../../../apps/desktop/src-tauri/src/writing/zotero/mirror.rs)                                                                                               | Reutilizar HTTP, paginación, diagnóstico y resolución de padres. El espejo `key/version/CSL` es caché del editor, no checkpoint durable de ingesta.                                       |
| Lista Zotero          | [writing-zotero.ts](../../../apps/desktop/src/lib/writing-zotero.ts)                                                                                                                                                                                     | Cambiar la entrega de strings CSL por registros con identidad nativa. Estado, promesas y respuestas deben corresponder a la biblioteca seleccionada.                                      |
| Citas canónicas       | [citations.ts](../../../packages/ui/src/components/WritingEditor/citations.ts), [citation-cluster.ts](../../../packages/ui/src/components/WritingEditor/citation-cluster.ts), [repository.rs](../../../apps/desktop/src-tauri/src/writing/repository.rs) | Conservar inserción, clusters y guardado transaccional; ampliar identidad en todo el recorrido. No alcanza con agregar columnas SQL.                                                      |
| CSL y exportación     | [render.rs](../../../apps/desktop/src-tauri/src/writing/csl/render.rs), [citation-clusters.ts](../../../apps/desktop/src/lib/citation-clusters.ts), [writing-export.ts](../../../apps/desktop/src/lib/writing-export.ts)                                 | Conservar Hayagriva y snapshots. Verificar narrativa, notas estructurales, localizadores y desambiguación documental.                                                                     |
| Lotes                 | [scheduler.rs](../../../apps/desktop/src-tauri/src/processing/scheduler.rs), [repository.rs](../../../apps/desktop/src-tauri/src/processing/repository.rs), [0032](../../../packages/store/src/migrations/0032_batch_processing.sql)                     | Ya hay leases, checkpoints, recibos, reintentos y recuperación. Ampliar sujetos y publicación: hoy dependen de assets documentales y operaciones OCR/embedding.                           |
| Extracción de Fuentes | [processing/ocr.rs](../../../apps/desktop/src-tauri/src/processing/ocr.rs), [ocr/pdf.rs](../../../apps/desktop/src-tauri/src/ocr/pdf.rs)                                                                                                                 | Referencia para preservar el comportamiento existente, no recorrido a reutilizar en Biblioteca. Su ejecutor, assets y reglas de elegibilidad quedan fuera de la extracción bibliográfica. |
| Embeddings            | [embeddings.rs](../../../apps/desktop/src-tauri/src/nlp/embeddings.rs), [eligibility.rs](../../../apps/desktop/src-tauri/src/processing/eligibility.rs)                                                                                                  | Separar configuración efectiva de constantes canónicas al admitir, reanudar, publicar y consultar trabajos bibliográficos.                                                                |
| Recuperación          | [rag/retrieval.rs](../../../apps/desktop/src-tauri/src/rag/retrieval.rs), [writing/retrieval.rs](../../../apps/desktop/src-tauri/src/writing/retrieval.rs)                                                                                               | Reutilizar primitivas de ranking/FTS; no su SQL, IDs documentales o política de un resultado por asset como modelo bibliográfico.                                                         |
| Esquema               | [runner.ts](../../../packages/store/src/runner.ts), [schema.ts](../../../packages/store/src/schema.ts), [0035](../../../packages/store/src/migrations/0035_writing_workspace.sql)                                                                        | Migraciones nuevas, registro efectivo del runner y esquema actualizado. `writing_zotero_citations` representa ocurrencias de citas, no catálogo de obras.                                 |
| Credenciales          | [settings.rs](../../../apps/desktop/src-tauri/src/settings.rs)                                                                                                                                                                                           | Reutilizar keyring e incorporar expresamente las nuevas claves protegidas. Un nombre que parezca secreto no garantiza almacenamiento seguro.                                              |

### Separación expresa de los recorridos de extracción

La aclaración definitiva del usuario distingue dos dominios:

- **Fuentes:** `PDF → asset por página → OCR por asset → chunks de ese asset`. Es el flujo existente y no se modifica por esta funcionalidad.
- **Biblioteca:** `adjunto PDF Zotero → texto nativo + layout → OCR selectivo de contenido sin texto utilizable → representación estructurada → chunks con procedencia`. Es un recorrido nuevo, no una adaptación del ejecutor OCR de Fuentes.

Se espera que predominen PDFs nativos en Biblioteca; es una expectativa de uso, no motivo para excluir escaneados o PDFs mixtos. Extraer layout no equivale a hacer OCR: orden de lectura, columnas, bloques y coordenadas se necesitan también cuando el texto ya existe.

No se exige dividir cada PDF bibliográfico en archivos/ assets de una página. Las páginas son localizadores internos del adjunto, no unidades obligatorias de almacenamiento o llamada a proveedor. La granularidad de OCR bibliográfico se decide por capacidades, calidad, privacidad y reanudación de su propio recorrido, sin heredar el límite de Fuentes ni enviar PDFs completos por defecto.

Se mantiene retirado el hallazgo que atribuía pérdida de páginas al OCR habitual de Fuentes. En Biblioteca sí hay que diseñar y verificar explícitamente la correspondencia entre texto estructurado, bloques y páginas del PDF; no suponer que la resuelve un asset documental.

## 3. Conexión con Zotero y matriz de capacidades

### 3.1. Estrategia

Preferir la API local para lectura y adjuntos disponibles en el equipo. La API web v3 es una conexión explícita alternativa para bibliotecas sincronizadas y operaciones autorizadas; no se activa como fallback silencioso ante un fallo local. El cambio de conexión no mezcla cursores ni sustituye automáticamente una biblioteca por otra.

Conector de navegador, API HTTP local y API web no son nombres intercambiables. Un ping del conector acredita respuesta, no permisos de lectura/escritura de la API ni identidad de la biblioteca.

| Conexión                                                     | Contrato requerido                                                                                                                                                                                                                            |
| ------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Local con identidad estable y versiones locales fiables      | Detectar capacidades; particionar por identidad de servidor. Usar lectura incremental y escrituras solo si la versión instalada las soporta y el usuario las autoriza.                                                                        |
| Local anterior sin identidad estable o sin versiones fiables | Mantener lectura compatible; crear ámbito de conexión confirmado por el usuario. No certificar identidad por `users/0` ni por un contador. Reconciliar contenido completo para detectar cambios locales, sin asumir que `since` captura todo. |
| Web v3                                                       | Verificar permisos personales/grupales, acceso a archivos y cuota. Guardar credencial en keyring; aplicar paginación, `Backoff`, `Retry-After` y precondiciones de escritura.                                                                 |
| No disponible o API deshabilitada                            | Diagnóstico honesto y copia offline según §5. No inferir que Zotero está desinstalado ni que los ítems fueron eliminados.                                                                                                                     |

La documentación oficial describe autorización de escritura, `Zotero-Server-ID` y versiones locales independientes en Zotero 10+. Para instalaciones anteriores no se prometen esas capacidades. La compatibilidad se determina por versión y respuestas verificadas, no solo por disponibilidad de `/connector/ping`.

Para una conexión legacy sin prueba de identidad estable, pedir reconfirmación del ámbito al reabrir la conexión o detectar cambio de biblioteca/perfil. Mientras no esté confirmada, no sincronizar sobre la partición anterior ni publicar derivados nuevos. Una confirmación asistida no debe rotularse como identificación automática infalible. Las citas históricas siguen disponibles por snapshot.

### 3.2. Registro obligatorio de E0

Registrar versión de Zotero, API, presencia de identidad de servidor, identificación personal/grupal, semántica de versiones, lectura de colecciones/adjuntos, resolución de archivos, rutas de apertura y disponibilidad de escrituras. Comprobar al menos una biblioteca personal y una grupal de prueba con autorización; no publicar sus datos privados.

La auditoría previa observó respuestas GET `200`, ausencia de `Zotero-Server-ID`, metadatos nativos con etiquetas/colecciones y un ítem cuyo `key` difería de su `id` CSL. Eso no certifica escrituras, grupos ni todas las versiones de Zotero.

#### Registro E0: decisiones, evidencia y bloqueos actuales

Las decisiones operativas de esta reanudación quedan congeladas así:

- **Acceso Zotero:** lectura y escritura únicamente sobre bibliotecas personal y grupal aisladas de prueba. La autorización no habilita una biblioteca privada ni una escritura sobre un destino no seleccionado.
- **Pruebas desde E1:** TDD estricto, RED → GREEN → REFACTOR.
- **Entrega:** trabajo directo en este worktree y rama, con commits de unidad; no se inicia una cadena de ramas ni PR apiladas.

La matriz siguiente distingue evidencia existente de capacidad ejercitada en vivo. No se registraron credenciales ni texto bibliográfico privado; solo se conservaron metadatos del fixture aislado y sus identificadores técnicos.

| Capacidad obligatoria | Evidencia disponible | Estado E0 y bloqueo |
| --- | --- | --- |
| Transporte local y diagnóstico | `GET /connector/ping` respondió `200`; endpoint `http://127.0.0.1:23119`; Zotero `9.0.3`, API `3`, esquema `42` | Ejercitado para la instancia local; el perfil personal aislado todavía requiere una sesión separada |
| JSON nativo, CSL e identidad `key`/`CSL.id` | El fixture devolvió key nativo `7EMV3G8H`, biblioteca `group/6680944`, tipo `book` y versión `3` | Identidad nativa y versión verificadas; la inclusión CSL queda pendiente de una prueba específica |
| Biblioteca personal y biblioteca grupal | Grupo privado `prueba`, `groupID=6680944`, `filesEditable=true`, inicialmente vacío; destino runtime `L9` resuelto sin hardcodear | Grupo aislado verificado; perfil personal aislado pendiente porque Zotero solo expone una biblioteca personal por instancia |
| Colecciones, etiquetas y adjuntos | El fixture quedó con la etiqueta `entropia-e0-probe`; no se usó PDF ni colección privada | Etiquetas y lectura básica verificadas; colecciones, adjuntos y archivos siguen pendientes |
| Resolución de archivo y apertura | No se creó ni resolvió ningún adjunto | No verificado; requiere fixture PDF aislado y ruta de apertura segura |
| Versiones y `Zotero-Server-ID` | Headers `X-Zotero-Version=9.0.3`, `Zotero-API-Version=3`, `Zotero-Schema-Version=42`, `Last-Modified-Version=3` | Versionado local verificado; `Zotero-Server-ID` no fue acreditado por esta conexión |
| Permisos y escrituras | `POST /connector/saveItems` respondió `201` y `POST /connector/updateSession` respondió `200`, sin API key, sobre `prueba` | Escritura local grupal verificada; el movimiento se ejercitó al mismo destino `L9`, no un cruce entre bibliotecas |
| Web v3, cuota y reintentos | Contratos oficiales citados en §22 | No ejercitado; no se agrega una key web ni fallback remoto implícito |

**Resultado live de E0 (grupo aislado):** con el grupo `prueba` visible en Zotero, la resolución segura de targets identificó `L9` como `prueba` y `filesEditable=true`. Se creó el fixture `EntropIA Zotero E0 write probe 2026-09-20` y se confirmó por `GET /api/groups/6680944/items/top` (`200`, un registro, `Last-Modified-Version=1`) y por la lectura puntual del ítem (`version=3`, `Last-Modified-Version=3`). La prueba acredita lectura/escritura local sobre una biblioteca grupal aislada sin credencial web, no acredita todavía la biblioteca personal aislada, adjuntos, CSL ni Web API. El fixture es controlado y queda disponible para las pruebas siguientes; no contiene bibliografía privada.

La operación de escritura no es atómica entre `saveItems` y `updateSession`: el primer endpoint usa el destino seleccionado en la UI y el segundo recibe el `treeViewID` explícito. E5 debe registrar el destino inicial antes de escribir, verificar el resultado después de mover y conservar una recuperación/limpieza explícita si el movimiento falla. Nunca hardcodear `L9`: el ID es local a la instalación y debe resolverse emparejando el grupo.

**Semilla de evaluación E0:** `zsb-eval-v1`. Antes de E3/E7 se formará con material sintético o autorizado y solo almacenará IDs opacos, consultas, juicios humanos por obra/pasaje, procedencia/localizador esperado, Recall@k, nDCG@k, MRR, latencia y coste. Incluirá tema, autor, obra conocida, pasaje poco representado en metadatos, filtros combinados, ausencia de resultados y falsos positivos graves. Los juicios se tomarán después de disponer de la biblioteca personal de prueba y antes de ajustar umbrales, reranking o expansión.

E0 ya acredita el transporte local y la escritura/lectura grupal aislada. E1 puede avanzar sobre ese ámbito; la biblioteca personal aislada, la resolución de adjuntos y cualquier Web API permanecen como bloqueos explícitos, no como capacidades supuestas.

**Commit de evidencia E0:** `78cc46f` (`docs: record live Zotero group capability`). No contiene cambios productivos ni credenciales.

## 4. Identidad e integridad de extremo a extremo

### 4.1. Identidad única y referencias

Separar la identidad del dato, su procedencia de sincronización y su representación CSL. Contrato propuesto, con nombres de tipos orientados a las fronteras Rust/TypeScript:

```ts
type ZoteroIdentity = {
  namespaceId: string
  libraryType: 'user' | 'group'
  libraryId: string
  itemKey: string
}

type ZoteroReference = {
  identity: ZoteroIdentity
  sourceOrigin: 'local' | 'web'
  sourceInstanceId: string | null
  itemVersion: number | null
  cslJsonSnapshot: string
  verifiedAt: number
}
```

- `namespaceId` identifica una partición de origen confirmada; no es un secreto ni un ID de documento.
- Unicidad sobre `(namespace_id, library_type, library_id, item_key)` mediante la relación con biblioteca.
- `itemKey` siempre procede de la respuesta nativa de Zotero. `CSL.id`, DOI, ISBN, título y nombre de archivo no lo sustituyen.
- Un contador de versión describe cambios dentro de su ámbito, no identifica el servidor ni es comparable entre local y web.
- Si dos conexiones representan la misma biblioteca, la equivalencia requiere una reconciliación confirmada. No deduplicar automáticamente entre particiones por similitud ni mantener dos catálogos del mismo origen cuando se haya establecido su equivalencia.
- Adjuntos y colecciones tienen identidad propia dentro de la misma biblioteca. Un adjunto debe resolver un padre bibliográfico regular; notas y PDFs sueltos sin padre no cuentan como obra verificadamente citable.
- La capa semántica entrega `ZoteroReference` y evidencia; nunca solamente `item_key`.

### 4.2. Citas y migración histórica

Conservar la identidad completa en selección Zotero, nodo canónico, editor de cita, proyección SQL, historial, procedencia y exportación. Cambiar el localizador no debe resetear biblioteca, versión u origen. Guardar el manuscrito no debe convertir una cita inaccesible en `valid` ni una web en `local` por constantes del INSERT.

Deduplicar por identidad completa tanto en clusters como en bibliografía. Asignar al procesador CSL un identificador interno estable derivado de esa identidad y conservar el identificador CSL original como dato externo. Dos bibliotecas con la misma clave no deben colapsar.

Para ocurrencias de una misma obra con snapshots distintos, usar consistentemente la última versión bibliográfica aceptada en el manuscrito para su renderizado documental; registrar la actualización. Una sincronización no modifica silenciosamente snapshots de versiones históricas. Resolver conflictos de identidad antes de unificar referencias.

Las citas existentes cuyo `item_key` contenga una URI o citation key se conservan como referencias históricas sin vínculo resuelto hasta poder verificarlas. No rellenar por título aproximado, ni invalidar su exportación, ni imponerles una FK obligatoria al nuevo catálogo. La restricción de identidad verificada se aplica a la indexación nueva, no a borrar citas antiguas.

## 5. Estados, uso offline y borrado

Mantener tres ejes separados; no comprimirlos en un único `SYNCED/INDEXED`:

| Eje           | Estados y significado                                                                                                        |
| ------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| Transporte    | disponible, endpoint no disponible, API deshabilitada, timeout, respuesta inválida; reutilizar el diagnóstico existente      |
| Vínculo       | verificado, pendiente de verificación, huérfano, acceso revocado, eliminado en Zotero                                        |
| Procesamiento | pendiente, ejecutando etapa, reintento, bloqueado, indexado, desactualizado, error, cancelado; derivados del trabajo durable |

`UNLINKED` pertenece a la bandeja de pendientes, no al catálogo indexable. Una obra verificada sin PDF sigue siendo apta para perfil global y muestra **Falta texto completo**, no error de vínculo.

**Política offline:** permitir consultar derivados ya publicados de obras verificadas anteriormente, identificados como copia local y con fecha de última verificación, si no hay evidencia de revocación/borrado ni cambio de ámbito. Permitir citar sus snapshots. No crear nuevos vínculos ni indexar pendientes por asumir que Zotero existe. Un timeout por sí solo no cambia todos los ítems a inaccesibles.

Borrado, revocación o vínculo huérfano comprobados excluyen inmediatamente la obra y sus derivados de la recuperación normal, aunque se conserven para revisión. Al reconectar, una nueva verificación puede rehabilitarla. Salir de una colección seleccionada no equivale a haber sido eliminado de Zotero; actualizar membresía y alcance.

**Limpieza local:** borrar extracciones, layout, copias y archivos temporales gestionados del adjunto, chunks, perfiles y vectores de la selección solicitada, tras cancelar su demanda y bloquear republicación de trabajos antiguos. Mantener manuscritos, snapshots citados y eventos históricos. No tocar originales enlazados ni emitir escrituras/borrados Zotero. La conservación temporal o revinculación de huérfanos requiere decisión explícita del usuario.

## 6. Persistencia propuesta y propiedad de archivos

### 6.1. Tablas y relaciones

Los siguientes nombres son el diseño propuesto, no tablas ya presentes. Usar migraciones nuevas y FK reales dentro del dominio; evitar una tabla polimórfica de vectores sin integridad referencial.

| Registro                                                                | Contenido e invariantes                                                                                                                                                  |
| ----------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `zotero_connections`                                                    | origen, ámbito, identidad de servidor observada, capacidades, referencia a credencial y estado de confirmación; nunca secretos en claro                                  |
| `zotero_libraries`                                                      | conexión/ámbito, tipo, ID externo, nombre, versión confirmada, fecha y alcance de sincronización; unicidad dentro del ámbito                                             |
| `bibliographic_items`                                                   | biblioteca obligatoria, key nativo, versión, JSON nativo y CSL, título, creadores, publicación/editorial, fecha, DOI, ISBN, resumen, idioma, URL, hash y vínculo         |
| `bibliographic_collections` y `bibliographic_item_collections`          | claves Zotero, jerarquía y relación muchos a muchos; cambios de nombre/membresía sin reextraer PDFs                                                                      |
| `bibliographic_tags` y `bibliographic_item_tags`                        | etiquetas originales normalizadas para filtros, sin perder su procedencia                                                                                                |
| `bibliographic_attachments`                                             | padre obligatorio, key de adjunto, MIME, modo de enlace, nombre, localizador resoluble, versión, hash de bytes y disponibilidad                                          |
| `bibliographic_pages`                                                   | localizador dentro del adjunto/revisión: ordinal físico, dimensiones, rotación y etiqueta impresa opcional; no es un asset ni exige un archivo separado                  |
| `bibliographic_extractions`                                             | adjunto/revisión, texto canónico, método/proveedor por tramo, hash y contratos de extracción/layout; mapa verificable entre texto, bloques y páginas                     |
| `bibliographic_layout_blocks`                                           | extracción, página, región/coordenadas, clase de bloque, orden de lectura y correspondencia con intervalos del texto canónico                                            |
| `bibliographic_semantic_profiles`                                       | obra, revisión, plantilla, texto canónico, procedencia de campos y hash de entrada                                                                                       |
| `bibliographic_chunks` y `bibliographic_chunk_spans`                    | extracción, orden, texto, hash y contrato de segmentación; cada span conserva página/bloque y offsets exactos del texto de origen, incluso si el chunk atraviesa páginas |
| `bibliographic_embedding_contracts` y `bibliographic_index_generations` | contrato efectivo inmutable, manifiesto de entradas esperadas, progreso y puntero de generación activa                                                                   |
| `bibliographic_item_embeddings` y `bibliographic_chunk_embeddings`      | FK a perfil/chunk, contrato/generación, vector, dimensión, hash de entrada y fecha; unicidad por objeto/generación                                                       |
| Índices FTS bibliográficos                                              | metadatos de obras y texto de fragmentos, con actualización/borrado transaccional y filtros de vínculo/alcance                                                           |
| `bibliographic_sync_runs` y errores asociados                           | reconciliación en curso, conjuntos vistos, objetos fallidos y cursor confirmado; no constituye otro scheduler                                                            |
| `bibliographic_imports`                                                 | operación de alta/vinculación, solicitud idempotente, archivo pendiente, decisiones confirmadas y recibos de creación/subida                                             |

Las obras tienen una sola identidad dentro de su biblioteca. Impedir que un chunk/adjunto se asocie a otra obra o biblioteca mediante IDs inconsistentes: preferir derivar el padre por FK y, si se duplica por rendimiento, imponer restricciones compuestas.

Índices de búsqueda, perfiles y extracciones son reconstruibles; manuscritos y decisiones de vinculación no lo son. No aplicar cascadas desde limpieza semántica hacia citas.

### 6.2. Ubicación y adjuntos

Guardar derivados bajo el directorio de datos administrado por EntropIA, separados de los originales Zotero y del corpus. Las rutas se resuelven desde identidad interna y hash, no desde nombres de archivo provistos sin validar. Registrar propiedad del archivo para distinguir copia gestionada de archivo enlazado del usuario.

El resolver distingue adjunto almacenado, archivo enlazado, URL sin archivo, archivo remoto no descargado, archivo ausente y permiso denegado. La presencia de metadatos no acredita disponibilidad del PDF. Si WebDAV o archivos enlazados no son accesibles desde la conexión elegida, mostrar **Texto completo no disponible en esta conexión**; ofrecer resolución local autorizada, no declarar que la obra no existe.

Aceptar inicialmente PDF como formato de texto completo, incluido escaneado y con OCR previo; otros MIME se muestran como no compatibles con esta extracción sin impedir citar ni indexar el perfil. Aplicar límites de tamaño, tiempo y recursos antes de procesar. Para cifrados reutilizar la política vigente de importación; no registrar contraseñas ni saltarse restricciones.

Una importación desde el corpus crea un vínculo/copia bibliográfica solo tras confirmación y verificación Zotero. No mueve ni borra la fuente primaria original. Cualquier reclasificación destructiva exige otra autorización explícita.

## 7. Sincronización durable e invalidación

### 7.1. Reconciliación

1. Verificar conexión, ámbito y permisos. Separar paginación, versiones y rutas de bibliotecas personales/grupales.
2. Leer JSON nativo para identidad, metadatos, etiquetas, colecciones, adjuntos y relaciones; conservar además CSL para citar. No reconstruir el primero a partir del segundo.
3. Obtener cambios y borrados según capacidades. Para local legacy sin contadores fiables, reconciliar contenido completo y hashes; no aceptar `versión igual` como prueba de ausencia de cambios.
4. Persistir por lotes acotados, validando padre y biblioteca. Registrar cada objeto que no se pudo procesar; nunca descartarlo mediante `filter_map` y declarar luego una copia íntegra.
5. Confirmar cursor únicamente cuando todos los objetos requeridos hayan quedado persistidos o en un conjunto durable de reintentos que se volverá a pedir independientemente del cursor. La UI distingue actualizado de actualizado con errores pendientes.
6. Si cambia la versión durante una lectura que requiere snapshot consistente, repetir la reconciliación con espera acotada. No deducir borrados a partir de una página parcial o una petición fallida.
7. Publicar metadatos, invalidaciones y demanda de procesamiento de manera transaccional. E2 consume esa demanda mediante el único Lotes; antes de E2 no se generan embeddings.
8. Actualizar el espejo/listado del editor desde el catálogo confirmado de esa conexión. No mantener dos autoridades editables para el mismo registro.

Cada respuesta al selector frontend lleva la identidad de la solicitud. Cambiar de biblioteca descarta respuestas atrasadas de otra selección; una promesa de sincronización de A no representa la sincronización de B. La persistencia y el estado visible se particionan del mismo modo.

### 7.2. Matriz de invalidación

| Cambio                                                 | Acción                                                                                                                                                               |
| ------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Colección o etiqueta                                   | Actualizar catálogo/FTS/filtros; recalcular perfil solo si su texto canónico cambió                                                                                  |
| Título, creadores, resumen u otro campo incluido       | Nueva revisión de perfil; no reextraer adjuntos sin cambios                                                                                                          |
| CSL, estilo, prefijo o localizador                     | Actualizar/renderizar cita; no regenerar embeddings por el formato de la cita                                                                                        |
| Sustitución de PDF, incluso mismo nombre/ruta/tamaño   | Hash de bytes distinto invalida páginas, extracciones, chunks y vectores de ese adjunto                                                                              |
| Nuevo adjunto                                          | Procesar solo ese adjunto; mantener los demás                                                                                                                        |
| Borrado de adjunto                                     | Excluir sus derivados, conservar snapshot histórico y aplicar política de limpieza                                                                                   |
| Padre huérfano, revocado o eliminado                   | Excluir inmediatamente todos sus resultados; impedir publicación de trabajos en vuelo                                                                                |
| Cambio de modelo/contrato                              | Crear generación nueva; no comparar ni reutilizar vectores/checkpoints incompatibles                                                                                 |
| Cambio de plantilla, extracción, layout o segmentación | Invalidar los descendientes afectados; un cambio de orden de lectura invalida texto/chunks dependientes, no obliga por sí solo a repetir OCR de contenido aún válido |

La comparación de revisión/hash y elegibilidad se repite al publicar, no solo al comenzar. Cancelación, limpieza, revocación o reemplazo del archivo incrementan el estado que invalida una publicación atrasada.

## 8. Lotes: un coordinador, sujetos separados

Ampliar el contrato actual de tareas para identificar un sujeto como `(domain, subject_kind, subject_id)`, además de operación, revisión de entrada y contrato. Dominios: `corpus` y `bibliography`. En corpus el sujeto sigue siendo un asset; en bibliografía puede ser biblioteca, obra, adjunto o página, con existencia y elegibilidad validadas en su repositorio.

No resolver una obra bibliográfica mediante `assets JOIN items`. No debilitar el validador documental para que acepte IDs inexistentes. Las FK bibliográficas permanecen en sus tablas; el scheduler selecciona el validador/publicador por dominio sin convertirse en otro catálogo.

Operaciones bibliográficas: sincronización de biblioteca, perfil/embedding de obra, extracción nativa y layout del adjunto, OCR selectivo de sus tramos cuando sea necesario, embeddings de chunks y alta/subida explícita. Su ejecutor de extracción y sus decisiones de calidad pertenecen al dominio bibliográfico; no invocan el ejecutor OCR de Fuentes. Reutilizar únicamente admisión idempotente, demanda compartida, leases, fencing, checkpoints y recibos del coordinador. No reutilizar una tarea documental solo porque coincide el string de su ID. Los checkpoints bibliográficos pueden referir páginas/rangos del adjunto sin convertirlos en assets ni determinar el tamaño de las llamadas OCR.

| Comportamiento      | Contrato                                                                                                                                                                                                                                |
| ------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Reanudación         | Tras cierre inesperado, lotes solicitados por usuario quedan interrumpidos y requieren reanudación explícita, como el flujo vigente. Sincronización automática solo se reanuda si estaba habilitada y mantiene permisos/consentimiento. |
| Reintentos          | Mantener límites/backoff/`Retry-After`; no reintentar errores permanentes de identidad, permisos, archivo inválido o contrato sin intervención.                                                                                         |
| Cancelación         | Cooperativa y por demanda. Cancelar un lote no elimina trabajo requerido por otro ni garantiza abortar una llamada remota ya enviada. No prometer devolución de costes.                                                                 |
| Publicación         | Resultado y recibo/estado terminal en una transacción, comprobando lease, revisión, contrato y vínculo vigente.                                                                                                                         |
| Checkpoints         | Compatibles solo con la misma entrada, contrato y checksum. Un cambio de proveedor/modelo no reutiliza vectores anteriores.                                                                                                             |
| Prioridad editorial | Demanda interactiva de una obra precede al backlog ordinario, sin interrumpir una unidad ya en ejecución; aplicar envejecimiento para que el lote de fondo no quede postergado indefinidamente.                                         |
| Progreso            | Mostrar biblioteca, obra, adjunto y páginas; errores individuales no detienen obras independientes.                                                                                                                                     |

Migrar tareas documentales existentes a su dominio sin cambiar su significado. Verificar recuperación y cancelación del corpus después de ampliar el esquema y antes de admitir tareas bibliográficas.

## 9. Contrato efectivo de embeddings y privacidad

### 9.1. Espacio vectorial

El contrato inmutable identifica proveedor, modelo/revisión resoluble, configuración que afecta la representación, dimensión, normalización, preprocesamiento e instrucciones document/query. La plantilla del perfil y la segmentación identifican también sus entradas. Si un proveedor no expone revisión estable, registrar esa limitación y requerir nueva generación al cambiar la configuración o detectarse incompatibilidad; no afirmar reproducibilidad binaria del proveedor remoto.

La consulta utiliza el contrato compatible con la generación activa. Igual dimensión no significa mismo espacio. Validar dimensiones y valores finitos de cada vector; no etiquetar todo resultado con constantes BGE-M3 si se permitió usar otro modelo.

Construir generaciones en staging sin sobrescribir los vectores activos. El cambio de puntero es atómico y exige completar el manifiesto de entradas elegibles de esa generación. Una generación parcial no se presenta como reindexación terminada. Mantener la anterior cuando sea válida; si no se puede consultar con su contrato, ofrecer búsqueda léxica rotulada mientras se termina la nueva, sin comparar espacios distintos.

Una entrada cuyo texto o vínculo cambió se excluye del resultado semántico viejo aunque la generación anterior siga activa para otras obras. La generación no autoriza a servir evidencia obsoleta de un adjunto reemplazado.

### 9.2. Pro/Lite y consentimiento

| Operación                              | Pro                                                                                                  | Lite                                                                                                                                    |
| -------------------------------------- | ---------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| Extracción nativa de PDF bibliográfico | Recorrido bibliográfico local, sin OCR cuando el texto es utilizable                                 | Igual, sin depender de `local-ml` para leer texto nativo                                                                                |
| Layout bibliográfico                   | Análisis de estructura/orden de lectura independiente del OCR; verificar motor y capacidades propios | Verificar soporte explícito; no suponer que layout requiere OCR o que no existe sin `local-ml`                                          |
| OCR bibliográfico selectivo            | Motor compatible con el recorrido bibliográfico, local o remoto autorizado                           | Motor bibliográfico disponible según compilación; si no hay opción local, explicar y pedir autorización antes de una alternativa remota |
| Embeddings                             | Local disponible o API elegida                                                                       | API elegida; nunca convertir una preferencia local en permiso implícito                                                                 |
| Resumen opcional y síntesis            | Proveedor efectivo configurado y autorizado                                                          | Igual condición de autorización                                                                                                         |

Antes de enviar texto, imágenes o archivos de adjuntos, metadatos, notas o una selección del manuscrito, informar destino, operación y alcance. Esto incluye un proveedor externo de layout aunque no se ejecute OCR. Persistir el consentimiento con alcance suficiente para ejecutar/reanudar el lote sin diálogos por unidad; revocarlo bloquea nuevos envíos. Cambiar de proveedor, pasar de local a remoto o ampliar de páginas seleccionadas al PDF completo exige autorización que cubra ese cambio.

Verificar esta condición tanto al admitir tareas como antes del envío, incluidas consultas de búsqueda. Una clave OpenRouter guardada no es consentimiento para transmitir bibliografía. Incorporar las claves Zotero al registro de secretos protegido de `settings.rs`; no almacenar tokens en SQL plano, URLs, exports o logs.

Respetar permisos personales/grupales, acceso a archivos y derechos de uso. Los logs contienen IDs técnicos, códigos de error y métricas, no tokens ni textos completos.

## 10. Perfil semántico de obra

Construir el perfil aunque no haya adjunto. Plantilla inicial versionada `bibliography-profile-v1`, con líneas etiquetadas para los campos presentes:

```text
Título: Revoluciones y cultura política
Autores: Pérez, Ana; Gómez, Luis
Año: 2018
Tipo: book
Publicación o editorial: Editorial Universitaria
Resumen: Estudio de asociaciones y prácticas políticas en el siglo XIX.
Palabras clave y etiquetas: asociaciones; cultura política; siglo XIX
```

El ejemplo es sintético. La implementación utiliza metadatos Zotero verificados, no esos literales. Conservar orden de creadores, normalizar espacios, ordenar etiquetas de forma estable y omitir campos vacíos. No introducir IDs administrativos ni referencias formateadas como contenido semántico dominante. El hash depende del texto canónico y la versión de plantilla.

Notas seleccionadas por el usuario y resumen del texto completo son extensiones opcionales del perfil: cada una requiere procedencia, revisión y consentimiento de proveedor cuando corresponda. Un resumen generado se identifica como tal, con modelo/plantilla/hash de entrada; nunca sustituye el abstract de la publicación ni se usa como cita textual del PDF. No activar estas extensiones silenciosamente al importar una biblioteca.

## 11. Extracción bibliográfica: PDF nativo, layout y OCR selectivo

### 11.1. Recorrido independiente por adjunto

1. Resolver el adjunto Zotero y verificar padre, permisos, MIME y hash. El PDF es la unidad documental; no importarlo a Fuentes ni materializar obligatoriamente un archivo/asset por página.
2. Extraer texto nativo y su geometría disponible. Evaluar calidad/cobertura por página o región para distinguir PDF nativo, OCR previo utilizable, escaneado y documento mixto. No ejecutar OCR sobre texto nativo utilizable solo para obtener layout.
3. Analizar layout como una responsabilidad explícita: bloques, columnas, orden de lectura, encabezados/pies, párrafos y posiciones en página. Distinguir cuerpo, notas y tablas cuando el documento lo permita; no aplanar una tabla o intercalar columnas sin informar la limitación. El motor concreto se elige con evidencia en E0/E4, no por herencia del pipeline de Fuentes.
4. Solicitar OCR únicamente para el contenido sin texto utilizable. Usar el ejecutor bibliográfico y registrar el alcance real enviado al proveedor. Puede procesar páginas, regiones o rangos según sus capacidades; si el proveedor requiere un archivo mayor que el contenido que necesita OCR, explicar y autorizar ese alcance. No asumir aquí ni una página obligatoria por llamada ni PDFs completos por defecto.
5. Integrar texto nativo y reconocido en una representación canónica del adjunto, sin duplicar el texto de páginas mixtas. Conservar procedencia por tramo, revisión/hash, contratos y un mapa verificable entre texto, bloques y páginas.
6. Segmentar por estructura y orden de lectura; generar embeddings y publicar solo si adjunto, extracción, layout y vínculo conservan las revisiones esperadas.

Las bibliotecas de parsing PDF o clientes de proveedor solo pueden compartirse como primitivas independientes del dominio, sin acceder a assets, persistencia, selección de motor o políticas de Fuentes. No es un requisito reutilizarlas: no rediseñar el OCR documental para forzar esa reutilización.

### 11.2. Layout, segmentación y localizadores

El layout forma parte del resultado bibliográfico aun si no se hace OCR. Guardar sus bloques y orden de lectura, con sistema de coordenadas, tamaño y rotación de página definidos para poder resaltar el original. Si no se puede reconstruir la estructura de forma fiable, informar **Layout incompleto** y no declarar ese adjunto completamente procesado ni presentar pasajes de lectura dudosa como evidencia verificada. La obra sigue siendo recuperable por metadatos.

La segmentación respeta párrafos y secciones y utiliza un presupuesto compatible con el modelo de embeddings. Determinar y registrar tamaño/solapamiento mediante evaluación de PDFs bibliográficos, no copiar automáticamente las ventanas de 800/100 caracteres del corpus. Un párrafo puede continuar en otra página; se permite un chunk multipágina si cada tramo conserva su localización verificable.

Offsets `start_char/end_char` cuentan caracteres Unicode dentro del texto canónico de una revisión de extracción, no bytes UTF-8 ni unidades UTF-16. Convertir explícitamente al interactuar con APIs del editor que usen otra unidad. Cada span del chunk remite a los intervalos de origen y a los bloques/páginas que le corresponden; normalizaciones como unión de palabras partidas deben mantener esa correspondencia y no inventar texto.

Un resultado lleva obra, adjunto, revisión/hash, chunk y lista ordenada de spans con página física, bloques, offsets y etiqueta impresa si fue verificada. La página física sirve para abrir el original; no rellenar automáticamente el localizador CSL con ella si la numeración impresa es distinta. Sin etiqueta impresa verificada, mostrar página/rango PDF y pedir confirmación del localizador de cita.

Varios adjuntos de una obra mantienen procedencia independiente. Ediciones, traducciones y versiones no se fusionan por similitud. Un adjunto no disponible o con extracción/layout incompletos no impide recuperar y citar la obra por sus metadatos.

## 12. Recuperación bibliográfica y evaluación

### 12.1. Obras primero, pasajes después

Nivel 1: combinar coincidencias exactas normalizadas de DOI/ISBN/título, búsqueda léxica de metadatos, similitud global y filtros por biblioteca, colección, autoría, fecha, tipo, etiqueta e idioma. Empezar con fusión por rangos; no sumar directamente scores heterogéneos como si midieran lo mismo. Identificadores ayudan a buscar, no sustituyen la identidad Zotero.

Nivel 2: búsqueda léxica y vectorial de chunks restringida a las obras candidatas en ambas ramas. Agrupar por obra y adjunto, permitiendo varios pasajes distintos de una misma página/adjunto cuando aportan evidencia. No heredar la deduplicación documental de un único resultado por asset.

La ampliación a todo el índice bibliográfico es una acción explícita **Ampliar búsqueda a pasajes de toda la biblioteca** y una condición evaluada del recuperador. Mostrar el alcance utilizado. No basarla exclusivamente en cantidad de obras: una lista llena también puede omitir la relevante. Probar obras con metadatos pobres cuyo pasaje sí responde a la consulta.

Resultados incluyen método, ranking/puntuaciones informativas, contrato/generación, filtros efectivos y si hubo ampliación. No presentar similitud como certeza histórica. Aplicar elegibilidad del vínculo al consultar y de nuevo al abrir/citar si su estado cambió.

### 12.2. Conjunto de evaluación

Antes de ajustar umbrales o activar reranking/expansión, formar un conjunto de consultas reales revisadas por el usuario: tema, autor, obra conocida, pasaje poco representado en metadatos, filtros combinados, ausencia de resultados y falsos positivos graves. Usar material permitido; no commitear bibliotecas privadas.

Registrar relevancia esperada por obra y pasaje, procedencia/localizador correcto, Recall@k, nDCG@k, MRR, latencia y coste. Comparar léxico, vectorial, híbrido y candidatos jerárquicos con ampliación global. Reranking y expansión quedan como técnicas a evaluar en E7, no como dependencias obligatorias para obtener una búsqueda útil.

Criterios duros: cero referencias inventadas, cero cruces de biblioteca por colisión de clave, cero pasajes de vínculo excluido y cero localizadores atribuidos a otra revisión/página. Los umbrales de pertinencia se fijan con los juicios humanos de E0, antes de optimizar sobre el conjunto.

## 13. Ingreso desde EntropIA: vínculo antes que índice

Este flujo es obligatorio en E5, no un comentario sin etapa asignada:

```text
PDF pendiente → búsqueda/confirmación de coincidencia → padre Zotero confirmado
              → adjunto creado o vinculado y comprobado → procesamiento bibliográfico
```

- Guardar el archivo en una bandeja local transitoria fuera de los índices. Estado y progreso deben sobrevivir al cierre.
- Buscar coincidencias por DOI/ISBN normalizados, identificadores del proveedor y metadatos. Mostrar ambigüedades; título aproximado no habilita vinculación automática.
- Si existe la obra, el usuario elige adjuntar, vincular un adjunto existente o cancelar. No duplicar el PDF por un reintento.
- Si no existe, crear primero un ítem Zotero regular usando metadatos confirmados por el usuario o un registro bibliográfico verificable. Un PDF sin metadatos suficientes exige edición/confirmación o creación asistida en Zotero; no fabricar autor/título/año desde un LLM ni aceptar un adjunto suelto como padre citable.
- Elegir explícitamente destino personal/grupal. Comprobar permiso de escritura y de archivos antes de crear/subir.
- Para API local con escritura soportada, solicitar su autorización. Para local sin esa capacidad, ofrecer API web autorizada o completar el alta en Zotero y luego verificarla. No ocultar el límite ni exigir nube para seguir usando lectura local.
- Persistir identidad y recibo del padre creado antes de crear el adjunto; persistir identidad y progreso del adjunto antes de subir bytes. Usar claves/precondiciones/tokens soportados, pero no confiar solo en tokens en memoria para sobrevivir a reinicios.
- Seguir el protocolo de autorización, subida y registro final del archivo; tras respuesta perdida, consultar/reconciliar antes de repetir. Verificar padre, adjunto y hash final antes de admitir extracción.
- Si el padre quedó creado pero falla la subida, reanudar desde ese padre. No eliminarlo automáticamente como compensación. Explicar el estado y permitir cancelar sin borrar Zotero.
- Manejar cuota, permisos vencidos, conflicto de versión, cancelación y archivo cambiado durante subida sin duplicar ítems.

**Idempotencia observable:** una misma solicitud reanudada produce como máximo un padre/adjunto elegido para esa operación; no hace falta que exista una única obra mundial para el DOI. Las decisiones ambiguas siguen siendo humanas.

## 14. Interfaz, editor y CSL existentes

### 14.1. Sección Bibliografía

Incluir selector de conexión/biblioteca/grupo, árbol de colecciones, filtros, búsqueda híbrida y ficha de obra. Mostrar disponibilidad de texto, fragmentos indexados, fecha de sincronización, proveedor/contrato y progreso de Lotes sin obligar al usuario a interpretar enums técnicos.

Estados visibles: **Sincronizado**, **Copia local sin verificar ahora**, **Falta texto completo**, **Pendiente de indexación**, **Indexado**, **Actualización disponible**, **Requiere vinculación con Zotero**, **Ítem no disponible en Zotero**, **Error de procesamiento**.

Acciones: **Abrir en Zotero**, **Ver adjunto**, **Buscar dentro**, **Insertar cita**, **Reindexar**, **Eliminar derivados locales** y revisión de pendientes. Las rutas de apertura usan identidad y capabilities verificadas, no concatenación de un `CSL.id` arbitrario ni URLs ejecutables sin validar.

### 14.2. Selección del manuscrito

**Buscar bibliografía relacionada** utiliza solo el texto seleccionado como consulta, con consentimiento si se envía a un proveedor. Conserva el ancla/revisión de la selección para no insertar sobre un pasaje cambiado. El usuario examina obras/pasajes, abre el original y acepta la cita; no se inserta texto generado sin confirmación.

Integrar resultados en la solapa Zotero compacta existente. No crear un segundo editor bibliográfico. Guardar en procedencia el vínculo selección–obra–adjunto–fragmento–spans de páginas/bloques con revisión/hash; un hash solo del texto no reemplaza su identidad.

### 14.3. Contrato de citas

Reutilizar los nodos, clusters, snapshots, renderizado documental, bibliografía y exportadores actuales. La capa semántica recomienda evidencia; Hayagriva produce el texto CSL con los datos Zotero. Renderizar a nivel de documento para desambiguar obras del mismo autor/año.

La aceptación cubre cita simple y múltiple, narrativa y parentética, localizadores, prefijos/sufijos, supresión/solo autor, estilos autor-fecha y notas, idioma configurado y bibliografía sin duplicados de identidad. Una opción visible debe tener efecto real; si un modo no está soportado, explicarlo y no presentarlo como cumplido.

Una cita en nota debe residir en una nota estructural del editor y conservar esa estructura en DOCX; seleccionar un estilo de notas no convierte por sí solo texto inline en nota. HTML/Markdown deben preservar una representación de nota navegable/legible según sus exportadores. Verificar el flujo existente de notas antes de ampliarlo, no inventar otro paralelo.

No se añade sincronización de campos vivos del plugin Zotero de Word: el alcance es el editor EntropIA y sus exportaciones. Esta exclusión no elimina el requisito de notas estructurales ni de citas verificables.

Borrar el índice o cerrar Zotero no impide renderizar/exportar snapshots ya aceptados. Un fallo CSL debe mostrarse; no presentar una cadena cacheada de otro estilo como renderización correcta del estilo nuevo.

## 15. Consulta combinada y asistencia

Definir resultados discriminados, no IDs documentales ficticios:

```ts
type BibliographicSpan = {
  pageId: string
  blockIds: string[]
  startChar: number
  endChar: number
}

type EvidenceReference =
  | { domain: 'corpus'; assetId: string; sourceRevision: string }
  | {
      domain: 'bibliography'
      work: ZoteroIdentity
      attachmentKey: string
      chunkId: string
      extractionRevision: string
      spans: BibliographicSpan[]
    }
```

Este tipo expresa identidad mínima; texto, localizador, puntuaciones y método viajan en el resultado correspondiente. `ZoteroIdentity` se define en §4. `spans` es una lista no vacía validada contra la revisión de extracción: debe cubrir los tramos de origen del chunk, sin atribuir todo un fragmento multipágina a una única página. Los registros de conversación/procedencia deben evolucionar sin volver ilegibles los snapshots documentales anteriores.

Ejecutar recuperaciones independientes con presupuestos separados de resultados y tokens. Mostrar **Fuentes documentales** y **Bibliografía académica** como grupos distintos; no producir una lista única ordenada solo por similitud.

El contexto del modelo distingue evidencia documental, interpretación bibliográfica y síntesis generada. Tratar títulos, notas y textos recuperados como datos no confiables, nunca como instrucciones. Los delimitadores no constituyen una garantía suficiente: verificar que toda referencia devuelta pertenece al conjunto suministrado y al dominio indicado. Una cita propuesta por el LLM no puede inventar otra identidad.

Extender capacidades del agente por dominio/proveedor, sin suponer que un booleano de retrieval habilita bibliografía. Si el motor externo necesita cambios, identificarlos como dependencia con alcance y revisión propios; no actualizar `entropia-agent` ni su pin incidentalmente. La recuperación bibliográfica manual no depende de habilitar todas las acciones del agente.

## 16. Migraciones y límites del cambio compartido

- Añadir migraciones numeradas nuevas en `packages/store/src/migrations/` y registrarlas en `runner.ts`; actualizar `schema.ts` y fixtures/exportación de esquema que realmente consuman ese contrato. No reescribir migraciones históricas para bases ya instaladas.
- Migrar el contrato JSON canónico de citas junto con sus proyecciones Rust/TS; probar reapertura y restauración de versiones antiguas, no solo apertura de una base nueva.
- Las columnas existentes `source_origin/source_instance_id` no acreditan que el recorrido operativo las conserve. La migración debe cubrir también productores, editor de citas y guardado.
- Versionar tareas/snapshots compartidos. Tras migración, tareas documentales pendientes, pausadas e interrumpidas conservan su demanda y no adquieren proveedores distintos.
- Crear respaldo y validar integridad en una copia antes de migrar datos reales. Las migraciones transaccionales deben dejar la versión anterior intacta ante fallo.
- Reversibilidad significa restaurar un respaldo compatible cuando no exista migración inversa segura. No prometer que un binario viejo entiende automáticamente el nuevo esquema/JSON.
- No modificar el recorrido OCR de Fuentes para implementar extracción bibliográfica. Limitar cambios compartidos al coordinador y a otros contratos efectivamente necesarios, con regresiones del comportamiento documental; no alterar búsqueda/exportación documental fuera de esos contratos.
- El código obsoleto por el nuevo contrato debe migrarse con sus consumidores; no dejar dos catálogos Zotero o dos autoridades editables de identidad. Mantener lectura de datos históricos no equivale a mantener una segunda implementación activa.

## 17. Mapa de archivos para la implementación

**Existentes a ampliar, no a reemplazar por duplicados:**

- Zotero: `apps/desktop/src-tauri/src/writing/zotero/{connector,mirror,mod}.rs`, `writing/commands.rs`, `apps/desktop/src/lib/writing-zotero.ts`.
- Cola compartida: `apps/desktop/src-tauri/src/processing/{repository,scheduler,recovery,commands}.rs`, solo para coordinación por dominio. Los ejecutores y reglas documentales de `processing/{ocr,eligibility,embedding}.rs` no se convierten en el recorrido bibliográfico; cualquier adaptación de su frontera con el coordinador debe preservar su comportamiento.
- Cómputo compartido cuando corresponda: `apps/desktop/src-tauri/src/nlp/embeddings.rs`, `settings.rs` y registro de comandos en `lib.rs`. La extracción bibliográfica tiene módulo propio; no se prescribe modificar `ocr/pdf.rs` ni el ejecutor de Fuentes.
- Citas: `packages/ui/src/components/WritingEditor/{document-contract,citations,citation-cluster,extensions}.ts`; `apps/desktop/src-tauri/src/writing/{repository,versions,commands}.rs` y `writing/csl/`.
- UI/citas: `apps/desktop/src/views/WritingZoteroTab.svelte`, `WritingCitationEditor.svelte`, `WritingView.svelte`; `apps/desktop/src/lib/{citation-clusters,writing-export,writing-zotero}.ts` y exportadores solo donde cambie su salida observable.
- Persistencia: `packages/store/src/{runner,schema}.ts`, nuevas migraciones y pruebas de migración existentes.

**Nuevos archivos propuestos, creados únicamente con su comportamiento funcional:**

- `apps/desktop/src-tauri/src/bibliography/mod.rs`: modelos y superficie pública del dominio.
- `apps/desktop/src-tauri/src/bibliography/repository.rs`: catálogo, revisiones y publicación transaccional.
- `apps/desktop/src-tauri/src/bibliography/sync.rs`: reconciliación y checkpoints Zotero, reutilizando transporte.
- `apps/desktop/src-tauri/src/bibliography/processing.rs`: adaptación de sujetos bibliográficos al único Lotes.
- `apps/desktop/src-tauri/src/bibliography/extraction.rs`: lectura del adjunto PDF, extracción nativa/layout, OCR selectivo y mapa de procedencia; sin depender del ejecutor OCR ni de assets de Fuentes.
- `apps/desktop/src-tauri/src/bibliography/retrieval.rs`: ranking de obras, búsqueda de pasajes y filtros.
- `apps/desktop/src-tauri/src/bibliography/ingestion.rs`: pendientes, decisiones y recuperación del alta/subida.
- `apps/desktop/src-tauri/src/bibliography/commands.rs`: frontera Tauri, autorización y validación de solicitudes.
- `apps/desktop/src/lib/bibliography.ts`: contratos frontend y llamadas a la frontera bibliográfica.
- `apps/desktop/src/views/BibliographyView.svelte`: sección bibliográfica con patrones UI existentes.

No crear todos esos archivos de antemano. Las firmas concretas y los cuerpos se resuelven al leer los consumidores de la etapa correspondiente; este plan fija invariantes y entregables, no presume que una API inexistente ya compile. Cualquier división adicional debe reducir responsabilidades reales, no agregar capas de forwarding.

## 18. Etapas y unidades de commit

Cada unidad sigue el mismo ciclo: leer consumidores y pruebas relevantes, fijar el comportamiento observable, reproducir la brecha con el caso de §19 cuando corresponda, implementar el recorrido completo, ejecutar la comprobación focalizada y el escenario real, y registrar evidencia antes del commit. No iniciar todas las pruebas de todas las etapas ni dejar scaffolds pendientes entre entregables.

### E0. Compatibilidad y decisiones comprobadas

**Archivos:** este plan; lectura de las rutas de §2. Sin cambios productivos.

**Consume:** instalación Zotero, variantes EntropIA y fuentes oficiales. **Produce:** matriz de capacidades ejercitadas, elección de conexión, ámbito confirmado y conjunto de evaluación con criterios de pertinencia.

- [x] Registrar la autorización de lectura/escritura limitada a bibliotecas personal y grupal aisladas de prueba.
- [x] Resolver TDD estricto para E1 y la entrega directa en este worktree con commits por unidad.
- [x] Comprobar el transporte local, la lectura nativa, la identidad grupal, las etiquetas y el versionado sobre material de prueba autorizado; CSL, colecciones, adjuntos, perfil personal y Web API siguen con bloqueo explícito.
- [x] Verificar la vía de escritura local elegida sobre el grupo aislado `prueba`, sin modificar bibliografía real: `saveItems` `201` + `updateSession` `200`, con el fixture técnico `7EMV3G8H`.
- [x] Registrar capacidades no ejercitadas como no verificadas, no como soportadas, y definir la semilla `zsb-eval-v1` con su momento de juicio.
- [x] Revisar las decisiones actuales; no se introdujo un proveedor o coste nuevo que requiera una elección adicional.

**Salida verificable:** cada función obligatoria tiene una vía compatible o un bloqueo concreto que se resuelve antes de su implementación; no se elimina del alcance. **Commit:** `docs: define verified Zotero bibliography capabilities` (`caa7a8d233ce194e0ddc285da7abf506b0f719b8`). **Reversión:** solo documentación; no datos ni aplicación.

### E1. Identidad y catálogo Zotero confiable

**Archivos:** Zotero/store/contrato canónico de §17, `bibliography/{mod,repository,sync,commands}.rs`, `bibliography.ts` y sección de catálogo en `BibliographyView.svelte`.

**Consume:** capacidades E0. **Produce:** `ZoteroReference`, catálogo persistente, filtros de metadatos y sincronización sin vectorización.

E1a se divide en slices verticales para preservar el ciclo RED → GREEN → REFACTOR y mantener la carga de revisión acotada; E1a-1 no introduce migraciones ni cambia la ruta `user/0`.

**Decisiones E1a-2:** `source_instance_id` desconocido permanece nullable y no habilita fusiones entre instancias/ámbitos no corroborados; el contrato canónico de citas avanza a schema v2, con lectura compatible de v1 legacy.

Slices previstos: E1a-2a (editor y igualdad de clusters), E1a-2b (proyección/persistencia y schema v2), E1a-2c (historial, legacy y exportación).

- [x] Unidad E1a: preservar clave nativa y biblioteca en listado, selección, guardado y edición de cita existente; probar un CSL id distinto y colisiones entre bibliotecas.
  - [x] Slice E1a-1: transportar `key`, `itemVersion`, `libraryType`, `libraryId` y snapshot CSL desde connector/mirror hasta listado, búsqueda e inserción de cita, manteniendo `user/0`.
  - [x] Slice E1a-2: conservar esa identidad en edición, proyección, clusters, historial y citas entre bibliotecas.
  - [x] E1a-2a: conservar identidad calificada en inserción/edición y evitar fusiones por colisiones de biblioteca, origen o instancia desconocida.
  - [x] E1a-2b: conservar identidad en proyección/persistencia y schema v2, con lectura v1.
  - [x] E1a-2c: conservar identidad en historial, legacy y exportación.

**Evidencia E1a-2c:** commit `85cc0ee` (`feat(writing): namespace Zotero export identities`), con IDs CSL derivados únicamente para render/export, deduplicación por identidad calificada, separación por ocurrencia cuando la instancia es NULL, round-trip legacy sin reescritura y snapshot v2 conservado en historial. Verificación focalizada: desktop 38 tests, UI 64 tests, Rust repository 31 + versions 11, typecheck y diff check limpios.

**Evidencia E1a-2b:** commit `3c0b9be` (`feat(writing): persist qualified Zotero citation identity`), con schema canónico v2, lectura lazy v1, proyección item-level de origen/instancia y persistencia SQL con NULL de instancia. Verificación focalizada: UI 91 tests, desktop 30 tests, Rust 31 tests, typechecks Pro/Lite y diff check limpios.

**Evidencia E1a-1:** commit `67140c0b4cba08e005c0f67152f179bf6bb9d33a` (`feat(writing): preserve qualified Zotero item identity`), con RED → GREEN → REFACTOR y verificaciones focalizadas documentadas en el task ODD.

**Evidencia E1a-2a:** commit `3333683e6488515e9dcfcb024b416e5df9094639` (`feat(writing): preserve qualified citation identity in editor`), con RED → GREEN y verificaciones desktop/UI focalizadas (67 tests), typecheck Pro/Lite y autofixer Svelte limpios. La inserción marca `sourceOrigin=local` y `sourceInstanceId=null`; la igualdad exige instancia corroborada para datos calificados y conserva el fallback itemKey-only legacy.

**Evidencia E1b-1a:** commit `c5f90b3` (`feat(bibliography): persist Zotero catalog foundation`), con migración `0038_bibliography_catalog`, tablas de conexiones/bibliotecas/obras, FK e identidad `(library_id, item_key)`, snapshots JSON nativo+CSL, validación de `creators_json` nullable y upsert transaccional idempotente. La RED → GREEN → REFACTOR cubrió 43 tests de store, 5 tests Rust de integración, typecheck/lint, `cargo check`, rustfmt y diff check; el runtime Rust pasó con `CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0`.

**Evidencia E1b-1b:** commit `352cdc3` (`feat(bibliography): persist Zotero relations and tombstones`), con migración `0039_bibliography_relations`, colecciones/etiquetas/adjuntos con snapshots nativos y versiones nullable, relaciones compuestas seguras por biblioteca, claves de padre opacas, metadatos de adjuntos sin resolución de archivos y side tables de tombstone explícito con recuperación por upsert vivo. La RED → GREEN → REFACTOR cubrió 48 tests de store, 12 tests Rust de integración, typecheck/lint, `cargo check`, rustfmt y diff check; el runtime Rust pasó con `CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0`.

**Evidencia E1b-2:** commit `f011489` (`feat(bibliography): persist reconciliation state`), con migración `0040_bibliography_reconciliation`, estado durable por biblioteca, run/phase/connection fences, seen-set normalizado, checkpoints atómicos, errores/reintentos recuperables, interrupción/resume, bloqueo explícito y finalización idempotente sin regresión de checkpoint. La RED → GREEN → REFACTOR cubrió 52 tests de store, 25 tests Rust (catálogo + reconciliación), typecheck/lint, `cargo check`, rustfmt y diff check; el runtime Rust pasó con `CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0`.

**Evidencia E1b-3:** commit `40abe01` (`feat(bibliography): expose confirmed catalog reads`), con proyección de lectura confirmada del catálogo local-personal en el repositorio (`confirmed_local_personal_catalog`), namespace único `local + user + NULL instance`, reconciliación `completed`/`finalize` con seen-set, versiones remotas, `verified_at` y exclusión de tombstones; `writing_zotero_cached` prefiere el catálogo confirmado (incluido el vacío confirmado) y cae resilientemente al mirror de archivos ante BD/catálogo ilegible, sin selector, apertura, red ni cambios de frontend. La RED → GREEN → REFACTOR cubrió 22 tests de catálogo (workaround `CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0` por `LNK1201` inicial), 13 tests de reconciliación, 3 tests de `writing::commands`, `cargo check` focalizado, rustfmt focalizado y diff check.
- [x] Unidad E1b: introducir tablas/relaciones y migraciones de catálogo; sincronizar altas, modificaciones, colecciones, etiquetas y adjuntos con cursor durable y errores recuperables.
  - [x] E1b-1a: crear el fundamento persistente de conexión/biblioteca/obra, con identidad nativa, JSON nativo+CSL y upsert idempotente entre bibliotecas.
  - [x] E1b-1b: agregar colecciones, etiquetas y adjuntos con relaciones FK y tombstones/metadatos sin resolver todavía archivos.
  - [x] E1b-2: agregar reconciliación durable por biblioteca, cursor confirmado, seen-set, errores y reintentos recuperables.
  - [x] E1b-3: exponer lectura del catálogo confirmado al seam existente de Zotero sin introducir selector ni apertura de E1c.
- [x] Unidad E1c: selector y ficha de obra, apertura Zotero, estados offline y exclusión por pérdida comprobada de vínculo; respuestas tardías de otra biblioteca no reemplazan la selección.
  - [x] E1c-1: transporte consciente de tipo de biblioteca (users/groups) y selección explícita en el store con aislamiento de respuestas tardías por biblioteca; sin UI de selector todavía.
  - [x] E1c-2: selector UI con bibliotecas conocidas localmente (mirrors + catálogo persistido + user/0) y alta manual validada por probe de versión barato; commit `3526d9b`.
  - [x] E1c-3: ficha de obra y estados offline/pérdida de vínculo (tombstone) sobre lecturas del catálogo confirmado; commit `911e581`.
  - [x] E1c-4: apertura en Zotero (`writing_zotero_open_item`) con identidad calificada estricta, deshabilitada hasta una sesión live autorizada que verifique `zotero://select` contra `prueba`; commits `796441e` + `3817ceb`.

**Decisiones E1c:** la lista del selector surge solo de bibliotecas conocidas localmente (mirrors + catálogo persistido + `user/0`); el alta manual valida existencia con un probe de versión y, con Zotero cerrado, queda como "Copia local sin verificar ahora". No se consume ningún endpoint de enumeración no verificado (regla E0). La apertura se implementa con validación estricta de identidad calificada y opener del SO validado, pero permanece deshabilitada hasta la verificación live del esquema `zotero://select` sobre el fixture `7EMV3G8H` de `prueba`; nunca se concatena un `CSL.id` ni se ejecutan URLs sin validar. Quedan fuera de E1c: scheduling de E2, resolución/apertura de archivos adjuntos, embeddings y Web API.

**Evidencia E1c-1:** commit `c5b6939` (`feat(writing): add typed Zotero library transport`), con seam `zotero::Library` (`user`/`group`, ids no vacíos, `storage_key` = id para user y `group-{id}` para grupos, `personal()` = user/0), los seis builders de URL y las funciones de lectura ruteando a `/api/users/{id}` o `/api/groups/{id}` sin mapeo silencioso de tipos desconocidos, probe fijado a user/0, mirror por clave de biblioteca con `library-0.json` byte-idéntico para user/0 y archivos `group-{id}` distintos y traversal-safe, comandos `writing_zotero_cached/sync/search` con parámetros planos camelCase `{libraryType, libraryId}` (+`query`), precedencia del catálogo confirmado solo para user con fallback a mirror para grupos, y store TS con selección explícita user/0 por defecto, `select()` que limpia estado y resetea tareas in-flight, y descarte por época/selección (+query en búsqueda) de respuestas tardías de otra biblioteca. La RED → GREEN → REFACTOR cubrió 231 tests Rust `writing::`, 37 tests TS (writing-zotero + WritingZoteroTab), typecheck Lite `VITE_LOCAL_ML=0`, `cargo check`, rustfmt focalizado (único hunk restante preexistente en mirror.rs) y diff check.

**Evidencia E1c-3:** commit `911e581` (`feat(writing): add Zotero work details`), con módulo `bibliography::detail` (`item_detail` reutiliza los predicados de confirmación del catálogo confirmado: namespace único local+user+NULL-instancia, run `completed`/`finalize`, seen+versión+`verified_at`; tombstone verificado antes que confirmed), comando `writing_zotero_item_detail` con estados taggeados `confirmed`/`lost_link`/`not_in_catalog`/`catalog_unavailable`, `invalid_library`/`invalid_item_key` en campos vacíos y fallback a `catalog_unavailable` cuando el archivo no abre; creadores omitidos (nunca error) si `creators_json` es inválido; adjuntos solo metadatos (sin `native_path`/md5/mtime ni resolución de archivos); componente `WritingZoteroDetails.svelte` con chip "Sincronizado" + fecha `verifiedAt`, "Ítem no disponible en Zotero" + razón para lost_link, "Copia local sin verificar ahora" desde el CSL retenido para not_in_catalog/catalog_unavailable, reintento honesto ante fallo de invoke y ningún texto que afirme que Zotero está cerrado; afluencia por ícono de ojo por fila en `WritingZoteroTab` sin tocar selección/cita/store; 23 claves i18n es/en. La RED → GREEN → REFACTOR cubrió 243 tests Rust `writing::` + 16 `bibliography::`, 62 tests TS (9 ficha + 11 tab + 36 store + 6 accesibilidad), typecheck Lite, `cargo check`, rustfmt focalizado (solo hunk preexistente mirror.rs) y diff check.

**Evidencia E1c-2:** commit `3526d9b` (`feat(writing): add Zotero library selector`), con comandos `writing_zotero_known_libraries` (user/0 siempre primero, fusión de nombres de archivo mirror + filas de catálogo con catálogo-gana, orden determinista, sin errores ante caché/tablas ausentes) y `writing_zotero_check_library` (estados `available`/`unverifiable`/`not_found` vía `library_version`, mapeo 404→`NotFound`, `invalid_library` en ids vacíos), módulo TS `writing-zotero-libraries` como autoridad única del listado (merge con localStorage `entropia:zotero:added-libraries`, dedupe por tipo+id, persistencia solo en `available`/`unverifiable`, nunca en `not_found`), selector + formulario de alta en `WritingZoteroTab.svelte` con 14 claves i18n es/en y wording "(sin verificar)" que nunca afirma que Zotero está cerrado. La RED → GREEN → REFACTOR cubrió 241 tests Rust `writing::`, 61 tests TS (16 lib + 9 tab + 36 store), typecheck Lite, `cargo check`, rustfmt focalizado (solo hunk preexistente mirror.rs) y diff check.

**Evidencia E1c-4:** commits `796441e` (`feat(writing): add Zotero open-item command`, gate apagado) y `3817ceb` (`feat(writing): enable Zotero open after live verification`), con `select_uri` de identidad estricta (clave `^[A-Z0-9]{8}$`, id de biblioteca numérico; personal → `zotero://select/library/items/{key}`, grupo → `zotero://select/groups/{id}/items/{key}`; nunca `CSL.id`), opener del SO sin shell (explorer/open/xdg-open) que rechaza URIs sin prefijo exacto `zotero://select/`, orden de errores `invalid_library`/`invalid_item_key` → `invalid_item_key` pre-spawn → `open_item_failed`, botón "Abrir en Zotero" en toda ficha con clave nativa y mensajes honestos por código, y probe `#[ignore]` reproducible. **Sesión live autorizada (2026, grupo `prueba` visible):** `live_open_select_prueba_fixture` abrió `zotero://select/groups/6680944/items/7EMV3G8H` y el usuario confirmó visualmente la selección del fixture en Zotero; recién entonces el gate pasó a `true` con TDD (8 tests `e1c4`, 17 `writing::commands`, `cargo check`, rustfmt focalizado, diff check).

**Aceptación:** cambiar una etiqueta no pierde el ítem, no crea vector; reiniciar conserva catálogo confirmado; citas previas se exportan. **Commits:** uno por E1a/E1b/E1c con pruebas relacionadas. **Reversión:** desactivar conexión nueva sin borrar manuscritos; recuperar respaldo si se vuelve a binario anterior al esquema.

### E2. Lotes admite sujetos bibliográficos sin alterar el corpus

**Archivos:** `processing/`, migración nueva, `bibliography/processing.rs`, publicación en su repositorio y UI de progreso existente.

**Consume:** catálogo E1. **Produce:** tareas por dominio/sujeto y revisión, sincronización programada mediante el mismo coordinador, cancelación y prioridad editorial.

- [x] Unidad E2a: ampliar identidad/migrar tareas, conservando recuperación y resultados documentales.
  - [x] E2a-1: columnas aditivas `domain`/`subject_kind`/`subject_id` (migración `0041`, defaults `corpus`/`asset`, backfill `subject_id = asset_id_snapshot`) con dual-write y cero cambio de comportamiento documental; commit `233be8e`.
  - [x] E2a-2: cutover de single-flight al índice único parcial `(domain, subject_kind, subject_id, kind)` con prueba de equivalencia documental (mismo task_id en attach, sin doble admisión concurrente, adversarial equal-string no reutiliza tareas corpus) y baja del índice viejo en la misma transacción; commit `fdb4fbf`.
  - [x] E2a-3: gates de claim/validate/commit con dispatch explícito por dominio (corpus verbatim, `bibliography` rechazada hasta E2b) + lock-in de recovery/cancel/retry/finalize sobre filas documentales y DTOs aditivos; commit `8cf04c5`.
- [ ] Unidad E2b: conectar sincronización bibliográfica funcional al scheduler; reintento de objetos fallidos y demanda compartida tras reinicio.
  - [x] E2b-1: migración `0043` (kind `bibliography_sync` en los CHECK de `processing_tasks`/`processing_batch_tasks` vía rebuild de tabla, origen de sistema `bibliography` dedicado) y admisión de sujetos `bibliography`/`library` por el core con verificación de existencia de la fila de biblioteca; corpus byte-idéntico; commit `2aacd1f`.
  - [x] E2b-2: dispatch del scheduler — executor `bibliography_sync` en `bibliography/processing.rs` con cliente Zotero inyectable (fake en tests), puente async con `block_on`, `StopFlag` entre páginas (nunca preempt de request en vuelo), validador por dominio con estados honestos (biblioteca ausente ≠ conexión offline). commit `a58f08b`.
  - [x] E2b-3: persistencia por página — funciones transacción-neutrales para upserts/checkpoint/finalize (los wrappers públicos conservan su comportamiento actual), transacción por página (upserts + checkpoint juntos), tombstones solo por enumeración completa confiable al cierre, receipt al commit de la tarea, y recuperación que converge también los runs de reconciliación; commit `56c73ba`.
  - [x] E2b-4: reintento/recuperación/demanda compartida + disparador manual (comando/botón 'Sincronizar biblioteca' por biblioteca) + reintento de página con objetos fallidos sin avances de cursor falsos ni descarte silencioso.
    - [x] Backend: comando manual por biblioteca, replay idempotente, demanda compartida y retry de página con cursor/seen-set preservados; commit `543787e`.
    - [x] UI: wrapper IPC tipado, acción de store segura frente a cambios de selección, request ID generado y feedback honesto de admisión; commit `2eb88a4`.
  - [ ] E2b-5: matriz de aceptación paralela (OCR documental + sync bibliográfico concurrentes por el mismo scheduler; cancelar uno no elimina la demanda del otro; strings iguales entre dominios permanecen aislados; restart converge ambos).
    - [x] WU-1: claim/publish con registro mixto y aislamiento de identidad ante strings iguales; commit `50e343f`.
    - [x] WU-2: cancelación por dominio sin retirar la demanda del otro; commit `5b4ebe1`.
    - [ ] WU-3: recovery/restart converge ambos dominios sin publicación duplicada.
- [ ] Unidad E2c: prioridad interactiva/progreso y barreras de publicación frente a cancelación, revocación y limpieza.

**Decisiones E2a:** `subject_id` documental es exactamente `asset_id_snapshot` (dual-write, nunca se elimina en E2a); los sujetos bibliográficos futuros usan ids internos de fila (`zotero_libraries.id`, `bibliographic_items.id`, `zotero_attachments.id`+rango), nunca claves nativas Zotero solas, rutas ni `CSL.id`. La revisión reusa los relojes existentes (corpus: `input_revision`+fingerprint+contrato sin tocar; biblio luego: `item_version`/`native_version`/`last_modified_version` + revisión local + tombstones como señal de revocación). El índice único viejo `(kind, asset_id_snapshot)` se da de baja en la misma transacción del cutover porque el compuesto lo vuelve redundante (autoridad única). El backfill jamás reescribe `input_fingerprint`/`contract_hash` (los checkpoints reanudan por esos valores). Se conserva FIFO por id — ninguna prioridad se cuela en E2a (eso es E2c). Ningún kind ni subject bibliográfico se admite hasta E2b.

**Evidencia E2a-1:** commit `233be8e` (`feat(processing): add task subject identity columns`), con migración `0041` (6 ALTERs + backfills `WHERE subject_id = ''` antes de crear el índice único parcial compuesto `idx_processing_tasks_subject_active_unique (domain, subject_kind, subject_id, kind)` que convive con el viejo), registro byte-idéntico en `runner.ts` por el path transaccional single-batch, columnas en `schema.ts`, fixture `schema_full.sql` regenerado, y dual-write `corpus`/`asset`/snapshot solo en los INSERT de `admit_or_attach`/`link_batch_task` — todos los lookups (`live_task`, `claim_next`, `validate_claim_input`, gates de commit) siguen consultando `(kind, asset_id_snapshot)` sin tocar prioridades ni orden. La RED → GREEN → REFACTOR cubrió 58 tests de store focalizados (suite completa 295), 57 tests Rust `processing::` + 1 `processing_recovery` con pruebas de upgrade byte-idéntico (estados/fingerprints/contratos/checkpoints intactos en pendientes/pausadas/interrumpidas), attach post-upgrade reusando el mismo task_id, compatibilidad de lectores legacy y DTOs sin cambios, typecheck/lint, `cargo check`, rustfmt focalizado y diff check.

**Evidencia E2a-2:** commit `fdb4fbf` (`feat(processing): cut over task single-flight to subject identity`), con migración `0042` (DROP INDEX IF EXISTS del índice viejo `(kind, asset_id_snapshot)`; el compuesto queda como única autoridad de single-flight) registrada byte-idéntica en `runner.ts` y fixture regenerado; `live_task` pasa a consultar `(domain, subject_kind, subject_id, kind)` con los tres sitios de admisión pasando `corpus`/`asset`/id explícitos, `INSERT OR IGNORE` + race-fallback apoyado en el compuesto, y colisión terminal sin cambios; los negativos adversariales prueban que una fila `bibliography/item` con `subject_id` y `asset_id_snapshot` colisionando en el mismo string jamás se adjunta ni bloquea la admisión corpus, y la doble admisión concurrente (dos conexiones con `busy_timeout`) produce exactamente una tarea física (created true+false, dos links); lock-ins documentales: attach reusa task_id, dos lotes comparten una tarea física, fila terminal + demanda nueva se rehúsa (solo retry explícito), pausar un lote conserva la demanda del otro, y el upgrade es byte-idéntico a través de 0041+0042. La RED → GREEN → REFACTOR cubrió 62 tests de store focalizados, 64 tests Rust `processing::` + 1 `processing_recovery`, typecheck/lint, `cargo check`, rustfmt focalizado y diff check; ningún otro lookup (claim/validate/commit/cancel/retry/finalize) cambió (verificado por grep de ausencia).

**Evidencia E2a-3:** commit `8cf04c5` (`feat(processing): dispatch task gates by domain`), con diseño wrapper/core (`TaskSubject` + núcleo `admit_subject_or_attach` que solo admite `corpus`/`asset` y rechaza lo demás con `unsupported_subject` sin fallback silencioso a corpus; `admit_or_attach`/`admit_repair_or_attach` conservan firma como wrappers corpus — cero cambios en callers externos de nlp/ocr/transcription/lib), `ClaimedTask` con subject leído al claim, `claim_next` filtrando `domain='corpus'` (filas bibliography jamás claimadas ni mutadas) con rechazo de kind desconocido intacto, `validate_claim_input`/`commit_success_with` con match defensivo por dominio (brazo corpus byte-idéntico) y DTOs aditivos `domain`/`subjectKind`/`subjectId` junto a `assetId` (Rust + tipos TS opcionales sin cambio de comportamiento). Lock-ins documentales: restart con lotes mixtos converge exactamente como `recover_session` hoy, cancel de un lote compartido asienta según demanda sobreviviente (`demand_lost`, nunca publica), transiciones claim-time idénticas, retry jamás resucita cancelled/succeeded. Verificación focalizada: 74 tests Rust `processing::` + 1 `processing_recovery`, `cargo check`, rustfmt focalizado, 7 tests TS de batch-processing, typecheck Lite y diff check — todos verdes (ejecutados por el parent tras un glitch de envelopes de subagentes que cross-wireó respuestas; el diff fue auditado por grep e inspección directa). La unidad E2a queda completa.

**Decisiones E2b (elegidas por el usuario):** persistencia **por página** — cada página en su transacción (upserts + `checkpoint_page` juntos), el cursor avanza solo tras persistir, tombstones/finalize solo con enumeración completa confiable al cierre, receipt al commit de la tarea, nunca tombstone por página parcial ni petición fallida. Transporte **de ítems primero** — `/items/top` + versiones (endpoints existentes del connector); colecciones/etiquetas/adjuntos y shapes `/deleted` quedan para slices posteriores. Disparador **manual** — comando/botón 'Sincronizar biblioteca' por biblioteca admite la demanda; el auto-agendado se difiere hasta que exista política de prioridad (E2c). Complementos técnicos del mapeo: identidad `domain='bibliography'`/`subject_kind='library'`/`subject_id=zotero_libraries.id` (id interno de fila, nunca id externo de Zotero) con kind `bibliography_sync` y `asset_id_snapshot` llevando el id interno como valor opaco de compatibilidad que bibliografía jamás interpreta como asset; origen de sistema `bibliography` dedicado (recovery/finalize special-casean system batches); objetos fallidos mantienen cursor y demanda con retry de página/run (per-object retry diferido hasta identidad provisional estable para ítems nuevos); el executor vive en `bibliography/processing.rs`, implementa `Executor`, usa cliente inyectable y puente `block_on`; las funciones de catálogo/reconciliación ganan variantes transacción-neutrales porque hoy abren sus propias transacciones y no pueden llamarse dentro del `BEGIN IMMEDIATE` del commit.

**Evidencia E2b-1:** commit `2aacd1f` (`feat(processing): admit bibliographic sync tasks`), con migración `0043` — rebuild de la cerradura FK completa de 8 tablas en UNA transacción (DROP child-first, CREATE+INSERT parent-first; 3 tablas cambiadas: kind `+bibliography_sync` en tasks/batch_tasks y origen `+bibliography` en batches; 5 dependientes recreadas verbatim) con preservación byte-idéntica probada sobre filas pending/paused/interrupted/running, ambos índices únicos, attempts y checkpoints — registro byte-idéntico en `runner.ts` y fixture regenerado; brazo de admisión `bibliography`/`library` en el subject core (solo con fila existente en `zotero_libraries` vía helpers `library_row_exists`/`library_sync_pin` — processing jamás consulta tablas biblio inline; `unknown_library` honesto si falta), `asset_id_snapshot` = id interno opaco, pins conservadores (`library|id|version`, contrato `bibliography_sync/v1`) documentados para E2b-2/3; claim sigue filtrando `domain='corpus'` con doble barrera (allowlist de kind + scan) — las tareas biblio quedan pending-only; `ensure_system_batch` y recovery tratan `bibliography` como manual|repair. Verificación: 66 tests de store focalizados, 75 Rust `processing::` (incl. adversariales corpus y biblio-pending-only) + 16 `bibliography::` + 1 `processing_recovery`, typecheck/lint, `cargo check`, rustfmt focalizado y diff check — todos verdes; nota info: `PRAGMA defer_foreign_keys` es no-op en SQLite dentro de la transacción, la seguridad descansa en el orden DROP/INSERT verificado.

**Evidencia E2b-2:** commit `a58f08b` (`feat(processing): dispatch bibliographic sync pages`), con seam `ZoteroPageSource` async inyectable y adapter local que reutiliza `/items/top`, páginas acotadas y shape JSON+CSL+snapshot nativo completo; `BibliographySyncExecutor` implementa `Executor` síncrono vía `tauri::async_runtime::block_on`, pagina secuencialmente y observa `StopFlag` solo entre requests. Claim/validate admite `bibliography_sync` únicamente con `bibliography`/`library`, revalida la fila interna y distingue `library_missing`, endpoint unreachable/timeout retryable, API disabled blocked, 404 fatal y respuesta inválida retryable; corpus mantiene sus gates y partitioning. No se registró el executor en producción ni se inventó publisher/receipt: enumeración completa queda `bibliography_publisher_pending` hasta E2b-3; los checkpoints son solo unidades de página del queue. Verificación inline: 87 tests `processing::`, 26 `bibliography::`, 9 `bibliography_processing`, 1 `processing_recovery`, 252 `writing::` (1 ignored), `cargo check --locked --no-default-features`, rustfmt focalizado y diff check — verdes. Review nativa pre-lineage bloqueada por `package-local-binary-missing` (sin mutación); requiere autorización de mantenimiento para ejecutar `node scripts/install-gentle-ai.mjs`.

**Evidencia E2b-3:** commit `56c73ba`, con núcleos transacción-neutrales de catálogo y reconciliación y wrappers públicos preservados; cada página persiste atómicamente snapshots, seen-set, cursor y checkpoint, mientras los tombstones se aplican solo tras una enumeración completa confiable y la finalización mantiene monotónica la versión de biblioteca. El scheduler publica el receipt al confirmar la tarea y la recuperación converge runs de reconciliación interrumpidos. La regresión RED → GREEN cubrió exactamente 16 tests `bibliography_processing`, 22 `bibliography_catalog`, 13 `bibliography_reconciliation` y 1 `processing_recovery`, además de `cargo check`, rustfmt focalizado y diff check, todos verdes. La review nativa sigue bloqueada pre-lineage por `package-local-binary-missing`; no se ejecutó mantenimiento sin autorización.

**Evidencia E2b-4 backend:** commit `543787e` (`feat(processing): add manual bibliography sync demand`). `processing_sync_bibliography_library` valida una única fila de catálogo para `(library_type, library_id)`, persiste la respuesta por `request_id` y utiliza la misma tarea física para demandas duplicadas. Una demanda nueva reabre trabajo `interrupted`, `blocked` o `failed` sin crear otro escritor; el retry fallido conserva cursor y seen-set, no publica tombstones parciales y reanuda desde la página confirmada. El caso `zotero_snapshot_changed` conserva su barrera y comienza un run nuevo desde cero. Verificación observada: 88 tests `processing::`, 19 `bibliography_processing`, 13 `bibliography_reconciliation`, 1 `processing_recovery` y `git diff --check`, todos verdes.

**Evidencia E2b-4 UI:** el frontend separa la admisión durable de bibliografía (`processing_sync_bibliography_library`) de la actualización inmediata de la lista (`writing_zotero_sync`). `WritingZoteroStore.requestBibliographySync` genera el request ID, une llamadas concurrentes de la misma selección, descarta respuestas tardías al cambiar de biblioteca y mantiene su error separado del estado del espejo Zotero. La pestaña muestra `Sincronizar biblioteca` con estados de solicitud, aceptación en segundo plano y error, en español e inglés. Verificación: `writing-zotero.test.ts` + `WritingZoteroTab.test.ts` (55 tests), typecheck desktop (0 errores/0 warnings) y `git diff --check`, verdes. `svelte-autofixer` no estaba instalado; no se ejecutó build completo.

**Evidencia E2b-5 WU-1:** commit `50e343f` agrega dos pruebas de aceptación contra el registro mixto real. Una drena OCR y `bibliography_sync` con dos `run_one` sucesivos y verifica que cada salida publica solo en sus tablas canónicas, con reconciliación bibliográfica completada. La otra usa exactamente el mismo string opaco para `assets.id` y `zotero_libraries.id`, prueba dos tareas físicas con identidades `(corpus,asset,ocr)` y `(bibliography,library,bibliography_sync)`, valida las rutas de commit y comprueba que no haya fuga de payload entre extracción y snapshot bibliográfico. Los tests quedaron verdes sobre el código existente (sin fix de producción): 2 tests filtrados, rustfmt focalizado (`rustfmt --edition 2021 --check`) y `git diff --check`.

**Evidencia E2b-5 WU-2:** commit `5b4ebe1` agrega dos casos con cancelación real por lote y base de datos fresca. Cancelar el lote corpus deja `bibliography_sync` en `pending/active`, con `execution_wanted=true`, y permite publicar únicamente catálogo/reconciliación. La dirección inversa deja OCR `pending/active`, permite publicar solo `extractions` y confirma que el fake bibliográfico no recibió requests. Ambos casos usan `control_batch(..., BatchAction::Cancel, ...)`; los tests, rustfmt focalizado y `git diff --check` quedaron verdes sin cambios de producción.

**Aceptación:** ejecutar en paralelo un lote documental y una sincronización bibliográfica; cancelar uno no elimina demanda del otro ni publica datos revocados. **Commits:** cada unidad con regresión del corpus. **Reversión:** detener demanda bibliográfica conservando catálogo y recibos; no borrar tareas documentales.

**Siguiente:** implementar E2b-5-WU3 con tests RED-first de recovery/restart y convergencia de ambos dominios.

### E3. Perfiles globales y búsqueda híbrida de obras

**Archivos:** contrato efectivo en embeddings, settings, `bibliography/{processing,repository,retrieval,commands}.rs`, tablas semánticas y UI de búsqueda.

**Consume:** E1/E2. **Produce:** perfiles reproducibles, generaciones activas y consulta de obras compatible con el modelo efectivo.

- [ ] Unidad E3a: contrato efectivo y consentimiento por operación/proveedor, cubriendo Pro→Lite sin fallback remoto implícito.
- [ ] Unidad E3b: perfil `bibliography-profile-v1`, tarea durable y publicación; obra sin PDF también se indexa.
- [ ] Unidad E3c: búsqueda léxica/vectorial/filtros, reindexación con staging y cambio atómico.

**Aceptación:** encontrar una obra sin PDF; corregir abstract actualiza solo su perfil; cambiar a otro modelo de igual dimensión no reutiliza vectores viejos. **Reversión:** mantener catálogo/FTS y generación compatible anterior; detener demanda sin tocar índice documental.

### E4. Extracción nativa, layout y pasajes bibliográficos

**Archivos:** resolver en dominio bibliográfico, `bibliography/{extraction,processing,repository,retrieval}.rs`, tablas de extracciones/layout/páginas/chunks/spans y vista de contexto. No usar `processing/ocr.rs` como ejecutor ni crear assets documentales.

**Consume:** identidad, Lotes y contrato E1–E3. **Produce:** texto estructurado de adjuntos, layout y pasajes con spans verificables, más búsqueda dentro de obras candidatas.

- [ ] Unidad E4a: resolución/propiedad de archivos y extracción bibliográfica nativa con layout; demostrar lectura de PDF multicolumna sin ejecutar OCR ni crear assets por página.
- [ ] Unidad E4b: OCR selectivo propio para escaneados/mixtos, integración sin duplicados, checkpoints y manejo explícito de layout incompleto. Verificar capacidades y granularidad del proveedor antes de enviar contenido.
- [ ] Unidad E4c: segmentación estructural, spans de una o varias páginas, embeddings e invalidación por hashes/contratos de extracción/layout.
- [ ] Unidad E4d: recuperación jerárquica, ampliación explícita, apertura y resaltado de los tramos en el PDF original.

**Aceptación:** PDF nativo con texto/layout verificables y cero llamadas OCR; PDF escaneado/mixto con reconocimiento solo donde se necesita y sin duplicar texto nativo; fragmento multipágina abre sus tramos correctos. Sustituir un PDF por otro del mismo tamaño no conserva evidencia vieja como vigente. El OCR por asset de Fuentes sigue intacto. **Reversión:** limpiar exclusivamente archivos/derivados bibliográficos gestionados; no borrar originales Zotero ni assets documentales.

### E5. Alta/vinculación de PDF desde EntropIA

**Archivos:** `bibliography/ingestion.rs`, transporte autorizado Zotero, credenciales, Lotes, migración de operaciones de importación y bandeja UI.

**Consume:** E1/E2/E4 y escritura comprobada en E0. **Produce:** operación reanudable que confirma padre/adjunto antes de indexar.

- [ ] Unidad E5a: bandeja, coincidencias y selección explícita de obra/biblioteca, incluyendo vínculo asistido de documentos existentes sin mover el original.
- [ ] Unidad E5b: creación/adjunto/subida con recibos, permisos y resolución de conflictos; impedir duplicados tras respuestas perdidas/reinicio.
- [ ] Unidad E5c: recuperación/cancelación/cuota y activación de procesamiento solo después de verificar resultado Zotero.

**Aceptación:** cortar después de crear el padre y reanudar sin crear otro; un PDF sin vínculo permanece fuera de FTS semántico y vectores. **Reversión:** cancelar demanda local conservando recibos; no revertir automáticamente creaciones en Zotero.

### E6. Búsqueda relacionada, citas y exportaciones

**Archivos:** editor/CSL/exports de §17, `bibliography.ts`, comandos y procedencia de escritura.

**Consume:** referencias completas y pasajes E1–E4. **Produce:** selección→búsqueda→verificación→cita aceptada con snapshot y evidencia persistente.

- [ ] Unidad E6a: consulta de selección y presentación compacta en solapa Zotero; no enviar el manuscrito completo.
- [ ] Unidad E6b: inserción/edición de cita simple/múltiple, narrativa/parentética y notas con identidad estable y localizadores confirmados.
- [ ] Unidad E6c: cambio de CSL, bibliografía, exportaciones y restauración histórica; preservar snapshots ante borrado de derivados.

**Aceptación:** editar localizador de una cita grupal conserva identidad; abrir/exportar sin Zotero funciona desde snapshot; nota DOCX queda en una nota, no texto inline disfrazado. **Reversión:** deshabilitar búsqueda nueva sin retirar la lectura del formato canónico ya escrito.

### E7. Consulta combinada y evaluación final

**Archivos:** recuperación/DTOs/procedencia y capacidades del agente, composición de contexto y persistencia de conversaciones que realmente consuman los nuevos resultados.

**Consume:** recuperadores independientes y referencias verificadas. **Produce:** consultas por dominio, síntesis con procedencia y evaluación comparativa.

- [ ] Unidad E7a: fuentes/bibliografía/ambos, cuotas separadas y snapshots persistentes compatibles con conversaciones anteriores.
- [ ] Unidad E7b: validación de referencias en propuestas del modelo y distinción entre evidencia, interpretación y síntesis.
- [ ] Unidad E7c: evaluar reranking, expansión, notas seleccionadas y resumen opcional de obra; activar solo las mejoras sustentadas por evaluación y consentimiento.

**Aceptación:** reabrir una consulta mixta conserva cada referencia y dominio; texto recuperado con instrucciones adversarias no habilita herramientas/escrituras ni identidades inventadas. **Reversión:** retirar consulta mixta conservando búsquedas independientes, manuscritos y conversaciones legibles.

## 19. Matriz de pruebas y escenarios observables

Estas son obligaciones de verificación por etapa, no un pedido de escribir tests que inspeccionen nombres de funciones o copias de campos. Conservar regresiones que fallen ante un bug plausible; usar escenarios de humo para integración real y métricas de recuperación para calidad.

| Caso                                                                     | Resultado exigido                                                                             | Etapa      |
| ------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------- | ---------- |
| Key nativo distinto de CSL id; misma key en dos bibliotecas              | Identidades y citas correctas, sin colapso                                                    | E1, E6     |
| Perfil local cambiado, contador igual o retrocedido                      | No reutilizar automáticamente la partición anterior                                           | E0, E1     |
| Cambio de biblioteca con petición anterior pendiente                     | No mostrar/publicar datos de la selección anterior                                            | E1         |
| Objeto inválido, fallo de disco, paginación concurrente                  | Cursor no pierde errores; reanudación recupera objetos                                        | E1         |
| Altas, cambios, borrados, papelera, permisos vencidos y límites API      | Estados correctos sin confundir timeout con borrado                                           | E1         |
| Colección renombrada, etiqueta cambiada o ítem fuera de selección        | Filtros/alcance actualizados sin reextraer archivos                                           | E1, E3     |
| Borrado de derivados locales                                             | Ninguna escritura Zotero; citas históricas intactas                                           | E1–E6      |
| Reinicio antes/después de recibo y cancelación compartida                | Sin publicación doble ni pérdida de demanda ajena                                             | E2         |
| Obra solicitada con backlog de fondo                                     | Prioridad interactiva sin inanición de lotes                                                  | E2         |
| Modelo cambiado por otro de igual dimensión                              | Sin mezcla de espacios ni checkpoints antiguos                                                | E3         |
| Generación nueva parcial, fallo o configuración distinta al consultar    | Activa consistente o fallback léxico explicado                                                | E3         |
| Pro→Lite con proveedor local y clave remota guardada                     | Ningún envío externo sin nueva autorización                                                   | E3, E4     |
| Obra sin adjunto; varios adjuntos; archivo enlazado/WebDAV no disponible | Perfil usable y disponibilidad honesta                                                        | E3, E4     |
| PDF nativo multicolumna con encabezados, notas y tablas                  | Texto/layout y orden verificables, cero OCR; limitaciones estructurales explícitas            | E4         |
| PDF con OCR previo utilizable, escaneado o mixto                         | Conservar texto utilizable, OCR selectivo y sin duplicación al integrar resultados            | E4         |
| PDF grande, corrupto, protegido o con layout incompleto                  | Límites y errores acotados; no declarar extracción completa si no lo está                     | E4         |
| Unicode, numeración impresa distinta y párrafo entre páginas             | Spans y geometría trazables, apertura de todos los tramos y localizador confirmado            | E4, E6     |
| Ejecución bibliográfica junto con OCR de Fuentes                         | Ejecutores, decisiones y persistencia separados; compartir Lotes no enruta un dominio al otro | E2, E4     |
| Reemplazo de archivo del mismo tamaño durante embedding                  | Publicación vieja rechazada; revisión nueva correcta                                          | E4         |
| Revocación/limpieza durante una llamada de proveedor                     | Resultado tardío no reaparece como indexado                                                   | E2–E4      |
| Alta con respuesta perdida, caída tras crear padre o cuota excedida      | Reanudar sin duplicar; no indexar pendiente                                                   | E5         |
| DOI/ISBN coincidente con ediciones/traducciones distintas                | Confirmación, no fusión automática                                                            | E5         |
| Simple/múltiple, narrativa/parentética/nota, prefijos/sufijos/páginas    | Forma observable correcta en editor y exportación                                             | E6         |
| Cambio de estilo, bibliografía, reapertura y restauración antigua        | Sin duplicados por identidad ni pérdida de snapshot                                           | E6         |
| Tema/autor/obra, filtros, metadatos pobres, ausencia/baja pertinencia    | Ranking evaluado y ampliación visible                                                         | E3, E4, E7 |
| Consulta mixta reabierta y referencia ajena al contexto                  | Procedencia conservada; identidad inventada rechazada                                         | E7         |
| Migración de base existente y ausencia de Zotero                         | Fuentes, OCR por asset y edición manual siguen operativos                                     | Todas      |

### Comandos y prueba real

Ejecutar desde la raíz salvo Rust. Estos comandos existen hoy; los nuevos casos se incorporan a los módulos correspondientes durante su etapa. Registrar número de casos y resultado real, no afirmar aprobación por compilar.

```powershell
# Identidad, proyección y persistencia del editor
pnpm --filter @entropia-pro/desktop test -- src/lib/writing-zotero.test.ts src/lib/writing-roundtrip.test.ts
pnpm --filter @entropia/ui test -- src/components/WritingEditor/citations.test.ts src/components/WritingEditor/document-contract.test.ts

# Migraciones y contrato del store
pnpm --filter @entropia/store test -- src/runner.test.ts src/schema-fixture.test.ts

# Exportación compartida
pnpm --filter @entropia-pro/desktop test -- src/lib/writing-export.test.ts src/lib/export-citations.test.ts
```

Para pruebas frontend Lite, establecer `$env:VITE_LOCAL_ML='0'`; para Pro, `'1'`. Restaurar después el valor anterior de la sesión, no dejar una variante aplicada inadvertidamente. Ejecutar typecheck de desktop con variante explícita y el de los paquetes tocados.

Rust, desde `apps/desktop/src-tauri`, para los cambios de backend:

```text
cargo test --no-default-features --lib writing::
cargo test --no-default-features --lib processing::
cargo test --no-default-features --lib bibliography::
```

El filtro `bibliography::` se usa cuando el módulo y sus pruebas existen: cero pruebas ejecutadas no demuestra nada. Verificar Pro también con `--features local-ml` cuando haya código condicionado, usando el entorno preparado; no disparar un build Tauri completo o compilación MNN incidental para una revisión documental. Los cambios locales que Cargo produzca en el lock no se incluyen en commits.

Para UI y flujos Tauri: usar un directorio de datos de prueba aislado y la configuración dev aislada existente, Zotero de prueba y ambas variantes aplicables. Ejecutar selección de biblioteca, sincronización, reinicio, búsqueda, apertura y cita/exportación real según etapa. Un mock de `invoke` no demuestra IPC ni acceso al adjunto. Si una capacidad no pudo ejercitarse, declararla no verificada y no cerrar su criterio.

Al terminar la implementación, ejecutar las comprobaciones de workspace (`pnpm lint`, `pnpm typecheck`, `pnpm test`) con entorno de variante explícito y revisar regresiones relevantes de Rust. Estas comprobaciones no sustituyen los escenarios reales ni se ejecutan para este traslado documental.

## 20. Aceptación y condiciones de entrega

### Núcleo bibliográfico

- [ ] Toda obra indexada tiene identidad Zotero verificada y unicidad compuesta.
- [ ] Fuentes y bibliografía conservan almacenamiento lógico, índices y resultados separados.
- [ ] Cambios incrementales invalidan solo los derivados dependientes y rechazan publicaciones atrasadas.
- [ ] Se encuentra una obra por semántica/metadatos y se verifican sus pasajes en adjunto y página correctos.
- [ ] PDFs nativos producen texto y layout sin OCR; escaneados/mixtos usan OCR bibliográfico selectivo sin cruzarse con el ejecutor ni los assets de Fuentes.
- [ ] Los chunks conservan spans y orden de lectura verificables, incluso cuando atraviesan páginas; layout incompleto no se presenta como procesamiento completo.
- [ ] Obras sin texto completo siguen siendo buscables/citables por metadatos.
- [ ] Lotes conserva recuperación, errores individuales, cancelación compartida y prioridad editorial.
- [ ] No se mezclan modelos/generaciones ni se envía contenido a un proveedor no autorizado.
- [ ] Huérfanos, eliminados y permisos revocados no aparecen normalmente; offline se distingue de revocación.
- [ ] Insertar, editar, reabrir y exportar citas conserva identidad y snapshot sin referencias inventadas.
- [ ] Limpiar derivados no modifica Zotero ni destruye citas previas.

### Entrega completa del plan

Además del núcleo, E5 debe demostrar alta/vinculación reanudable desde EntropIA y E7 debe demostrar consulta mixta con procedencia. No declarar completo el plan solo porque funciona una búsqueda por título o porque el núcleo parcial compila.

Antes de cada commit: revisar propósito único, archivos incluidos, resultado de prueba focalizada, escenario real, límites y reversión. Añadir solo rutas de esa unidad; nunca `git add .` que capture `Cargo.lock` o trabajo ajeno. Mantener pruebas y documentación con el comportamiento que verifican. No fusionar ni publicar al finalizar; informar commits, evidencia y estado de la rama.

## 21. Correspondencia con el documento original

Esta tabla conserva el alcance y permite revisar la reformulación sin reconstruirlo de memoria.

| Sección original                           | Destino y ajuste                                                                                 |
| ------------------------------------------ | ------------------------------------------------------------------------------------------------ |
| 1. Propósito                               | Encabezado y §1; mismo objetivo, estado documental explícito                                     |
| 2. Principio rector                        | Restricciones, §§4–5; vínculo verificado, no solo string                                         |
| 3. Alcance funcional                       | §1 y E1–E7; incluye grupos, filtros, pasajes y consulta mixta                                    |
| 4. Separación entre fuentes y bibliografía | Restricciones, §§6, 15; dominios distintos sin assets ficticios                                  |
| 5. Arquitectura conceptual                 | §§1–3; transporte concreto y reutilización delimitada                                            |
| 6. Identidad e integridad                  | §§4–5; ámbito de origen, key nativo y offline                                                    |
| 7. Modelo de datos                         | §6; páginas como localizadores del adjunto, layout, spans, etiquetas, operaciones y generaciones |
| 8. Representación por obra                 | §10; plantilla reproducible y extensiones opcionales con procedencia                             |
| 9. Ingesta controlada                      | §§7, 13 y E5; alta/subida recuperable asignada a etapa                                           |
| 10. Sincronización e invalidación          | §7; cursor durable y publicación condicionada                                                    |
| 11. Recuperación jerárquica                | §§11–12; extracción bibliográfica independiente, spans verificables y ampliación evaluada        |
| 12. Recuperación combinada                 | §15 y E7; cuotas y referencias discriminadas                                                     |
| 13. Interfaz propuesta                     | §14 y E1/E6; sección bibliográfica y solapa Zotero compacta                                      |
| 14. Citas y CSL                            | §§4, 14; referencia completa, Hayagriva, notas y exportación                                     |
| 15. Duplicados y coincidencias             | §§4, 11, 13; identidad distinta de similitud bibliográfica                                       |
| 16. Privacidad y local                     | §9; consentimiento efectivo y matriz Pro/Lite                                                    |
| 17. Segundo plano                          | §8 y E2; Lotes real ampliado, no otra cola                                                       |
| 18. Etapas                                 | §18; dependencias explícitas y unidades de commit                                                |
| 19. Migraciones                            | §16; SQL, canon JSON, tareas y restauración                                                      |
| 20. Pruebas necesarias                     | §19; escenarios originales y riesgos auditados                                                   |
| 21. Evaluación de calidad                  | §12; métricas y juicios humanos antes de optimizar                                               |
| 22. Aceptación del núcleo                  | §20; núcleo separado de entrega completa                                                         |
| 23. Decisiones previas                     | §§3–16 y E0; capacidades empíricas antes de código                                               |
| 24. Restricciones explícitas               | Restricciones globales y contratos por dominio                                                   |
| 25. Resultado esperado                     | §1 y §20; Zotero administra identidad, EntropIA recupera y el editor conecta evidencia           |

## 22. Fuentes y límites de la evidencia

- [API local de Zotero](https://www.zotero.org/support/dev/web_api/v3/local_api): capacidades por versión, identidad, autorizaciones y límites de la conexión local.
- [Sincronización web v3](https://www.zotero.org/support/dev/web_api/v3/syncing): versiones, cambios, borrados, concurrencia y tratamiento de errores de guardado.
- [Subida de archivos](https://www.zotero.org/support/dev/web_api/v3/file_upload): creación de adjunto, autorización, transferencia y registro final.
- [API web v3](https://www.zotero.org/support/dev/web_api/v3/): documentación de permisos, formatos y operaciones.
- Evidencia del repositorio enlazada en §2; resultados GET acotados descritos en §3.2.
- Aclaración definitiva del usuario: OCR por asset/página se aplica solo a Fuentes. Biblioteca tiene un recorrido independiente centrado en adjuntos PDF, texto nativo y layout, con OCR selectivo; recogido en restricciones, §§2, 6, 8, 9, 11 y E4. No trasladar la granularidad de Fuentes a Biblioteca ni presentar la rutina multipágina como prueba de un problema del flujo documental actual.

La lectura de código establece contratos y riesgos de reutilización, no demuestra por sí sola un fallo ejecutado en producción. La ausencia actual de una función propuesta no es un bug del corpus. La implementación debe producir evidencia por etapa antes de afirmar compatibilidad o cierre.
