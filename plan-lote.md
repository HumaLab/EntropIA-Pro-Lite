# Procesamiento por lote — Plan de implementación

> **Para agentes implementadores:** ejecutar por unidades revisables con las habilidades `subagent-driven-development` o `executing-plans`. Las casillas de la sección 12 son el registro de ejecución; este documento no implementa la funcionalidad.

**Objetivo:** incorporar en Configuración una solapa «Procesamiento por lote» para ejecutar OCR y embeddings sobre una o varias colecciones, sin bloquear la aplicación y conservando resultados, cola e historial ante interrupciones.

**Arquitectura:** un módulo Rust de procesamiento controla una cola SQLite durable y reutiliza los motores actuales. Svelte consulta snapshots y recibe avisos de cambios; no ejecuta ni mantiene la cola. Resultado publicado y finalización se confirman en una misma transacción.

**Tecnologías:** Svelte 5, Tauri 2, Rust/Tokio, rusqlite y SQLite; esquema/migraciones en @entropia/store. Mantener Node 22+ y pnpm 9.x. No incorporar Redis, un broker remoto ni un servicio del sistema operativo.

## 1. Decisiones y alcance

- La solapa permite seleccionar colecciones y OCR, embeddings o ambas operaciones. El backend determina elegibilidad; la UI no carga todos los assets ni decide si están procesados.
- El trabajo continúa al cambiar de vista o minimizar. Salir de EntropIA detiene la ejecución; al volver se recupera el estado y se ofrece reanudar. No se promete ejecución con el proceso cerrado.
- Tras un inicio nuevo, los lotes no terminales quedan detenidos para revisión. No se reinician llamadas potencialmente facturables sin una acción «Reanudar».
- No ejecutar OCR sobre resultados ya existentes, tampoco porque cambió el motor configurado. La regeneración de OCR existente queda fuera del alcance inicial. Embeddings sí se regeneran por invalidación del texto o incompatibilidad del contrato vigente.
- El alcance de un lote se fija al crear el borrador de preparación y se confirma antes de ejecutar. Los assets incorporados a una colección después de ese snapshot quedan para un nuevo lote. Una eliminación o edición durante la ejecución se concilia antes de publicar.
- La cola es local a esta instalación/archivo compartido entre Pro y Lite; no sincronizar estados de ejecución, leases ni secretos entre equipos. Los resultados conservan las reglas de sincronización ya existentes.
- Las propuestas que siguen son cambios futuros. La sección 2 separa la evidencia del repositorio de la arquitectura elegida.

### Alternativas consideradas

| Alternativa | Evaluación |
|---|---|
| Cola en stores de Svelte/localStorage | Rechazada: depende de la vista, no permite confirmar atómicamente resultados y estado, ni ofrece durabilidad suficiente. |
| Cola SQLite administrada por Rust | Elegida: comparte transacciones con resultados, reutiliza workers y permite recuperación sin infraestructura externa. |
| Servicio independiente del sistema | Permitiría continuar con la app cerrada, pero agrega instalación, actualizaciones y coordinación multiproceso. No lo exige este alcance. |

### Garantía de recuperación

Se garantiza conservar las **unidades confirmadas**: resultados canónicos y checkpoints cuyo COMMIT durable finalizó. Un cálculo que terminó en CPU o en un proveedor, pero cuya respuesta aún no se guardó, no puede declararse durable. Ese intento puede necesitar repetirse. El diseño ofrece publicación idempotente, no ejecución remota «exactamente una vez» ni ausencia absoluta de cobros duplicados.

SQLite con WAL y synchronous=FULL protege los commits frente a los fallos previstos si el sistema de archivos y el dispositivo respetan las operaciones de sincronización. No cubre destrucción física del disco, corrupción externa ni hardware que mienta sobre flush; eso requiere backups. No borrar ni copiar aisladamente el archivo WAL.


## 2. Base real del repositorio

| Área inspeccionada | Estado actual y consecuencia para el plan |
|---|---|
| apps/desktop/src/views/SettingsView.svelte | SettingsTab y TabList/TabButton concentran las solapas; hay traducciones mediante t(). Agregar una solapa, no una página de navegación paralela. |
| packages/store/src/schema.ts | collections → items → assets; extractions, layouts y ragChunks son resultados existentes. El alcance se resuelve por items.collection_id. |
| apps/desktop/src-tauri/src/ocr/mod.rs | OcrQueue usa canal de 64 posiciones y worker serial, sin cola durable. persist_processed_ocr_atomic ya confirma extracción/layout/invalidation de corrección conjuntamente; debe incluir el recibo de tarea. El progreso 100 puede emitirse antes del guardado. |
| apps/desktop/src-tauri/src/nlp/mod.rs | NlpQueue deduplica en memoria. start_embedding_scheduler escanea 16 candidatos cada 15 minutos y reserva capacidad foreground, pero no lleva lotes durables. |
| apps/desktop/src-tauri/src/nlp/embeddings.rs | list_asset_embedding_candidates_at reconoce ausencia/contrato y rag_incomplete, pero no toda modificación de texto o ausencia de chunks. persist_asset_embedding_with_rag_state_at no es una transacción única de vector+chunks+finalización. |
| apps/desktop/src-tauri/src/nlp/commands.rs | embed_asset usa la cola; backfill_asset_embeddings calcula fuera de ella. Deben converger para impedir carreras. |
| apps/desktop/src-tauri/src/db/open.rs | Centraliza WAL, FK y busy_timeout de 15 segundos; no establece synchronous explícito. |
| packages/store/src/runner.ts y src/lib/db.ts | El registro de migraciones está en TypeScript; initDb espera initStore. Workers actuales arrancan en lib.rs antes de esa señal. Agregar gate verificable de schema-ready. |
| packages/store/src/migrations/0021_drop_unused_processing_table.sql | Elimina jobs; no hay que basar el diseño en esa tabla antigua. |
| migrations/0016_asset_unique_ocr_transcription.sql y 0024_pdf_page_assets.sql | Ya hay identidades únicas de extracción y padre/página; reutilizar, no duplicar resultados. |
| apps/desktop/src/lib/i18n.ts | Catálogos es/en en el mismo archivo. Todo texto nuevo debe entrar en ambos. |

Precisiones que afectan la recuperación:

- OCR local puede finalizar correctamente con texto vacío. La producción GLM procesa actualmente el archivo completo; sus helpers de persistencia por páginas no prueban que existan checkpoints productivos. El bucle PDF local acumula páginas antes del guardado final.
- No reutilizar la limpieza destructiva clear_glm_pdf_page_state para reanudar: elimina resultados/hijos y datos asociados. Los checkpoints nuevos deben preservar páginas confirmadas y anotaciones.
- Embeddings usan vec_assets y rag_chunks. Existe rag_asset_embedding_state con rag_incomplete/backoff creado en Rust; migrar su información al nuevo estado e invalidación, luego retirar su uso y creación ad hoc. No mantener dos políticas de reparación. Un marcador antiguo incompleto se convierte en trabajo pendiente recuperable; un error conserva su mensaje y fecha de reintento.
- El contrato canónico inspeccionado usa BGE-M3 de 1024 dimensiones y contratos de agregación/chunking propios. Reutilizar las funciones que los definen; no hardcodear esos valores en la UI ni aceptar un modelo arbitrario incompatible.
- OCR y transcripción encolan embeddings después del resultado, y modificaciones/correcciones de texto pueden depender del refresco frontend. Sustituir esos huecos por invalidación transaccional.
- Pro y Lite comparten NLP; Lite no tiene un módulo NLP alternativo. El OCR Lite actual necesita GLM incluso para PDF con texto nativo. La solapa debe consultar capacidades reales, no deducirlas de la extensión o del selector light/high.

Estas observaciones surgen de lectura del código; no son resultados de pruebas de ejecución.


## 3. Selección automática y flujo completo

### 3.1 Preparación durable

1. El usuario selecciona una o varias colecciones y operaciones. «Analizar selección» crea un borrador persistente en estado preparing y devuelve su ID inmediatamente.
2. Rust enumera IDs de assets del alcance mediante INSERT…SELECT en una transacción corta. Esa membresía es el snapshot del lote, no una lista mantenida en el navegador. Conservar colección y título de origen para el historial, aunque luego se muevan o eliminen.
3. Evaluar ese snapshot por páginas de 200 miembros, guardando cursor, clasificación e inserciones en la misma transacción. No mantener una transacción abierta al leer archivos, renderizar PDFs ni llamar APIs. Una interrupción durante la preparación retoma el cursor confirmado sin volver a enumerar nuevos assets.
4. Mostrar candidatos OCR, embeddings nuevos, embeddings desactualizados, dependencias OCR→embeddings, ya resueltos, no compatibles y sin texto. El resumen incluye proveedor/modelo y advertencia de transferencia/coste remoto; no inventar estimaciones monetarias sin tarifas verificadas.
5. «Iniciar lote» confirma el borrador y el contrato de ejecución. Solo ahora pueden reclamarse tareas. La respuesta IPC confirma persistencia, no finalización. Una colección vacía o un conjunto sin candidatos se muestra como «Sin trabajo pendiente» y nunca dispara motores.
6. El scheduler reclama trabajo elegible, ejecuta fuera de la UI, guarda checkpoints y publica resultados. El error de un asset no detiene los siguientes. Si una página falla, cerrar ese intento del asset conservando sus checkpoints; continuar con otros assets. Reintentar el asset retoma las páginas faltantes, no borra las anteriores.
7. El lote termina cuando no quedan tareas activas, pendientes, recuperables ni dependencias resolubles: completed si todo está resuelto; completed_with_errors si hay fallos. Cancelado es un resultado distinto. Se conserva el detalle para reintentos posteriores.

El doble clic o una respuesta IPC perdida no crea otro lote: el cliente conserva un requestId hasta recibir confirmación; UNIQUE(request_id) devuelve el borrador original. Cancelar un borrador no borra resultados ni dispara procesamiento.

### 3.2 Regla de OCR

Unidad visible: asset; páginas existentes se tratan como assets propios. Son elegibles imágenes/PDF compatibles sin extracción OCR ni extracción nativa exitosa aplicable. Una extracción nativa válida satisface la necesidad de texto y no justifica rasterizar un PDF que ya lo tiene.

- La presencia de una extracción canónica existente evita el OCR automático, incluso si su texto está vacío. No confundir «sin texto reconocido» con «no ejecutado».
- Para nuevas ejecuciones, registrar outcome explícito text/no_text junto con el resultado; OCR vacío exitoso queda finalizado y no vuelve a la cola.
- No considerar layouts aislados, eventos de UI o una barra en 100% como prueba de finalización.
- Audio y tipos no soportados quedan excluidos con razón; este lote no agrega transcripción.
- PDF raíz con páginas hijas canónicas: trabajar sobre las páginas, no sobre raíz y páginas a la vez. Validar cobertura/número de páginas; una importación parcial no equivale a un documento completo.
- PDF multipágina sin páginas hijas: mantener un asset visible y checkpoints por página dentro de su tarea. Reutilizar el perfil nativo/escaneado y los motores actuales; mover al backend la coordinación necesaria, sin depender de que esté abierto el visor. Solo publicar la extracción agregada al completar todas las páginas requeridas.
- No crear assets sintéticos para checkpoints internos. Si el flujo existente exige páginas hijas, su materialización debe ser idempotente por padre+página y durable antes de marcarlas listas; nunca duplicarlas al reiniciar.

Para datos antiguos, conservar extracciones existentes sin forzar OCR para obtener metadatos nuevos. No certificar como completo un PDF parcialmente procesado por contar una fila aislada del padre: consultar sus páginas y la cobertura conocida. Un archivo ilegible o inconsistente descubierto después del snapshot produce un error por elemento.

### 3.3 Regla de embeddings

Unidad visible: asset; unidades internas: vector de asset y chunks de cada fuente de extracción/transcripción, según el contrato actual. No equivale a «hay alguna fila en vec_assets».

Elegible si tiene texto utilizable y falta el vector o un conjunto requerido de chunks, si existe invalidación persistida, si cambió el hash de la fuente, o si modelo/contrato/dimensiones/chunking ya no coinciden. Reutilizar la lógica actual de candidatos y contratos; extraer un único predicado compartido para lote, reparación e invocaciones manuales.

- Conservar embeddings vigentes; no borrarlos al encolar ni al comenzar un intento. Publicar el reemplazo completo en una transacción.
- Antes del cómputo, fijar manifest de fuentes, hashes y contrato efectivo. Antes del COMMIT, releerlos y comparar; un resultado calculado sobre texto viejo nunca limpia la invalidación nueva.
- No hay texto: excluir como no_source_text, no error del proveedor. Si OCR seleccionado puede producirlo, crear dependencia y esperar su resultado.
- OCR+embeddings: la tarea dependiente no captura un hash ficticio vacío. Resuelve y fija su entrada después del COMMIT de OCR. Si OCR termina sin texto, queda skipped/no_source_text; si falla, blocked/dependency_failed. El resto del lote continúa.
- Al reintentar OCR exitosamente, desbloquear automáticamente sus dependientes, excepto solicitudes canceladas. «Reintentar fallidos» debe explicar que incluye la cadena necesaria, no lanzar embeddings sin la fuente.
- Si cambia la fuente mientras se procesa, conservar checkpoints compatibles y devolver la tarea a pending con nueva revisión. Registrar el motivo source_changed; no contar como fallo del proveedor. Tras tres invalidaciones durante el mismo ciclo, dejar blocked/source_unstable y pedir reanudar cuando deje de editarse.
- Los cambios de configuración no alteran silenciosamente un lote: guardar snapshot no secreto del contrato. Un cambio de contrato global incompatible bloquea las tareas restantes con configuration_changed; «Reanudar con configuración actual» muestra el cambio y reevalúa pendientes. Los éxitos anteriores permanecen en el historial; otro lote cubrirá lo que el contrato nuevo invalide.

## 4. Arquitectura y propiedad de la ejecución

Módulo propuesto: apps/desktop/src-tauri/src/processing. Su interface concentra preparar, iniciar, consultar y controlar lotes. Internamente separa repositorio transaccional, elegibilidad, scheduler, recuperación y adaptadores OCR/embeddings.

La cadena normal es: Configuración → comando Tauri → SQLite → scheduler Rust → adaptador del motor → transacción de resultado+estado → evento de invalidación → snapshot actualizado.

**Una sola autoridad por operación y asset.** Los canales existentes pueden despertar workers o transportar un taskId, pero nunca ser la única copia del trabajo. Eliminar la doble planificación de embeddings: reparación automática, invocación manual y lote deben solicitar trabajo mediante el mismo módulo. Mantener NER/FTS y otras operaciones fuera del alcance, salvo los puntos donde comparten recursos o generan invalidación. La admisión manual conserva sus opciones explícitas de procesamiento/regeneración actuales: la regla estricta de «solo OCR faltante» se aplica a los lotes automáticos, no elimina una acción manual deliberada. Una regeneración manual también serializa por asset y valida revisiones antes de publicar.

Para no mezclar cancelaciones entre lotes solapados:

- processing_tasks representa el trabajo físico reutilizable.
- processing_batch_tasks representa la solicitud de un lote sobre ese trabajo; dos lotes pueden observar la misma tarea sin duplicarla.
- Una solicitud pausada/cancelada deja de habilitar ejecución, pero no cancela a otros interesados. El trabajo puede completarse por otra solicitud; la UI lo explica.
- Las solicitudes manuales y de reparación tienen origen registrado y participan en las mismas reglas. La reparación no debe recrear inmediatamente una solicitud cancelada para la misma revisión: guardar supresión por revisión hasta reintento explícito o cambio real de entrada.
- Un resultado compartido cuenta una vez dentro de cada lote y una sola vez en el trabajo global único. Las estadísticas distinguen «solicitudes de lotes» de «assets únicos».

## 5. Persistencia y durabilidad

Usar entropia.sqlite, no un JSON, localStorage o una segunda base para la cola. Reutilizar open_archive_connection y configurar explícitamente WAL, foreign_keys=ON, busy_timeout y synchronous=FULL en **todas** las conexiones que escriben resultados, invalidaciones o cola. Verificar valores efectivos por conexión; no asumir que synchronous se hereda.

### 5.1 Esquema propuesto

Nombres nuevos, sin resucitar la antigua tabla jobs retirada. Cada tabla debe tener DDL en la migración nueva y representación concordante en el registro y schema.ts cuando corresponda.

| Tabla | Campos y restricciones esenciales |
|---|---|
| processing_batches | id PK, request_id UNIQUE, origin (user/manual/repair), state, desired_state (run/pause/cancel), operations, config_snapshot_json sin secretos, planning_cursor, planning_done, revision, created_at, updated_at, started_at, finished_at, last_error. |
| processing_batch_collections | batch_id, collection_id_snapshot, name_snapshot; PK compuesta. Conserva la selección histórica. |
| processing_batch_members | batch_id, ordinal, asset_id_snapshot, item_id_snapshot, collection_id_snapshot, title_snapshot, classification y reason; UNIQUE(batch_id, asset_id_snapshot). Snapshot durable y cursor de preparación. |
| processing_tasks | id PK, kind (ocr/embedding), asset_id_snapshot, input_revision, input_fingerprint, contract_hash, state, stage, progress_done/total, outcome, attempt_count, retry_cycle, retry_count, next_retry_at, owner_session, lease_epoch, heartbeat_at, lease_expires_at, last_error_code/message, result_receipt_json, timestamps. |
| processing_batch_tasks | batch_id, task_id, kind, asset_id_snapshot, request_state (active/paused/cancelled), dependency_task_id nullable; PK(batch_id, task_id). Evitar dos solicitudes de la misma operación/asset en el mismo lote. |
| processing_requests | request_id PK, action, batch_id, payload_hash, state, selection_cursor, response_json, created_at. Registro durable de controles/reintentos masivos idempotentes; rechazar mismo ID con payload distinto. |
| processing_attempts | task_id, attempt_number, lease_epoch, started_at, finished_at, outcome, retryable, error_code, error_message, provider_request_id nullable; UNIQUE(task_id, attempt_number). Historial append-only, salvo cerrar un intento abierto. |
| processing_checkpoints | task_id, unit_key (página o fuente+chunk), input_fingerprint, contract_hash, payload, payload_checksum, created_at; PK(task_id, unit_key). Solo unidades completas y validadas, no tokens parciales. |
| processing_asset_revisions | asset_id PK con borrado en cascada, source_revision monotónica, embedding_completed_revision, invalidated_at, invalidation_reason, auto_suppressed_revision nullable. Se actualiza con las mutaciones relevantes en su misma transacción. |

Los IDs históricos de asset/colección no usan CASCADE que destruya intentos al borrar contenido. La cola conserva IDs y nombres de snapshot; referencias vivas se validan al ejecutar. FK entre tablas propias de procesamiento sí mantiene integridad. Limitar mensajes de error y no guardar texto documental completo en logs; el payload necesario pertenece al resultado/checkpoint.

Índices: tareas reclamables por (state,next_retry_at,id), solicitudes por (task_id,request_state), miembros por (batch_id,ordinal), listados por (batch_id,task_id), intentos por (task_id,attempt_number), y exclusión UNIQUE parcial sobre (kind,asset_id_snapshot) para tareas no terminales. Esa exclusión serializa incluso contratos diferentes: un cambio actualiza/bloquea la tarea, no permite dos writers sobre el mismo asset.

Persistir invalidación con triggers sobre extractions/transcriptions y mutaciones relevantes del asset, cubriendo SQL de UI, Rust y sync. Cambios de proveedor/modelo se detectan por contrato; no aumentar revisiones al cambiar título u otro metadato que no altere el texto. Instalar triggers mediante el mecanismo del runner que soporte cuerpos BEGIN…END; no pasarlos por un divisor ingenuo de sentencias por punto y coma.

### 5.2 Transacciones e invariantes

- Reclamar con BEGIN IMMEDIATE, comprobar estado, dependencia y solicitudes activas, actualizar owner/epoch/lease e insertar intento. Hacer COMMIT antes del cómputo. Nunca SELECT y UPDATE independientes sin condición.
- Para cada transición, CAS por taskId+estado+lease_epoch. Un worker antiguo no puede escribir después de recuperación, cancelación o nueva reclamación.
- El commit final valida fuente, contrato, lease y autoridad de la solicitud; escribe resultado canónico, receipt y succeeded; cierra intento, actualiza revisión completada, desbloquea dependientes y aumenta revision del lote. **Todo en la misma transacción/conexión.**
- El adaptador no puede hacer commits internos opacos. Separar calcular de publicar, o pasar la transacción al publicador existente. Ningún evento complete precede al COMMIT.
- Si falla el COMMIT, no declarar éxito. Releer tras restablecer conexión: o existe el receipt completo, o el intento es recuperable. Nunca volver a llamar al proveedor sin esa conciliación.
- Un checkpoint solo avanza progress_done después de persistirse junto a su payload. La barra visual de la llamada actual puede ser aproximada; no es evidencia durable.
- Archivos derivados imprescindibles: escribir temporal en el mismo volumen, flush/sync y renombrar atómicamente antes de referenciarlos en DB; conservar checksum. Un huérfano previo al COMMIT se limpia, una referencia sin archivo se invalida. No usar cachés/miniaturas como prueba de trabajo.
- Historial terminado se conserva hasta limpieza explícita; nunca purgar pendientes, interrumpidos ni checkpoints activos. Eliminar historial no elimina OCR ni vectores canónicos. Definir limpieza segura de tareas compartidas solo cuando no tengan solicitudes ni dependientes.


## 6. Estados y acciones

### 6.1 Máquina de estados por tarea física

| Estado | Significado | Salidas permitidas |
|---|---|---|
| pending | Admitida durablemente y todavía no reclamada. | running, blocked, skipped, cancelled. |
| blocked | Espera OCR, configuración, recurso o estabilización de fuente; razón obligatoria. | pending cuando se resuelve; failed si su dependencia acaba en fallo definitivo; cancelled. |
| running | Intento con propietario y fencing token válidos. | succeeded, retry_wait, failed, interrupted, pending por nueva revisión, cancelled tras detenerlo. |
| retry_wait | Fallo transitorio; next_retry_at persistido. | pending al vencer y existir permiso; cancelled. |
| interrupted | Intento abandonado o detenido antes de completar la unidad. Checkpoints preservados. | succeeded por conciliación de receipt; pending tras reanudar; blocked; cancelled. |
| succeeded | Resultado y recibo confirmados para la revisión/contrato registrados. | No vuelve a running; una revisión nueva produce trabajo nuevo. |
| failed | Error terminal del ciclo de intentos, con código y mensaje. | pending por reintento explícito en un nuevo ciclo; cancelled si se descarta la solicitud pendiente de resolución. |
| skipped | Ya satisfecho, no hay texto o no aplica; razón explícita. | Terminal; nuevo alcance/revisión se evalúa en otro trabajo. |
| cancelled | Sin interesados activos y ejecución detenida; no revierte resultados previos. | Terminal; una solicitud nueva crea trabajo nuevo y puede adoptar checkpoints compatibles. |

Los excluidos durante la preparación viven en processing_batch_members, no como tareas artificiales ejecutadas. skipped corresponde a una tarea que dejó de necesitar trabajo después de admitirse. El estado failed de dependencia solo se fija cuando ya no hay otra vía activa que pueda resolverla; conservar dependency_task_id para reabrirla después de reintentar OCR.

**Pausa no es éxito ni error.** Se persiste desired_state del lote y request_state de sus solicitudes. La tarea queda pending si no empezó o interrupted si se detuvo en un checkpoint; la presentación puede decir «Pausada». No duplicar paused dentro de la máquina física cuando otro lote aún usa la tarea.

### 6.2 Estado del lote

preparing → ready → running → completed / completed_with_errors. Durante preparación o ejecución: pausing → paused; tras un reinicio: interrupted; por cancelación: cancelling → cancelled. La continuación de preparing usa planning_done=false y el cursor, no comienza ejecución prematuramente.

Guardar estado deseado y derivar estado observado/counters de las solicitudes y tareas dentro de snapshots consistentes. Un lote puede mostrar «En ejecución: 2; bloqueadas: 1»; un fallo no fuerza failed global. Si solo quedan dependencias fallidas, resolverlas como fallo y finalizar con errores; si solo faltan credenciales, mostrar «Bloqueado: requiere configuración», no un spinner eterno.

### 6.3 Semántica de las acciones

| Acción | Comportamiento durable |
|---|---|
| Pausar lote / pausar todo | Deshabilita nuevas reclamaciones de sus solicitudes y persiste la intención antes de responder. Deja terminar y guardar la unidad en curso; no inicia otra. «Pausando» hasta alcanzar el punto seguro. Pausar todo incluye reparación automática del ámbito mostrado. |
| Reanudar | Concilia interrumpidas, verifica recursos/contrato y activa solicitudes pendientes. No toca succeeded. Respeta el backoff pendiente. |
| Cancelar lote | Confirma intención persistida; cancela sus solicitudes, no las de otros lotes. Si no quedan interesados, revoca lease y detiene el intento. Conserva resultados confirmados y checkpoints; no promete cancelar facturación ya enviada. |
| Reintentar elemento | Solo failed o bloqueado por fallo resuelto; crea ciclo nuevo con intento histórico creciente, revalida elegibilidad y conserva checkpoints compatibles. No duplica la tarea si otro interesado ya la reactivó. |
| Reintentar todos los fallidos | Selecciona IDs fallidos del lote en una operación durable e idempotente, incluye dependencias fallidas necesarias y procesa admisión por páginas. No reinicia éxitos, cancelados ni tareas todavía ejecutándose. |
| Descartar historial | Solo para lotes detenidos/terminales, con confirmación. Nunca significa borrar resultados del archivo. |

Carrera cancelar/finalizar: decide el orden de commits. Si el resultado se confirmó primero, permanece succeeded. Si se revocó primero el lease sin otros interesados, el resultado tardío no se publica; cerrar el intento como cancelado. En pausa, sí se permite confirmar la unidad ya iniciada.

## 7. Concurrencia y uso normal de la aplicación

- Un scheduler por archivo activo; adquirir un lock de proceso dedicado asociado a la ruta canónica del archivo antes de recuperar leases, no mantener una transacción SQLite abierta como mutex. Si otra instancia ya lo administra, no recuperar sus tareas ni lanzar un segundo scheduler; mostrar conflicto y mantener disponible el resto de la UI cuando sea seguro.
- Empezar con **un trabajo pesado local global**, no uno por lote; compartir permiso con usos interactivos de los mismos motores. Máximo **dos peticiones remotas globales**, una OCR y una embedding inicialmente; bajar por límites del proveedor. Mantener el worker serial de cada motor mientras no exista evidencia de thread-safety.
- No crear un task Tokio por cada asset. Leer un número acotado de reclamables y alimentar solo los slots disponibles. El tamaño de colección no debe determinar la RAM consumida.
- Cálculo CPU, render PDF e inferencia bloqueante en threads dedicados/spawn_blocking, nunca sobre el hilo UI o durante una transacción SQLite. Una transacción de lectura breve obtiene un snapshot coherente del texto; el cómputo usa ese snapshot fuera del lock.
- Prioridad interactiva con equidad: atender hasta tres unidades interactivas consecutivas antes de una de lote disponible; rotar entre lotes/colecciones. No prometer preempción dentro de una inferencia nativa indivisible.
- Heartbeat cada 5 segundos, lease de 60 segundos renovado mientras el supervisor mantiene el intento vivo. Expirar una lease no demuestra que un thread murió: revocar el epoch antes de recuperación, detener/recolectar el proceso anterior y no iniciar otra inferencia sobre el mismo motor hasta liberar el slot.
- Timeout remoto explícito por intento: conexión 15 segundos y solicitud total 180 segundos inicialmente, configurable dentro del backend. Conservar timeout local PaddleVL de 900 segundos y documentarlo en UI; al vencer, matar y recolectar el subproceso. Una llamada nativa no cancelable mantiene «Cancelando/pausando» hasta retornar; no liberar ficticiamente recursos ni aceptar escrituras de un lease revocado.
- Un PDF grande se procesa por páginas/checkpoints, con contadores de al menos 64 bits; no heredar casts a u8 para cantidades de páginas. Chunks se generan y guardan por unidades acotadas con backpressure.
- SQLITE_BUSY espera el timeout existente y luego reprograma; nunca dormir con una transacción abierta. SQLITE_FULL/IOERR/corrupción son fallos de persistencia: detener reclamaciones de forma segura y mostrar error global de almacenamiento, no fingir que se pudo guardar failed. Los errores individuales de contenido/proveedor no disparan esa parada global.

## 8. Recuperación ante cierres, crash y corte de energía

### 8.1 Secuencia de arranque

1. Abrir SQLite con la configuración durable y adquirir propiedad exclusiva del scheduler.
2. Esperar las migraciones reales. Agregar processing_initialize después de initStore en initDb; Rust verifica la versión esperada antes de habilitar admisión o recuperación. El comando es idempotente. Un fallo deja la solapa en error recuperable; no usar un catch best-effort que arranque workers igualmente.
3. Migrar los marcadores RAG antiguos una vez. Verificar receipts/checkpoints necesarios para los trabajos abiertos; no recalcular todos los resultados históricos.
4. En una transacción, cerrar intentos running de la sesión muerta como interrupted y revocar epochs; preservar succeeded. Restablecer intenciones de pause/cancel ya confirmadas; lotes activos no cancelados se muestran interrupted hasta autorización.
5. Concluir cancelaciones persistidas, incluso si el proceso cayó durante cancelling. Recuperar también borradores a medio preparar y reintentos con next_retry_at futuro.
6. Mostrar aviso global «Se recuperaron N lotes pendientes: X tareas completadas, Y por continuar, Z interrumpidas». Acciones Ver detalle, Reanudar seleccionados y Mantener pausados.
7. Al reanudar, reconciliar cada tarea y solo después volver a reclamarla. No depender de que el usuario abra la solapa para detectar la recuperación; la inicialización global obtiene el resumen.

### 8.2 Decisión de recuperación por elemento

| Evidencia durable | Decisión |
|---|---|
| succeeded + receipt confirmado | Conservar éxito; cero invocaciones del motor. Cambios de fuente posteriores son otro trabajo. |
| Checkpoints completos compatibles, falta publicación final | Publicar desde checkpoints, con validación de revisión/lease, sin repetir inferencia. |
| Algunas páginas/chunks confirmados | Continuar solo unidades faltantes. Mostrar «Reanudado desde 7/10 páginas». |
| running sin respuesta/checkpoint durable | interrupted; reiniciar la unidad incompleta de forma segura tras autorización. |
| Resultado preexistente satisface exactamente la entrada actual | skipped/already_satisfied, sin sobreescribir texto editado ni llamar al motor. |
| Checkpoint de otra revisión, contrato o checksum inválido | No reutilizarlo. Registrar motivo; recalcular únicamente las unidades que ya no sean compatibles. |
| Asset borrado o archivo desaparecido | skipped/source_deleted si hubo eliminación de catálogo; failed/file_missing si sigue referenciado y falta físicamente. Nunca recrear contenido borrado. |

No cargar ni ejecutar código serializado de checkpoints. Payload versionado y validado. Para embeddings, staging en processing_checkpoints conserva vector calculado y chunks; al finalizar se publican vec_assets y el conjunto completo de rag_chunks juntos. Esto evita repetir chunks confirmados y evita búsquedas sobre mezclas de revisiones.

PDF remoto: introducir procesamiento por página para el lote usando las primitivas existentes, o guardar una respuesta completa como checkpoint antes de transformarla. La ruta elegida para el lote es por página, preservando orden y cobertura del manifest; verificar con GLM el soporte del PDF de una página y no usar el helper de páginas dormant como si ya estuviera conectado. No se afirma que un proveedor permita recuperar peticiones antiguas: guardar request ID e idempotency key solo cuando su contrato real lo soporte.

Si el texto pasa a vacío o se elimina una fuente, la conciliación también retira vectores/chunks obsoletos mediante transacción condicionada a esa revisión y marca ausencia de fuente; no basta con excluir el asset del próximo scan y dejar resultados semánticos viejos disponibles.

## 9. Errores individuales y reintentos

Cada intento fallido registra código estable, mensaje comprensible sanitizado, etapa, fecha, número de intento, retryable y eventual request ID. Nunca almacenar API keys, headers Authorization, respuestas con secretos ni rutas innecesarias en eventos/logs.

| Clase | Política inicial |
|---|---|
| Red desconectada, timeout, HTTP 408/429/5xx | Máximo 3 intentos por ciclo (1 inicial + 2 reintentos). Esperas 5 y 30 segundos con jitter ±20%; respetar Retry-After mayor. Persistir la fecha concreta calculada. |
| Credencial inválida, permiso, modelo/dependencia ausente | blocked/configuration_required. No gastar reintentos en bucle; ofrecer Configurar y luego Reanudar. Otros proveedores/operaciones sanos siguen. |
| Archivo corrupto, PDF cifrado sin acceso, respuesta/vector inválido o dimensiones incompatibles | failed con causa y solución posible. Sin reintento automático; individual tras corregir. |
| Fallo de dependencia OCR | No ejecutar embeddings; registrar dependency_failed y asociar el enlace al elemento origen. |
| Error SQLite transitorio | Reprogramar acceso; no repetir cómputo si ya hay checkpoint. Conciliar antes de cualquier nueva llamada. |
| Disco lleno/IO no durable | Parada segura del scheduler, conservar el último estado confirmado y mostrar alerta global. Reanudar después de reparar almacenamiento. |

No dormir dentro del worker durante backoff: devolver el slot y atender otro asset. No resetear intentos por reiniciar EntropIA. Un reintento manual crea un nuevo ciclo de hasta tres intentos, preservando toda la historia. «Reintentar ahora» puede omitir el backoff local solo mediante acción explícita, nunca un Retry-After impuesto por el proveedor.

Mantener fallback de proveedor únicamente si lo autoriza el modo guardado; registrar proveedor efectivo, contrato y advertencia de degradación. Un fallback no puede etiquetarse como OCR de alta calidad del proveedor original. Fallos repetidos de un servicio bloquean temporalmente ese recurso, no la cola completa ni otras colecciones.


## 10. Interfaz, progreso y notificaciones

### 10.1 Solapa en Configuración

Agregar activeTab='batch' a SettingsTab y un TabButton «Procesamiento por lote», con contenido aislado en BatchProcessingTab.svelte. Reutilizar TabList, Button, Card, ConfirmDialog y ActionIcon de @entropia/ui. No meter la lógica del scheduler en SettingsView ni acoplar iniciar/pausar al botón Guardar preferencias.

Distribución propuesta:

1. **Nuevo lote:** selector buscable de colecciones con selección múltiple, «Seleccionar todas», cantidad elegida y colecciones vacías identificadas. «Todas» significa todas las colecciones del alcance consultado, no solo filas visibles; enviar IDs explícitos.
2. **Operaciones:** OCR / Embeddings, capacidades disponibles, proveedor/modelo efectivo y dependencia «Generar embeddings cuando termine el OCR». Usar configuración guardada; advertir si existen cambios de preferencias sin guardar.
3. **Análisis previo:** desglose por colección/operación y motivos de exclusión; continuar navegando durante preparing. Botones Iniciar lote y Descartar borrador. Mostrar que la elegibilidad vuelve a verificarse antes de ejecutar.
4. **Lotes activos/recuperados:** progreso durable, estado, operación en curso, contadores y Pausar/Reanudar/Cancelar/Reintentar fallidos según estado.
5. **Historial:** completados, completados con errores y cancelados, con fecha, duración activa y totales; filtros y paginación desde DB.
6. **Detalle de lote:** tabla paginada por elemento, filtros Todos/Pendientes/En ejecución/Interrumpidos/Bloqueados/Fallidos/Completados/Cancelados/Omitidos. Columnas colección, documento, asset/página, operación, etapa, avance confirmado, intentos, última actualización, error y acciones.

Detalle expandido: unidades completadas/total, próxima fecha de reintento, historial de intentos, motivo de interrupción, si el trabajo está compartido con otro lote y enlace al asset existente. «Reintentar» visible junto al error individual y «Reintentar todos los fallidos» en la cabecera; cancelados no entran en ese conjunto.

Accesibilidad: tabs con asociación al panel y teclado según el patrón existente, controles etiquetados, foco conservado tras refrescar filas, estados con texto además de color, mensajes importantes en live region moderada. No anunciar cada cambio porcentual al lector de pantalla. Confirmar cancelación sin bloquear la navegación normal.

### 10.2 Indicadores de progreso

No sumar porcentajes de OCR y embeddings como si costaran lo mismo. Usar progreso por tareas y progreso interno por unidades:

- total = solicitudes admitidas únicas del lote una vez planning_done=true.
- settled = succeeded + skipped + failed + cancelled de sus solicitudes; blocked/pending/running/retry_wait/interrupted no están resueltas.
- avance de resolución = settled/total; éxitos = succeeded, mostrados aparte. «100% resuelto; 3 fallidos» no se presenta como éxito total.
- Para total=0 mostrar «Sin trabajo pendiente», no división por cero. Durante preparación mostrar cantidad analizada y estado indeterminado, no un porcentaje sobre un total todavía variable.
- Progreso por elemento: páginas o chunks confirmados/esperados más etapa actual. Si el proveedor no informa avance interno, indicador indeterminado durante esa unidad. Solo llegar a «Completado» al confirmar la transacción final.
- Al reabrir fallidos, settled puede bajar: mostrar «Ciclo de reintento N» y mantener cifras históricas. No ocultar esa variación con una barra artificialmente monótona.
- Duración activa excluye pausa/backoff; tiempo transcurrido total se muestra aparte. ETA solo si hay muestras suficientes de la operación/proveedor actual; si no, «Calculando», nunca tiempo inventado.

Solicitudes canceladas cuentan como canceladas en su lote aunque la tarea física termine para otro interesado. Solicitudes pausadas pueden reflejar éxito compartido; se explica que el lote no generó nuevas reclamaciones. No sumar a la vez el padre PDF y sus checkpoints como tareas globales: checkpoints alimentan el avance del asset, no inflan el denominador.

### 10.3 Sin depender de eventos

Crear store de lectura global en lib/batch-processing.ts, inicializado desde App.svelte, y un indicador compacto en AppShell.svelte con enlace a Configuración → lote. El store mantiene solo resúmenes y la página visible, no la cola completa.

Protocolo: suscribir evento processing:changed, obtener snapshot inicial y conciliar por revision creciente; ignorar eventos viejos. Eventos son avisos de invalidación después del COMMIT, no la fuente de verdad. Si se pierde un evento, consultar al volver a foco/abrir la solapa y, mientras haya actividad, cada 3 segundos. Coalescer cambios a un máximo aproximado de 4 avisos/segundo; no emitir el texto OCR ni vectores.

Snapshot global, contadores y página de detalle se leen en transacción de lectura corta, con revision. Cuando cambie la revisión, refrescar filtros/cursor si procede. Listados por keyset estable, límite por defecto 50 y máximo 200. No bloquear el resto de la app esperando el detalle de un lote grande.

## 11. Interface backend/frontend propuesta

Los nombres de este apartado son **nuevos contratos**, no comandos existentes. DTOs Rust con serde y nombres camelCase explícitos en el payload; wrapper TypeScript tipado. Registrar comandos en lib.rs y revisar las capacidades existentes sin ampliar acceso al sistema de archivos.

| Comando | Entrada | Respuesta/efecto |
|---|---|---|
| processing_initialize | Sin entrada de esquema desde el cliente. | Backend verifica migración, recupera una sola vez por sesión; devuelve capacidades y resumen recuperado. |
| processing_prepare | requestId, collectionIds[], operations[], opciones no secretas. | batchId inmediato después del snapshot durable; preparación asíncrona. |
| processing_start | batchId, expectedRevision. | Snapshot tras confirmar lote ready; conflicto tipado si cambió. |
| processing_list_batches | estados/origen, cursor, limit. | Página de BatchSummary y nextCursor. |
| processing_get_batch | batchId. | BatchSnapshot: estado deseado/observado, config, revision, contadores por estado/operación/colección y preparación. |
| processing_list_tasks | batchId, filtros de estado/operación, cursor, limit. | TaskSummary[] sin payloads de resultados ni todos los intentos. |
| processing_get_task | batchId, taskId, cursor de intentos. | Detalle de etapas/checkpoints agregados, error e intentos paginados. |
| processing_control | batchId o scope=all, action=pause/resume/cancel, expectedRevision. | Confirmación durable y snapshot; no espera a que termine la inferencia en curso. |
| processing_retry | requestId, batchId, taskId o failedOnly=true. | Número seleccionado, ID de operación durable y snapshot. Reutiliza admisión única; no llama directamente motores. |

Errores de comando: invalid_selection, schema_not_ready, revision_conflict, storage_unavailable, capability_unavailable, invalid_transition. Separarlos de last_error de un elemento. Los comandos no aceptan rutas arbitrarias: resuelven asset e identidad desde DB; validar pertenencia al alcance y no confiar en proveedor/modelo recibido del cliente.

Interface interna mínima del repositorio: prepare_batch, admit_or_attach, claim_next, save_checkpoint, commit_success, finish_attempt, control_batch, recover_session y read_snapshot. Todos usan las mismas invariantes; no exponer SQL mutable a Svelte. Admisión de reintento y alta nueva se serializan: si ya existe una tarea activa para kind+asset, adjuntar la solicitud y mantener el intento histórico, no violar el índice ni crear otro worker.

Pseudocódigo del protocolo que deben implementar esos métodos (no es DDL listo para copiar):

~~~text
claim_next(session):
  BEGIN IMMEDIATE
  t := primera tarea reclamable con alguna solicitud active y dependencia resuelta
  comprobar que el lote admite ejecución y el recurso tiene slot
  CAS t.state -> running; aumentar lease_epoch; asignar session/lease
  insertar intento abierto; aumentar revisiones afectadas
  COMMIT
  devolver t + epoch + snapshot de entrada

execute(t, epoch):
  para cada unidad faltante del manifest:
    verificar cancelación, revisión y permiso
    calcular fuera de transacción
    save_checkpoint(t, epoch, unidad, resultado validado)
  commit_success(t, epoch)

commit_success(t, epoch):
  BEGIN IMMEDIATE
  verificar lease, revisión, contrato, cobertura y autoridad de publicación
  publicar extracción/layout o vector+chunks desde checkpoints
  guardar receipt; marcar succeeded; cerrar intento
  resolver dependencias; invalidar derivados según la operación
  COMMIT
  avisar processing:changed
~~~

No implementar el protocolo como persist_processed_ocr_atomic(); UPDATE task='succeeded' separado. Para embeddings, extraer inferencia fuera de la transacción actual de backfill y conservar UPSERT de vec_assets y las operaciones de reemplazo de chunks compatibles con los triggers FTS.

## 12. Plan de implementación por unidades revisables

Orden obligatorio 1→2→3→4→5→6. La UI puede prepararse después de fijar DTOs, pero no se considera entregada con datos simulados. Antes de modificar símbolos exportados, localizar referencias mediante LSP y migrar todos los productores identificados. Los archivos nuevos de esta sección son propuestas; el resto son puntos existentes inspeccionados o localizados.

### Unidad 1 — Esquema durable, migración y gate de arranque

**Archivos:** crear packages/store/src/migrations/0032_batch_processing.sql (usar el siguiente ordinal libre al implementar); modificar packages/store/src/runner.ts, schema.ts y runner.test.ts; crear apps/desktop/src-tauri/src/processing/{mod.rs,repository.rs}; modificar db/open.rs, lib.rs y apps/desktop/src/lib/db.ts. Regenerar apps/desktop/src-tauri/tests/fixtures/schema_full.sql mediante packages/store/scripts/export-schema.mjs.

**Consume:** tablas actuales y open_archive_connection. **Produce:** repositorio durable, esquema registrado y processing_initialize.

- [ ] Escribir regresión de migración sobre archivo SQLite temporal: base anterior con éxitos OCR, marcador RAG incompleto y datos de usuario; migrar y reabrir sin perder resultados ni historial convertido.
- [ ] Definir tablas/índices de §5 y constraints de estados; activar FULL por conexión. Registrar la migración completa con triggers, evitando splitStatements.
- [ ] Hacer atómico el DDL nuevo y su registro en _migrations; cubrir caída entre ambos. No cambiar migraciones históricas ya aplicadas. Hacer importación de rag_asset_embedding_state idempotente y retirar su creación/uso al completar la unidad 4.
- [ ] Agregar gate explícito tras initStore. Hasta schema-ready, ninguna vía OCR/embedding migrada puede ejecutar; un comando temprano devuelve schema_not_ready.
- [ ] Verificar fresh install, upgrade y reinicio de migración interrumpida. Comprobar PRAGMA journal_mode/synchronous/foreign_keys/busy_timeout en conexiones de UI y workers.

**Criterio de aceptación:** crear una tarea, confirmar, terminar el proceso y reabrir el archivo conserva la tarea; una migración incompleta nunca permite arrancar el scheduler. Commit sugerido: feat(processing): persist queue schema and gate startup.

### Unidad 2 — Admisión, elegibilidad e invalidación

**Archivos:** crear processing/eligibility.rs; ampliar processing/repository.rs; modificar nlp/embeddings.rs y los triggers de la migración nueva. Ampliar pruebas Rust del módulo y de runner. Consultar collection.repo.ts y las identidades de asset/página sin replicar su modelo en frontend.

**Consume:** repositorio/schema listo. **Produce:** prepare_batch y admit_or_attach, predicado compartido de vigencia, snapshot durable del alcance.

- [ ] Escribir casos discriminantes: OCR vacío ya realizado, nativo ya extraído, padre con páginas, embeddings actuales, vector legacy, chunks faltantes, fuente editada/eliminada y ausencia total de texto.
- [ ] Implementar membresía snapshot y análisis por cursor con commits acotados; preview y ejecución usan la misma elegibilidad revalidada.
- [ ] Añadir revisión transaccional de fuente y comparación de hashes/contratos; no reutilizar summarize_asset_embedding_coverage como prueba completa de vigencia.
- [ ] Probar dos admisiones simultáneas de mismo asset/operación y dos colecciones solapadas en solicitudes: una sola tarea activa con enlaces separados.
- [ ] Probar crash durante preparación: el cursor retoma, no admite duplicados ni assets agregados después del snapshot.

**Criterio de aceptación:** el planificador devuelve exactamente los candidatos esperados y conserva la selección tras reabrir; ningún caso ya resuelto llama al motor. Commit sugerido: feat(processing): select missing work and track source revisions.

### Unidad 3 — Scheduler, estados, control y recuperación

**Archivos:** crear processing/scheduler.rs, recovery.rs y commands.rs; ampliar repository.rs y lib.rs; agregar pruebas de comportamiento dentro de processing/tests.rs.

**Consume:** admisión y revisiones. **Produce:** claim_next, control_batch, recover_session y comandos de lectura/control.

- [ ] Escribir pruebas con reloj controlable, archivo SQLite real y executor de prueba con barreras: claim exclusivo, fallo aislado, lease viejo rechazado y cancelación frente a commit.
- [ ] Implementar slots acotados, equidad, heartbeat/fencing y supervisor. Los timers despiertan la cola persistida, no contienen el único estado.
- [ ] Implementar pausa/reanudar/cancelar por solicitud y lote, solicitudes compartidas, historial de intentos y backoff persistente.
- [ ] Implementar recuperación de preparing/running/retry_wait/pausing/cancelling; no reiniciar succeeds ni reactivar solicitudes canceladas.
- [ ] Probar que un worker fallido libera su slot; que una inferencia no cancelable no libera ficticiamente su recurso; que un disco no escribible detiene admisión sin inventar un estado durable.

**Criterio de aceptación:** navegación no es requisito del worker, reiniciar conserva estados y reanudar no ejecuta tareas confirmadas. Commit sugerido: feat(processing): schedule and recover durable work.

### Unidad 4 — Motores reales, checkpoints y cutover completo

**Archivos:** crear processing/ocr.rs y embedding.rs; modificar ocr/mod.rs, ocr/commands.rs, ocr/glm_ocr.rs, ocr/pdf.rs, nlp/mod.rs, nlp/embeddings.rs, nlp/commands.rs y lib.rs. Migrar productores en transcription/mod.rs, transcription/commands.rs y llm/ocr_correction.rs; revisar los cambios de fuente en sync/apply.rs. Adaptar apps/desktop/src/lib/ocr.ts y nlp.ts y sus llamadas en ItemView.svelte si cambia el contrato. Mantener file-import.ts/CollectionView.svelte sin cambios salvo que la integración de páginas realmente los requiera.

**Consume:** ejecución durable, lease y revisión. **Produce:** ejecución OCR/embedding real con commit_success atómico, checkpoints y un solo camino de admisión.

- [ ] Antes de cambiar workers, escribir pruebas de crash en las fronteras antes/después de checkpoint y commit de resultados; usar entradas pequeñas reales o respuestas HTTP controladas para contar invocaciones.
- [ ] Extraer persistencia OCR que acepta la transacción del scheduler; preservar UPSERT, layouts y la invalidación de corrección existente. Guardar éxito vacío y proveedor efectivo.
- [ ] Introducir manifest PDF y checkpoints por página; probar página en blanco, documento mixto, fallo en página intermedia y más de 255 páginas sin overflow. No usar borrado masivo de páginas al reanudar.
- [ ] Separar snapshot, inferencia y publicación embedding. Guardar chunks calculados en staging, publicar conjunto completo+vector+receipt atómicamente y no limpiar invalidación más nueva.
- [ ] Llevar extract_text, embed_asset, backfill_asset_embeddings, start_embedding_scheduler y productores OCR/transcripción a admisión única. Preservar el comportamiento público manual solicitado, pero retirar rutas directas de cómputo y dedup únicamente en memoria de las operaciones migradas; NER/FTS quedan intactos.
- [ ] Migrar/retirar el estado RAG viejo y su backoff, incluidas referencias de pruebas y limpieza de borrado. Un barrido de reconciliación puede permanecer como productor de solicitudes, nunca como segundo ejecutor.
- [ ] Resolver capacidades por variante/proveedor sin inicializar Paddle para un trabajo remoto; conservar disponibilidad de PDF en Lite sin prometer OCR local inexistente. Agregar timeouts y errores tipados en la llamada GLM usada por la cola.
- [ ] Verificar edición de texto por UI, corrección/restauración y sync: todas persisten invalidación sin depender de una ventana abierta ni de debounce frontend.

**Criterio de aceptación:** OCR y embeddings reales terminan sobre dos colecciones; el fallo de un asset no frena al siguiente; caída tras siete páginas/chunks confirmados no vuelve a invocarlos; fuentes editadas nunca reciben un vector viejo. Commit sugerido: feat(processing): integrate OCR and embedding checkpoints.

### Unidad 5 — Solapa, estado global y controles

**Archivos:** crear apps/desktop/src/views/BatchProcessingTab.svelte y lib/batch-processing.ts; modificar SettingsView.svelte, App.svelte, layout/AppShell.svelte y lib/i18n.ts. Reutilizar lib/navigation.ts para abrir Configuración y transportar selección de solapa/lote sin agregar una ruta paralela. Crear BatchProcessingTab.test.ts solo para las transiciones de usuario con riesgo real; extender pruebas existentes pertinentes de SettingsView sin fijar texto/markup incidental.

**Consume:** DTOs y comandos reales de §11. **Produce:** UI de §10 y acceso global al resumen recuperado.

- [ ] Implementar wrappers tipados y store de snapshots con reconciliación por revision, suscripción previa a snapshot, polling acotado y liberación de listeners.
- [ ] Incorporar selección múltiple, análisis previo y confirmación; no cargar todos los assets. Mantener preferencias y ejecución como acciones independientes.
- [ ] Implementar listado/detalle paginados, filtros por estado, errores e intentos; acciones individuales y conjuntas con estado pendiente hasta confirmación durable.
- [ ] Agregar aviso de recuperación y progreso fuera de Configuración; al abrir su enlace seleccionar la solapa/lote sin descartar preferencias sin guardar.
- [ ] Completar traducciones es/en y accesibilidad del patrón existente. Iconos exclusivamente mediante ActionIcon; reutilizar los disponibles antes de ampliar el catálogo.
- [ ] Verificar visualmente con la aplicación real: seleccionar dos colecciones, iniciar, salir de Configuración, seguir usando visor/búsqueda, volver, pausar/reanudar y reintentar un fallido. Comprobar que desmontar el panel no cancela el worker.

**Criterio de aceptación:** UI representa el snapshot durable incluso tras perder eventos o recargar WebView; se puede controlar cada fallo sin bloquear navegación. Commit sugerido: feat(settings): add batch processing tab and progress.

### Unidad 6 — Pruebas de interrupción, variantes y documentación de uso

**Archivos:** pruebas Rust en processing/tests.rs y, para terminación de procesos, crear apps/desktop/src-tauri/tests/processing_recovery.rs con un ejecutable de prueba aislado si lo requiere el harness; reutilizar fixtures generadas. Actualizar Manual-de-Uso.md y README.md/README.en.md solo en las secciones afectadas durante la implementación, y el changelog existente si lo hubiera. Este encargo actual entrega únicamente plan-lote.md.

**Consume:** funcionalidad integrada. **Produce:** evidencia de aceptación, no un build que solo compila.

- [ ] Ejecutar matriz de §13 sobre una base temporal y archivos de prueba; nunca matar el proceso que usa el archivo personal del usuario ni probar cortes sobre datos reales.
- [ ] Probar Pro con local-ml/VITE_LOCAL_ML=1 y Lite sin features/VITE_LOCAL_ML=0; en Lite usar proveedor remoto y mostrar límites reales. No descargar modelos/llamar proveedores facturables sin autorización de la persona que verifica.
- [ ] Ejecutar pruebas específicas, luego checks workspace una sola vez al concluir; confirmar con smoke real de Tauri, no solo navegador con mocks.
- [ ] Registrar salida de pruebas, conteos de invocaciones y snapshots antes/después del reinicio; probar integridad de la copia SQLite reabierta.
- [ ] Documentar cómo iniciar, pausar, recuperar y reintentar, límites de cierre y facturación remota. Eliminar scripts temporales y revisar que no queden rutas viejas ni estados de mentira.

**Criterio de aceptación:** todas las filas de §13 satisfechas en las variantes aplicables; ninguna afirmación de corte de energía se deriva solo de un test in-memory. Commit sugerido: test(processing): verify crash recovery and document batch workflow.

## 13. Matriz de aceptación y comandos de verificación

Los comandos siguientes son para la **implementación futura**. No se ejecutan ni se declaran aprobados por crear este plan.

| Escenario | Evidencia exigida |
|---|---|
| Dos colecciones, OCR existente y faltante | Solo se invocan assets faltantes; extracción vacía exitosa no se repite. |
| Embeddings actuales, ausentes, legacy, chunks incompletos y texto editado | Se conserva lo vigente y se regenera exactamente lo inválido. |
| OCR+embedding dependiente, OCR vacío y OCR fallido | Orden correcto; no_source_text distinguido de error; dependiente fallido no bloquea otros assets. |
| Una tarea falla entre tareas correctas | Las restantes terminan; mensaje/intent count persisten tras reiniciar. |
| Reintento individual/conjunto con doble clic | Un solo ciclo admitido por requestId; éxitos intactos, historial preservado. |
| Salir de Configuración y recargar WebView | Trabajo sigue; snapshot reconstruye avance sin historial de eventos. |
| Cierre normal, crash/kill del proceso, reinicio del sistema | Recupera pendientes y running→interrupted, conserva commits, pide reanudar. |
| Corte durante preparación | Membresía/cursor consistentes, sin duplicados ni nuevos assets del futuro. |
| Corte después de checkpoint y antes del commit final | Reusa páginas/chunks confirmados y publica sin repetir esas invocaciones. |
| Corte después de COMMIT y antes del evento | Succeeded reconstruido desde DB; contador del motor no aumenta. |
| Corte durante COMMIT | Resultado+receipt son ambos visibles o ninguno; nunca mezcla parcial. |
| Pausar/cancelar durante request y reiniciar | Intención respetada; no nueva reclamación inadvertida ni resultado tardío con lease viejo. |
| Dos lotes comparten asset; cancelar uno | Una ejecución física; el otro sigue y los contadores no se mezclan. |
| Editar, vaciar, mover o borrar fuente durante cómputo | No sobreescribe nueva versión ni revive eliminados; retira vectores obsoletos cuando ya no hay texto. |
| Proveedor 429, timeout, key ausente y fallo de modelo | Backoff durable, slots liberados, configuración bloqueada sin frenar recursos sanos. |
| Disco lleno o IOERR | No muestra éxito ficticio; pausa segura y relectura consistente al reparar. |
| PDF mixto/blanco/grande y fuente modificada | Cobertura por página, sin pérdida de anotaciones ni overflow; checkpoints incompatibles no se reutilizan. |
| Cola de 10 000 assets y lectura concurrente | Paginación/memoria acotadas, UI interactiva, sin llamadas remotas dentro de transacciones largas. |
| Evento perdido/duplicado/desordenado | Snapshot por revision corrige UI; no duplica conteos ni tareas. |
| Fresh install/upgrade y tablas sync | Schema-ready real, migración atómica/idempotente y cola excluida de replicación. |

### Comandos concretos

Desde la raíz, una vez implementados los archivos:

~~~powershell
pnpm --filter @entropia/store test -- src/runner.test.ts
pnpm --filter @entropia-pro/desktop test -- src/views/BatchProcessingTab.test.ts
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --no-default-features processing
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --features local-ml processing
pnpm lint
pnpm typecheck
pnpm test
~~~

Para typecheck/pruebas Lite, usar env VITE_LOCAL_ML=0 en el proceso o guardar/restaurar el valor PowerShell en try/finally. Para Pro usar 1 explícitamente. Los filtros Rust de ejemplo requieren que los tests nuevos lleven processing en su ruta/nombre; verificar que se ejecutaron casos y no aceptar una salida con cero tests como prueba.

Regeneración de fixture: pnpm --filter @entropia/store export-schema. El script actual importa TypeScript nativo y declara requerir Node 24+; usar ese runtime para el export, sin convertirlo en requisito nuevo del resto de la aplicación.

Smoke Tauri **solo dentro de una VM desechable o cuenta de Windows de prueba**, desde apps/desktop: pnpm exec tauri dev --config src-tauri/tauri.lite.conf.json --no-default-features con VITE_LOCAL_ML=0. Para Pro, pnpm exec tauri dev --features local-ml y VITE_LOCAL_ML=1. path_utils.rs:data_dir usa com.entropia.shared para Lite, Pro y dev: tauri.dev.conf.json cambia el identificador pero **no aísla el archivo de datos**. No usar ese nombre de configuración como prueba de aislamiento. No lanzar builds release/bundles para validar una cola; Pro puede compilar MNN por primera vez.

Prueba de energía: usar VM/entorno desechable con disco de prueba, cortar sin apagado ordenado en puntos instrumentados y volver a abrir la base. Complementar con fault injection de SQLite/VFS cuando esté disponible. Un kill demuestra muerte de proceso, **no** comportamiento completo del dispositivo ante pérdida eléctrica. Registrar esa diferencia en el informe de verificación.

## 14. Condiciones de entrega

La funcionalidad no se considera terminada si solo existen la solapa, una cola en memoria, un porcentaje persistido o reintentos globales. Deben quedar unidos: selección real por colección, admisión durable, ejecución en segundo plano, resultado+receipt atómicos, checkpoints recuperables, aislamiento de fallos y controles individuales/conjuntos.

Puntos no negociables de revisión: ningún motor desde Svelte; ninguna cola OCR/embedding alternativa; ningún succeeded antes del COMMIT; ningún resultado viejo que limpie una invalidación nueva; ninguna recuperación que borre páginas confirmadas; ninguna promesa de ejecución con EntropIA cerrada o de llamadas remotas exactamente una vez.
