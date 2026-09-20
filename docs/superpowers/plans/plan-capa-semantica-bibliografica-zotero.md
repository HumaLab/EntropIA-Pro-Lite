# Capa semántica bibliográfica de Zotero — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. No ejecutar etapas sin autorización del usuario para iniciar implementación.

**Goal:** Hacer buscables obras y pasajes académicos vinculados obligatoriamente a Zotero, verificarlos en su adjunto e incorporarlos al editor sin confundir bibliografía con fuentes documentales.

**Architecture:** Zotero conserva la autoridad bibliográfica; EntropIA mantiene una réplica derivada, índices bibliográficos separados y evidencia por página. Se amplían los contratos compartidos de Zotero, Lotes, embeddings y citas solamente donde esta función lo necesita. Se reutilizan el procesamiento por página y el motor CSL existentes, sin crear otra cola ni otro sistema de citas.

**Tech Stack:** Svelte 5, TypeScript, Tauri 2, Rust, SQLite/Drizzle, FTS5, proveedores de embeddings existentes, Hayagriva/citationberg y API de Zotero.

**Estado:** especificación técnica reformulada y plan maestro por etapas para revisión. Este documento no acredita funcionalidades implementadas, pruebas aprobadas ni compatibilidad todavía no ejercitada. Los contratos siguientes son objetivos de implementación, no descripciones de interfaces ya disponibles.

**Origen:** documento del usuario trasladado desde `S:/Descargas/plan-capa-semantica-bibliografica-zotero.md`. El commit `9d5fc99` conserva sus 590 líneas originales sin cambios. Esta revisión incorpora la auditoría y la aclaración del usuario sobre OCR por asset/página.

## Global Constraints

- Trabajar exclusivamente en `feature/zotero-bibliografia-semantica`. No modificar `main`, fusionar ni publicar sin autorización.
- Antes de crear la rama, comprobar árbol limpio y actualizar la referencia remota de `main`; ante cambios locales sin confirmar, detenerse sin descartarlos ni sobrescribirlos. Esa comprobación ya se realizó para el traslado documental.
- Entregar unidades de comportamiento verificables, con commits pequeños. Una etapa puede requerir varios commits, pero ninguno debe presentarse como una funcionalidad completa si solo contiene estructura vacía.
- El `Cargo.lock` reescrito por el patch local de `entropia-agent` no se incluye en los commits de este trabajo. No ejecutar Cargo para validar cambios exclusivamente documentales.
- Ninguna obra vectorizada puede carecer de una identidad Zotero previamente verificada. Una clave con formato válido no prueba que exista el ítem.
- No crear una biblioteca autónoma que compita con Zotero ni inferir referencias finales mediante un LLM.
- Fuentes y bibliografía mantienen catálogos, índices, filtros y tipos de evidencia separados. Compartir SQLite o infraestructura no significa compartir las tablas del corpus.
- No introducir bibliografía como assets ficticios en `assets/items`, `vec_assets` o `rag_chunks` para reutilizar consultas o Lotes.
- **En EntropIA cada página de PDF se procesa como un asset independiente. El OCR, incluido GLM-OCR, recibe una sola página. No cambiar este flujo ni enviar PDFs multipágina a GLM como parte de esta función.**
- No reemplazar Hayagriva, duplicar las citas del editor ni crear una segunda cola. Los cambios compartidos son parte de las etapas de Bibliografía, no una refactorización general previa.
- Preservar proyectos, manuscritos, exportaciones, citas históricas e índice documental. No reclasificar documentos automáticamente.
- No borrar ni modificar datos Zotero como efecto de limpiar derivados locales. Las escrituras explícitas de ingesta requieren autorización propia.
- Conservar el uso manual del editor y de fuentes aunque Zotero o un proveedor de modelos no estén disponibles.
- Node 22+, pnpm 9.x. Pro: `VITE_LOCAL_ML=1` y feature Rust `local-ml`; Lite: `VITE_LOCAL_ML=0` sin features Rust.
- Reutilizar componentes, tokens y `ActionIcon`. No añadir dependencias sin justificar que las existentes no resuelven el requisito.

---

## 1. Lectura rápida y alcance completo

El recorrido final será:

```text
Zotero → catálogo verificado → perfil de obra / adjuntos por página
       → Lotes → índices bibliográficos → obras → pasajes → cita existente

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
| Texto completo, OCR por página, chunks y recuperación jerárquica                             | E4                    |
| Arrastrar un PDF, vincularlo o crear primero su ítem y adjunto en Zotero                     | E5                    |
| Buscar desde una selección, examinar evidencia e insertar/editar citas                       | E6                    |
| Consulta solo fuentes, solo bibliografía o combinada                                         | E7                    |
| Evaluación, privacidad y compatibilidad Pro/Lite y proyectos existentes                      | E0 y todas las etapas |

**Condición de arranque:** revisar este documento con el usuario antes de tocar código de aplicación. E0 verifica capacidades concretas del entorno y registra sus resultados aquí. Si contradicen una decisión del plan, se presenta la diferencia antes de ejecutar la etapa afectada; no se sustituye silenciosamente una función por otra más pequeña.

## 2. Lo que ya existe y qué se modifica

Las rutas enlazadas son el punto de entrada para el relevamiento; sus funciones deben volver a leerse al ejecutar cada etapa. Un módulo existente no garantiza que su contrato sirva sin cambios.

| Área              | Evidencia existente                                                                                                                                                                                                                                      | Uso y límite para Bibliografía                                                                                                                                  |
| ----------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Zotero local      | [connector.rs](../../../apps/desktop/src-tauri/src/writing/zotero/connector.rs), [mirror.rs](../../../apps/desktop/src-tauri/src/writing/zotero/mirror.rs)                                                                                               | Reutilizar HTTP, paginación, diagnóstico y resolución de padres. El espejo `key/version/CSL` es caché del editor, no checkpoint durable de ingesta.             |
| Lista Zotero      | [writing-zotero.ts](../../../apps/desktop/src/lib/writing-zotero.ts)                                                                                                                                                                                     | Cambiar la entrega de strings CSL por registros con identidad nativa. Estado, promesas y respuestas deben corresponder a la biblioteca seleccionada.            |
| Citas canónicas   | [citations.ts](../../../packages/ui/src/components/WritingEditor/citations.ts), [citation-cluster.ts](../../../packages/ui/src/components/WritingEditor/citation-cluster.ts), [repository.rs](../../../apps/desktop/src-tauri/src/writing/repository.rs) | Conservar inserción, clusters y guardado transaccional; ampliar identidad en todo el recorrido. No alcanza con agregar columnas SQL.                            |
| CSL y exportación | [render.rs](../../../apps/desktop/src-tauri/src/writing/csl/render.rs), [citation-clusters.ts](../../../apps/desktop/src/lib/citation-clusters.ts), [writing-export.ts](../../../apps/desktop/src/lib/writing-export.ts)                                 | Conservar Hayagriva y snapshots. Verificar narrativa, notas estructurales, localizadores y desambiguación documental.                                           |
| Lotes             | [scheduler.rs](../../../apps/desktop/src-tauri/src/processing/scheduler.rs), [repository.rs](../../../apps/desktop/src-tauri/src/processing/repository.rs), [0032](../../../packages/store/src/migrations/0032_batch_processing.sql)                     | Ya hay leases, checkpoints, recibos, reintentos y recuperación. Ampliar sujetos y publicación: hoy dependen de assets documentales y operaciones OCR/embedding. |
| OCR por página    | [processing/ocr.rs](../../../apps/desktop/src-tauri/src/processing/ocr.rs), [ocr/pdf.rs](../../../apps/desktop/src-tauri/src/ocr/pdf.rs)                                                                                                                 | Reutilizar cómputo de una página; persistencia bibliográfica independiente. No cambiar la unidad de procesamiento del corpus.                                   |
| Embeddings        | [embeddings.rs](../../../apps/desktop/src-tauri/src/nlp/embeddings.rs), [eligibility.rs](../../../apps/desktop/src-tauri/src/processing/eligibility.rs)                                                                                                  | Separar configuración efectiva de constantes canónicas al admitir, reanudar, publicar y consultar trabajos bibliográficos.                                      |
| Recuperación      | [rag/retrieval.rs](../../../apps/desktop/src-tauri/src/rag/retrieval.rs), [writing/retrieval.rs](../../../apps/desktop/src-tauri/src/writing/retrieval.rs)                                                                                               | Reutilizar primitivas de ranking/FTS; no su SQL, IDs documentales o política de un resultado por asset como modelo bibliográfico.                               |
| Esquema           | [runner.ts](../../../packages/store/src/runner.ts), [schema.ts](../../../packages/store/src/schema.ts), [0035](../../../packages/store/src/migrations/0035_writing_workspace.sql)                                                                        | Migraciones nuevas, registro efectivo del runner y esquema actualizado. `writing_zotero_citations` representa ocurrencias de citas, no catálogo de obras.       |
| Credenciales      | [settings.rs](../../../apps/desktop/src-tauri/src/settings.rs)                                                                                                                                                                                           | Reutilizar keyring e incorporar expresamente las nuevas claves protegidas. Un nombre que parezca secreto no garantiza almacenamiento seguro.                    |

### Corrección expresa de la auditoría sobre OCR

Se retira el hallazgo que atribuía al flujo habitual la concatenación de un PDF multipágina con pérdida de página. La existencia de una rutina capaz de unir páginas no demuestra que ese sea el recorrido del usuario.

El contrato vigente es `PDF → asset por página → OCR por asset → chunks de ese asset`. Un chunk puede obtener su página desde la identidad del asset, sin duplicarla en cada fila ni depender de regiones de layout. La función nueva debe preservar el mismo principio en el dominio bibliográfico, no reparar un defecto de OCR que la auditoría no demostró.

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

**Limpieza local:** borrar extracción, páginas copiadas, chunks, perfiles y vectores de la selección solicitada, tras cancelar su demanda y bloquear republicación de trabajos antiguos. Mantener manuscritos, snapshots citados y eventos históricos. No tocar originales enlazados ni emitir escrituras/borrados Zotero. La conservación temporal o revinculación de huérfanos requiere decisión explícita del usuario.

## 6. Persistencia propuesta y propiedad de archivos

### 6.1. Tablas y relaciones

Los siguientes nombres son el diseño propuesto, no tablas ya presentes. Usar migraciones nuevas y FK reales dentro del dominio; evitar una tabla polimórfica de vectores sin integridad referencial.

| Registro                                                                | Contenido e invariantes                                                                                                                                          |
| ----------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `zotero_connections`                                                    | origen, ámbito, identidad de servidor observada, capacidades, referencia a credencial y estado de confirmación; nunca secretos en claro                          |
| `zotero_libraries`                                                      | conexión/ámbito, tipo, ID externo, nombre, versión confirmada, fecha y alcance de sincronización; unicidad dentro del ámbito                                     |
| `bibliographic_items`                                                   | biblioteca obligatoria, key nativo, versión, JSON nativo y CSL, título, creadores, publicación/editorial, fecha, DOI, ISBN, resumen, idioma, URL, hash y vínculo |
| `bibliographic_collections` y `bibliographic_item_collections`          | claves Zotero, jerarquía y relación muchos a muchos; cambios de nombre/membresía sin reextraer PDFs                                                              |
| `bibliographic_tags` y `bibliographic_item_tags`                        | etiquetas originales normalizadas para filtros, sin perder su procedencia                                                                                        |
| `bibliographic_attachments`                                             | padre obligatorio, key de adjunto, MIME, modo de enlace, nombre, localizador resoluble, versión, hash de bytes y disponibilidad                                  |
| `bibliographic_pages`                                                   | asset de página del dominio bibliográfico: adjunto, revisión, ordinal físico, etiqueta impresa opcional y ruta del archivo de una sola página                    |
| `bibliographic_extractions`                                             | página/revisión, texto, método/proveedor, hash, offsets locales y layout opcional; no depende de layout para saber la página                                     |
| `bibliographic_semantic_profiles`                                       | obra, revisión, plantilla, texto canónico, procedencia de campos y hash de entrada                                                                               |
| `bibliographic_chunks`                                                  | extracción/página, orden, texto, offsets dentro de la página, hash y contrato de segmentación                                                                    |
| `bibliographic_embedding_contracts` y `bibliographic_index_generations` | contrato efectivo inmutable, manifiesto de entradas esperadas, progreso y puntero de generación activa                                                           |
| `bibliographic_item_embeddings` y `bibliographic_chunk_embeddings`      | FK a perfil/chunk, contrato/generación, vector, dimensión, hash de entrada y fecha; unicidad por objeto/generación                                               |
| Índices FTS bibliográficos                                              | metadatos de obras y texto de fragmentos, con actualización/borrado transaccional y filtros de vínculo/alcance                                                   |
| `bibliographic_sync_runs` y errores asociados                           | reconciliación en curso, conjuntos vistos, objetos fallidos y cursor confirmado; no constituye otro scheduler                                                    |
| `bibliographic_imports`                                                 | operación de alta/vinculación, solicitud idempotente, archivo pendiente, decisiones confirmadas y recibos de creación/subida                                     |

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

| Cambio                                               | Acción                                                                                  |
| ---------------------------------------------------- | --------------------------------------------------------------------------------------- |
| Colección o etiqueta                                 | Actualizar catálogo/FTS/filtros; recalcular perfil solo si su texto canónico cambió     |
| Título, creadores, resumen u otro campo incluido     | Nueva revisión de perfil; no reextraer adjuntos sin cambios                             |
| CSL, estilo, prefijo o localizador                   | Actualizar/renderizar cita; no regenerar embeddings por el formato de la cita           |
| Sustitución de PDF, incluso mismo nombre/ruta/tamaño | Hash de bytes distinto invalida páginas, extracciones, chunks y vectores de ese adjunto |
| Nuevo adjunto                                        | Procesar solo ese adjunto; mantener los demás                                           |
| Borrado de adjunto                                   | Excluir sus derivados, conservar snapshot histórico y aplicar política de limpieza      |
| Padre huérfano, revocado o eliminado                 | Excluir inmediatamente todos sus resultados; impedir publicación de trabajos en vuelo   |
| Cambio de modelo/contrato                            | Crear generación nueva; no comparar ni reutilizar vectores/checkpoints incompatibles    |
| Cambio de plantilla, extracción o segmentación       | Invalidar solamente descendientes que dependan de ese contrato                          |

La comparación de revisión/hash y elegibilidad se repite al publicar, no solo al comenzar. Cancelación, limpieza, revocación o reemplazo del archivo incrementan el estado que invalida una publicación atrasada.

## 8. Lotes: un coordinador, sujetos separados

Ampliar el contrato actual de tareas para identificar un sujeto como `(domain, subject_kind, subject_id)`, además de operación, revisión de entrada y contrato. Dominios: `corpus` y `bibliography`. En corpus el sujeto sigue siendo un asset; en bibliografía puede ser biblioteca, obra, adjunto o página, con existencia y elegibilidad validadas en su repositorio.

No resolver una obra bibliográfica mediante `assets JOIN items`. No debilitar el validador documental para que acepte IDs inexistentes. Las FK bibliográficas permanecen en sus tablas; el scheduler selecciona el validador/publicador por dominio sin convertirse en otro catálogo.

Operaciones bibliográficas: sincronización de biblioteca, perfil/embedding de obra, extracción de página, embeddings de sus chunks y alta/subida explícita. Reutilizar admisión idempotente, demanda compartida, leases, fencing, checkpoints y recibos. No reutilizar una tarea documental solo porque coincide el string de su ID.

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

| Operación                       | Pro                                                    | Lite                                                                                |
| ------------------------------- | ------------------------------------------------------ | ----------------------------------------------------------------------------------- |
| Extracción nativa de página PDF | Sin proveedor externo cuando hay texto utilizable      | Igual                                                                               |
| OCR de página escaneada         | Local si el motor está disponible, o remoto autorizado | Remoto cuando el motor local no está compilado; explicar/bloquear si no se autoriza |
| Embeddings                      | Local disponible o API elegida                         | API elegida; nunca convertir una preferencia local en permiso implícito             |
| Resumen opcional y síntesis     | Proveedor efectivo configurado y autorizado            | Igual condición de autorización                                                     |

Antes de enviar texto de adjuntos, metadatos, notas o una selección del manuscrito, informar destino y operación. Persistir el consentimiento con alcance suficiente para ejecutar/reanudar el lote sin diálogos por página; revocarlo bloquea nuevos envíos. Cambiar de proveedor o pasar de local a remoto exige nueva autorización.

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

## 11. Adjuntos, páginas y fragmentos

1. Resolver adjunto y verificar padre, permisos, MIME y hash.
2. Preparar **un asset bibliográfico por página**, con archivo de una página y ordinal físico ligado al adjunto/revisión. No insertarlo en las tablas del corpus.
3. Intentar extracción nativa de esa página. Si el texto es insuficiente, aplicar OCR a esa misma página según capacidades y consentimiento.
4. Guardar texto, hash, método y contrato de extracción. Layout es opcional; la página se conoce por identidad, no por segmentación visual.
5. Segmentar dentro de la página. La configuración inicial usa ventanas de hasta 800 caracteres Unicode con solapamiento de hasta 100, respetando límites seguros del texto y del proveedor; la versión del contrato registra cualquier ajuste posterior evaluado.
6. Generar embeddings y publicar únicamente si adjunto, página, extracción y vínculo conservan la revisión esperada.

Offsets `start_char/end_char` son relativos a la extracción inmutable de esa página y cuentan caracteres Unicode, no bytes UTF-8 ni unidades UTF-16. Convertir explícitamente al interactuar con APIs del editor que usen otra unidad. Los chunks no atraviesan páginas en este diseño. La vista de contexto puede recuperar páginas vecinas sin mezclar sus localizadores.

Un resultado lleva obra, adjunto, revisión/hash, página física, etiqueta impresa si fue verificada, offsets y texto. La página física sirve para abrir el original; no rellenar automáticamente el localizador CSL con ella si el usuario necesita numeración impresa distinta. Sin etiqueta impresa verificada, mostrar la página PDF y pedir confirmación del localizador de cita.

Varios adjuntos de una obra mantienen procedencia independiente. Ediciones, traducciones y versiones no se fusionan por similitud. Un adjunto no disponible no impide recuperar la obra por sus metadatos.

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

Integrar resultados en la solapa Zotero compacta existente. No crear un segundo editor bibliográfico. Guardar en procedencia el vínculo selección–obra–adjunto–página–fragmento con revisión/hash; un hash solo del texto no reemplaza su identidad.

### 14.3. Contrato de citas

Reutilizar los nodos, clusters, snapshots, renderizado documental, bibliografía y exportadores actuales. La capa semántica recomienda evidencia; Hayagriva produce el texto CSL con los datos Zotero. Renderizar a nivel de documento para desambiguar obras del mismo autor/año.

La aceptación cubre cita simple y múltiple, narrativa y parentética, localizadores, prefijos/sufijos, supresión/solo autor, estilos autor-fecha y notas, idioma configurado y bibliografía sin duplicados de identidad. Una opción visible debe tener efecto real; si un modo no está soportado, explicarlo y no presentarlo como cumplido.

Una cita en nota debe residir en una nota estructural del editor y conservar esa estructura en DOCX; seleccionar un estilo de notas no convierte por sí solo texto inline en nota. HTML/Markdown deben preservar una representación de nota navegable/legible según sus exportadores. Verificar el flujo existente de notas antes de ampliarlo, no inventar otro paralelo.

No se añade sincronización de campos vivos del plugin Zotero de Word: el alcance es el editor EntropIA y sus exportaciones. Esta exclusión no elimina el requisito de notas estructurales ni de citas verificables.

Borrar el índice o cerrar Zotero no impide renderizar/exportar snapshots ya aceptados. Un fallo CSL debe mostrarse; no presentar una cadena cacheada de otro estilo como renderización correcta del estilo nuevo.

## 15. Consulta combinada y asistencia

Definir resultados discriminados, no IDs documentales ficticios:

```ts
type EvidenceReference =
  | { domain: 'corpus'; assetId: string; sourceRevision: string }
  | {
      domain: 'bibliography'
      work: ZoteroIdentity
      attachmentKey: string
      pageId: string
      chunkId: string
      contentRevision: string
    }
```

Este tipo expresa identidad mínima; texto, offsets, localizador, puntuaciones y método viajan en el resultado correspondiente. `ZoteroIdentity` se define en §4. Los registros de conversación/procedencia deben evolucionar sin volver ilegibles los snapshots documentales anteriores.

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
- No modificar búsqueda documental, OCR por página o exportación fuera de los contratos que Bibliografía necesita. Cada ampliación compartida tiene una prueba de no regresión del comportamiento previo.
- El código obsoleto por el nuevo contrato debe migrarse con sus consumidores; no dejar dos catálogos Zotero o dos autoridades editables de identidad. Mantener lectura de datos históricos no equivale a mantener una segunda implementación activa.

## 17. Mapa de archivos para la implementación

**Existentes a ampliar, no a reemplazar por duplicados:**

- Zotero: `apps/desktop/src-tauri/src/writing/zotero/{connector,mirror,mod}.rs`, `writing/commands.rs`, `apps/desktop/src/lib/writing-zotero.ts`.
- Cola: `apps/desktop/src-tauri/src/processing/{repository,scheduler,recovery,eligibility,embedding,ocr,commands}.rs`. Tocar cada archivo solo si su contrato realmente cambia.
- Cómputo: `apps/desktop/src-tauri/src/nlp/embeddings.rs`, helpers por página de `ocr/pdf.rs`, `settings.rs` y registro de comandos en `lib.rs`.
- Citas: `packages/ui/src/components/WritingEditor/{document-contract,citations,citation-cluster,extensions}.ts`; `apps/desktop/src-tauri/src/writing/{repository,versions,commands}.rs` y `writing/csl/`.
- UI/citas: `apps/desktop/src/views/WritingZoteroTab.svelte`, `WritingCitationEditor.svelte`, `WritingView.svelte`; `apps/desktop/src/lib/{citation-clusters,writing-export,writing-zotero}.ts` y exportadores solo donde cambie su salida observable.
- Persistencia: `packages/store/src/{runner,schema}.ts`, nuevas migraciones y pruebas de migración existentes.

**Nuevos archivos propuestos, creados únicamente con su comportamiento funcional:**

- `apps/desktop/src-tauri/src/bibliography/mod.rs`: modelos y superficie pública del dominio.
- `apps/desktop/src-tauri/src/bibliography/repository.rs`: catálogo, revisiones y publicación transaccional.
- `apps/desktop/src-tauri/src/bibliography/sync.rs`: reconciliación y checkpoints Zotero, reutilizando transporte.
- `apps/desktop/src-tauri/src/bibliography/processing.rs`: adaptación de sujetos bibliográficos al único Lotes.
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

- [ ] Comprobar lectura nativa/CSL, grupos, colecciones, archivos y comportamiento de versiones con material de prueba autorizado.
- [ ] Verificar la vía de escritura elegida sin modificar bibliografía real sin autorización; si se requiere alta de prueba, usar biblioteca de prueba explícitamente seleccionada.
- [ ] Registrar capacidades no ejercitadas como no verificadas, no como soportadas.
- [ ] Revisar con el usuario cambios de decisión o costes/proveedores que requieran elección.

**Salida verificable:** cada función obligatoria tiene una vía compatible o un bloqueo concreto que se resuelve antes de su implementación; no se elimina del alcance. **Commit:** `docs: define verified Zotero bibliography capabilities`. **Reversión:** solo documentación; no datos ni aplicación.

### E1. Identidad y catálogo Zotero confiable

**Archivos:** Zotero/store/contrato canónico de §17, `bibliography/{mod,repository,sync,commands}.rs`, `bibliography.ts` y sección de catálogo en `BibliographyView.svelte`.

**Consume:** capacidades E0. **Produce:** `ZoteroReference`, catálogo persistente, filtros de metadatos y sincronización sin vectorización.

- [ ] Unidad E1a: preservar clave nativa y biblioteca en listado, selección, guardado y edición de cita existente; probar un CSL id distinto y colisiones entre bibliotecas.
- [ ] Unidad E1b: introducir tablas/relaciones y migraciones de catálogo; sincronizar altas, modificaciones, colecciones, etiquetas y adjuntos con cursor durable y errores recuperables.
- [ ] Unidad E1c: selector y ficha de obra, apertura Zotero, estados offline y exclusión por pérdida comprobada de vínculo; respuestas tardías de otra biblioteca no reemplazan la selección.

**Aceptación:** cambiar una etiqueta no pierde el ítem, no crea vector; reiniciar conserva catálogo confirmado; citas previas se exportan. **Commits:** uno por E1a/E1b/E1c con pruebas relacionadas. **Reversión:** desactivar conexión nueva sin borrar manuscritos; recuperar respaldo si se vuelve a binario anterior al esquema.

### E2. Lotes admite sujetos bibliográficos sin alterar el corpus

**Archivos:** `processing/`, migración nueva, `bibliography/processing.rs`, publicación en su repositorio y UI de progreso existente.

**Consume:** catálogo E1. **Produce:** tareas por dominio/sujeto y revisión, sincronización programada mediante el mismo coordinador, cancelación y prioridad editorial.

- [ ] Unidad E2a: ampliar identidad/migrar tareas, conservando recuperación y resultados documentales.
- [ ] Unidad E2b: conectar sincronización bibliográfica funcional al scheduler; reintento de objetos fallidos y demanda compartida tras reinicio.
- [ ] Unidad E2c: prioridad interactiva/progreso y barreras de publicación frente a cancelación, revocación y limpieza.

**Aceptación:** ejecutar en paralelo un lote documental y una sincronización bibliográfica; cancelar uno no elimina demanda del otro ni publica datos revocados. **Commits:** cada unidad con regresión del corpus. **Reversión:** detener demanda bibliográfica conservando catálogo y recibos; no borrar tareas documentales.

### E3. Perfiles globales y búsqueda híbrida de obras

**Archivos:** contrato efectivo en embeddings, settings, `bibliography/{processing,repository,retrieval,commands}.rs`, tablas semánticas y UI de búsqueda.

**Consume:** E1/E2. **Produce:** perfiles reproducibles, generaciones activas y consulta de obras compatible con el modelo efectivo.

- [ ] Unidad E3a: contrato efectivo y consentimiento por operación/proveedor, cubriendo Pro→Lite sin fallback remoto implícito.
- [ ] Unidad E3b: perfil `bibliography-profile-v1`, tarea durable y publicación; obra sin PDF también se indexa.
- [ ] Unidad E3c: búsqueda léxica/vectorial/filtros, reindexación con staging y cambio atómico.

**Aceptación:** encontrar una obra sin PDF; corregir abstract actualiza solo su perfil; cambiar a otro modelo de igual dimensión no reutiliza vectores viejos. **Reversión:** mantener catálogo/FTS y generación compatible anterior; detener demanda sin tocar índice documental.

### E4. Adjuntos por página y recuperación de pasajes

**Archivos:** resolver en dominio bibliográfico, `bibliography/{processing,repository,retrieval}.rs`, helpers de una página existentes, tablas de páginas/extracciones/chunks y vista de contexto.

**Consume:** identidad, Lotes y contrato E1–E3. **Produce:** pasajes con adjunto/revisión/página y búsqueda dentro de obras candidatas.

- [ ] Unidad E4a: resolución/propiedad de archivos, páginas bibliográficas y extracción nativa/OCR por una sola página.
- [ ] Unidad E4b: chunks por página, embeddings e invalidación por hash; varios adjuntos sin confundir procedencia.
- [ ] Unidad E4c: recuperación jerárquica, ampliación explícita y apertura del pasaje original.

**Aceptación:** verificar una cita en la página correcta de un PDF nativo y uno escaneado; sustituir un PDF por otro del mismo tamaño no conserva evidencia vieja como vigente. **Reversión:** limpiar exclusivamente archivos/derivados bibliográficos gestionados; no borrar originales Zotero ni assets documentales.

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

| Caso                                                                     | Resultado exigido                                         | Etapa      |
| ------------------------------------------------------------------------ | --------------------------------------------------------- | ---------- |
| Key nativo distinto de CSL id; misma key en dos bibliotecas              | Identidades y citas correctas, sin colapso                | E1, E6     |
| Perfil local cambiado, contador igual o retrocedido                      | No reutilizar automáticamente la partición anterior       | E0, E1     |
| Cambio de biblioteca con petición anterior pendiente                     | No mostrar/publicar datos de la selección anterior        | E1         |
| Objeto inválido, fallo de disco, paginación concurrente                  | Cursor no pierde errores; reanudación recupera objetos    | E1         |
| Altas, cambios, borrados, papelera, permisos vencidos y límites API      | Estados correctos sin confundir timeout con borrado       | E1         |
| Colección renombrada, etiqueta cambiada o ítem fuera de selección        | Filtros/alcance actualizados sin reextraer archivos       | E1, E3     |
| Borrado de derivados locales                                             | Ninguna escritura Zotero; citas históricas intactas       | E1–E6      |
| Reinicio antes/después de recibo y cancelación compartida                | Sin publicación doble ni pérdida de demanda ajena         | E2         |
| Obra solicitada con backlog de fondo                                     | Prioridad interactiva sin inanición de lotes              | E2         |
| Modelo cambiado por otro de igual dimensión                              | Sin mezcla de espacios ni checkpoints antiguos            | E3         |
| Generación nueva parcial, fallo o configuración distinta al consultar    | Activa consistente o fallback léxico explicado            | E3         |
| Pro→Lite con proveedor local y clave remota guardada                     | Ningún envío externo sin nueva autorización               | E3, E4     |
| Obra sin adjunto; varios adjuntos; archivo enlazado/WebDAV no disponible | Perfil usable y disponibilidad honesta                    | E3, E4     |
| PDF nativo, OCR previo, escaneado, grande, corrupto o protegido          | Procesamiento por página; errores acotados y explicados   | E4         |
| Unicode, página impresa distinta y párrafo continuo entre páginas        | Offsets definidos y localizadores sin inventar rangos     | E4, E6     |
| Reemplazo de archivo del mismo tamaño durante embedding                  | Publicación vieja rechazada; revisión nueva correcta      | E4         |
| Revocación/limpieza durante una llamada de proveedor                     | Resultado tardío no reaparece como indexado               | E2–E4      |
| Alta con respuesta perdida, caída tras crear padre o cuota excedida      | Reanudar sin duplicar; no indexar pendiente               | E5         |
| DOI/ISBN coincidente con ediciones/traducciones distintas                | Confirmación, no fusión automática                        | E5         |
| Simple/múltiple, narrativa/parentética/nota, prefijos/sufijos/páginas    | Forma observable correcta en editor y exportación         | E6         |
| Cambio de estilo, bibliografía, reapertura y restauración antigua        | Sin duplicados por identidad ni pérdida de snapshot       | E6         |
| Tema/autor/obra, filtros, metadatos pobres, ausencia/baja pertinencia    | Ranking evaluado y ampliación visible                     | E3, E4, E7 |
| Consulta mixta reabierta y referencia ajena al contexto                  | Procedencia conservada; identidad inventada rechazada     | E7         |
| Migración de base existente y ausencia de Zotero                         | Fuentes, OCR por asset y edición manual siguen operativos | Todas      |

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

| Sección original                           | Destino y ajuste                                                                       |
| ------------------------------------------ | -------------------------------------------------------------------------------------- |
| 1. Propósito                               | Encabezado y §1; mismo objetivo, estado documental explícito                           |
| 2. Principio rector                        | Restricciones, §§4–5; vínculo verificado, no solo string                               |
| 3. Alcance funcional                       | §1 y E1–E7; incluye grupos, filtros, pasajes y consulta mixta                          |
| 4. Separación entre fuentes y bibliografía | Restricciones, §§6, 15; dominios distintos sin assets ficticios                        |
| 5. Arquitectura conceptual                 | §§1–3; transporte concreto y reutilización delimitada                                  |
| 6. Identidad e integridad                  | §§4–5; ámbito de origen, key nativo y offline                                          |
| 7. Modelo de datos                         | §6; agrega páginas, etiquetas, operaciones y generaciones                              |
| 8. Representación por obra                 | §10; plantilla reproducible y extensiones opcionales con procedencia                   |
| 9. Ingesta controlada                      | §§7, 13 y E5; alta/subida recuperable asignada a etapa                                 |
| 10. Sincronización e invalidación          | §7; cursor durable y publicación condicionada                                          |
| 11. Recuperación jerárquica                | §§11–12; página propia y ampliación evaluada                                           |
| 12. Recuperación combinada                 | §15 y E7; cuotas y referencias discriminadas                                           |
| 13. Interfaz propuesta                     | §14 y E1/E6; sección bibliográfica y solapa Zotero compacta                            |
| 14. Citas y CSL                            | §§4, 14; referencia completa, Hayagriva, notas y exportación                           |
| 15. Duplicados y coincidencias             | §§4, 11, 13; identidad distinta de similitud bibliográfica                             |
| 16. Privacidad y local                     | §9; consentimiento efectivo y matriz Pro/Lite                                          |
| 17. Segundo plano                          | §8 y E2; Lotes real ampliado, no otra cola                                             |
| 18. Etapas                                 | §18; dependencias explícitas y unidades de commit                                      |
| 19. Migraciones                            | §16; SQL, canon JSON, tareas y restauración                                            |
| 20. Pruebas necesarias                     | §19; escenarios originales y riesgos auditados                                         |
| 21. Evaluación de calidad                  | §12; métricas y juicios humanos antes de optimizar                                     |
| 22. Aceptación del núcleo                  | §20; núcleo separado de entrega completa                                               |
| 23. Decisiones previas                     | §§3–16 y E0; capacidades empíricas antes de código                                     |
| 24. Restricciones explícitas               | Restricciones globales y contratos por dominio                                         |
| 25. Resultado esperado                     | §1 y §20; Zotero administra identidad, EntropIA recupera y el editor conecta evidencia |

## 22. Fuentes y límites de la evidencia

- [API local de Zotero](https://www.zotero.org/support/dev/web_api/v3/local_api): capacidades por versión, identidad, autorizaciones y límites de la conexión local.
- [Sincronización web v3](https://www.zotero.org/support/dev/web_api/v3/syncing): versiones, cambios, borrados, concurrencia y tratamiento de errores de guardado.
- [Subida de archivos](https://www.zotero.org/support/dev/web_api/v3/file_upload): creación de adjunto, autorización, transferencia y registro final.
- [API web v3](https://www.zotero.org/support/dev/web_api/v3/): documentación de permisos, formatos y operaciones.
- Evidencia del repositorio enlazada en §2; resultados GET acotados descritos en §3.2.
- Corrección del usuario: OCR exclusivamente por asset/página, recogida en restricciones y §2. No volver a presentar la rutina multipágina como prueba de un problema del flujo actual.

La lectura de código establece contratos y riesgos de reutilización, no demuestra por sí sola un fallo ejecutado en producción. La ausencia actual de una función propuesta no es un bug del corpus. La implementación debe producir evidencia por etapa antes de afirmar compatibilidad o cierre.
