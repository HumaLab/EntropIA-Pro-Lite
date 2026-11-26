# Mapa de pantallas de EntropIA

> Generado por `apps/desktop/scripts/build-help-screen-map.mjs`; no editar a mano.
> Fuentes: `apps/desktop/src/lib/i18n.ts` (bloque español), `apps/desktop/src/lib/navigation.ts` (vistas) y
> `apps/desktop/src/views/SettingsView.svelte` (pestañas).
> Regenerar desde `apps/desktop` con `pnpm help:screen-map`;
> comprobar con `pnpm help:screen-map:check`.

## Pantallas

### Vistas de la app

- «Inicio» `home`
- «Colecciones» `collections`
- `collection`
- `item`
- «Base de datos» `db-browser`
- «Chat» `rag-chat`
- «Investigación» `research`
- `investigation`
- «Escritura» `writing`
- «Biblioteca» `biblioteca`
- `bibliography-work`
- «Configuración» `settings`
- «Navegador» `navegador`

### Pestañas de Configuración

- «APIs remotas» `settings.remoteApisTab`
- «Motores» `settings.enginesTab`
- «Modelos locales» `settings.localModelsTab`
- «Prompts» `settings.promptsTab`
- «Model Params» `settings.modelParamsTab`
- «RAG Params» `settings.ragParamsTab`
- «Agente» `settings.agentParamsTab`
- «Sincronización» `settings.syncTab`
- «Asistentes» `settings.assistantsTab`
- «Ayuda» `settings.helpTab`
- «Dependencias de IA» `settings.dependenciesTab`
- «Lotes» `settings.batchTab`
- «Alias» `settings.aliasesTab`
- «Logs» `settings.logsTab`
- «Copias» `settings.backupTab`
- «Mantenimiento» `settings.maintenanceTab`
- «Apariencia» `settings.appearanceTab`

## Aplicación

### Varios

- «Inicializando...» `app.initializing`
- «No apagues el equipo. La ventana se cierra sola al terminar.» `app.compactingBody`
- «Optimizando el archivo…» `app.compactingTitle`
- «Preparando tu espacio de trabajo» `app.startupTitle`
- «Reintentar» `app.retryInit`
- «Reintentá. Si vuelve a fallar, el detalle queda en el registro de diagnóstico.» `app.initErrorUnknown`

## Ventana

### Varios

- «Archivo, OCR y análisis asistido.» `appshell.caption`
- «Cerrar explorador (Esc)» `appshell.explorerDrawerClose`
- «Colapsar panel (Ctrl+B)» `appshell.sidebarCollapse`
- «Desarrollado por» `appshell.developedBy`
- «Expandir panel (Ctrl+B)» `appshell.sidebarExpand`
- «Explorador de documentos del panel activo» `appshell.explorerDrawerAria`
- «Filtrar colecciones» `appshell.sidebarFilterCollections`
- «Filtrar colecciones...» `appshell.sidebarFilterCollectionsPlaceholder`
- «GitHub» `appshell.githubAria`
- «Nueva colección» `appshell.sidebarNewCollection`
- «Panel lateral» `appshell.sidebarAria`
- «Repositorio en GitHub» `appshell.githubTitle`

## Asistentes

### install

- «Conectar empareja un cliente de solo lectura y registra EntropIA en el asistente. Si falla, revoca ese cliente. Para copiar el comando, creá un emparejamiento con el formulario de abajo.»
- «Conectar {assistant}»
- «El acceso de asistentes está apagado: activalo arriba para que {assistant} pueda conectarse.»
- «No pude confirmar la revocación; el emparejamiento podría seguir activo. Revisá los clientes emparejados: {detail}»
- «no encontré {assistant} en esta computadora»
- «o copiá el comando»
- «{assistant} conectado»
- «{assistant} está instalado mediante un script que no puedo ejecutar de forma segura; usá el comando para copiar»
- «{assistant} no respondió en 30 segundos; usá el comando para copiar»
- «{assistant}: el cliente ya no está emparejado»
- «{assistant}: la configuración no es un objeto JSON válido o es un enlace; no la modifiqué: {detail}»
- «{assistant}: {detail}»

### Varios

- «Copiar instrucción para cualquier asistente» `assistants.instruction.copy`
- «Escribí un nombre (Antigravity, Qwen, Grok…). Al copiar se crea un cliente de solo lectura. Pegá la instrucción en tu asistente para que configure la conexión.» `assistants.instruction.help`
- «Instrucción copiada para {name}» `assistants.instruction.copied`
- «sin usar» `assistants.unused`

## Lotes

### Varios (1/4)

- «1 omitido» `batch.planSkipped.one`
- «1 ya procesado» `batch.planDone.one`
- «Abrir procesamiento por lote» `batch.openBatchTab`
- «Acciones» `batch.colActions`
- «Agregar campo» `batch.schemaFieldAdd`
- «Alta» `batch.priorityHigh`
- «Analizando selección…» `batch.preparing`
- «Analizar selección» `batch.analyze`
- «Análisis listo: no hay nada que procesar.» `batch.planNothing`
- «Análisis listo: {parts}.» `batch.planReady`
- «Bibliografía: esperando resultados de tus otros dispositivos…» `batch.statusBibliographyWaitingSync`
- «Bibliografía: fichas {worksDone}/{worksTotal} · pasajes {passagesDone}/{passagesTotal}» `batch.statusBibliography`
- «Bibliografía: fichas {worksDone}/{worksTotal} · pasajes {passagesDone}/{passagesTotal} · OCR {pagesDone}/{pagesTotal}» `batch.statusBibliographyOcr`
- «Bibliografía: fichas {worksDone}/{worksTotal} · pasajes {passagesDone}/{passagesTotal} · OCR {pagesDone}/{pagesTotal} · {blocked} en espera» `batch.statusBibliographyWaitingOcr`
- «Bibliografía: fichas {worksDone}/{worksTotal} · pasajes {passagesDone}/{passagesTotal} · {blocked} en espera» `batch.statusBibliographyWaiting`
- «Bibliografía: {blocked} en espera: {reason}» `batch.statusBibliographyBlocked`
- «Bloqueados» `batch.stateBlocked`
- «Borrar el esquema y sus resultados» `batch.schemaDelete`
- «Borrar este flujo» `batch.flowDelete`
- «Busca personas, lugares y organizaciones en cada documento con texto. Saltea los que ya tienen entidades.» `batch.opNerHint`
- «Campo» `batch.schemaFieldName`
- «Cancelado» `batch.attemptCancelled, batch.batchStateCancelled`
- «Cancelados» `batch.stateCancelled`
- «Cancelando» `batch.batchStateCancelling`
- «Cancelando…» `batch.cancelling`
- «Cancelar» `batch.cancel, batch.schemaCancel`
- «Cancelar lote» `batch.confirmCancel, batch.confirmCancelTitle`
- «Cargar más» `batch.loadMore`
- «Ciclo de reintento {n}» `batch.cycle`
- «Colecciones» `batch.collections`
- «Compartida con {ids}» `batch.sharedWith`
- «Completado» `batch.attemptSucceeded, batch.batchStateCompleted`
- «Completado con errores» `batch.batchStateCompletedWithErrors`
- «Completados» `batch.stateSucceeded`
- «Con el modo OCRH en local, el lote usa OCRL (layout + PaddleOCR, segundos por página). Marcado, usa layout + OCRH local (PaddleOCR-VL o GLM-OCR): más preciso y mucho más lento.» `batch.opOcrLocalHighHint`
- «Corrección OCR» `batch.opOcrCorrection`
- «Corrige el texto OCR de cada página con el modelo de Prompts; guarda el original.» `batch.opOcrCorrectionHint`
- «Código de error» `batch.colErrorCode`

### Varios (2/4)

- «Definí los campos que querés sacar de cada documento (por ejemplo barco, carga, destino) y el lote arma una tabla con una fila por caso.» `batch.schemaHint`
- «Descargar CSV» `batch.schemaDownload`
- «Descartar borrador» `batch.discard`
- «Detalle del lote» `batch.detail`
- «Documento» `batch.colDocument, batch.schemaDocument`
- «Editar» `batch.schemaEdit`
- «Ejecutá OCR y embeddings sobre colecciones enteras en segundo plano, con reanudación ante interrupciones.» `batch.subtitle`
- «El modelo general» `batch.schemaModelPlaceholder`
- «Elegir a mano» `batch.flowNone`
- «Emb.» `batch.colEmbeddingsShort`
- «Embeddings» `batch.opEmbeddings`
- «En curso» `batch.attemptRunning`
- «En ejecución» `batch.batchStateRunning, batch.stateRunning`
- «En esta versión cada uno es una consulta paga a OpenRouter.» `batch.extractionPaidNotice`
- «Entidades» `batch.opNer`
- «Errores» `batch.colErrors, batch.summaryErrors`
- «Escribe un resumen por página.» `batch.opSummaryHint`
- «Esquema propio» `batch.schema`
- «Estado» `batch.colState, batch.filterState`
- «Este equipo no tiene una GPU que pueda correr GLM-OCR (NVIDIA, o AMD, Intel o Apple con 4 GB): OCRH local corre en el procesador y tarda unos 15 minutos por página.» `batch.ocrhCpuNotice`
- «Extrae relaciones sujeto-verbo-objeto de cada documento con texto. Saltea los que ya tienen tripletes.» `batch.opTriplesHint`
- «Fecha» `batch.colDate`
- «Flujo guardado» `batch.flow`
- «Fondo» `batch.priorityBackground`
- «Genera embeddings cuando el OCR produzca texto. Si lo destildás, esas páginas no se vectorizan solas y quedan fuera de la búsqueda semántica hasta que las vectorices.» `batch.opEmbeddingsHint`
- «Guardar» `batch.flowSave`
- «Guardar como flujo» `batch.flowSaveAs`
- «Guardar esquema» `batch.schemaSave`
- «Historial» `batch.history`
- «Historial de lotes» `batch.historyTable`
- «Incluida» `batch.opIncluded`
- «Incluir texto nativo» `batch.opOcrIncludeNative`
- «Iniciar lote» `batch.start`
- «Intentos» `batch.colAttempts`
- «Interactiva» `batch.priorityInteractive`
- «Interrumpido» `batch.attemptInterrupted, batch.batchStateInterrupted`
- «Interrumpidos» `batch.stateInterrupted`
- «La base todavía está migrando. Reintentá en un momento.» `batch.schemaNotReady`
- «Listo» `batch.batchStateReady`
- «Lote» `batch.colBatch`
- «Lotes activos y recuperados» `batch.active`
- «Mantener pausados» `batch.keepPaused`
- «Mensaje» `batch.colMessage`
- «Modelo» `batch.schemaModel`
- «Ninguno» `batch.schemaNone`
- «No incluida» `batch.opNotIncluded`
- «Nombre del esquema» `batch.schemaName`
- «Nombre del flujo» `batch.flowName`
- «Nuevo esquema» `batch.schemaNew`
- «Nuevo lote» `batch.newBatch`

### Varios (3/4)

- «OCR» `batch.colOcrShort, batch.opOcr`
- «OCRH local» `batch.opOcrLocalHigh`
- «Omitidos» `batch.stateSkipped`
- «Opcional. Un modelo de OpenRouter solo para este esquema, por ejemplo anthropic/claude-sonnet-4.5.» `batch.schemaModelHint`
- «OpenRouter está desactivada en Configuración → APIs remotas («Usar esta API»). Activala para pedir embeddings, entidades, tripletes, correcciones o esquemas.» `batch.openrouterDisabled`
- «Operaciones» `batch.operations`
- «Operación» `batch.colOperation`
- «Paginación de tareas» `batch.pagination`
- «Pasa por OCR también las páginas que solo tienen el texto nativo del PDF y reemplaza ese texto. Las páginas que ya pasaron por OCR se saltean igual.» `batch.opOcrIncludeNativeHint`
- «Pausado» `batch.batchStatePaused, batch.paused`
- «Pausando» `batch.batchStatePausing`
- «Pausando…» `batch.pausing`
- «Pausar» `batch.pause`
- «Pendientes» `batch.statePending, batch.summaryPending`
- «Preparando» `batch.batchStatePreparing`
- «Primera página» `batch.firstPage`
- «Primero cargá tu API key de OpenRouter, en Ajustes → OpenRouter. Sin ella no se pueden pedir embeddings, entidades, tripletes ni esquemas.» `batch.needsOpenrouterKey`
- «Prioridad» `batch.priority`
- «Procesamiento por lote» `batch.title`
- «Procesando {count}» `batch.statusRunning`
- «Progreso» `batch.colProgress`
- «Próximo reintento: {when}» `batch.nextRetry`
- «Puede repetirse» `batch.schemaFieldRepeatable`
- «Página anterior» `batch.prevPage`
- «Página siguiente» `batch.nextPage`
- «Página {page}» `batch.goToPage`
- «Quitar el campo» `batch.schemaFieldRemove`
- «Qué es (ayuda al modelo)» `batch.schemaFieldDescription`
- «Reanudar» `batch.resume, batch.resumeSelected`
- «Reintentando…» `batch.retrying`
- «Reintentar» `batch.retry`
- «Resueltos» `batch.summarySettled`
- «Resúmenes» `batch.opSummary`
- «Se detendrá el procesamiento y se conservará el trabajo confirmado. La unidad en curso puede terminarse; no se iniciarán unidades nuevas. ¿Cancelar el lote?» `batch.confirmCancelMessage`
- «Se recuperaron {batches} lotes pendientes: {done} tareas completadas, {todo} por continuar, {stuck} interrumpidas.» `batch.recovered`
- «Seleccionar todas» `batch.selectAll`
- «Sin actividad» `batch.statusIdle`
- «Sin resultados para este filtro» `batch.noFilterResults`
- «Sin trabajo pendiente» `batch.noWork`
- «Tareas con total de páginas o fragmentos aún desconocido: {count}.» `batch.unknownProgressTotals`
- «Tareas del lote» `batch.tasksTable`
- «Todavía no hay resultados con este esquema: corré un lote con él.» `batch.schemaNoResults`
- «Todos» `batch.filterAll`
- «Total» `batch.colTotal, batch.summaryTotal`

### Varios (4/4)

- «Total remoto desconocido; todavía no hay cursor confirmado» `batch.bibliographyTotalUnknown`
- «Total remoto: {total}; todavía no hay cursor confirmado» `batch.bibliographyRemoteTotalOnly`
- «Tripletes» `batch.opTriples`
- «Ver detalle» `batch.viewDetail`
- «Ver intentos de {kind} · {asset}» `batch.expandTask`
- «Ver resultados» `batch.schemaResults`
- «{count} checkpoints» `batch.checkpointCount`
- «{count} documentos pasan por el esquema propio.» `batch.schemaCount`
- «{count} documentos pasan por la búsqueda de entidades.» `batch.nerCount`
- «{count} documentos pasan por la extracción de tripletes.» `batch.triplesCount`
- «{count} filas» `batch.schemaResultCount`
- «{count} intentos» `batch.attempts`
- «{count} omitidos» `batch.planSkipped.other`
- «{count} para procesar» `batch.planAdmitted`
- «{count} páginas reciben un resumen. Las que ya tienen uno se saltean.» `batch.summaryCount`
- «{count} páginas se corrigen con el modelo de Prompts. Las que ya están corregidas se saltean.» `batch.ocrCorrectionCount`
- «{count} requieren atención» `batch.statusAttention`
- «{count} resultados» `batch.resultCount`
- «{count} seleccionadas» `batch.selectedCount`
- «{count} ya procesados» `batch.planDone.other`
- «{done} de {total} páginas o fragmentos procesados» `batch.unitsOf`
- «{done} de {total} registros sincronizados» `batch.bibliographyProgress`
- «{done} de {total} resueltos» `batch.resolvedOf`
- «{done} registros sincronizados; total remoto desconocido» `batch.bibliographyProgressUnknown`
- «{from}–{to} de {total}» `batch.pageRange`
- «Última página» `batch.lastPage`

## Bibliografía

### reprocess

- «Adjuntos»
- «Buscando obras con texto dañado…»
- «Cancelar»
- «Cerrar»
- «Costo estimado»
- «El avance, la pausa y la cancelación están en la pestaña «{tab}» de Configuración.»
- «En proceso, no encolados: {busy} adjuntos.»
- «Encolados: {queued} adjuntos.»
- «GLM-OCR: USD 0,03 por millón de tokens · supuesto: ~3000 tokens por página.»
- «Leyendo {done} de {total} adjuntos»
- «Leyendo {done} de {total} adjuntos · {percent} % del actual»
- «No encolados: {notQueued} adjuntos.»
- «No hay obras con texto dañado.»
- «No se encolan»
- «Páginas»
- «Páginas corregidas sin OCR»
- «Páginas de OCR reutilizadas»
- «Páginas para GLM-OCR»
- «Reprocesar (≈ USD {usd})»
- «Reprocesar texto» `bibliography.reprocess.action, bibliography.reprocess.title`
- «Revisando {done} de {total} adjuntos»
- «archivo demasiado grande»
- «archivo no disponible»
- «en proceso»
- «no es un PDF»
- «no se pudo leer ({reason})»
- «sin adjunto en el catálogo»
- «sin costo de OCR»
- «≈ USD {usd} (estimado)»

### Varios

- «Abrir pasaje» `bibliography.passageOpen`
- «Buscar» `bibliography.searchAction`
- «Buscar desde la selección» `bibliography.searchFromSelection`
- «Buscar obras» `bibliography.searchTitle`
- «Búsqueda híbrida sobre perfiles verificados: léxico más vectores cuando hay una generación activa.» `bibliography.searchHint`
- «Citar pasaje» `bibliography.passageCite`
- «Consulta» `bibliography.searchLabel`
- «La búsqueda envía tu consulta al motor de embeddings cuando hay un espacio vectorial activo. Buscar desde la selección envía exactamente el texto seleccionado, solo cuando lo pedís.» `bibliography.searchConsent`
- «No hay texto seleccionado en el manuscrito.» `bibliography.searchSelectionEmpty`
- «No se buscaron pasajes: la Biblioteca todavía no tiene vectores para buscar por significado (el indexado no terminó).» `bibliography.passagesNotice.noEmbeddings`
- «No se buscaron pasajes: no hay ninguna biblioteca sincronizada en EntropIA.» `bibliography.passagesNotice.noLibrarySynced`
- «No se buscaron pasajes: no se pudo calcular el vector de la consulta (revisá el proveedor de embeddings).» `bibliography.passagesNotice.embeddingUnavailable`
- «Pasajes» `bibliography.passagesTitle`
- «Sin pasajes para esta consulta.» `bibliography.passagesEmpty`
- «Tema, autor u obra…» `bibliography.searchPlaceholder`

## Anotaciones

### Varios (1/2)

- «Abrí Zotero para sincronizar las anotaciones.» `bibliographyAnnotations.statusAccountUnknown`
- «Agregar un comentario…» `bibliographyAnnotations.commentPlaceholder`
- «Anotaciones» `bibliographyAnnotations.title`
- «Anotar la selección» `bibliographyAnnotations.popover`
- «Aplicar» `bibliographyAnnotations.conflictApply`
- «Campo {field}» `bibliographyAnnotations.fieldOther`
- «Cancelar» `bibliographyAnnotations.cancel`
- «Color» `bibliographyAnnotations.fieldColor`
- «Color {color}» `bibliographyAnnotations.color`
- «Comentario» `bibliographyAnnotations.fieldComment`
- «Comentario de la anotación» `bibliographyAnnotations.commentLabel`
- «Conservar EntropIA» `bibliographyAnnotations.conflictLocal`
- «Conservar la anotación» `bibliographyAnnotations.conflictKeep`
- «Dibujo» `bibliographyAnnotations.kindInk`
- «El PDF no está en este dispositivo.» `bibliographyAnnotations.pdfUnavailable`
- «Eliminar» `bibliographyAnnotations.confirmDelete`
- «Eliminar anotación» `bibliographyAnnotations.delete`
- «Eliminarla» `bibliographyAnnotations.conflictDiscard`
- «En conflicto» `bibliographyAnnotations.stateConflict`
- «En la papelera de Zotero ({count})» `bibliographyAnnotations.trashGroup`
- «EntropIA: {value}» `bibliographyAnnotations.conflictLocalValue`
- «Esperando a Zotero» `bibliographyAnnotations.stateWaitingParent`
- «Esperando que el adjunto llegue a Zotero desde {when}.» `bibliographyAnnotations.waitingSince`
- «Herramientas de anotación» `bibliographyAnnotations.toolbar`
- «La clave de Zotero es de una cuenta distinta de la abierta en Zotero.» `bibliographyAnnotations.statusOtherAccount`
- «La clave de Zotero no tiene permiso de escritura en la biblioteca personal.» `bibliographyAnnotations.statusNoWrite`
- «Las anotaciones están disponibles solo en la biblioteca personal.» `bibliographyAnnotations.statusNotPersonal`
- «Nota» `bibliographyAnnotations.kindNote, bibliographyAnnotations.toolNote`
- «Para sincronizar con Zotero, guardá en Ajustes una clave de la Web API con permiso de escritura.» `bibliographyAnnotations.statusNoKey`
- «Pendiente» `bibliographyAnnotations.statePending`
- «Posición» `bibliographyAnnotations.fieldPosition`
- «Página {page}» `bibliographyAnnotations.page`
- «Resaltado» `bibliographyAnnotations.kindHighlight`
- «Resaltar» `bibliographyAnnotations.toolHighlight`
- «Restaurar» `bibliographyAnnotations.restore`
- «Se eliminó en Zotero.» `bibliographyAnnotations.existenceDeleted`
- «Se modificó en Zotero después de que la eliminaste.» `bibliographyAnnotations.existenceModified`
- «Se movió a la papelera en Zotero.» `bibliographyAnnotations.existenceTrashed`
- «Seleccionar» `bibliographyAnnotations.toolSelect`

### Varios (2/2)

- «Sin conexión con Zotero. Los cambios quedan pendientes.» `bibliographyAnnotations.statusUnreachable`
- «Sincronizada» `bibliographyAnnotations.stateSynced`
- «Sincronizando…» `bibliographyAnnotations.syncing`
- «Sincronizar con Zotero» `bibliographyAnnotations.syncNow`
- «Su adjunto se eliminó en Zotero. Copiá el comentario si lo necesitás y descartala.» `bibliographyAnnotations.existenceAttachmentDeleted`
- «Subrayado» `bibliographyAnnotations.kindUnderline`
- «Subrayar» `bibliographyAnnotations.toolUnderline`
- «Texto» `bibliographyAnnotations.fieldText, bibliographyAnnotations.kindText, bibliographyAnnotations.toolText`
- «Todavía no hay anotaciones. Seleccioná texto o elegí una herramienta.» `bibliographyAnnotations.empty`
- «Usar Zotero» `bibliographyAnnotations.conflictZotero`
- «Zotero no reconoce la clave guardada.» `bibliographyAnnotations.statusInvalidKey`
- «Zotero pidió esperar. Los cambios se envían más tarde.» `bibliographyAnnotations.statusPaused`
- «Zotero tiene cambios de esta anotación que no aceptaste ({fields}). ¿La eliminás igual?» `bibliographyAnnotations.deleteConflictConfirm`
- «Zotero y EntropIA cambiaron esta anotación» `bibliographyAnnotations.conflictTitle`
- «Zotero: {value}» `bibliographyAnnotations.conflictZoteroValue`
- «{count} anotaciones en otro formato (solo en Zotero)» `bibliographyAnnotations.otherFormat`
- «¿Eliminar la anotación?» `bibliographyAnnotations.deleteConfirmTitle`
- «Área» `bibliographyAnnotations.kindImage, bibliographyAnnotations.toolImage`

## Obra

### Varios

- «Adjunto» `bibliographyWork.attachmentLabel`
- «Adjuntos» `bibliographyWork.attachmentsTitle`
- «Cargando la obra…» `bibliographyWork.loading`
- «El texto extraído aparecerá cuando esté disponible.» `bibliographyWork.textEmpty`
- «Elegir adjunto» `bibliographyWork.attachmentMenu`
- «Esta obra no tiene un original visible en la aplicación.» `bibliographyWork.originalEmpty`
- «La obra no tiene adjuntos catalogados.» `bibliographyWork.attachmentsEmpty`
- «Metadatos» `bibliographyWork.tabMetadata`
- «Original» `bibliographyWork.tabOriginal`
- «Preparando el original…» `bibliographyWork.opening`
- «Página {page}» `bibliographyWork.textPage`
- «Reintentar» `bibliographyWork.retry`
- «Se muestra el PDF original del adjunto.» `bibliographyWork.originalNote`
- «Se muestra el texto guardado de la captura HTML.» `bibliographyWork.snapshotNote`
- «Secciones de la obra» `bibliographyWork.tabs`
- «Texto» `bibliographyWork.tabText`
- «Texto extraído» `bibliographyWork.textTitle`
- «Ver en Original» `bibliographyWork.attachmentOpen`

## Biblioteca

### Varios

- «Abrir Escritura» `biblioteca.emptyAction`
- «Biblioteca» `biblioteca.eyebrow`
- «Buscar obras» `biblioteca.searchLabel`
- «Cargando más obras…» `biblioteca.loadingMore`
- «Cargando obras…» `biblioteca.loading`
- «Cargar más obras» `biblioteca.loadMore`
- «Elegir biblioteca» `biblioteca.libraryMenu`
- «Elegir orden» `biblioteca.sortMenu`
- «Limpiar búsqueda» `biblioteca.searchClear`
- «Obras de la biblioteca» `biblioteca.gridAria`
- «Obras de tus bibliotecas de Zotero, sincronizadas en EntropIA.» `biblioteca.subtitle`
- «Recientes» `biblioteca.sortRecent`
- «Reintentar» `biblioteca.retry`
- «Sin resultados para "{query}"» `biblioteca.emptySearch`
- «Sincronizá una biblioteca de Zotero desde Escritura › Zotero.» `biblioteca.emptyHint`
- «Todas las bibliotecas» `biblioteca.libraryAll`
- «Todavía no hay obras sincronizadas.» `biblioteca.empty`
- «Título» `biblioteca.sortTitle`
- «Título, autor o significado…» `biblioteca.searchPlaceholder`
- «s. f.» `biblioteca.noYear`
- «{count} obra» `biblioteca.worksCount.one`
- «{count} obras» `biblioteca.worksCount.other`

## Colección

### importSummary

- «Abrimos el último documento importado: {title}.»
- «Algunos archivos no se pudieron importar. Revisá el detalle antes de continuar.»
- «Algunos archivos ya estaban en esta colección y no se volvieron a importar.»
- «Archivo actual: {name}.»
- «Cerrar resumen»
- «Errores»
- «Estamos copiando archivos y creando documentos.»
- «Etapa: {stage}.»
- «Importados»
- «Importando archivos»
- «Omitidos»
- «Omitidos: {files}»
- «Progreso de la importación»
- «Resumen de importación»
- «Revisá la colección para continuar.»
- «Ya estaban importados en esta colección: {files}»
- «archivo procesado»
- «copiando archivo»
- «creando documento»
- «guardando documento»
- «inspeccionando PDF»
- «renderizando PDF»
- «{completed} de {total} archivos procesados.» `collection.importSummary.progressDescription, collection.importSummary.progressMilestone`

### ocrExport

- «(texto nativo del PDF)»
- «CSV (.csv)»
- «Cerrar aviso»
- «Descargar el texto de la colección»
- «Markdown (.md)»
- «Ningún documento de esta colección tiene texto extraído.»
- «PDF (.pdf)»
- «Texto (.txt)»
- «Texto de la colección guardado en {path}»
- «Word (.docx)»
- «[Imagen, página {page}]»
- «archivo»
- «documento»
- «nativo»
- «ocr»
- «origen»
- «texto»

### pipelineCount

- «{count} Embed»
- «{count} NER»
- «{count} OCR»
- «{count} STT»
- «{count} Triplets»
- «{count} audio»
- «{count} audios»
- «{count} documento»
- «{count} documentos»
- «{count} imagen»
- «{count} imágenes»
- «{count} página»
- «{count} páginas»

### Varios

- «Aproximado: {words}» `collection.foundAs`
- «Buscar documentos» `collection.searchAria`
- «Buscar documentos...» `collection.searchPlaceholder`
- «Cargando documentos...» `collection.loading`
- «Cargando más documentos...» `collection.loadingMore`
- «Cargar más documentos» `collection.loadMore`
- «Colección activa» `collection.active`
- «Documentos» `collection.documentsFallback`
- «Documentos de la colección» `collection.gridAria`
- «Eliminar documento» `collection.deleteItemAria, collection.deleteItemTitle`
- «Eliminar página» `collection.deleteAssetAria, collection.deleteAssetTitle`
- «Esta página está citada en {count} documento(s) de Escritura: {documents}. Las citas no se borran — conservan el fragmento y los metadatos que registraste — pero dejan de poder abrir la fuente.» `collection.deleteAssetCited`
- «Exportando...» `collection.exporting`
- «Exportar JSON» `collection.export`
- «Importando...» `collection.importing`
- «Importar desde Internet Archive» `collection.importFromInternetArchive`
- «Importar documento» `collection.import`
- «Importá, explorá y mantené ordenados los documentos de esta colección.» `collection.subtitle`
- «Limpiar búsqueda» `collection.searchClear`
- «No encontramos documentos para esa búsqueda. Probá ajustando el texto o limpiando el filtro.» `collection.emptySearch`
- «No hay más documentos» `collection.endOfCollection`
- «Reintentar» `collection.retryPage`
- «Soltá archivos acá para importarlos a esta colección.» `collection.dropHint`
- «Todavía no hay documentos en esta colección. Importá archivos para empezar a trabajar.» `collection.empty`
- «archivo desconocido» `collection.unknownFile`
- «{count} importados ({details})» `collection.importNotice.summary`
- «{count} no soportados omitidos: {files}» `collection.importNotice.rejected`
- «¿Seguro que querés eliminar el documento {name}? Se borrarán todas sus páginas, textos extraídos y datos de procesamiento. Esta acción no se puede deshacer.» `collection.deleteItemMessage`
- «¿Seguro que querés eliminar {name}? También se borrarán el texto OCR asociado y los jobs de procesamiento. Esta acción no se puede deshacer.» `collection.deleteAssetMessage`

## Análisis

### Varios (1/2)

- «Agregar todas sus relaciones a la red» `collectionAnalysis.graphExpandNode`
- «Ampliar el panel al ancho completo» `collectionAnalysis.graphPanelWide`
- «Analizando el texto de la colección...» `collectionAnalysis.loading`
- «Análisis textual» `collectionAnalysis.title`
- «Armando el grafo…» `collectionAnalysis.graphLoading`
- «Cerrar el detalle» `collectionAnalysis.graphClose`
- «Colección activa» `collectionAnalysis.eyebrow`
- «Descargar gráfico de barras» `collectionAnalysis.downloadBars`
- «Descargar nube de palabras» `collectionAnalysis.downloadCloud`
- «En la red ({count})» `collectionAnalysis.graphRelationsShown`
- «Entre {min} y {max}» `collectionAnalysis.termCountHint`
- «Guardar la red para Gephi (.gexf)» `collectionAnalysis.graphExport`
- «Las stopwords personalizadas excluyeron todas las palabras. Ajustalas en Parámetros.» `collectionAnalysis.emptyFiltered`
- «Mostrar análisis textual» `collectionAnalysis.toggleOpen`
- «No dibujadas en la red ({count})» `collectionAnalysis.graphRelationsHidden`
- «No hay texto extraído en esta colección. Ejecutá OCR o transcripción en los documentos.» `collectionAnalysis.empty`
- «Ocultar análisis textual» `collectionAnalysis.toggleClose`
- «Parámetros» `collectionAnalysis.settingsTab`
- «Quedan fuera por el máximo de entidades o los filtros de Parámetros. Se pueden abrir en el texto igual.» `collectionAnalysis.graphRelationsHiddenHint`
- «Redimensionar panel de análisis» `collectionAnalysis.resizeAria`
- «Reintentar» `collectionAnalysis.retry`
- «Relaciones» `collectionAnalysis.graphRelations`
- «Relaciones 3D» `collectionAnalysis.graphTab`
- «Se ocultan {count} nombres por el máximo de {max} entidades visibles (Parámetros → Relaciones 3D).» `collectionAnalysis.graphHiddenByCaps`
- «Se ocultan {count} nombres por los umbrales mínimos (Parámetros → Relaciones 3D).» `collectionAnalysis.graphHiddenByThresholds`
- «Separá las palabras con comas o saltos de línea. Se excluyen del análisis de esta colección.» `collectionAnalysis.stopwordsHelp`
- «Stopwords personalizadas» `collectionAnalysis.stopwordsLabel`
- «Todavía no hay relaciones. Se extraen en Configuración → Lotes, con el paso de relaciones.» `collectionAnalysis.graphEmpty`
- «Top 20 palabras» `collectionAnalysis.barsTitle`
- «Top {count} palabras» `collectionAnalysis.cloudTitle`
- «Términos en la nube» `collectionAnalysis.termCountLabel`
- «Ver dónde aparece en el texto (documentos: {count})» `collectionAnalysis.graphOpenEntity`
- «Ver el pasaje con esta relación resaltada» `collectionAnalysis.graphOpenRelation`
- «Ver esta relación en el texto (documentos: {count})» `collectionAnalysis.graphOpenLink`

### Varios (2/2)

- «Vistas de análisis» `collectionAnalysis.tabsAria`
- «Visualización» `collectionAnalysis.vizTab`
- «Visualización textual» `collectionAnalysis.textSettingsTitle`
- «Volver a la red filtrada» `collectionAnalysis.graphCollapseNode`
- «Volver el panel a la mitad» `collectionAnalysis.graphPanelNarrow`
- «palabra1, palabra2, ...» `collectionAnalysis.stopwordsPlaceholder`
- «{label} · +{count} relaciones sin dibujar» `collectionAnalysis.graphNodeHiddenLabel`
- «{nodes} nombres y {links} relaciones. Arrastrá para girar, rueda para acercar; pasá el mouse por un punto o una línea para leerla; tocá un punto o una línea para ver dónde aparece en el texto.» `collectionAnalysis.graphSummary`
- «{words} palabras distintas · {tokens} tokens» `collectionAnalysis.meta`
- «×{count}» `collectionAnalysis.graphRelationCount`

## Colecciones

### Varios

- «+ Nueva colección» `collections.new`
- «Buscar colecciones» `collections.searchAria`
- «Buscar colecciones...» `collections.searchPlaceholder`
- «Cancelar» `collections.cancel`
- «Cargando colecciones...» `collections.loading`
- «Colecciones» `collections.title`
- «Contexto para la IA» `collections.aiContextLabel`
- «Corpus» `collections.eyebrow`
- «Crear colección» `collections.createAction`
- «Creá un espacio para agrupar documentos, notas y análisis relacionados.» `collections.createDescription`
- «Descripción (opcional)» `collections.descriptionPlaceholder`
- «Eliminar» `collections.deleteConfirm`
- «Eliminar colección» `collections.deleteAria, collections.deleteTitle`
- «Escribí un nombre para la colección.» `collections.nameRequired`
- «Gestioná tus espacios de trabajo y organizá el archivo por tema.» `collections.subtitle`
- «Guardar» `collections.save`
- «Limpiar búsqueda» `collections.searchClear`
- «Lo leen el chat, la investigación, la corrección de OCR y el agente de redacción: tema, período, convenciones y grafías de nombres.» `collections.aiContextHint`
- «No encontramos colecciones para esa búsqueda. Probá con otro nombre o limpiá el filtro.» `collections.emptySearch`
- «Nombre» `collections.editNamePlaceholder`
- «Nombre de la colección» `collections.namePlaceholder`
- «Nueva colección» `collections.createTitle`
- «Todavía no hay colecciones. Creá una para empezar a ordenar el material.» `collections.empty`
- «{count} colecciones» `collections.visibleCount.other`
- «{count} colección» `collections.visibleCount.one`
- «{count} documento» `collections.cardDocumentCount.one`
- «{count} documentos» `collections.cardDocumentCount.other`
- «¿Estás seguro que querés eliminar la colección "{name}"? Se eliminarán todos sus documentos y datos asociados.» `collections.deleteMessage`

## Base de datos

### Varios

- «Anterior» `dbBrowser.previousPage`
- «BLOB · {bytes} bytes» `dbBrowser.blobSummary`
- «Base de datos» `dbBrowser.eyebrow`
- «Cargando tablas disponibles...» `dbBrowser.loadingTables`
- «Cerrar» `dbBrowser.expandDialogClose`
- «Consulta DB» `dbBrowser.title`
- «Consultando filas...» `dbBrowser.loadingRows`
- «Copiar» `dbBrowser.copyCell`
- «Copiar valor completo de {column}» `dbBrowser.copyExpandedAria`
- «Copiar valor de {column}» `dbBrowser.copyCellAria`
- «Esta tabla no tiene filas para mostrar.» `dbBrowser.emptyRows`
- «Expandir» `dbBrowser.expandCell`
- «Expandir valor de {column}» `dbBrowser.expandCellAria`
- «Explorá tablas del SQLite de EntropIA en modo solo lectura, con búsqueda, orden y paginación.» `dbBrowser.subtitle`
- «Exportar CSV» `dbBrowser.exportCsv`
- «Exportar JSON» `dbBrowser.exportJson`
- «Filas por página» `dbBrowser.pageSizeLabel`
- «Filtrar por texto...» `dbBrowser.searchPlaceholder`
- «Filtro simple» `dbBrowser.searchLabel`
- «Limpiar búsqueda» `dbBrowser.searchClear`
- «Mostrando {from}-{to} de {total}» `dbBrowser.pageSummary`
- «No hay resultados para ese filtro.» `dbBrowser.emptyFiltered`
- «No hay tablas navegables disponibles.» `dbBrowser.emptyTables`
- «Paginación de la tabla» `dbBrowser.paginationGroup`
- «Primera» `dbBrowser.firstPage`
- «Página {page} de {totalPages}» `dbBrowser.pageStatus`
- «Representación binaria codificada en Base64» `dbBrowser.expandDialogEmbedding`
- «Representación binaria codificada en Base64 · {dimensions} dimensiones» `dbBrowser.expandDialogEmbeddingDimensions`
- «Siguiente» `dbBrowser.nextPage`
- «Tabla» `dbBrowser.tableLabel`
- «Tabla exportada.» `dbBrowser.exportSuccess`
- «Valor completo de {column}» `dbBrowser.expandDialogTitle`
- «Valor copiado.» `dbBrowser.copySuccess`
- «Vista completa del contenido textual.» `dbBrowser.expandDialogText`
- «Vista completa formateada como JSON legible.» `dbBrowser.expandDialogJson`
- «{count} columnas» `dbBrowser.columnsCount`
- «{count} filas» `dbBrowser.totalRows`
- «Última» `dbBrowser.lastPage`
- «—» `dbBrowser.noValue`

## deps

### models

- «Descargando {name}»
- «Descargar»
- «Falta»
- «Listo»
- «Lo que usan los motores locales. Los que necesitan GPU (NVIDIA, o AMD, Intel o Apple con memoria suficiente) se indican y no se descargan en equipos sin ella.»
- «Modelos y servidores locales»
- «No aplica en este equipo»
- «Reintentar»
- «Revisando los modelos instalados…»

### prepare

- «Descarga la base de IA, instala los paquetes de Python y baja todos los modelos que este equipo puede usar. Las descargas siguen en segundo plano; cada fila muestra su avance.»
- «Iniciando las descargas de modelos…»
- «Instalando los paquetes de Python…»
- «No se pueden instalar los paquetes de Python: la base de IA todavía no está lista.»
- «Preparando la base de IA…»
- «Preparar EntropIA Pro»
- «Prepará EntropIA Pro para trabajar en este equipo»

### reset

- «Borra el entorno de IA administrado y su configuración. Después hay que volver a preparar EntropIA Pro.»
- «Confirmación requerida»
- «Confirmar reseteo»
- «Confirmar reseteo del entorno»
- «Esta acción borra el entorno de IA administrado y las rutas de Python que usan el OCR, la transcripción y el NLP.»
- «Para confirmar, escribí»
- «Resetear entorno»

### runtime

- «Base de IA no disponible»
- «Capacidades afectadas: {list}»
- «EntropIA Pro la prepara sola apenas haya una fuente válida disponible.»
- «EntropIA Pro necesita descargarla y verificarla la primera vez. Conectate a internet y volvé a verificar. Mientras tanto, el OCR, la transcripción y el NLP locales quedan en pausa.»
- «Esta compilación trae una base de IA de prueba, no la publicada. Las funciones locales que la necesitan no van a funcionar.»
- «Esta versión de EntropIA Pro todavía no tiene su runtime publicado.»
- «Hay un entorno de desarrollo disponible para instalar los paquetes de Python en este equipo.»
- «La base de IA se descarga sola la primera vez. Hasta que esté lista, las funciones locales que la necesitan quedan en pausa.»
- «La lista firmada aún no está publicada, así que no se puede descargar ni verificar de forma segura. No es un problema de tu equipo; la app la toma sola apenas se publique.»
- «La lista firmada existe, pero no incluye la base de IA para esta versión y sistema. No es un problema de tu equipo: actualizá EntropIA Pro o volvé a verificar más tarde.»
- «No pudimos llegar a la fuente de la base de IA: parece que estás sin conexión.»
- «Reintentar conexión»
- «Reparar runtime»
- «Tampoco hay un entorno de desarrollo que la reemplace.»
- «Todavía no hay una fuente de descarga confiable para la base de IA.»
- «Volver a verificar»

### uv

- «Detectado: uv {version} en {path}»
- «Fallback dev disponible»
- «Gestión automática pausada hasta que la base de IA esté lista.»
- «La base de IA publicada no está lista ({summary}). El entorno de desarrollo no la reemplaza ni la verifica.»
- «No se pueden instalar paquetes automáticamente: faltan herramientas en este equipo.»
- «Paquetes de Python detectados; no hace falta hacer nada.»
- «Revisando uv…»
- «entorno virtual en {path}»
- «entorno virtual local en {path}»
- «runtime embebido · sin venv administrado»
- «sin entorno virtual»
- «sin entorno virtual local»

### Varios

- «Con la base de IA para GPU, PaddleOCR-VL lee las páginas en la placa de video, mucho más rápido. Se descargan {size}; la base actual sigue funcionando hasta que la nueva esté lista.» `deps.gpuRuntime.body`
- «Instalando {name}» `deps.installing`
- «Instalar» `deps.install`
- «Instalar todo» `deps.installAll`
- «Los paquetes de Python están instalados, pero el runtime de EntropIA Pro necesita atención antes de habilitar OCR, transcripción y NLP.» `deps.installedButRuntime`
- «Ocultar detalle» `deps.hideDetail`
- «Paquete que administra EntropIA Pro.» `deps.managedByPro`
- «Primero tiene que estar lista la base de IA (o un entorno de desarrollo en este equipo).» `deps.installAllNeedsRuntime`
- «Reintentar» `deps.retry`
- «Requerido» `deps.required`
- «Revisando dependencias…» `deps.checking`
- «Todas las dependencias están instaladas y listas para usar.» `deps.allInstalled`
- «Tu equipo tiene una GPU NVIDIA» `deps.gpuRuntime.title`
- «Usar la GPU» `deps.gpuRuntime.action`
- «Ver detalle del error» `deps.showDetail`
- «{pct}% instalado» `deps.installedPct`

## Alias

### dialog

- «Cancelar»
- «Ej.: Carmela Cuevas»
- «Elegí una para usarla como nombre canónico.»
- «Es la misma persona que…»
- «Guardar»
- «La mención no se modifica: solo se guarda el alias y todas las vistas lo leen sin reescribir nada.»
- «Mención original (alias)»
- «Nombre canónico»
- «Otras menciones de este documento»

### suggestions

- «Aceptando…»
- «Aceptar»
- «Aceptar la sugerencia {canonical}»
- «Buscando sugerencias…»
- «Descartar»
- «Descartar la sugerencia {canonical}»
- «Grupos que parecen la misma persona (mismo apellido, iniciales, tratamientos). Nada se une solo: cada grupo se acepta o se descarta.»
- «Nombre canónico»
- «Nombre canónico: {canonical}. Elegí otro del grupo»
- «Sin sugerencias por ahora: no hay grupos de menciones parecidas entre las personas del archivo.»
- «Sugerencias»
- «{count} menciones»

### Varios

- «Alias de entidades» `entityAliases.title`
- «Cada alias une una forma alternativa con su nombre canónico en todo el archivo.» `entityAliases.description`
- «Cancelar» `entityAliases.cancel`
- «Cargando alias…» `entityAliases.loading`
- «Editar el alias {alias}» `entityAliases.editAria`
- «Eliminar» `entityAliases.deleteConfirm`
- «Eliminar el alias {alias}» `entityAliases.deleteAria`
- «Guardar» `entityAliases.save`
- «Todavía no hay alias: cuando dos menciones nombran a la misma persona, unificalas con «Es la misma persona que…».» `entityAliases.empty`
- ««{alias}» dejará de resolverse como «{canonical}». Las menciones no cambian.» `entityAliases.deleteMessage`
- «¿Eliminar el alias?» `entityAliases.deleteTitle`

## Errores

### batch

- «Fallido»
- «Fallidos»
- «No se pudo cargar el procesamiento por lote: {error}»
- «No se pudo completar la acción: {error}»
- «Reintentar fallidos»

### collection

- «Esta colección ya no existe (se borró en este equipo o en otro sincronizado). Elegí o creá otra colección.»
- «Fallidos»
- «Formato no soportado: {files}»
- «No se encontró una página para eliminar en este documento.»
- «No se pudieron cargar los documentos»
- «No se pudieron importar los archivos»
- «No se pudieron importar los archivos soltados»
- «No se pudo eliminar el documento. No se eliminó ningún archivo: {message}»
- «No se pudo eliminar la página: {message}»
- «No se pudo exportar el texto de la colección»
- «No se pudo exportar la colección»
- «creando documento»
- «error: {message}»
- «importando {name}»
- «seleccionando archivos»

### entityAliases

- «El alias no puede estar vacío.»
- «El nombre canónico no puede estar vacío.»
- «No se pudieron cargar las sugerencias: {error}»
- «No se pudieron cargar los alias: {error}»
- «No se pudo guardar el alias.»
- ««{alias}» ya es el nombre correcto, así que no hace falta un alias. Para unificar otra forma de este nombre, abrí «Es la misma persona que…» desde esa otra mención y elegí «{alias}» como nombre canónico.»
- ««{alias}» ya se usa como nombre canónico de otra entrada: cada unión se decide por separado.»
- ««{canonical}» ya es un alias de otro nombre: elegí el nombre canónico final directamente.»

### help

- «No se pudo cargar el historial.»
- «No se pudo hacer»
- «No se pudo obtener el diagnóstico.»
- «No se pudo responder el pedido. Probá de nuevo.»
- «No se pudo responder. Probá de nuevo.»

### internetArchive

- «Con error»
- «Con error: {count}»
- «Demasiadas redirecciones al contactar Internet Archive.»
- «El documento está en otra colección.»
- «El documento importado no se encontró.»
- «El documento tiene demasiadas páginas para importarlo.»
- «Error de almacenamiento local.»
- «Error de red (HTTP {status}).»
- «Error de red al contactar Internet Archive.»
- «Error del archivo de datos.»
- «Fallida»
- «Falta el archivo temporal de la descarga.»
- «Internet Archive bloqueó la solicitud.»
- «La búsqueda no es válida. Revisa los filtros.»
- «La colección de destino no existe.»
- «La importación no está en un estado válido para esta acción.»
- «La importación no tiene colección de destino.»
- «La importación quedó incompleta.»
- «La procedencia no coincide.»
- «La respuesta de Internet Archive fue demasiado grande.»
- «No hay ningún archivo disponible para importar.»
- «No se encontró en Internet Archive.»
- «No se pudieron leer las colecciones.»
- «No se pudieron leer los archivos de {count} registros. Probá de nuevo o abrilos de a uno.»
- «No se pudo leer la respuesta de Internet Archive.»
- «Ocurrió un error inesperado.»
- «Quedó un resto parcial sin limpiar.»
- «Solicitud no válida.»
- «Verificación fallida del archivo.»
- «Verificación fallida.»
- «Ya existe.»
- «Ya hay una importación en curso.»

### investigation

- «La investigación se detuvo por un error»
- «No se pudieron cargar las rutas de la fuente.»
- «No se pudieron enviar las respuestas.»
- «No se pudo aplicar la decisión sobre el plan.»
- «No se pudo cargar el detalle.»
- «No se pudo cargar la vista previa del documento. El botón de abajo igual lo abre.»
- «No se pudo descargar el informe: {message}»
- «No se pudo ejecutar la acción.»
- «No se pudo modificar la sección.»
- «Reintentar»
- «Último error del backend»

### item

- «Error de embeddings: {error}»
- «Falló la extracción: {error}»
- «Falló la transcripción: {error}»
- «No se pudieron cargar las anotaciones de esta página.»
- «No se pudieron guardar las anotaciones. Los cambios quedan locales hasta reintentar.»
- «No se pudieron guardar los metadatos»
- «No se pudieron leer las versiones anteriores: {error}»
- «No se pudieron listar las versiones: {error}»
- «No se pudo abrir el link de la nota»
- «No se pudo actualizar la nota»
- «No se pudo agregar la entidad»
- «No se pudo aplicar la edición: {message}»
- «No se pudo cargar el PDF»
- «No se pudo cargar el documento»
- «No se pudo cargar el historial: {error}»
- «No se pudo cargar el layout: {error}»
- «No se pudo copiar desde el inspector.»
- «No se pudo copiar el texto extraído»
- «No se pudo corregir el texto con OCR.»
- «No se pudo duplicar la página: {message}»
- «No se pudo ejecutar la búsqueda full-text.»
- «No se pudo eliminar la entidad»
- «No se pudo eliminar la nota»
- «No se pudo exportar el historial: {error}»
- «No se pudo generar el resumen.»
- «No se pudo generar la exportación»
- «No se pudo guardar la entidad»
- «No se pudo guardar la nota»
- «No se pudo guardar la ubicación.»
- «No se pudo reconocer la caja: {error}»
- «No se pudo renderizar la página»
- «No se pudo restaurar la versión original del OCR.»
- «No se pudo restaurar la versión: {error}»
- «No se pudo transcribir el audio.»
- «error»

### navegador (1/2)

- «Abre una página antes de capturar.»
- «Con error»
- «El PDF es demasiado grande para copiarlo a Zotero.»
- «El PDF guardado ya no coincide con el que se verificó al guardarlo, así que no se abre.»
- «El PDF guardado ya no coincide con lo que se verificó al guardarlo.»
- «El PDF guardado ya no está en el disco.»
- «El archivo del PDF ya no está en este equipo. Vuelve a descargarlo desde la página de origen.»
- «El archivo ya no está en la carpeta temporal. Vuelve a descargarlo.»
- «El contenido es demasiado grande para capturarlo.»
- «El contenido no coincide con su huella (SHA-256), así que no se guardó.»
- «Error del archivo de datos ({message}).»
- «Esta captura no conserva texto.»
- «Esta captura no es un PDF.» `navegador.pdf.error.not_a_pdf, navegador.zotero.error.not_a_pdf`
- «Esta captura ya no está disponible. Vuelve a capturar la página.»
- «Esta captura ya no existe.»
- «Esta fuente o captura ya no existe.»
- «Esta página es un PDF en el visor integrado: no tiene texto para leer.»
- «Este PDF ya está en tus fuentes, así que no se guardó otra copia.»
- «Este PDF ya no está disponible. Vuelve a descargarlo.»
- «Falló»
- «Hay dos bibliotecas con ese nombre en Zotero; renombra una para poder elegir.»
- «La biblioteca elegida no es válida.»
- «La biblioteca elegida no está disponible para escribir en Zotero (no existe, o es de solo lectura).»
- «La captura no es válida y no se guardó.»
- «La dirección de la página no está permitida.»
- «La página no permitió leer su contenido.»
- «La página no respondió a tiempo.»
- «La respuesta de la página no es válida.»
- «No hay texto seleccionado en la página.»
- «No se pudieron completar los campos por la Web API; el elemento quedó enlazado sin cambios.»
- «No se pudieron leer las colecciones: {message}»
- «No se pudieron leer las fuentes guardadas: {message}»
- «No se pudo abrir Zotero. Ábrelo a mano.»
- «No se pudo abrir el PDF: identificador no válido.»
- «No se pudo abrir el PDF: {message}»
- «No se pudo abrir en Zotero.»
- «No se pudo abrir la página: {message}»
- «No se pudo adjuntar el PDF por la Web API. El elemento quedó enlazado.»
- «No se pudo capturar: {message}»
- «No se pudo copiar la dirección.»
- «No se pudo copiar: el archivo del PDF ya no está en este equipo.»
- «No se pudo copiar: error del archivo de datos ({message}).»
- «No se pudo copiar: esta captura no conserva texto.»
- «No se pudo copiar: esta captura no es un PDF.»
- «No se pudo copiar: esta captura ya no existe.»
- «No se pudo copiar: identificador no válido.»
- «No se pudo copiar: lo guardado ya no coincide con lo que se verificó al guardarlo.»

### navegador (2/2)

- «No se pudo copiar: no se creó ningún documento. Prueba de nuevo.»
- «No se pudo copiar: {message}» `navegador.copy.error.import_failed, navegador.copy.error.unknown, navegador.zotero.done.failed`
- «No se pudo eliminar la fuente y no se borró nada: {message}»
- «No se pudo escribir el archivo: {message}»
- «No se pudo guardar: {message}»
- «No se pudo leer el archivo de datos: {message}»
- «No se pudo leer el archivo descargado.»
- «No se pudo leer la fuente: {message}»
- «No se pudo registrar en el archivo de datos, y no quedó nada guardado: {message}»
- «No se pudo usar esa carpeta: {message}»
- «Zotero aceptó el elemento pero aún no lo devuelve. Reintenta: no se duplicará.»
- «Zotero rechazó la escritura: {message}»
- «Zotero tiene desactivada la API local. Actívala en Ajustes de Zotero > Avanzado.»
- «{message}»

### ragChat

- «La búsqueda en la Biblioteca falló; la respuesta no la incluye.»
- «No se pudieron buscar las conversaciones.»
- «No se pudo abrir el original: {reason}»
- «No se pudo consultar el estado de las bibliotecas de Zotero.»
- «No se pudo copiar la respuesta.»
- «No se pudo descargar la conversación.»
- «No se pudo guardar el nombre de la conversación.»
- «No se pudo leer el pasaje: {reason}»

### settings

- «Error en la descarga»
- «No se pudieron leer los parámetros del motor: {error}»
- «No se pudo aplicar: {error}»
- «No se pudo cargar la configuración guardada: {error}»
- «No se pudo guardar: {error}»
- «No se pudo leer el ajuste: {error}»
- «No se pudo revisar: {error}»

### sync

- «Actualizá la app: el servidor pide un esquema más nuevo.»
- «Almacenamiento del servidor lleno.»
- «Credenciales inválidas o sesión revocada.»
- «El perfil de desarrollo solo puede sincronizar con el servidor local indicado en ENTROPIA_DEV_SYNC_SERVER.»
- «El registro está cerrado en este servidor.»
- «Error de sincronización»
- «La sincronización está desactivada en el perfil de desarrollo aislado.»
- «La sincronización necesita el llavero del sistema y no hay ninguno disponible. En Linux, instalá gnome-keyring o KWallet y creá un llavero predeterminado (por ejemplo, con la app «Contraseñas y claves»); después volvé a iniciar sesión.»
- «No se pudieron cargar las notificaciones.»
- «No se pudieron cargar los planes.»
- «No se pudo completar la operación de sincronización.»
- «Revisá el reloj del dispositivo.»
- «Tu cuenta está suspendida. Contactá al administrador para reactivarla.»
- «Tu suscripción venció. Podés seguir descargando, pero no vas a poder subir cambios hasta renovarla.»
- «Ya existe una cuenta con ese email.»
- «Ya tenés una solicitud de cambio de plan en revisión.»

### writing

- «Error de guardado»
- «Falló la búsqueda por significado; se muestran solo las coincidencias de texto.»
- «La sincronización falló: {detail}»
- «No se pudo abrir en Zotero. Probá de nuevo.»
- «No se pudo comprobar la biblioteca. Probá de nuevo.»
- «No se pudo exportar el documento: {message}. No se guardó ningún archivo.»
- «No se pudo guardar: {message}. Lo que escribiste sigue acá.»
- «No se pudo leer la ficha. Probá de nuevo.»
- «No se pudo leer la papelera: {error}»
- «No se pudo obtener una propuesta: {message}»
- «No se pudo quitar el autor de esta cita: el estilo no lo renderiza de una forma que permita hacerlo. La cita queda completa.»
- «No se pudo renderizar: {message}»
- «No se pudo restaurar: {error}»
- «No se pudo solicitar la sincronización: {detail}»
- «Último error: {error}»

### Varios

- «Cerrar error» `deps.closeError`
- «Error» `bibliographyAnnotations.stateError`
- «Error al actualizar la colección» `collections.error.update`
- «Error al registrar listeners: {error}» `deps.listenersFailed`
- «Fallado» `research.status.failed`
- «La búsqueda de pasajes falló.» `bibliography.passagesNotice.failed`
- «No se pudieron cambiar los modos: {error}» `home.localModes.failed`
- «No se pudieron cargar las colecciones» `collections.error.load`
- «No se pudieron cargar las obras.» `biblioteca.error`
- «No se pudieron cargar los documentos de ejemplo: {error}» `home.firstRun.sampleFailed`
- «No se pudieron cargar los trabajos.» `research.loadError`
- «No se pudieron reanudar los embeddings: {error}» `home.corpus.resumeEmbeddingsFailed`
- «No se pudieron revisar las dependencias: {error}» `deps.checkFailed`
- «No se pudo abrir Microsoft Store.» `storeUpdate.openError`
- «No se pudo analizar el texto de la colección.» `collectionAnalysis.error`
- «No se pudo armar el grafo de relaciones.» `collectionAnalysis.graphError`
- «No se pudo borrar la investigación.» `research.deleteError`
- «No se pudo cargar el explorador» `explorer.loadError`
- «No se pudo cargar la obra.» `bibliographyWork.error`
- «No se pudo completar la búsqueda. Probá de nuevo.» `topbar.searchError`
- «No se pudo completar la operación: {reason}» `bibliography.reprocess.error`
- «No se pudo copiar el valor.» `dbBrowser.copyError`
- «No se pudo copiar la instrucción para {name}: {error}. El cliente sigue en la lista; podés revocarlo o copiar sus comandos.» `assistants.instruction.failed`
- «No se pudo crear el documento.» `home.actions.newDocumentError`
- «No se pudo crear la colección» `collections.error.create`
- «No se pudo exportar la tabla.» `dbBrowser.exportError`
- «No se pudo guardar el archivo de la red.» `collectionAnalysis.graphExportError`
- «No se pudo guardar la anotación: {error}» `bibliographyAnnotations.saveFailed`
- «No se pudo inicializar la base de datos» `app.initError`
- «No se pudo iniciar la investigación.» `research.startError`
- «no se pudo leer el archivo» `bibliography.reprocess.unreadableReadFailed`
- «{assistant}: salida {exitCode}; {detail}» `assistants.install.command_failed`

## Explorador

### Varios

- «Abrir explorador de documentos» `explorer.expand`
- «Cargando explorador...» `explorer.loading`
- «Cerrar explorador de documentos» `explorer.collapse`
- «Colecciones» `explorer.collections`
- «Documentos» `explorer.documents`
- «Explorador» `explorer.title`
- «Explorador de documentos» `explorer.aria`
- «Navegación» `explorer.eyebrow`
- «No hay colecciones disponibles.» `explorer.emptyCollections`
- «No hay documentos en esta colección.» `explorer.emptyDocuments`
- «No hay páginas en este documento.» `explorer.emptyAssets`
- «Páginas» `explorer.assets`
- «Redimensionar explorador» `explorer.resize`

## Grafo

### Varios

- «Algoritmo de disposición» `graphSettings.layout`
- «Anillos desde los sujetos que inician relaciones hacia los objetos que las reciben. Para ver quién origina y quién recibe.» `graphSettings.layoutHint.radial`
- «Cambian solo lo que se ve en Relaciones 3D de esta colección: las entidades y relaciones guardadas no se modifican.» `graphSettings.hint`
- «Comunidades» `graphSettings.layout.communities`
- «Cuantas más relaciones tiene una entidad, más repele (estilo ForceAtlas2): los centros se separan y sus vecinos se agrupan alrededor. Para ver grupos.» `graphSettings.layoutHint.communities`
- «Cómo se acomoda y se mueve la red. Valores más altos de repulsión separan más los nodos.» `graphSettings.sectionExploreHint`
- «Distancia entre nodos conectados» `graphSettings.linkDistance`
- «Exploración» `graphSettings.sectionExplore`
- «Fuerza de repulsión» `graphSettings.repulsion`
- «Fuerzas» `graphSettings.layout.force`
- «Incluir nodos aislados» `graphSettings.includeIsolated`
- «Los umbrales deciden qué entidades entran a la red; cada una entra con todas sus relaciones y las entidades del otro extremo, aunque no alcancen el umbral. Cuando hay que recortar, entran primero las más conectadas, siempre con sus relaciones completas.» `graphSettings.sectionShownHint`
- «Los vínculos atraen y todos los nodos se repelen por igual (estilo Fruchterman-Reingold). La disposición general.» `graphSettings.layoutHint.force`
- «Mostrar el vínculo al pasar el mouse» `graphSettings.showLinkLabels`
- «Mostrar flechas» `graphSettings.showArrows`
- «Máximo de entidades visibles» `graphSettings.maxEntities`
- «Máximo de relaciones visibles» `graphSettings.maxRelations`
- «Mínimo de apariciones como objeto» `graphSettings.minObjectFrequency`
- «Mínimo de apariciones como sujeto» `graphSettings.minSubjectFrequency`
- «Mínimo de conexiones por entidad» `graphSettings.minDegree`
- «Mínimo de repeticiones por relación» `graphSettings.minRelationWeight`
- «Opacidad de las relaciones» `graphSettings.linkOpacity`
- «Qué se muestra» `graphSettings.sectionShown`
- «Radial jerárquico» `graphSettings.layout.radial`
- «Relaciones 3D» `graphSettings.title`
- «Representación» `graphSettings.sectionLook`
- «Restaurar predeterminados» `graphSettings.restoreDefaults`
- «Rotación automática» `graphSettings.autoRotate`
- «Se aplican sobre la red visible, sin volver a armarla.» `graphSettings.sectionLookHint`
- «Tamaño de los nodos» `graphSettings.nodeSizeScale`
- «Tiempo de acomodamiento (segundos)» `graphSettings.cooldownSeconds`

## Ayuda

### Varios (1/2)

- «Abrir Configuración» `help.openSettings`
- «Aprobado» `help.approvalOutcomeApproved`
- «Aprobar» `help.approve`
- «Aprobaste: {summary}» `help.approvalApproved`
- «Ayuda» `help.title`
- «Borrar» `help.confirmDelete`
- «Borrar historial» `help.clearHistory`
- «Buscando en el manual…» `help.thinking`
- «Cancelar» `help.cancel`
- «Cargando el diagnóstico…» `help.diagnosticsLoading`
- «Cargando el historial…» `help.historyLoading`
- «Cargá tu clave en Configuración › APIs remotas › OpenRouter y asegurate de que tu cuenta tenga crédito disponible.» `help.noKeyBody`
- «Cerrar ayuda» `help.closePanel`
- «Comprobando tu clave de OpenRouter…» `help.statusLoading`
- «De tus documentos» `help.documentsLabel`
- «Del manual» `help.sourceManual, help.sourcesLabel`
- «Del mapa de pantallas» `help.sourceScreenMap`
- «Detalle de fuentes y pasos» `help.metaDetails`
- «Detener» `help.stop`
- «Diagnóstico» `help.includeDiagnostics`
- «El modelo no pudo usar herramientas en este turno; respondió sin usar herramientas.» `help.toolsUnavailable`
- «Eliminar conversación» `help.deleteConversation`
- «Enviar» `help.send`
- «Escribí tu pregunta…» `help.placeholder`
- «Falló una transcripción, ¿qué hago?» `help.suggestion3`
- «Falta el modelo de texto local (Gemma): descargalo en Configuración › Modelos locales.» `help.engineLocalMissing`
- «Fragmentos de tus documentos viajarán a OpenRouter solo por esta pregunta.» `help.documentsHint`
- «Fuentes y pasos ({count})» `help.metaToggle`
- «Gemma local no puede leer tus documentos. Marcá «OpenRouter» para esta pregunta.» `help.documentsNeedRemote`
- «Historial» `help.history`
- «Historial ({count})» `help.historyCount`
- «La ayuda pide aprobación» `help.approvalTitle`
- «La ayuda usa tu cuenta de OpenRouter» `help.noKeyTitle`
- «Llegó al límite de rondas de herramientas; la respuesta puede estar incompleta.» `help.roundLimitReached`
- «Lo lee Gemma local en este equipo: no viaja a ningún servicio. No incluye claves.» `help.diagnosticsHintLocal`
- «Maximizar» `help.maximizePanel`
- «Mis documentos» `help.includeDocuments`
- «Más información sobre Diagnóstico» `help.diagnosticsInfo`
- «Más información sobre Mis documentos» `help.documentsInfo`
- «Nueva conversación» `help.newConversation`
- «Ocultar» `help.previewHide`
- «OpenRouter» `help.useRemote`
- «Preguntá cómo usar EntropIA o qué hacer cuando algo falla» `help.subtitle`
- «Preguntá, por ejemplo, cómo importar un PDF o qué hacer si falla una transcripción.» `help.emptyState`
- «Probá con:» `help.suggestionsLabel`
- «Rechazado» `help.approvalOutcomeRejected`
- «Rechazar» `help.reject`

### Varios (2/2)

- «Rechazaste: {summary}» `help.approvalRejected`
- «Responde Gemma local, en este equipo y solo con el manual: no hace acciones ni lee tus documentos.» `help.engineLocal`
- «Responde {model} vía OpenRouter.» `help.engineRemote`
- «Respuesta de Gemma local, solo con el manual» `help.localModelAnswer`
- «Respuesta local, sin consultar a OpenRouter» `help.localAnswer`
- «Restaurar» `help.restorePanel`
- «Se eliminan todas las conversaciones guardadas en este equipo. No se puede deshacer.» `help.clearHistoryMessage`
- «Todavía no hay conversaciones guardadas.» `help.historyEmpty`
- «Usar OpenRouter ({model}) para esta pregunta» `help.useRemoteHint`
- «Usó» `help.stepsUsed`
- «Venció» `help.approvalOutcomeExpired`
- «Ver el manual en línea» `help.openManual`
- «Ver input» `help.previewToggle`
- «Viaja a OpenRouter junto con tu pregunta. No incluye claves ni el contenido de tus documentos.» `help.diagnosticsHint`
- «ahora mismo» `help.historyNow`
- «¿Borrar el historial?» `help.clearHistoryTitle`
- «¿Cómo cargo mi clave de OpenRouter?» `help.suggestion2`
- «¿Cómo importo un PDF?» `help.suggestion1`

## Inicio

### continuar

- «Colección»
- «Continuar»
- «Documento sin título»
- «Retomar →»
- «{count} palabra»
- «{count} palabras»

### corpus

- «Colecciones»
- «Documentos»
- «Embeddings»
- «Estado del corpus»
- «OCR»
- «Reanudar»
- «STT»
- «Texto»
- «con OCR»
- «con STT»
- «con embeddings»
- «con texto»
- «{count} embeddings en pausa: cambió la configuración»
- «{count} pendientes de OCR»
- «{count} pendientes de embeddings»

### firstRun

- «Cargando los ejemplos…»
- «Crear colección»
- «Empezá con EntropIA»
- «Importá tus primeras fuentes y creá una colección para comenzar a trabajar.»
- «Inicio rápido»
- «Probar con documentos de ejemplo»

### importDialog

- «+ Nueva colección»
- «Cancelar»
- «Cerrar»
- «Desde Internet Archive»
- «Destino»
- «Elegir archivos»
- «Importar fuentes»
- «Ir a la colección»
- «No se pudieron cargar las colecciones»
- «Nombre de la colección»
- «Nombre de la nueva colección»

### localModes

- «Dictado y transcripción con Parakeet, en lugar de AssemblyAI»
- «Este archivo usa servicios en la nube»
- «IA de texto con el modelo local, en lugar de OpenRouter»
- «Los modelos se descargan desde Configuración → APIs remotas. Podés volver a la nube cuando quieras.»
- «OCRH con GLM-OCR local, en lugar de la API de GLM-OCR»
- «Seguir con la nube»
- «Usar los motores locales»
- «Viene configurado desde EntropIA Lite. EntropIA Pro puede hacer esto en este equipo, sin enviar el contenido afuera:»

### quickAccess

- «Acceso rápido»
- «Consulta sobre el corpus»
- «Corpus documental»
- «Indagación asistida»
- «Redacción con fuentes»

### Varios

- «Actividad reciente» `home.activity.title`
- «Colección» `home.activity.columnCollection`
- «Documento» `home.activity.columnDocument`
- «Embeddings» `home.activeProcess.kind.embeddings`
- «Escribir» `home.actions.newDocument`
- «Espacio de trabajo» `home.heading`
- «Importar» `home.actions.import`
- «Inicio» `home.title`
- «OCR» `home.activeProcess.kind.ocr`
- «Para organizar, procesar, explorar, analizar y escribir con trazabilidad.» `home.description`
- «Recuperar» `home.actions.newChat`
- «Ver lote →» `home.activeProcess.open`
- «{count} Documento» `home.recent.itemCount.one`
- «{count} Documentos» `home.recent.itemCount.other`
- «{done} / {total} páginas · {percent} %» `home.activeProcess.progress`
- «Última actividad» `home.activity.columnLast`

## Internet Archive

### collection

- «Esta colección no tiene resultados en esta página.»
- «Explorar»
- «Identificador de la colección»
- «Leyendo la colección…»
- «p. ej. opensource»

### destination

- «+ Nueva colección»
- «Destino»
- «Leyendo las colecciones…»
- «Nombre de la colección»
- «Nombre de la nueva colección»

### done

- «Archivo demasiado grande: supera el límite.»
- «Cancelado.»
- «Demasiadas páginas para importar.»
- «Demasiado grande para dividir.»
- «Error de red.»
- «Importación terminada»
- «Importados: {count}»
- «Internet Archive pidió esperar antes de reintentar.»
- «La importación se canceló.»
- «No se encontró el archivo.»
- «No se pudo importar.»
- «Omitidos: {count}»
- «Restringido: no se puede descargar.»
- «Sin espacio en el disco.»
- «Ya estaban en la colección: {count}»

### fileState

- «Cancelado»
- «Descargando»
- «En cola»
- «Importado»
- «Importando»
- «No disponible»
- «Omitido (duplicado)»
- «Verificado»
- «Verificando»

### item

- «Archivos»
- «Elegí un resultado para ver sus archivos.»
- «Esta ficha es una colección. Explórala en la pestaña Colección.»
- «Explorar en la pestaña Colección»
- «Leyendo la ficha…»
- «Ningún archivo de este registro es de los tipos elegidos.»
- «Origen: {source}»
- «Registro seleccionado»
- «Se muestran los primeros PDF; la lista está truncada.»
- «Seleccionar {name}»
- «Sin PDF: esta ficha no ofrece ningún PDF descargable.»
- «Tamaño: {size}»
- «derivado»
- «no determinado»
- «original»

### jobState

- «Borrador»
- «Cancelada»
- «Cancelando»
- «Completada»
- «En curso»
- «Lista»
- «Parcial»

### kind

- «Elegí al menos un tipo de archivo.»
- «Imágenes»
- «PDF»
- «PDF + texto»
- «Tipo de archivo»

### link

- «Abrir»
- «Identificador o enlace»
- «Identificador o https://archive.org/details/…»
- «Pega un identificador o una dirección de archive.org/details/….»
- «Resolviendo…»

### queue

- «Continuar la cola»
- «Descartar la cola»
- «Importación por tandas pendiente»
- «La cola quedó en pausa: faltan {count} registros. Se retoma al volver a abrir el importador.»
- «Preparando la importación de {count} registros…»
- «Preparando la próxima tanda…»
- «Quitar la cola»
- «Revisando los registros de la tanda: {checked} de {total}»
- «Tanda de {count} registros · {finished} de {total} ya procesados»
- «Toda la búsqueda en cola: {count} registros»
- «Toda la búsqueda quedó importada ({total} registros).»
- «{label}: {finished} de {total} registros procesados»

### record

- «Buscando archivos de {count} registros…»
- «Buscando los {count} registros…»
- «Se importan por tandas; si cerrás el importador, la cola se retoma al volver.»
- «Se ponen en cola los primeros {count} de {total} resultados (el máximo que entrega Internet Archive).»
- «Seleccionar los {count} resultados»
- «Seleccionar registro {title}»
- «Seleccionar todos los registros de la página»
- «Sin archivos de los tipos elegidos»

### running

- «Ahora: {name}»
- «Cancelando…»
- «Importando…»
- «Progreso de la importación»
- «{done} de {total} archivos»

### search

- «1 resultado»
- «Anterior»
- «Autor»
- «Buscando…»
- «Buscar en el archivo»
- «Desde el año»
- «Hasta el año»
- «Página {page} de {pages}»
- «Siguiente»
- «Sin resultados para esta búsqueda.»
- «Texto libre»
- «Título»
- «Ver archivos»
- «{count} resultados»

### selection

- «Ocultar selección»
- «Quitar {name}»
- «Seleccionados: {count}»
- «Selección»
- «Todavía no elegiste ningún archivo. Marca los archivos que quieras importar.»
- «Ver selección»

### summary

- «1 archivo sin tamaño conocido»
- «Archivos a importar: {count}»
- «Atención: supera 1 GB de volumen sincronizado estimado (cada PDF ocupa cerca del doble entre el original y sus páginas).»
- «Con lo omitido no queda ningún archivo para importar.»
- «Duplicados»
- «Existe en otra colección.»
- «Hay una versión nueva.»
- «Importar una copia independiente»
- «No se encontró en la ficha.»
- «Omitir»
- «Qué hacer»
- «Rechazados»
- «Revisa la preparación»
- «Tamaño conocido: {size}»
- «Volumen sincronizado estimado: {size}»
- «Ya importado en esta colección.»
- «{count} archivos sin tamaño conocido»

### Varios

- «Abrir colección» `internetArchive.openCollection`
- «Buscar» `internetArchive.discover.tab.search`
- «Cancelar» `internetArchive.cancel`
- «Cerrar» `internetArchive.close`
- «Cifrado: el archivo está cifrado y no se puede importar.» `internetArchive.availability.encrypted`
- «Colección» `internetArchive.discover.tab.collection`
- «Continuar» `internetArchive.continue`
- «Descubrimiento» `internetArchive.discover.group`
- «Enlace» `internetArchive.discover.tab.link`
- «Importaciones en curso» `internetArchive.resume.title`
- «Iniciar importación» `internetArchive.start`
- «Internet Archive» `internetArchive.title`
- «Nombre no válido: el nombre del archivo no es seguro.» `internetArchive.availability.unsafe_name`
- «Preparando…» `internetArchive.preparing`
- «Preparar importación» `internetArchive.prepare`
- «Privado: el archivo no es público.» `internetArchive.availability.private`
- «Restringido: Internet Archive no permite descargarlo.» `internetArchive.availability.item_restricted`
- «Resultados» `internetArchive.results.region`
- «Resultados por página» `internetArchive.pageSize.label`
- «Retomar importación» `internetArchive.resume.action`
- «Volver» `internetArchive.back`
- «desconocido» `internetArchive.unknownSize`
- «restringido» `internetArchive.restrictedBadge`
- «{state} · {done} de {total} archivos» `internetArchive.resume.job`

## Informes de investigación

### clarification

- «Con el diseño y el plan a la vista, esto es lo que necesito que definas. Podés dejar alguna sin responder, pero no todas: las que queden vacías se declaran en el informe.»
- «Enviando respuestas…»
- «Preguntas antes de armar el informe»
- «Responder y seguir»
- «Respondé al menos una pregunta: el encuadre del informe depende de esto.»

### degradation

- « (era de {tokens} tokens)»
- « ({count} veces)»
- «{step}: algunos lotes se tuvieron que leer en partes más chicas{times}. Los documentos se leyeron igual; si pasa seguido, probá con un modelo más capaz.»
- «{step}: el modelo devolvió una respuesta con un formato que el programa no entiende{times}, así que ese paso siguió sin su resultado. Esa parte del informe puede estar incompleta; probá con un modelo más capaz.»
- «{step}: la respuesta del modelo se cortó por largo{times}, así que ese paso siguió sin su resultado. Esa parte del informe puede estar incompleta: subí el «Tope de salida» en Configuración › {settingsTab}{cap} o usá un modelo que admita respuestas más largas.»
- «{step}: {text}{times}.»

### design

- «Alcance»
- «Completá la hipótesis, el alcance y al menos un criterio de cierre.»
- «Criterios de cierre»
- «Criterios de cierre, uno por línea»
- «Diseño de la investigación»
- «Editar diseño»
- «Hipótesis»

### planGate

- «Búsquedas»
- «Búsquedas antes de ir al corpus»
- «Búsquedas, una por línea»
- «Consultas bibliográficas»
- «Consultas bibliográficas, una por línea»
- «Esto es lo que voy a buscar en el archivo. Todavía no se gastó presupuesto de búsqueda.»
- «Tus respuestas no cambiaron el plan: las búsquedas son las mismas que antes de la ronda.»

### report

- «Cobertura del recorte consultado»
- «Colección»
- «Con fragmentos»
- «Consulta»
- «Detalle técnico»
- «Encuadre acordado»
- «Fuentes citadas»
- «Informe descargado en {path}»
- «Items»
- «Limitaciones»
- «Sin procesar»
- «Sin respuesta del investigador»
- «Total»
- «Ver la fuente»

### role

- «Búsqueda de fragmentos»
- «Exploración del archivo»
- «Fuente bibliográfica»
- «Lectura de documentos»
- «Lectura de la bibliografía»
- «Planificación de la investigación»
- «Redacción de secciones»
- «Redacción del informe»
- «Un paso de la investigación»
- «Verificación de afirmaciones»

### section

- «Cancelar»
- «Editada por el historiador: el texto no pasó por la verificación.»
- «Editar»
- «Guardar»
- «Indicación para el redactor»
- «Reescribe con la misma evidencia verificada, sin sumar fuentes. La llamada se suma al costo aunque el presupuesto esté agotado.»
- «Reescribiendo la sección…»
- «Reescribir»
- «Reescrita con la indicación: «{instruction}»»
- «Texto de la sección»
- «Título de la sección»

### source

- «Abrir el documento»
- «Buscando el documento…»
- «Cerrar fuente»
- «Este informe se armó antes de que las citas llevaran su fuente: no se puede abrir el documento.»
- «Fuente»
- «Pasaje citado»

### working

- «Estoy armando el enfoque…»
- «Estoy contrastando lo encontrado…»
- «Estoy eligiendo qué buscar…»
- «Estoy escribiendo el informe…»
- «Estoy leyendo las fuentes…»
- «Estoy trabajando en tu pregunta…»
- «Estoy viendo qué hay en el archivo…»
- «Necesito que me aclares algunas cosas antes de seguir…»

### Varios

- «Abrir fuente» `investigation.openSource`
- «Actividad de la investigación» `investigation.activity.label`
- «Advertencia: la cobertura puede ser insuficiente» `investigation.blockedTitle`
- «Ajustar presupuesto» `investigation.adjustBudget`
- «Aprobar y buscar» `investigation.approve`
- «Artefactos» `investigation.artifacts`
- «Cancelar» `investigation.cancel, investigation.cancelBudget, investigation.cancelRevision`
- «Continuar» `investigation.resume`
- «Continuar de todos modos» `investigation.continueCoverage`
- «Editar búsquedas» `investigation.revise`
- «Fuentes reales» `investigation.sources`
- «Gates» `investigation.gates`
- «Guardar presupuesto» `investigation.saveBudget`
- «Guardar y buscar» `investigation.saveRevision`
- «ID del evento» `investigation.eventId`
- «Investigación» `investigation.eyebrow`
- «Lote {batch}: reintento con un lote más chico ({budget} KB, {items} documentos) — {error}» `investigation.archiveRetry.line`
- «Línea de tiempo» `investigation.timeline`
- «No hay artefactos persistidos.» `investigation.noArtifacts`
- «No hay fuentes persistidas.» `investigation.noSources`
- «No se encontró una ruta de página para esa fuente.» `investigation.sourceUnavailable`
- «Obsoleto» `investigation.obsolete`
- «Pausar» `investigation.pause`
- «Podés continuar con el material disponible. Al confirmar, aceptás estas limitaciones; quedarán registradas en la investigación y en el informe. No se inventará evidencia faltante.» `investigation.blockedGuidance`
- «Preparando…» `investigation.loading`
- «Qué falta según la prospección» `investigation.blockedGaps`
- «Rechazar» `investigation.reject`
- «Se detuvo. Continuá para que termine el informe.» `investigation.pausedHint`
- «Todavía no hay eventos.» `investigation.noEvents`
- «Todavía no hay gates.» `investigation.noGates`
- «Todavía no hay informe.» `investigation.reportPending`
- «Van {calls} llamadas y {cost} de costo: el nuevo límite no puede quedar por debajo.» `investigation.budgetFloor`
- «Ver payload» `investigation.eventPayload`
- «{current} de {max}» `investigation.budgetValue, investigation.callsValue`

## Documento

### entityType

- «Custom»
- «Fecha»
- «Institución»
- «Lugar»
- «Misc»
- «Organización»
- «Persona»

### ftsDebug

- «Documentos hidratados»
- «Estrategia»
- «Filas indexadas»
- «IDs resultado»
- «Matches DB»
- «Query original»
- «Sanitizada»

### layoutFilter

- «Figuras»
- «Notas»
- «Tablas»
- «Texto»
- «Todos»
- «Títulos»

### Mapa

- «Arrastrá el marcador a la ubicación correcta y guardá el cambio.»
- «Cancelar»
- «Crear ubicación»
- «Editar ubicación»
- «Guardar»
- «No hay ubicaciones georreferenciadas»
- «Restablecer ubicación automática»
- «Ubicación»

### noteEditor

- «Abrir Dependencias de IA»
- «Agregar link»
- «Barra de formato»
- «Cancelar»
- «Cita»
- «Cursiva»
- «Código inline»
- «Detener dictado»
- «Dictado»
- «El micrófono no envía sonido: desconectá y reconectá el dispositivo.»
- «Encabezado 1»
- «Encabezado 2»
- «Encabezado 3»
- «Estilo de texto»
- «Estructura»
- «Ingresá una URL válida, por ejemplo https://entropia.app.»
- «Ingresá una URL válida.»
- «Iniciar dictado»
- «Insertar» `item.noteEditor.group.insert, item.noteEditor.linkSubmit`
- «Insertar link»
- «Lista con viñetas»
- «Lista ordenada»
- «Negrita»
- «No hay micrófono disponible en este dispositivo.»
- «No se detectó texto en el audio.»
- «No se pudo capturar audio del micrófono.»
- «Pegá una URL válida para el texto seleccionado.»
- «Procesando dictado...»
- «Quitar link»
- «Se alcanzó el máximo de {duration}. Procesando audio...»
- «Se alcanzó el máximo de {duration}. Texto insertado.»
- «Subrayado»
- «Texto insertado desde el micrófono.»
- «Tip: seleccioná texto para aplicar formato o links.»
- «Transcribiendo audio...»
- «URL»
- «Usá una URL http o https válida.»
- «https://...»

### ocrStage

- «analizando layout del documento»
- «corriendo OCR»
- «enviando a GLM-OCR»
- «esperando respuesta de GLM-OCR»
- «extrayendo texto nativo»
- «leyendo archivo»
- «procesando respuesta de GLM-OCR»

### process

- «Borrado de región»
- «Borrado en página PDF»
- «Cambio de versión de la imagen»
- «Corrección de OCR con LLM»
- «Deshacer edición»
- «Duplicado»
- «Edición manual de la transcripción»
- «Edición manual del texto»
- «Embeddings»
- «Entidades»
- «Importación»
- «OCR»
- «OCR de caja corregida»
- «Recorte de imagen»
- «Recorte de página PDF»
- «Rehacer edición»
- «Reparación de versión (mantenimiento)»
- «Restauración del OCR original»
- «Resumen»
- «Rotación 90°»
- «Rotación de página PDF»
- «Rotación fina»
- «Separación en página»
- «Texto reemplazado por el asistente»
- «Transcripción»
- «Tripletas»

### similarAssetPreview

- «Cargando texto completo...»
- «Cerrar consulta y volver a la página original»
- «Consulta por similitud semántica»
- «Controles de consulta del documento»
- «Similitud {value}%»
- «Texto relacionado»

### toolbar

- «Acento»
- «Acercar»
- «Advertencia»
- «Alejar»
- «Borrar región (relleno blanco)»
- «Colapsar barra»
- «Colapsar barra de anotaciones»
- «Color de anotación {label}»
- «Deshacer»
- «Deshacer última edición»
- «Duplicar página»
- «Eliminar anotación seleccionada»
- «Expandir barra»
- «Expandir barra de anotaciones»
- «Herramienta de anotación rectangular»
- «Herramienta de subrayado»
- «Herramientas de edición de documentos»
- «Mover imagen (mano)»
- «Peligro»
- «Recortar a la selección»
- «Rehacer»
- «Rehacer última edición»
- «Rotación fina a la derecha, grado a grado»
- «Rotación fina a la izquierda, grado a grado»
- «Rotación fina {degrees}»
- «Rotar 90° a la derecha»
- «Rotar 90° a la izquierda»
- «Éxito»

### Varios (1/7)

- « para esta página» `item.noTriplesPageSuffix`
- «Actual» `item.imageVersionCurrent`
- «Agregar campo» `item.metadataAddField`
- «Agregar entidad» `item.addEntity`
- «Agregar nota» `item.addNote`
- «Agregar tripleta» `item.addTriple`
- «Ajustar caja» `item.layoutBoxEditAdjust`
- «Análisis» `item.analysisTab`
- «Aplicar nombres» `item.renameSpeakersApply`
- «Arrastrá los bordes o las esquinas de la caja resaltada en el documento; arrastrá el interior para moverla.» `item.layoutBoxEditDragHint`
- «Audio» `item.audioLabel`
- «Avanzar 5 segundos» `item.audioSkipForward`
- «BBox bloque» `item.layoutBlockBbox`
- «BBox copiado.» `item.layoutCopiedBbox`
- «BBox overlay» `item.layoutOverlayBbox`
- «Bloque seleccionado · #{order}» `item.layoutSelectedBlock`
- «Buscando textos similares...» `item.ftsSearching`
- «Buscar» `item.audioSeek`
- «Buscar por texto similar (FTS)» `item.searchBySimilarText`
- «Búsquedas» `item.searchTab`
- «Cada proceso que sufrió este documento, con la herramienta, el modelo y las versiones que lo produjeron. Se guarda en este dispositivo.» `item.processHistoryHelp`
- «Campo» `item.metadataFieldLabel`
- «Cancelar» `item.entityCancelTitle, item.layoutBoxEditCancel, item.tripleCancelTitle`
- «Cancelar edición» `item.cancelEdit`
- «Cancelar edición de la entidad» `item.entityCancelAria`
- «Cancelar edición de la tripleta» `item.tripleCancelAria`
- «Cargando PDF...» `item.viewerPdfLoading`
- «Cargando historial…» `item.processHistoryLoading`
- «Cargando layout…» `item.layoutLoading`
- «Cargando versiones…» `item.imageVersionsLoading`
- «Cargando...» `item.loading, item.loadingStatus`
- «Cerrar aviso» `item.exportExtractedTextDismiss, item.ocrAllPagesDismiss`
- «Clave» `item.metadataKeyPlaceholder`
- «Configurá OpenRouter o instalá el corrector local» `item.ocrCorrectUnavailable`
- «Confirmar eliminación de entidad {value}» `item.entityConfirmDeleteAria`
- «Confirmar eliminación de tripleta {subject}» `item.tripleConfirmDeleteAria`
- «Copiar» `item.copyExtractedText`
- «Copiar JSON» `item.layoutCopyJson`
- «Copiar bbox» `item.layoutCopyBbox`
- «Copiar texto» `item.layoutCopyText`
- «Copiar texto extraído» `item.copyExtractedTextAria`
- «Corrección OCR» `item.ocrCorrectTitle, item.pdfCorrectTitle`
- «Corrección, resumen y triples requieren OpenRouter. Configuralo en Configuración.» `item.llmUnavailableHint`
- «Corregir caja» `item.layoutBoxEditTitle`
- «Correr OCRH igual» `item.ocrhCpuContinue`
- «Corresponde al layout» `item.imageVersionMatchesLayout`
- «Corresponde al texto» `item.imageVersionMatchesText`
- «Descargar» `item.downloadExtractedText`
- «Descargar texto extraído» `item.downloadExtractedTextAria`

### Varios (2/7)

- «Descartar la nueva entidad» `item.newEntityCancelAria`
- «Descartar la nueva tripleta» `item.newTripleCancelAria`
- «Disponible después de corregir el OCR» `item.restoreOriginalOcrUnavailable`
- «Documento» `item.documentTab, item.viewerImageAlt`
- «Documento activo» `item.activeDocument`
- «EMBED» `item.embedAction`
- «Editar entidad» `item.entityEditTitle`
- «Editar entidad {value}» `item.entityEditAria`
- «Editar nota» `item.editNote`
- «Editar tripleta» `item.tripleEditTitle`
- «Editar tripleta {subject}» `item.tripleEditAria`
- «Editar valor de entidad» `item.entityEditValueAria`
- «Ejecutando… {progress}%» `item.extractionRunning`
- «Ejecutando… {progress}% · {stage}» `item.extractionRunningStage`
- «El texto tiene correcciones hechas a mano. Si volvés a correr el OCR, se reemplazan. ¿Continuar?» `item.reocrWarning`
- «Eliminar entidad» `item.entityDeleteTitle`
- «Eliminar entidad {value}» `item.entityDeleteAria`
- «Eliminar nota» `item.deleteNote, item.deleteNoteTitle`
- «Eliminar tripleta» `item.tripleDeleteTitle`
- «Eliminar tripleta {subject}» `item.tripleDeleteAria`
- «Entidad {value}» `item.entityAria`
- «Entidades» `item.entities`
- «Entrada (SHA-256)» `item.processHistoryInput`
- «EntropIA» `item.processHistoryApp`
- «Es la misma persona que {value}» `item.entityAliasAria`
- «Es la misma persona que…» `item.entityAliasTitle`
- «Escribí para buscar...» `item.ftsPlaceholder`
- «Escribí una nota...» `item.writeNote`
- «Esta imagen tiene anotaciones. Pueden no coincidir con la versión que elegiste. ¿Continuar?» `item.imageVersionAnnotationsWarning`
- «Esta página todavía no tiene layout persistido.» `item.layoutMissing`
- «Este audio no tiene versiones anteriores.» `item.transcriptVersionsEmpty`
- «Este audio ya tiene una transcripción. Si lo volvés a transcribir, se reemplaza; la actual, con sus nombres de hablante, queda en «Versiones anteriores» y la podés restaurar. ¿Continuar?» `item.retranscribeExistingWarning`
- «Este equipo no tiene una GPU que pueda correr GLM-OCR (NVIDIA, o AMD, Intel o Apple con 4 GB): OCRH local corre en el procesador y tarda unos 15 minutos por página. OCRL tarda segundos.» `item.ocrhCpuWarning`
- «Este layout corresponde a otra versión de la imagen ({width}×{height}); la actual mide {currentWidth}×{currentHeight}.» `item.layoutOtherVersionSized`
- «Este layout corresponde a otra versión de la imagen.» `item.layoutOtherVersion`
- «Este texto se extrajo de otra versión de la imagen. Puede no coincidir con lo que ves.» `item.textOtherVersion`
- «Exportar JSON» `item.processHistoryExport`
- «Extracción de texto» `item.textExtraction`

### Varios (3/7)

- «Extracción en progreso…» `item.ocrFastBusyTitle, item.ocrHighBusyTitle`
- «Extraer texto» `item.ocrFastTitle`
- «Extraer tripletas» `item.triplesTitle`
- «Extraé o transcribí primero» `item.summaryNeedsText`
- «Extraé texto primero» `item.ocrCorrectNeedsText`
- «FTS Debug (solo dev)» `item.ftsDebugTitle`
- «Filtrar bloques por tipo» `item.layoutFilterGroup`
- «Formatos de descarga del texto extraído» `item.downloadExtractedTextMenu`
- «Generando exportación…» `item.exportExtractedTextWorking`
- «Generando resumen…» `item.generatingSummary`
- «Generar resumen» `item.summaryTitle`
- «Grupo» `item.layoutGroup`
- «Guardado en {path}» `item.exportExtractedTextSaved`
- «Guardando...» `item.saving`
- «Guardar cambios» `item.entitySaveTitle, item.tripleSaveTitle`
- «Guardar entidad» `item.entitySaveAria`
- «Guardar la nueva entidad» `item.newEntitySaveAria`
- «Guardar la nueva tripleta» `item.newTripleSaveAria`
- «Guardar nota» `item.saveNote`
- «Guardar tripleta» `item.tripleSaveAria`
- «Hay layout guardado, pero no trae bloques navegables.» `item.layoutNoBlocks`
- «Herramienta» `item.processHistoryTool`
- «Historial de procesos» `item.processHistory`
- «Historial exportado.» `item.processHistoryExported`
- «INDEX» `item.indexAction`
- «Imagen OCR no disponible» `item.ocrImageUnavailable`
- «Ingresá un término para ver resultados.» `item.ftsPrompt`
- «Iniciando extracción…» `item.extractionStarting`
- «Iniciando transcripción…» `item.transcriptionStarting`
- «Inspector» `item.layoutInspector`
- «JSON copiado.» `item.layoutCopiedJson`
- «La similitud semántica requiere OpenRouter configurado en Configuración.» `item.searchReadiness.openRouterNeeded`
- «La transcripción tiene correcciones hechas a mano. Si la volvés a transcribir, se reemplaza; la versión corregida queda en «Versiones anteriores». ¿Continuar?» `item.retranscribeWarning`
- «Label» `item.layoutLabel`
- «Layout» `item.layoutTab`
- «Layout no disponible para audios.» `item.layoutUnavailableForAudio`
- «Leer con OCRH cada página que todavía no tiene texto» `item.ocrAllPagesHighTitle`
- «Leer con OCRL cada página que todavía no tiene texto» `item.ocrAllPagesFastTitle`
- «Limpiar búsqueda» `item.ftsClear`
- «Los archivos que dejó cada edición de esta imagen. Elegí cuál mostrar: no se borra ninguno.» `item.imageVersionsHelp`
- «Mapa» `item.mapTab`
- «Markdown (.md)» `item.exportExtractedTextMarkdown`
- «Metadatos» `item.metadata, item.metadataTab`
- «Metadatos del archivo» `item.fileMetadata`
- «Metadatos personalizados» `item.customMetadata`
- «Miniatura: {label}» `item.imageVersionThumbnailAlt`
- «Modelo» `item.processHistoryModel`

### Varios (4/7)

- «Mostrando {visible} de {total} bloques.» `item.layoutShowing`
- «Mostrar overlay» `item.layoutToggleShow`
- «NER» `item.nerAction`
- «No hay archivos de esta imagen en el disco.» `item.imageVersionsEmpty`
- «No hay bloques del tipo seleccionado para la página visible.» `item.layoutNoFilterBlocks`
- «No hay bloques para la página visible.» `item.layoutNoPageBlocks`
- «No hay metadatos cargados para este documento.» `item.metadataEmpty`
- «No hay páginas adjuntas a este documento.» `item.noAssets`
- «No hay resultados para esa búsqueda.» `item.ftsNoResults`
- «Nota vacía» `item.noteEmptyPreview`
- «Notas» `item.notes, item.notesTab`
- «Nueva caja a continuación» `item.layoutBoxEditInsert`
- «Nuevo tipo de entidad» `item.newEntityType`
- «Nuevo valor de entidad» `item.newEntityValue`
- «OCR de alta precisión» `item.ocrHighTitle`
- «OCRC» `item.ocrCorrectAction, item.pdfCorrectAction`
- «OCRH» `item.ocrHighAction`
- «OCRH local sin GPU» `item.ocrhCpuTitle`
- «OCRL» `item.ocrFastAction`
- «OCRR» `item.summaryAction, item.summaryPdfAction`
- «Objeto de la nueva tripleta» `item.newTripleObjectAria`
- «Objeto de la tripleta» `item.tripleObjectAria`
- «Ocultar» `item.renameSpeakersHide`
- «Ocultar overlay» `item.layoutToggleHide`
- «Ocultar versiones anteriores» `item.transcriptVersionsHide`
- «OpenRouter no configurado» `item.summaryUnavailable, item.triplesUnavailable`
- «Orden» `item.layoutOrder`
- «Original» `item.imageVersionOriginal`
- «Overlay de anotaciones de imagen» `item.viewerImageOverlay`
- «Overlay de layout del documento» `item.viewerLayoutOverlay`
- «Overlay source» `item.layoutOverlaySource`
- «PDF (.pdf)» `item.exportExtractedTextPdf`
- «Page {page}» `item.assetPageLabel`
- «Panel de la página» `item.assetPanel`
- «Panel derecho del documento» `item.rightPanel`
- «Para ver similitud semántica, generá embeddings desde Análisis.» `item.searchReadiness.embeddingNeeded`
- «Parámetros» `item.processHistoryParams`
- «Pausar» `item.audioPause`
- «Predicado de la nueva tripleta» `item.newTriplePredicateAria`
- «Predicado de la tripleta» `item.triplePredicateAria`
- «Presioná otra vez para confirmar eliminación» `item.entityConfirmDeleteTitle, item.tripleConfirmDeleteTitle`
- «Primero extraé o transcribí texto para que la búsqueda y la similitud tengan material para comparar.» `item.searchReadiness.textNeeded`
- «Proceso: {process}» `item.processHistoryUnknown`
- «P{page}» `item.pageShort`
- «Página» `item.layoutPage`
- «Página anterior» `item.previousPage`
- «Página siguiente» `item.nextPage`
- «Página {page}» `item.page`
- «Página {page} de {total}» `item.pageOf`

### Varios (5/7)

- «Páginas en cola: {queued}. Salteadas: {skipped} (ya tenían texto, estaban corregidas a mano o ya estaban en cola).» `item.ocrAllPagesQueued`
- «Páginas similares» `item.similarAssets`
- «Páginas similares (por página {page})» `item.similarAssetsPage`
- «Quitar campo» `item.metadataRemoveField`
- «Recargar» `item.processHistoryReload`
- «Reconocer texto de la caja» `item.layoutBoxEditApply`
- «Reconociendo…» `item.layoutBoxEditApplying`
- «Reemplazada el {date} · {model} · {count} caracteres» `item.transcriptVersionMeta`
- «Región a borrar» `item.viewerEraseRegion`
- «Región de layout {label}» `item.viewerLayoutRegion`
- «Región de recorte» `item.viewerCropRegion`
- «Renombrar hablantes» `item.renameSpeakers`
- «Reproducir» `item.audioPlay`
- «Restaurando OCR original…» `item.restoreOriginalOcrWorking`
- «Restaurar» `item.transcriptVersionRestore`
- «Restaurar OCR original» `item.restoreOriginalOcr`
- «Restaurar versión original del OCR» `item.restoreOriginalOcrAria`
- «Resumen» `item.summary, item.summaryAudioAction`
- «Retroceder 5 segundos» `item.audioSkipBack`
- «STT» `item.transcribeShortAction`
- «STTR» `item.transcribeBusyAction`
- «Salida (SHA-256)» `item.processHistoryOutput`
- «Seleccionar anotación {id}» `item.viewerAnnotation`
- «Seleccionar página del layout» `item.layoutPageSelect`
- «Seleccioná un bloque para ver label, orden, página, bbox, source y texto completo.» `item.layoutEmptyInspector`
- «Seleccioná una página antes de generar embeddings y similitud a nivel página.» `item.analysisNeedAsset`
- «Seleccioná una página para ver resultados de similitud por página.» `item.similarAssetsNeedSelection`
- «Si GLM-OCR cortó o no detectó texto, ajustá la caja sobre el documento y volvé a reconocerla. El bloque conserva su lugar en el orden de lectura.» `item.layoutBoxEditHelp`
- «Si la búsqueda no trae resultados, indexá el texto desde Análisis.» `item.searchReadiness.ftsIndexNeeded`
- «Sin OCR disponible para previsualizar.» `item.similarAssetsNoPreview`
- «Sin preview textual» `item.layoutNoPreview`
- «Sin página seleccionada» `item.assetNoSelection`
- «Sin texto completo para este bloque.» `item.layoutNoFullText`
- «Sujeto de la nueva tripleta» `item.newTripleSubjectAria`
- «Sujeto de la tripleta» `item.tripleSubjectAria`
- «Sujeto, predicado y objeto no pueden quedar vacíos.» `item.tripleEmptyFields`
- «TRIPLET» `item.triplesAction`
- «Texto» `item.textTab`
- «Texto / preview ampliado» `item.layoutPreview`
- «Texto copiado.» `item.layoutCopiedText`
- «Texto del bloque #{order} actualizado.» `item.layoutBoxEditDone`
- «Texto extraído» `item.extractedText, item.extractedTextTab`

### Varios (6/7)

- «Texto extraído copiado» `item.copyExtractedTextSuccess`
- «Texto reconocido, pero no encontré dónde iba en la extracción: quedó agregado al final.» `item.layoutBoxEditAppended`
- «Tocá o pasá el mouse sobre un bloque para resaltarlo en el viewer.» `item.layoutHelp`
- «Todas las páginas» `item.ocrAllPages`
- «Todas las páginas ya tienen texto o están en cola.» `item.ocrAllPagesNothing`
- «Todavía no hay entidades extraídas.» `item.entitiesEmpty`
- «Todavía no hay notas.» `item.noNotes`
- «Todavía no hay procesos registrados para este documento. Los documentos importados antes de la versión con historial no tienen sus procesos anteriores.» `item.processHistoryEmpty`
- «Todavía no hay páginas similares. Generá embeddings para esta página y compararla contra el resto.» `item.similarAssetsEmpty`
- «Todavía no hay texto extraído para este documento.» `item.noExtractedText`
- «Todavía no hay tripletas extraídas{suffix}.» `item.noTriples`
- «Transcribiendo…» `item.transcribing`
- «Transcribiendo… {progress}%» `item.transcriptionRunning`
- «Transcribir» `item.transcribeAction`
- «Transcribir de nuevo» `item.retranscribeAction`
- «Transcribir este archivo de audio» `item.transcribeTitle`
- «Transcripción» `item.transcription`
- «Transcripción de audio» `item.audioTranscription`
- «Transcripción en progreso…» `item.transcribeBusyTitle`
- «Tripletas semánticas» `item.semanticTriples`
- «Tópicos» `item.topics`
- «Usar OCRL» `item.ocrhCpuUseOcrl`
- «Usar esta versión» `item.imageVersionUse`
- «Valor» `item.metadataValueLabel, item.metadataValuePlaceholder`
- «Versiones anteriores» `item.transcriptVersions`
- «Versiones de la imagen» `item.imageVersions`
- «Versión restaurada. La transcripción que estaba quedó en «Versiones anteriores».» `item.transcriptVersionRestored`
- «Versión {n}» `item.imageVersionNumber`
- «Volumen» `item.audioVolume`
- «Volver a correr el OCR» `item.textRerun`
- «Volver a correr el layout» `item.layoutRerun`
- «Word (.docx)» `item.exportExtractedTextDocx`
- «corre en este equipo con {model}» `item.ocrCorrectEngineLocal`
- «desconocido» `item.ocrMethodUnknown`
- «en cola» `item.jobStatus.pending`
- «en curso» `item.jobStatus.running`
- «enviando a AssemblyAI» `item.transcriptionStage.submitting_remote`
- «envía el texto a {tool} ({model})» `item.ocrCorrectEngineRemote`
- «esperando respuesta remota» `item.transcriptionStage.polling_remote`
- «hecho» `item.jobStatus.done, item.jobStatus.stored`
- «no se pudo leer el archivo de la imagen para comprobar a qué versión corresponde.» `item.layoutVersionUnknown`
- «nueva» `item.layoutBoxEditNewLabel`
- «p. {page}» `item.pageBreadcrumb`

### Varios (7/7)

- «página {assetId} · documento {itemId} · colección {collectionId}» `item.assetMetaLine`
- «sin hacer» `item.jobStatus.idle`
- «subiendo audio» `item.transcriptionStage.uploading`
- «{count} bloques» `item.layoutBlocksCount`
- «{count} caracteres» `item.characters`
- «{count} regiones» `item.layoutRegionsCount`
- «{count}s» `item.audioDurationSeconds`
- «· Página {page}» `item.pageInline`
- «¿Eliminar?» `item.entityDeletePrompt, item.tripleDeletePrompt`
- «¿Seguro que querés eliminar esta nota? Esta acción no se puede deshacer.» `item.deleteNoteMessage`

## Navegación

### Varios

- «Base de datos» `nav.dbBrowser`
- «Biblioteca» `nav.biblioteca`
- «Chat» `nav.ragChat`
- «Colecciones» `nav.collections`
- «Configuración» `nav.settings`
- «Investigación» `nav.research`
- «Navegador» `nav.navegador`

## Navegador

### capture

- «Captura (borrador)»
- «Capturando…»
- «Consultada (UTC)»
- «Copia HTML: {size}»
- «Descartar»
- «Dirección final»
- «El contenido superó el tope y se cortó.»
- «Página»
- «SHA-256 de la cita»
- «SHA-256 del HTML»
- «Selección»
- «Texto: {count} caracteres»

### copy

- «+ Nueva colección»
- «Abrir documento»
- «Cancelar»
- «Cerrar»
- «Colección de destino»
- «Copia creada en «{collection}»: «{title}».»
- «Copiando el PDF…»
- «Copiar»
- «Copiar a una colección»
- «Copiar otra vez»
- «Esta captura ya está copiada en «{collection}» como «{title}». Si copias otra vez, se crea un segundo documento independiente.»
- «Leyendo las colecciones…»
- «Nombre de la colección»
- «Nombre de la nueva colección»
- «Se crea un documento nuevo en la colección con una copia de «{title}». La fuente web no cambia y la copia es independiente: borrar la fuente no la toca.»
- «Se crea un documento nuevo en la colección: un PDF con el texto de «{title}», precedido de una cabecera con la dirección, la fecha de consulta y el hash de la captura. No reproduce el diseño ni las imágenes de la página. La fuente web no cambia y la copia es independiente: borrar la fuente no la toca.»

### download

- «Cambiar»
- «Carpeta de descargas:»
- «Descargando…»
- «Descargas»
- «Desde {host}»
- «Desde {host} · {title}»
- «El archivo está vacío.»
- «Elegir la carpeta de descargas»
- «Guardado»
- «Guardado en {folder}»
- «Hay demasiadas descargas en curso.»
- «La descarga se interrumpió.»
- «La dirección de la descarga no está permitida.»
- «Limpiar»
- «Motivo desconocido.»
- «No es un PDF.»
- «PDF verificado»
- «Quitar {name} de la lista»
- «Rechazado»
- «Supera el tamaño máximo.»
- «Ver fuente»
- «Ver la fuente que ya guarda {name}»
- «Ya está en tus fuentes»

### save

- «Guardado»
- «Guardado como fuente web»
- «Guardado {name}»
- «Guardando {name}»
- «Guardando…»
- «Guardar»
- «Guardar {name}»

### sources

- «Abrir en el navegador»
- «Abrir página de origen»
- «Actualizada: {when}»
- «Buscar en las fuentes guardadas»
- «Cancelar»
- «Capturas»
- «Capturas: {count}»
- «Cargando…»
- «Cerrar las fuentes guardadas»
- «Consultada»
- «Copia HTML guardada en este equipo.»
- «Copiar a colección»
- «Copiar dirección»
- «Descargando el archivo desde tu cuenta de sincronización…»
- «Dirección canónica»
- «Dirección copiada.»
- «Dirección final»
- «Dirección original»
- «El archivo guardado no está disponible en este equipo.»
- «El texto es largo y está guardado en un archivo.»
- «Eliminar»
- «Eliminar fuente»
- «Eliminar fuente guardada»
- «Eliminar {title}»
- «Esta fuente no tiene capturas.»
- «Esta fuente ya no existe.»
- «Fuente eliminada.»
- «Fuente eliminada. Algunos archivos no se pudieron borrar ahora y se limpian al iniciar la app.»
- «Fuentes guardadas» `navegador.sources.open, navegador.sources.title`
- «La fuente ya no existe.»
- «Ninguna fuente coincide con la búsqueda.»
- «No se pudo eliminar: identificador no válido.»
- «No se pudo eliminar: {message}»
- «PDF»
- «PDF guardado en este equipo.»
- «Primera consulta (UTC)»
- «Página»
- «SHA-256 de la cita»
- «SHA-256 del HTML»
- «SHA-256 del PDF»
- «Se eliminan «{title}», sus {count} capturas y sus archivos guardados en este equipo. Las copias que hayas hecho en colecciones no se tocan. No se puede deshacer.»
- «Selección»
- «Tamaño: {size}»
- «Todavía no guardaste ninguna fuente. Captura una página, una selección o un PDF y presiona Guardar.»
- «Título, dirección o texto»
- «UTC: {utc}»
- «Ver PDF guardado»
- «Volver a la lista»

### tabs

- «Abrir solapa nueva»
- «Cerrar solapa {title}»
- «Nueva solapa»
- «Solapa nueva: ya hay {max} abiertas»
- «Solapas del navegador»

### zotero (1/2)

- «Abriendo Zotero… la cola sigue cuando responda.»
- «Abrir Zotero»
- «Abrir Zotero desde aquí no está disponible en este sistema.»
- «Abrir en Zotero»
- «Aceptar»
- «Algunos campos difieren de los de Zotero; se dejaron como están (nunca se pisan).»
- «Biblioteca de destino»
- «Biblioteca: {library}»
- «Campos completados: {fields}.»
- «Campos que editaste en Zotero y se respetan: {fields}.»
- «Campos que podrían completarse y no se modifican desde aquí: {fields}.»
- «Cancelado»
- «Cancelar»
- «Cancelar copia»
- «Cerrar»
- «Completar en Zotero»
- «Comprobando si ya está en Zotero…»
- «Con tu API key de Zotero puedes completar los campos vacíos del elemento y adjuntarle el PDF.»
- «Copiado»
- «Copiado a «{library}».»
- «Copiando…»
- «Copiar»
- «Copiar a Zotero» `navegador.zotero.copy, navegador.zotero.title`
- «Copias a Zotero»
- «Detalle técnico: {reason}.»
- «El PDF llega a tu Zotero local cuando Zotero sincroniza, y los archivos cuando sincroniza también archivos.»
- «El PDF no se adjuntó: el elemento ya existía y Zotero solo permite adjuntar al crearlo.»
- «El PDF ya estaba adjunto.»
- «El elemento cambió en Zotero mientras se completaba y no se modificó. Reintenta la copia.»
- «El elemento todavía no está en el servidor de Zotero (se creó hace poco en tu Zotero local). Vuelve a copiar cuando Zotero termine de sincronizar.»
- «El elemento ya existía en la biblioteca: se enlazó, no se duplicó.»
- «En cola»
- «Enviando a Zotero…»
- «Esperando a Zotero»
- «Esta fuente no tiene un PDF guardado: se copiará solo la página.»
- «La API key de la Web API pertenece a otra cuenta de Zotero que el Zotero abierto: no se completó nada.»
- «La copia a «{library}» se canceló.»
- «Leyendo las bibliotecas…»
- «Mi biblioteca»
- «No faltaba ningún campo en Zotero: no se cambió nada.»
- «No se adjuntó el PDF: tu almacenamiento de Zotero está lleno. El elemento quedó enlazado.»
- «No se encontró Zotero en las carpetas habituales. Ábrelo a mano.»
- «No se pudo adjuntar el PDF: esa biblioteca no admite archivos.»
- «No se pudo leer la cuenta del Zotero abierto (¿biblioteca vacía?): no se completó nada por la Web API.»
- «PDF adjunto.»
- «Reintentar»
- «Se adjuntará el PDF guardado el {date}.»
- «Se completaron campos vacíos del elemento en Zotero a través de la Web API. Llegan a tu Zotero local cuando Zotero sincroniza.»
- «Se crea «{title}» en Zotero como una página web. La fuente guardada no cambia y no se mueve.»
- «Si Zotero no está abierto, la copia espera en cola y se envía sola cuando Zotero responda. Si el elemento ya está en la biblioteca, no se duplica.»
- «Ya estaba en Zotero»

### zotero (2/2)

- «Ya estaba en «{library}»: no se duplicó ni se modificó el elemento existente.»
- «Ya estaba en «{library}»: no se duplicó y se adjuntó el PDF.»
- «Ya estaba en «{library}»: no se duplicó y se agregaron los datos que faltaban y el PDF.»
- «Ya estaba en «{library}»: no se duplicó y se agregaron los datos que faltaban.»
- «Ya está en Zotero («{library}»).»
- «Zotero no responde. La copia a «{library}» queda en cola y se enviará cuando Zotero esté abierto.»
- «Zotero no responde: esto sale del registro de EntropIA, no de Zotero.»
- «Zotero no responde: se muestran las bibliotecas conocidas por EntropIA, que pueden estar incompletas.»
- «Zotero se abrió hace un momento; espera a que cargue.»
- «Zotero ya está abierto.»
- «Zotero ya no reconoce la API key guardada: no se completó nada. Revísala en Configuración.»
- «dirección»
- «fecha de acceso»
- «sitio web»
- «título»

### Varios

- «Abriendo el PDF…» `navegador.pdf.loading`
- «Adelante» `navegador.forward`
- «Atrás» `navegador.back`
- «Bloqueado: {reason}» `navegador.blocked`
- «Capturar página» `navegador.capturePage`
- «Capturar selección» `navegador.captureSelection`
- «Cerrar el PDF» `navegador.pdf.close`
- «Dirección» `navegador.address`
- «Escribe una dirección web» `navegador.addressPlaceholder`
- «Escribe una dirección y presiona Enter para empezar.» `navegador.idle`
- «Navegador» `navegador.title`
- «PDF guardado» `navegador.pdf.region`
- «Página {page} de {total}» `navegador.pdf.page`
- «Recargar» `navegador.reload`
- «Área de la página web» `navegador.pageArea`

## Campos numéricos

### Varios

- «Aumentar» `numberField.increase`
- «Disminuir» `numberField.decrease`

## Ocurrencias

### Varios

- «Anterior» `occurrence.previous`
- «Aparición {index} de {total}» `occurrence.count`
- «En otra parte del documento (también resaltado): {parts}.» `occurrence.elsewhereParts`
- «Inferido (subrayado punteado): {parts}.» `occurrence.inferredParts`
- «Inferido por el modelo a partir del contexto, sin palabras que lo expresen en el documento: {parts}.» `occurrence.missingParts`
- «Literal en el texto: {parts}.» `occurrence.literalParts`
- «Localizando apariciones…» `occurrence.loading`
- «No se encontró el pasaje en el texto disponible.» `occurrence.none`
- «Objeto» `occurrence.object`
- «Punteado: reconstruido» `occurrence.legendReconstructed`
- «Referencia aproximada: solo se encontró una entidad, no el vínculo.» `occurrence.approximate`
- «Relación reconstruida por el modelo a partir de esta oración.» `occurrence.reconstructed`
- «Siguiente» `occurrence.next`
- «Sujeto» `occurrence.subject`
- «Vínculo» `occurrence.predicate`

## Frases

### Varios

- «Antes» `phraseAnalysis.before`
- «Después» `phraseAnalysis.after`
- «Frases» `phraseAnalysis.tab`
- «Frases frecuentes» `phraseAnalysis.phrasesTitle`
- «Ninguna palabra aparece cerca más de lo esperable.» `phraseAnalysis.noCollocates`
- «No hay frases que se repitan con esta longitud.» `phraseAnalysis.noPhrases`
- «Palabra» `phraseAnalysis.word`
- «Palabras asociadas» `phraseAnalysis.collocatesTitle`
- «Palabras por frase» `phraseAnalysis.length`
- «Por ejemplo: guerra» `phraseAnalysis.wordPlaceholder`
- «Una palabra en contexto» `phraseAnalysis.wordTitle`
- «{count} veces cerca · G² {g2}» `phraseAnalysis.collocateStats`
- ««{word}» no aparece en esta colección.» `phraseAnalysis.absent`
- «Árbol de palabras» `phraseAnalysis.treeTitle`

## Chat

### Varios (1/2)

- «1 documento espera sus pasajes: se indexa en segundo plano y, hasta que termine, el chat no encuentra su texto nuevo.» `ragChat.embeddingQueueOne`
- «Abrir fuente» `ragChat.openSource`
- «Abrir original» `ragChat.passageOpenOriginal`
- «Abrir pasaje» `ragChat.openPassage`
- «Ambos» `ragChat.scopeBoth`
- «Biblioteca» `ragChat.scopeBiblioteca`
- «Bibliotecas de Zotero» `ragChat.librariesMenu`
- «Buscando conversaciones…» `ragChat.searchingConversations`
- «Buscando en la base de conocimiento…» `ragChat.thinking`
- «Buscar conversaciones» `ragChat.searchConversations`
- «Buscar en» `ragChat.scopeLabel`
- «Buscar por título o contenido…» `ragChat.searchConversationsPlaceholder`
- «Captura web guardada: se muestra su texto, con el pasaje citado marcado.» `ragChat.passageOriginalSnapshotNote`
- «Cargando el pasaje…» `ragChat.passageLoading`
- «Cerrar» `ragChat.passageClose`
- «Cerrar original» `ragChat.passageOriginalClose`
- «Chat de investigación» `ragChat.title`
- «Colecciones del corpus» `ragChat.collectionsMenu`
- «Consultá la base de conocimiento de transcripciones y documentos OCR» `ragChat.subtitle`
- «Conversaciones» `ragChat.conversations`
- «Copiado» `ragChat.copiedResponse`
- «Copiar respuesta» `ragChat.copyResponse`
- «Corpus» `ragChat.scopeCorpus`
- «Descargar conversación en PDF» `ragChat.downloadConversation`
- «Editar nombre de la conversación» `ragChat.editConversationName`
- «El PDF se abre en la página citada; el pasaje queda marcado en el texto de al lado.» `ragChat.passageOriginalNote`
- «El archivo original no está disponible en este equipo.» `ragChat.passageOriginalUnavailable`
- «El nombre de la conversación no puede estar vacío.» `ragChat.emptyConversationTitle`
- «Elegí al menos una colección para buscar en el corpus.» `ragChat.collectionsRequired`
- «Eliminar» `ragChat.confirmDelete`
- «Eliminar conversación» `ragChat.deleteConversation`
- «Entendiendo la pregunta…» `ragChat.stage.understanding`
- «Enviar» `ragChat.send`
- «Escribí tu pregunta…» `ragChat.placeholder`
- «Este pasaje no está en la biblioteca de este equipo (la conversación viene de otro dispositivo o la obra ya no está). Se muestra el fragmento guardado.» `ragChat.passageMissing`
- «Fuentes» `ragChat.sources`
- «Generando PDF…» `ragChat.downloadingConversation`
- «Hacé una pregunta sobre tus transcripciones y documentos. Las respuestas citan las fuentes.» `ragChat.emptyState`
- «Hay bibliotecas sincronizadas, pero todavía no tienen vectores para buscar por significado: los pasajes no se pueden consultar hasta que termine el indexado.» `ragChat.libraryStatusNoEmbeddings`
- «Leyendo las fuentes y escribiendo la respuesta…» `ragChat.stage.writing`

### Varios (2/2)

- «Limpiar búsqueda» `ragChat.searchClear`
- «Ninguna colección» `ragChat.collectionsNone`
- «No encontré contenido relevante en la base de conocimiento para esa pregunta.» `ragChat.noResults`
- «No se buscó en la Biblioteca: no hay ninguna de las bibliotecas elegidas sincronizada en EntropIA.» `ragChat.biblioNotice.noLibrarySynced`
- «No se buscó en la Biblioteca: no se pudo calcular el vector de la pregunta (revisá el proveedor de embeddings).» `ragChat.biblioNotice.embeddingUnavailable`
- «No se buscó en la Biblioteca: todavía no tiene vectores para buscar por significado (el indexado no terminó).» `ragChat.biblioNotice.noEmbeddings`
- «No se encontraron conversaciones» `ragChat.noMatchingConversations`
- «Nueva conversación» `ragChat.clear`
- «Ocurrió un error al consultar.» `ragChat.errorGeneric`
- «Original» `ragChat.passageOriginalEyebrow`
- «PDF descargado» `ragChat.downloadedConversation`
- «Pasajes indexados en {chunked} de {total} documentos con texto en este equipo. El resto se busca por página entera, con menos precisión.» `ragChat.chunkCoveragePartial`
- «Revisando las bibliotecas de Zotero…» `ragChat.libraryStatusLoading`
- «Se va a eliminar la conversación y sus mensajes. Esta acción no se puede deshacer.» `ragChat.deleteConversationMessage`
- «Sin conversaciones todavía» `ragChat.noConversations`
- «Texto de la captura» `ragChat.passageOriginalSnapshot`
- «Texto de la página {page}» `ragChat.passageOriginalText`
- «Todas las bibliotecas» `ragChat.librariesAll`
- «Todas las colecciones» `ragChat.collectionsAll`
- «Todavía no hay ninguna biblioteca de Zotero sincronizada en EntropIA. Sincronizá una con «Sincronizar biblioteca», en la pestaña Zotero de Escritura.» `ragChat.libraryStatusNone`
- «p. {n}» `ragChat.locationPage`
- «pp. {from}–{to}» `ragChat.locationPages`
- «párr. {from}–{to}» `ragChat.locationParagraphs`
- «párr. {n}» `ragChat.locationParagraph`
- «{count} de {total} bibliotecas» `ragChat.librariesSome`
- «{count} de {total} colecciones» `ragChat.collectionsSome`
- «{count} documento» `ragChat.collectionItemsOne`
- «{count} documentos» `ragChat.collectionItemsMany`
- «{count} documentos esperan sus pasajes: se indexan en segundo plano y, hasta que terminen, el chat no encuentra su texto nuevo.» `ragChat.embeddingQueueMany`
- «{works} obras · {passages} pasajes» `ragChat.libraryCounts`
- «¿Eliminar esta conversación?» `ragChat.deleteConversationTitle`

## Investigación

### status

- «Cobertura limitada: requiere confirmación»
- «En curso»
- «Esperando revisión humana»
- «Pausado»
- «Planeado»
- «Terminado»

### Varios

- «Alcance de colecciones» `research.collectionsLabel`
- «Anteriores» `research.jobsTitle`
- «Borrar» `research.confirmDelete`
- «Borrar alcance» `research.clearScope`
- «Borrar la investigación» `research.deleteTitle`
- «Cargando…» `research.loading`
- «Contexto traído desde el chat» `research.handoffTitle`
- «Contexto: {messages} mensajes y {sources} fuentes» `research.contextSummary`
- «Costo máximo» `research.maxCostLabel`
- «Deseleccionar todas» `research.deselectAll`
- «El contexto del chat se preserva como entrada, no como evidencia verificada.» `research.handoffDescription`
- «El presupuesto debe ser un número válido.» `research.invalidBudget`
- «Elegí al menos una biblioteca de Zotero sincronizada para buscar en la Biblioteca.» `research.needLibrary`
- «Elegí una o más colecciones explícitamente; el alcance nunca se adivina.» `research.collectionsHint`
- «Escribí una pregunta sobre material que ya esté en el archivo.» `research.formInvalid`
- «Investigación» `research.eyebrow`
- «Investigando…» `research.starting`
- «Investigar» `research.start, research.title`
- «Las llamadas LLM deben ser un entero mayor que cero.» `research.invalidCalls`
- «Llamadas LLM» `research.callsLabel`
- «Llamadas LLM máximas» `research.maxLlmCallsLabel`
- «Modelo» `research.modelLabel`
- «No hay colecciones disponibles para elegir.» `research.noCollections`
- «Nombre corto del proyecto» `research.projectPlaceholder`
- «Nueva investigación» `research.formTitle`
- «Presupuesto» `research.budgetLabel`
- «Profundizar con el Agente» `research.deepen`
- «Proyecto» `research.projectLabel`
- «Revisar mensajes preservados» `research.handoffPreview`
- «Se borra «{title}»: el informe, la evidencia, los juicios y los archivos. No se puede deshacer.» `research.deleteMessage`
- «Seleccionar todas» `research.selectAll`
- «Sin límite» `research.maxCostPlaceholder`
- «Todavía no hay investigaciones. Escribí una pregunta para empezar.» `research.empty`
- «Título de la investigación» `research.titlePlaceholder`
- «{count} colecciones» `research.collectionsMany`
- «{count} colección» `research.collectionsOne`
- «{count} fuentes» `research.handoffSources`
- «{count} seleccionada» `research.selectedCollectionsOne`
- «{count} seleccionadas» `research.selectedCollectionsMany`
- «¿Qué querés investigar?» `research.questionPlaceholder`

## Configuración

### Agente

- «Aclaración y consultas»
- «Archivo»
- «Bytes con que se parte una evidencia que no entra en un fragmento.»
- «Bytes máximos del insumo de un lote de archivo, de mayor a menor: cada reintento usa el tope siguiente.»
- «Candidatos de la fusión que entran al rerank.»
- «Candidatos por pierna — leg_k»
- «Candidatos que trae cada pierna de recuperación antes de fusionar.»
- «Caracteres de fuente que el informe reproduce por cita.»
- «Caracteres por fragmento que ven el reranker y la búsqueda léxica.»
- «Caracteres por fragmento — snippet_max»
- «Consultas por modalidad — max_consultas»
- «Consultas que el plan puede proponer en cada modalidad.»
- «Cuánto azar admite el modelo: más alto, más variación; más bajo, respuestas más literales.»
- «Cuánto pesa estar primero en una pierna frente a estar en las dos.»
- «Escalera de lotes de archivo — lotes_archivo_bytes»
- «Esperas entre reintentos — esperas_reintento_s»
- «Modelo y salida»
- «Parámetros del Agente de Investigación»
- «Por defecto: {value}»
- «Preguntas de la ronda de encuadre; si el modelo trae menos, se completan con los ejes de la modalidad.»
- «Preguntas mínimas — preguntas_minimas»
- «Profundidad del rerank — rerank_depth»
- «Recuperación»
- «Red y reintentos»
- «Restaurar valores por defecto»
- «Segundos de espera antes de cada reintento de red, separados por comas. Vacío deja un solo intento.»
- «Segundos máximos de espera por intento antes de abortar.»
- «Suavizado de la fusión — rrf_k»
- «Tamaño del fragmento — fragmento_archivo_bytes»
- «Temperatura — temperatura»
- «Timeout por intento — timeout_s»
- «Todos los parámetros del agente. Se congelan al crear la investigación: cambiarlos acá no altera las que ya existen.»
- «Tokens que el modelo puede escribir en un turno; el razonamiento también cuenta. Un tope corto corta las respuestas largas del archivo.»
- «Tope de salida — max_tokens_salida»
- «Ventana de cita — ventana_cita»

### Copias

- «Carpeta de las copias»
- «Copia automática»
- «Copia guardada en»
- «Copias a guardar»
- «Elegir carpeta»
- «Entre 1 y 100. Las más viejas se borran solas.»
- «Hacer una copia ahora»
- «Nunca»
- «Sin elegir»
- «Todos los días»
- «Una copia guarda la base de datos y todos los archivos de EntropIA (PDF, imágenes, audios) en una carpeta nueva. Para recuperar, se copia de vuelta.»
- «Una vez por semana»
- «ninguna todavía»
- «Última copia:»

### embeddingProvider

- «API OpenRouter»
- «Abrir carpeta BGE-M3»
- «Carpeta del modelo local BGE-M3»
- «Debe contener model.onnx, model.onnx_data y tokenizer.json. Si queda vacío, {product} usa la carpeta de datos de la app.»
- «Dejá baai/bge-m3 salvo que tu cuenta de OpenRouter use otro identificador compatible.»
- «Embeddings BGE-M3»
- «Estado BGE-M3 local»
- «Instalando BGE-M3…»
- «Instalar BGE-M3 local»
- «Local ONNX»
- «Modelo BGE-M3 local completo»
- «Modelo BGE-M3 local incompleto»
- «Modelo de embeddings»
- «Privacidad: si usás embeddings por API, el texto extraído se envía a OpenRouter para vectorizarlo.»
- «Usa OpenRouter con baai/bge-m3. Requiere API key y conexión.»
- «Usa un modelo BGE-M3 ONNX instalado en este equipo. Sin internet.»
- «Vacío = carpeta predeterminada en AppData»
- «{product} instala los archivos ONNX desde {repo} y conserva los nombres esperados por el proveedor local.»
- «{product} vectoriza por API usando OpenRouter con el contrato BGE-M3 de 1024 dimensiones.»

### glmOcrLocal

- «Descargando modelos…»
- «Descargar modelos (~1,4 GB)»
- «Falta llama-server (se instala con la app).»
- «Faltan modelos:»
- «Modelos GLM-OCR instalados.»
- «Reintentar descarga»

### Ayuda

- «Ayuda»
- «Cada vuelta es una consulta paga a OpenRouter (1-20)»
- «Contexto del manual (caracteres)»
- «Contexto: caracteres del manual y del mapa de pantallas enviados al modelo (4000-60000)»
- «Historial: cantidad de turnos previos incluidos (0-12)»
- «Largo máximo de la respuesta (tokens)»
- «Las preguntas que van al modelo se cobran en tu cuenta de OpenRouter. El diagnóstico y tus documentos solo viajan si los activás en cada pregunta.»
- «Modelo»
- «Modelo de la ayuda: vacío usa el modelo general de Configuración»
- «Modelo, contexto y límites de la ayuda en la app. Los valores mostrados son los vigentes.»
- «Parámetro de ayuda inválido: {param}»
- «Responde las preguntas frecuentes en este equipo, sin consultar a OpenRouter. Apagado: todas las preguntas van al modelo.»
- «Respuesta: tope de tokens generados por el modelo (200-8000)»
- «Respuestas locales»
- «Restaurar defaults»
- «Temperatura»
- «Temperatura: gradúa la creatividad de la respuesta (0-1)»
- «Turnos de historial»
- «Vueltas máximas del agente»

### llamaServer

- «Descargando llama-server…»
- «Descargar llama-server»
- «Falta llama-server, el programa que ejecuta estos modelos en la GPU. Se descarga el build de la placa de este equipo (CUDA, HIP, Vulkan o Metal).»
- «Falta llama-server: se descarga solo en equipos con GPU NVIDIA, o AMD, Intel o Apple con al menos 4 GB de memoria de video. Estos modelos no corren en el procesador.»
- «La GPU de este equipo tiene menos de {gb} GB de memoria de video: este modelo no entra, así que no se usa.»
- «Reintentar descarga»

### llmMode

- «API remota. Requiere API key y conexión a internet.»
- «Automático»
- «Local»
- «Motor LLM local vía llama.cpp. Sin conexión a internet.»
- «OpenRouter»
- «Prioriza el motor local y cae a OpenRouter cuando haga falta.»
- «Prioriza el motor local y usa OpenRouter sólo como respaldo.»
- «Procesa con el motor LLM local disponible en este equipo.»
- «Usa exclusivamente tu cuenta remota de OpenRouter.»

### localModel

- «Abrir carpeta de modelos»
- «Cargando estado del modelo local…»
- «Descarga completada»
- «Descargando modelo…»
- «Descargar modelo»
- «Descargá el modelo y colocalo en la carpeta indicada. El nombre de archivo debe ser exactamente:»
- «El motor local requiere un archivo GGUF. Sin él, las funciones de corrección OCR, resumen y extracción de triples usan OpenRouter (si está configurado) o quedan deshabilitadas.»
- «Encontrado»
- «Estado»
- «Fuente de descarga»
- «Indicá una fuente de descarga para el modelo local.»
- «Listo para descargar desde la app»
- «Modelo local (Gemma)»
- «No encontrado»
- «Nombre de archivo esperado»
- «Ruta esperada»
- «Tamaño»
- «Usá un nombre terminado en .gguf, sin rutas ni separadores.»
- «Usá una URL HTTPS válida y absoluta.»

### Modelos locales

- «Borrar modelos»
- «Corre PaddleOCR-VL en la GPU de este equipo con llama-server, también en AMD, Intel o Apple con 4 GB o más, no solo en NVIDIA. Se usa cuando el Modo OCRH es Local y el motor local es PaddleOCR-VL, o Automático sin GLM-OCR.»
- «Embeddings locales (BGE-M3)»
- «Modelo de OCR de alta precisión que corre en la GPU de este equipo (NVIDIA, o AMD, Intel o Apple con 4 GB o más). Se usa cuando el Modo OCRH es Local.»
- «Modelo opcional de OCRL, unas tres veces más rápido en CPU que el incluido. Se usa cuando el Perfil de OCRL (en Motores) es Rápido.»
- «Modelo que transcribe audio y dictado en este equipo, en 25 idiomas. Se usa cuando el Modo STT está en Local o Automático.»
- «Modelo que vectoriza el texto en este equipo para la búsqueda semántica y el Chat. Se usa cuando Embeddings está en Local ONNX.»
- «Necesita una GPU NVIDIA, o AMD, Intel o Apple con al menos {gb} GB de memoria de video: este equipo no tiene, así que no se descarga. Esta función usa el motor remoto o PaddleOCR-VL.»
- «OCRH local (GLM-OCR)»
- «OCRH local (PaddleOCR-VL en la GPU)»
- «OCRL rápido (PP-OCRv6 Tiny)»
- «Podés borrarlos para liberar unos {gb} GB.»
- «Se borran sus archivos de este equipo y se liberan unos {gb} GB. Para volver a usarlos hay que descargarlos de nuevo.»
- «Transcripción local (Parakeet)»
- «¿Borrar los modelos de {model}?»

### Mantenimiento

- «Abrir»
- «Abrí cada una y usá «Volver a correr el layout» en su pestaña Layout. Eso vuelve a hacer su OCR y reemplaza el texto extraído.»
- «Anotar también el origen de {count} layout anterior»
- «Anotar también el origen de {count} layouts anteriores»
- «Aplicar»
- «Busca imágenes que muestran una versión anterior a aquella sobre la que se calculó su layout. Primero lista lo que encuentra: no cambia nada hasta que lo confirmes.»
- «Cancelar»
- «El archivo cambió después de la revisión y no se aplicó nada. Volvé a revisar.»
- «El archivo de la imagen no está en disco»
- «Imágenes a reparar: {count}»
- «Imágenes con el layout de otra versión: {count}»
- «Imágenes con layout revisadas»
- «La imagen es más nueva que su layout: hay que volver a correr el layout»
- «La imagen mostrada coincide con su layout»
- «La versión que coincide se guardó después del layout: no se tocan»
- «Listo. Imágenes reparadas: {repaired}. Orígenes de layout anotados: {completed}.»
- «Mostrar {count} más»
- «Muestra {from}; el layout es de {to}»
- «Muestran una versión anterior a la de su layout»
- «Más de una versión coincide con el layout: no se tocan»
- «Ninguna»
- «Ninguna versión en disco coincide con el layout»
- «No hay imágenes para reparar.»
- «No se borra ni se modifica ningún archivo.»
- «Reparar versiones de imágenes»
- «Resultado de la revisión»
- «Revisando {done} de {total} imágenes…»
- «Revisando…»
- «Revisar»
- «Revisar versiones de imágenes»
- «Seleccionadas: {count}»
- «Seleccionar todas»
- «Son layouts calculados antes de que la app registrara de qué archivo salieron, y su imagen coincide sin ambigüedad. Solo se anota el archivo de origen.»
- «Vas a anotar el origen de {count} layout.»
- «Vas a anotar el origen de {count} layouts.»
- «Vas a cambiar la versión mostrada de {count} imagen.»
- «Vas a cambiar la versión mostrada de {count} imágenes.»

### Model Params

- «Configuración avanzada.»
- «Los valores mostrados son los defaults efectivos enviados a OpenRouter para OCR correction, Summary, NER y Triplets.»
- «Model Params»
- «Modelo OpenRouter usado exclusivamente por este proceso»
- «Parámetro inválido en {flow}: {param}»
- «Penalización de frecuencia: reduce repeticiones de palabras frecuentes (-2 a 2)»
- «Penalización de presencia: incentiva mencionar temas nuevos (-2 a 2)»
- «Restaurar defaults»
- «Secuencias de parada: hasta 4 coincidencias exactas, una por línea»
- «Temperatura: gradúa la creatividad de la respuesta generada (0-2)»
- «Tokens máximos: limita la longitud de la respuesta generada (1-16000)»
- «Top K: limita candidatos (0 conserva el default desactivado de OpenRouter)»
- «Top P: limita opciones según probabilidad acumulada (0-1)»
- «Una secuencia por línea»

### ocrhLocalEngine

- «Automático»
- «GLM-OCR local»
- «GLM-OCR local en la GPU cuando está instalado y la placa alcanza; si no, PaddleOCR-VL.»
- «Motor OCRH local»
- «PaddleOCR-VL»
- «Siempre GLM-OCR local. Requiere los modelos y una GPU NVIDIA, o AMD, Intel o Apple con 4 GB; falla si faltan.»
- «Siempre PaddleOCR-VL local.»

### ocrhMode

- «API remota. Requiere API key y conexión a internet.»
- «Automático»
- «GLM-OCR (z.ai)»
- «Intenta GLM-OCR y cae a local si falla o responde vacío.»
- «Local»
- «PaddleOCR-VL local con fallback a OCR plano cuando corresponda.»
- «Prioriza GLM-OCR cloud y usa OCRH local como fallback controlado.»
- «Usa GLM-OCR como proveedor cloud para el botón OCRH.»
- «Usa el pipeline local actual de OCR de alta precisión.»

### ocrlProfile

- «PP-OCRv6 Tiny no está descargado: OCRL usa PP-OCRv5 Latin hasta que lo descargues en Modelos locales.»
- «Perfil de OCRL»
- «Preciso (PP-OCRv5 Latin)»
- «Reconocimiento local liviano, sin API ni GPU. El perfil rápido lee unas tres veces más rápido y se equivoca un poco más en palabras con tilde.»
- «Rápido (PP-OCRv6 Tiny)»
- «Unas tres veces más rápido en CPU. Se descarga en Modelos locales (3 MB).»
- «Viene con la app. Recomendado para textos en español.»

### parakeetModel

- «Descargando Parakeet»
- «Descargar Parakeet ({size})»
- «El modelo Parakeet no está descargado: el dictado y la transcripción local no funcionan hasta descargarlo. Si la descarga se corta, continúa desde donde quedó.»
- «Modelo Parakeet descargado.»
- «Reintentar descarga»

### promptValidation

- «Debe incluir el placeholder {text}.»
- «El prompt no puede estar vacío.»
- «NER debe conservar estas etiquetas: {labels}.»
- «NER debe incluir el placeholder {text}.»
- «Prompt válido.»
- «Triplets debe conservar estas claves: {keys}.»
- «Triplets debe incluir el placeholder {text}.»

### Prompts

- «Estos templates son la fuente real usada en runtime. Usá»
- «Prompts»
- «Restaurar default»
- «Validar cambios»
- «para insertar el texto activo.»

### qwenCorrectLocal

- «Corrector local (Qwen3-VL)»
- «Corrige OCR en este equipo con un modelo de 4B cuando el modo LLM no es OpenRouter. Requiere GPU NVIDIA, o AMD, Intel o Apple con 6 GB; sin ella, la corrección usa el modelo remoto.»
- «Descargando modelos…»
- «Descargar modelos (~2,9 GB)»
- «Faltan modelos:»
- «Modelos Qwen instalados.»
- «Reintentar descarga»

### RAG Params

- «Candidatos fusionados: cuántos sobreviven a la combinación de búsquedas (4-200)»
- «Candidatos por búsqueda: amplía opciones antes de combinar resultados (4-200)»
- «Contexto: limita el texto total enviado al modelo (1000-350000; Pro local admite ventana de 131K tokens)»
- «Documentos: define cuántos resultados entran al contexto (1-20)»
- «Estos parámetros ajustan la recuperación del chat de investigación. Los valores mostrados son los vigentes.»
- «Fragmento: limita caracteres tomados de cada resultado (200-8000)»
- «Historial: cantidad de turnos previos incluidos (0-20)»
- «Modelo del chat: hereda el modelo general de OpenRouter; cambialo acá para que el chat use otro»
- «Modelo del reranker (solo variante API/OpenRouter): reordena los candidatos antes de responder»
- «Parámetro RAG inválido: {param}»
- «Pasaje: limita cada pasaje indexado con sus vecinos; si lo excede, se recorta por pasajes enteros (800-12000)»
- «Profundidad de reordenamiento: cuántos candidatos se reevalúan; menos es más rápido (1-200)»
- «RAG Params»
- «RRF K: suaviza el ranking al fusionar búsquedas (1-500)»
- «Respuesta: limita tokens generados por el modelo (64-16000)»
- «Restaurar defaults»
- «Similitud mínima: filtra resultados débiles; 0 desactiva el filtro (0-1)»
- «Temperatura: gradúa la creatividad del modelo (0-2)»
- «Turno histórico: limita caracteres por mensaje previo (100-4000)»
- «fusionCandidateLimit no puede ser menor que topK.»
- «rerankDepth debe estar entre topK y fusionCandidateLimit.»
- «snippetMaxChars no puede superar contextMaxChars.»

### reranker

- «Abrir carpeta del reranker»
- «BGE Reranker v2 M3 vuelve a ordenar los candidatos RAG en este equipo antes de generar una respuesta.»
- «Descarga el modelo ONNX INT8 verificado desde {repo}. Tamaño aproximado: 561 MB.»
- «Instalando reranker…»
- «Instalar reranker local»
- «Listo para usar»
- «Modelo sin instalar»
- «Reranker RAG local»

### sttLocalEngine

- «Motor STT local»
- «Parakeet (rápido)»
- «Transcripción local con Parakeet TDT. Descarga el modelo una vez (~2,5 GB).»
- «Whisper»
- «faster-whisper local (respaldo si el idioma no está entre los 25 de Parakeet).»

### sttMode

- «API remota. Requiere API key y conexión a internet.»
- «AssemblyAI»
- «Audio local. No sale de tu dispositivo.»
- «Automático»
- «Intenta local primero y cae a AssemblyAI si hace falta.»
- «Local»
- «Prioriza el motor local y usa AssemblyAI como fallback.»
- «Procesa el audio en este equipo, sin subirlo a ningún servicio.»
- «Usa exclusivamente AssemblyAI como proveedor cloud de voz a texto.»

### vercel

- «Borrar»
- «Clave de Vercel AI Gateway»
- «Clave guardada»
- «Conexión lista: Vercel contestó.»
- «Guardar»
- «La clave es válida, pero Vercel no atiende pedidos hasta que cargues una tarjeta de crédito en tu cuenta (eso libera también el crédito gratis).»
- «Opcional. Si OpenRouter no responde, está saturado o se quedó sin crédito, los pedidos de texto van a Vercel con el mismo modelo.»
- «Respaldo: Vercel AI Gateway»

### zotero

- «API key de Zotero»
- «Biblioteca personal: lectura y escritura»
- «Biblioteca personal: sin acceso»
- «Biblioteca personal: solo lectura»
- «Cuenta de Zotero · id {userId}»
- «Cuenta de Zotero: {username} · id {userId}»
- «Grupo {id}: lectura y escritura»
- «Grupo {id}: solo lectura»
- «Grupos: sin acceso»
- «Guardá una API key de la Web API de Zotero para completar los datos de referencias que ya están en tu biblioteca y adjuntarles PDFs. Queda en el almacén de credenciales del sistema.»
- «Ingresá una API key de Zotero antes de verificarla.»
- «Los cambios hechos por la Web API llegan a tu Zotero local cuando Zotero sincroniza.»
- «No se pudo contactar a Zotero. Revisá tu conexión e intentá de nuevo.»
- «Pegá tu API key de Zotero»
- «Todos tus grupos: lectura y escritura»
- «Todos tus grupos: solo lectura»
- «Verificando...»
- «Verificar clave»
- «Zotero»
- «Zotero no reconoce esta clave. Revisala o creá una nueva.»

### Varios (1/3)

- «API Key» `settings.apiKey`
- «APIs remotas» `settings.remoteApisTab`
- «Activado» `settings.optionEnabled`
- «Agente» `settings.agentParamsTab`
- «Ajustá proveedores, credenciales y parámetros de inteligencia artificial para {product}.» `settings.subtitle`
- «Alias» `settings.aliasesTab`
- «Apariencia» `settings.appearanceTab`
- «Asistentes» `settings.assistantsTab`
- «AssemblyAI» `settings.assemblyai.title`
- «Ayuda» `settings.helpTab`
- «Borrar PP-OCRv6 Tiny» `settings.ppocrV6Local.delete`
- «Cambios sin guardar» `settings.unsavedChanges`
- «Cargando la configuración…» `settings.loading`
- «Clave cargada: {key}» `settings.loadedKey`
- «Clave en formato anterior (texto plano); guardá de nuevo para protegerla en el almacén de credenciales» `settings.keyStoredLegacyPlaintext`
- «Clave guardada en el almacén del sistema» `settings.keyStoredInCredentialManager`
- «Conexión lista · AssemblyAI validó tu cuenta.» `settings.assemblyAiConnectionReady`
- «Conexión lista · GLM-OCR aceptó la solicitud de prueba.» `settings.glmOcrConnectionReady`
- «Conexión lista · {count} modelos disponibles.» `settings.connectionReady`
- «Configuración» `settings.title`
- «Configuración guardada. Ya podés usar esta preferencia en toda la app.» `settings.saved`
- «Configurá la cuenta remota para OCR de alta precisión, validá el acceso y usala como proveedor OCRH cloud.» `settings.glmOcr.description`
- «Configurá la cuenta remota para transcripción, validá el acceso y usala como proveedor STT cloud.» `settings.assemblyai.description`
- «Configurá la cuenta remota, validá el acceso y elegí los modelos de generación y embeddings.» `settings.openrouter.description`
- «Contraste» `settings.appearance.contrastLabel`
- «Copias» `settings.backupTab`
- «Dependencias de IA» `settings.dependenciesTab`
- «Desactivada: EntropIA no envía nada a {service}. La clave queda guardada; marcá la casilla para volver a usarla.» `settings.apiSwitch.offNow`
- «Desactivada: EntropIA no envía nada a {service}. La clave queda guardada; marcá la casilla y guardá para volver a usarla.» `settings.apiSwitch.off`
- «Desactivado» `settings.optionDisabled`
- «Descargable» `settings.badge.downloadable`
- «Descargar modelos (3 MB)» `settings.ppocrV6Local.download`
- «Descargar modelos (~1,8 GB)» `settings.paddleVlLocal.download`
- «Descartar» `settings.discardConfirm`
- «Descartar cambios» `settings.discardTitle`
- «Detectar hablantes» `settings.sttLocalDiarize.label`
- «Disponible» `settings.badge.available`
- «El índice de búsqueda se actualiza en segundo plano.» `settings.searchIndex.rebuilding`

### Varios (2/3)

- «Elegí el idioma de la UI. El contenido del usuario no se traduce.» `settings.languageDescription`
- «En el texto de OCR, «trabaja-» al final de una línea y «dores» al comienzo de la siguiente se buscan como «trabajadores». El texto guardado no cambia.» `settings.searchIndex.rejoinHyphens.description`
- «English» `settings.languageOptionEn`
- «EntropIA usa {service} cuando una función lo necesita.» `settings.apiSwitch.on`
- «Español» `settings.languageOptionEs`
- «Etiqueta cada segmento con Hablante A, B… (máximo 4, solo Parakeet). Requiere descargar el modelo de diarización (~400 MB).» `settings.sttLocalDiarize.description`
- «GLM-OCR (z.ai)» `settings.glmOcr.title`
- «Guardar cambios» `settings.save`
- «Identificación de hablantes en audio de colección» `settings.assemblyAiSpeakerLabels`
- «Idioma» `settings.languageLabel`
- «Idioma de la interfaz» `settings.languageTitle`
- «Ingresá una API key antes de probar la conexión.» `settings.enterApiKey`
- «Ingresá una API key de AssemblyAI antes de probar la conexión.» `settings.enterAssemblyAiApiKey`
- «Ingresá una API key de GLM-OCR antes de probar la conexión.» `settings.enterGlmOcrApiKey`
- «Ir a Configuración» `settings.modelList.openSettings`
- «Logs» `settings.logsTab`
- «Lotes» `settings.batchTab`
- «Mantenimiento» `settings.maintenanceTab`
- «Model Params» `settings.modelParamsTab`
- «Modelo generativo» `settings.model`
- «Modelo no encontrado» `settings.badge.notFound`
- «Modelos PP-OCRv6 Tiny instalados.» `settings.ppocrV6Local.modelsReady`
- «Modelos PaddleOCR-VL instalados.» `settings.paddleVlLocal.modelsReady`
- «Modelos locales» `settings.localModelsTab`
- «Modelos sugeridos desde OpenRouter» `settings.suggestedModels`
- «Modo LLM» `settings.llmModeTitle`
- «Modo OCRH» `settings.ocrhModeTitle`
- «Modo STT» `settings.sttModeTitle`
- «Mostrar API key» `settings.showApiKey`
- «Motores» `settings.enginesTab`
- «No está en el catálogo de OpenRouter: se valida al crear.» `settings.modelList.unknownModel`
- «No se pudo guardar la clave: el sistema no tiene un llavero disponible. En Linux, instalá gnome-keyring o KWallet y creá un llavero predeterminado (por ejemplo, con la app «Contraseñas y claves»); después volvé a guardar.» `settings.credentialStoreUnavailable`
- «Obtener API key» `settings.getApiKeyShort`
- «Obtener API key en {provider}» `settings.getApiKeyLink`
- «Ocultar API key» `settings.hideApiKey`
- «OpenRouter» `settings.openrouter.title`
- «Para elegir un modelo de la lista, primero cargá la clave de OpenRouter.» `settings.modelList.noKey`
- «Preferencias» `settings.preferences`

### Varios (3/3)

- «Privacidad: si usás AssemblyAI o modo automático con fallback cloud, el audio se envía a un proveedor externo para transcribirlo.» `settings.sttPrivacyNotice`
- «Privacidad: si usás GLM-OCR o modo automático con fallback cloud, la imagen o PDF se envía a un proveedor externo para OCR.» `settings.ocrhPrivacyNotice`
- «Probando...» `settings.testingConnection`
- «Probar conexión» `settings.testConnection`
- «Prompts» `settings.promptsTab`
- «Quitar clave» `settings.clearKeyConfirm`
- «Quitar clave de {service}» `settings.clearKey`
- «RAG Params» `settings.ragParamsTab`
- «Reintentar carga» `settings.retryLoad`
- «Se va a eliminar la clave guardada de {service}. Las funciones que dependan de ella van a quedar sin configurar hasta que cargues una nueva.» `settings.clearKeyMessage`
- «Secciones de configuración» `settings.tabsAria`
- «Seguir editando» `settings.discardCancel`
- «Servidor OpenAI-compatible (opcional)» `settings.openaiCompatibleBaseUrl.label`
- «Si lo completás, la generación remota va a ese servidor (vLLM, Ollama, pasarela propia) en vez de OpenRouter, usando el modelo ya configurado. Sin clave: ideal para un servidor local.» `settings.openaiCompatibleBaseUrl.description`
- «Sin clave guardada» `settings.keyNotConfigured`
- «Sincronización» `settings.syncTab`
- «Sólo aplica a transcripciones STT de audios de colección. El dictado de notas siempre envía speaker_labels=false.» `settings.assemblyAiSpeakerLabelsHint`
- «Tema» `settings.appearance.themeLabel`
- «Tenés cambios sin guardar en la configuración. ¿Querés descartarlos y salir?» `settings.discardMessage`
- «Unir palabras cortadas con guion al final de línea» `settings.searchIndex.rejoinHyphens.label`
- «Usar esta API» `settings.apiSwitch.label`
- «aai-...» `settings.assemblyAiApiKeyPlaceholder`
- «openai/gpt-6-luna» `settings.modelPlaceholder`
- «sk-or-v1-...» `settings.apiKeyPlaceholder`
- «zai-... / sk-...» `settings.glmOcrApiKeyPlaceholder`
- «{count} más: seguí escribiendo para acotar» `settings.modelList.moreMatches`
- «¿Quitar la clave de {service}?» `settings.clearKeyTitle`
- «Índice de búsqueda» `settings.searchIndex.title`

## Actualizaciones

### Varios

- «Actualización disponible» `storeUpdate.title`
- «Cerrar» `storeUpdate.close`
- «Cerrar aviso de actualización» `storeUpdate.closeLabel`
- «Hay una actualización de EntropIA Lite disponible en Microsoft Store.» `storeUpdate.body`
- «Ver actualización» `storeUpdate.view`

## Sincronización

### card (1/2)

- «Actualizar conflictos»
- «Actualizar lista»
- «Actualizar uso»
- «Almacenamiento del servidor»
- «Archivos: {count} ({size})»
- «Borra todos tus datos y archivos del servidor. Tus datos locales no se tocan.»
- «Borrar datos del servidor»
- «Borrar mis datos del servidor»
- «Borrar todo»
- «Cancelar» `sync.card.deleteAccountCancel, sync.card.fullResyncCancel, sync.card.revokeDeviceCancel`
- «Cerrando sesión…»
- «Cerrar sesión»
- «Conflicto de sincronización: {reason} en {table} · {row}»
- «Conflictos»
- «Contraseña»
- «Cuenta registrada. Ya podés iniciar sesión.»
- «Cuota: {used} (sin límite)»
- «Cuota: {used} de {total}»
- «Datos del servidor borrados. Sesión local cerrada.»
- «Desconectado»
- «Dispositivos»
- «El dispositivo «{name}» perderá el acceso a la cuenta. ¿Confirmás?»
- «Email»
- «Este dispositivo»
- «Esto borra de forma permanente todos tus datos y archivos del servidor. Tus datos locales quedan intactos. Ingresá tu contraseña para confirmar.»
- «Filas: {count}»
- «Fuerza una re-subida de tus archivos. Útil tras restaurar el servidor.»
- «Iniciando sesión…»
- «Iniciar sesión»
- «Intervalo (minutos)»
- «La sincronización está desactivada en el perfil de desarrollo aislado: esta copia no toca tu cuenta ni tu sesión.»
- «Marcar como visto»
- «Marcar vistos»
- «Motivo: {reason}»
- «Mínimo 10 caracteres»
- «No hay conflictos pendientes.»
- «No hay otros dispositivos en esta cuenta.»
- «No hay una sesión de sincronización activa.»
- «Ocultar detalle»
- «Plan: {plan}»
- «Re-verificación encolada.»
- «Re-verificar archivos»
- «Registrando…»
- «Registrar cuenta»
- «Rehacer sincronización»
- «Rehacer sincronización completa»
- «Resincronización completa en marcha.»
- «Revocar» `sync.card.deviceRevoke, sync.card.revokeDeviceConfirm`
- «Revocar dispositivo»
- «Se vuelve a descargar todo lo que está en el servidor para corregir diferencias entre equipos. Los cambios que todavía no se subieron se conservan. Puede tardar unos minutos.»
- «Sesión»
- «Sesión iniciada como {email}»
- «Sincronización automática»
- «Sincronización completa.»
- «Sincronización en la nube»
- «Sincronizando…»
- «Sincronizar ahora»
- «Sincronizar automáticamente»
- «Sincronizá tus datos y archivos entre dispositivos. Es opcional: si no la activás, nada sale de tu equipo.»
- «Tabla {table} · fila {row}»
- «Todavía no se sincronizó»
- «URL del servidor»
- «Usá una URL https (o http solo para localhost).»
- «Ver detalle»
- «Visto: {when}»
- «Vuelve a bajar todo del servidor. Sirve si dos equipos muestran datos distintos.»
- «Zona de riesgo»
- «https://sync.tu-dominio.com»
- «tu@email.com»

### card (2/2)

- «{count} conflictos pendientes. El detalle se registró como advertencia en Logs.»
- «¿Rehacer la sincronización completa?»

### notif

- «Cargando notificaciones…»
- «Cerrar»
- «Eliminar notificación»
- «Marcar como leída»
- «Marcar todas como leídas»
- «No tenés notificaciones.»
- «Notificaciones» `sync.notif.bellLabel, sync.notif.panelTitle`
- «Notificaciones, {count} sin leer»
- «Sin leer»

### statusbar

- «Abrir configuración de sincronización»
- «Archivos pendientes: {count}»
- «Cambios pendientes: {count}»
- «Conflictos: {count}»
- «Estado de sincronización»
- «Revisá el reloj del dispositivo»
- «Sin conexión»
- «Sincronización al día»
- «Sincronizando…»
- «Todavía no se sincronizó»
- «{count} conflictos sin resolver»
- «Última sincronización: {when}»

### upgrade

- «Cancelar»
- «Contanos brevemente para qué necesitás más espacio…»
- «Elegí un plan…»
- «Enviando…»
- «Enviar solicitud»
- «Esto es una SOLICITUD que revisa un administrador. No es un pago: no se cobra nada al enviarla.»
- «Plan actual»
- «Plan al que querés cambiar»
- «Solicitar cambio de plan» `sync.upgrade.button, sync.upgrade.title`
- «Solicitud en revisión»
- «Solicitud en revisión: {plan}»
- «gratis»
- «sin límite»
- «{name} · {quota}»
- «¿Para qué lo necesitás? (opcional)»

### Varios

- «Cancelar» `sync.preflight.cancel`
- «Esta primera sincronización subirá aproximadamente {size} de archivos. Puede tardar y consumir datos. ¿Continuás?» `sync.preflight.message`
- «Primera sincronización» `sync.preflight.title`
- «Sí, sincronizar» `sync.preflight.confirm`

## Pestañas

### Varios

- «Abrir nueva pestaña» `tabs.newAria`
- «Cerrar pestaña {title}» `tabs.closeAria`
- «Límite de 4 pestañas alcanzado» `tabs.newDisabledTitle`
- «Nueva pestaña» `tabs.new`
- «Pestañas abiertas» `tabs.list`

## Barra superior

### Varios

- «Abrir Biblioteca» `topbar.bibliotecaAria`
- «Abrir Colecciones» `topbar.collectionsAria`
- «Abrir Escritura» `topbar.writingAria`
- «Abrir Inicio» `topbar.homeAria`
- «Abrir agente de investigación» `topbar.researchAria`
- «Abrir chat de investigación» `topbar.ragChatAria`
- «Abrir configuración» `topbar.settingsAria`
- «Abrir el Navegador» `topbar.navegadorAria`
- «Abrir navegador de base de datos» `topbar.dbBrowserAria`
- «Agente de investigación» `topbar.researchTitle`
- «Alternar vista dividida» `topbar.splitAria`
- «Aumentar zoom» `topbar.zoomIn`
- «Base de datos» `topbar.dbBrowserTitle`
- «Biblioteca» `topbar.bibliotecaTitle, topbar.searchBibliographyGroup`
- «Breadcrumb» `topbar.breadcrumb`
- «Buscando...» `topbar.searchSearching`
- «Buscar documentos por nombre o texto» `topbar.searchAria`
- «Buscar por nombre o texto...» `topbar.searchPlaceholder`
- «Chat de investigación» `topbar.ragChatTitle`
- «Colecciones» `topbar.collectionsTitle`
- «Configuración» `topbar.settingsTitle`
- «Dependencias de IA pendientes» `topbar.depsWarningAria`
- «Dependencias de IA pendientes - click para configurar» `topbar.depsWarningTitle`
- «Documento anterior» `topbar.previousDocument`
- «Documento siguiente» `topbar.nextDocument`
- «Eliminar página activa» `topbar.deleteAssetAria`
- «Escritura» `topbar.writingTitle`
- «Inicio» `topbar.homeTitle`
- «La ventana es muy angosta para la vista dividida» `topbar.splitDisabledTitle`
- «Limpiar búsqueda» `topbar.searchClear`
- «Navegador» `topbar.navegadorTitle`
- «Reducir zoom» `topbar.zoomOut`
- «Restablecer zoom» `topbar.zoomReset`
- «Sin resultados para "{query}"» `topbar.searchNoResults`
- «Usar tema menos oscuro» `topbar.themeDim`
- «Usar tema oscuro» `topbar.themeDark`
- «Vista dividida» `topbar.splitTitle`
- «Zoom» `topbar.zoomTitle`
- «Zoom actual: {value}%» `topbar.zoomLevelAria`
- «← Volver» `topbar.back`

## Tipografía

### Varios

- «Académica» `typography.academic`
- «Archivo» `typography.archive`
- «Editorial» `typography.editorial`
- «Investigación, archivo y escritura» `typography.sampleReading`
- «Moderna» `typography.modern`
- «Tipografía» `typography.title`

## Paneles

### Varios

- «El Navegador está abierto en otra pestaña.» `workpane.navegadorElsewhereNotice`
- «Escritura está abierta en otra pestaña.» `workpane.writingElsewhereNotice`
- «Ir a esa pestaña» `workpane.writingElsewhereAction`

## Escritura

### agentAction

- «Acortar»
- «Buscar contraejemplos»
- «Buscar evidencia»
- «Buscar evidencia en contra»
- «Corregir ortografía»
- «Desarrollar»
- «Detectar contradicciones»
- «Detectar reiteraciones»
- «Mejorar argumentación»
- «Mejorar claridad»
- «Reformular»
- «Relacionar con mis notas»
- «Resumir»
- «Revisar redacción»

### agentActionShort

- «Acortar»
- «Argumentación»
- «Claridad»
- «Contradicciones»
- «Contraejemplos»
- «Contraevidencia»
- «Desarrollar»
- «Evidencia»
- «Notas»
- «Ortografía»
- «Redacción»
- «Reformular»
- «Reiteraciones»
- «Resumir»

### color

- «Amarillo»
- «Azul»
- «Gris»
- «Naranja»
- «Rojo»
- «Rosa»
- «Verde»
- «Violeta»

### editor

- «Contiene un elemento que esta versión de EntropIA no conoce. El documento no se modificó.»
- «Contiene un formato que esta versión de EntropIA no conoce. El documento no se modificó.»
- «El documento está intacto y no se modificó. El detalle técnico es este:»
- «Este documento no se puede abrir en esta versión»
- «Fue escrito por una versión más nueva de EntropIA. El documento no se modificó.»
- «Manuscrito»
- «No se pudo abrir el editor»
- «Su contenido no se pudo interpretar. El documento no se modificó.»

### exportElement

- «Alineación de párrafo»
- «Citas Zotero»
- «Citas documentales»
- «Citas en bloque»
- «Interlineado»
- «Jerarquía de títulos»
- «Listas»
- «Listas numeradas»
- «Notas al pie»
- «Resaltado»
- «Sangría de párrafo»
- «Subrayado»
- «Subíndice»
- «Superíndice»
- «Tablas»
- «Tamaño y color de texto»
- «Vínculos a notas»

### image

- «Alinear imagen a la derecha»
- «Alinear imagen a la izquierda»
- «Centrar imagen»
- «Imagen no disponible»
- «Pie de foto»
- «Redimensionar imagen»
- «Texto alternativo»

### stats

- «Caracteres (con espacios)»
- «Caracteres (sin espacios)»
- «Estadísticas»
- «Notas al pie»
- «Palabras»
- «Párrafos»

### sync

- «Copia por conflicto»
- «Es una copia guardada después de un conflicto de sincronización.»
- «Estado de sincronización»
- «Faltan imágenes o archivos por transferir.»
- «Hay cambios todavía sin enviar.»
- «Hay una actualización de otro equipo esperando para aplicarse.»
- «Sincronización pendiente»

### tab

- «Agente»
- «Corpus»
- «Exportar»
- «Notas»
- «Obras»
- «Zotero»

### tabPending

- «Acá el agente va a proponer texto con su procedencia a la vista.»
- «Acá elegís cómo se exportan las citas y si se incluye la bibliografía. El formato se elige al descargar.»
- «Acá vas a buscar en tus colecciones e insertar citas documentales que vuelvan a la fuente.»
- «Acá vas a buscar obras por tema, autor o título sobre los perfiles verificados.»
- «Acá vas a citar tu biblioteca de Zotero y armar la bibliografía.»
- «Acá vas a tener a mano las notas de los documentos mientras escribís.»

### toolbar

- «+ Columna»
- «+ Fila»
- «1»
- «1,15»
- «1,5»
- «2»
- «Alinear a la derecha»
- «Alinear a la izquierda»
- «Aumentar sangría»
- «Aumentar tamaño de fuente»
- «Borrar formato»
- «Buscar»
- «Cambiar mayúsculas y minúsculas»
- «Capitalizar palabras»
- «Centrar»
- «Cerrar la búsqueda»
- «Cita en bloque»
- «Coincidencia anterior»
- «Coincidencia siguiente»
- «Color de resaltado»
- «Color de texto»
- «Controles de tabla»
- «Cursiva»
- «Código»
- «Deshacer»
- «Disminuir sangría»
- «Disminuir tamaño de fuente»
- «Eliminar tabla»
- «Enlace»
- «Formato»
- «Insertar imagen»
- «Insertar tabla»
- «Interlineado»
- «Justificar»
- «Lista»
- «Lista ordenada»
- «MAYÚSCULAS»
- «Más herramientas»
- «Negrita»
- «Nota al pie»
- «Predeterminado»
- «Quitar enlace»
- «Reemplazar»
- «Reemplazar por»
- «Reemplazar todo»
- «Rehacer»
- «Sin coincidencias»
- «Sin color»
- «Subrayado»
- «Subíndice»
- «Superíndice»
- «Tachado»
- «Tipo oración»
- «Título 1»
- «Título 2»
- «Título 3»
- «minúsculas»
- «− Columna»
- «− Fila»

### trash

- «Eliminado el {date}»
- «La papelera está vacía.»
- «Ocultar la papelera»
- «Papelera»
- «Restaurar»
- «Restaurar «{title}»»
- ««{title}» volvió a la lista.»

### Varios (1/8)

- «(sin verificar)» `writing.zoteroLibraryUnverified`
- «45, 45-50, 3…» `writing.citeLocatorPlaceholder`
- «<1 min» `writing.zoteroEtaUnderMinute`
- «Abrir en Zotero» `writing.zoteroDetailOpenInZotero`
- «Abrir en Zotero todavía está en verificación en esta versión. Probá de nuevo más tarde.» `writing.zoteroDetailOpenPending`
- «Abrir en el panel» `writing.publishOpenAdmin`
- «Abrir la fuente» `writing.sourceOpen`
- «Abrí un documento para insertar la cita.» `writing.zoteroNoDocument`
- «Abrí un documento para insertar la nota.» `writing.notesNoDocument`
- «Abrí un documento para poder insertar la cita.» `writing.corpusNoDocument`
- «Acciones» `writing.agentActions`
- «Acciones de la sección» `writing.sectionActions`
- «Actualizar» `writing.zoteroReload`
- «Adjuntos» `writing.zoteroDetailAttachments`
- «Agregar» `writing.zoteroLibraryAdd`
- «Agregar biblioteca» `writing.zoteroAddLibrary`
- «Agregar una sección debajo» `writing.sectionAdd`
- «Ajustar la cita» `writing.citeDialogTitle`
- «Ancho del esquema» `writing.outlineWidth`
- «Ancho del panel de investigación» `writing.researchWidth`
- «Antes de la cita» `writing.citePrefix`
- «Antes de recibir cambios de otra cuenta» `writing.historyBeforeReceive`
- «Antes de restaurar otra versión» `writing.historyBeforeRestore`
- «Apartado» `writing.historySectionLabel`
- «Aproximado: {words}» `writing.corpusFoundAs`
- «Archivo» `writing.corpusFile`
- «Archivos» `writing.corpusPages`
- «Asistencia del agente» `writing.agentTitle`
- «Así queda» `writing.citePreview`
- «Autores» `writing.zoteroDetailCreators`
- «Bajar la sección» `writing.sectionDown`
- «Bibliografía» `writing.exportBibliography`
- «Biblioteca» `writing.zoteroLibrary`
- «Biblioteca al día» `writing.zoteroBibliographySyncUpToDate`
- «Buscando cambios en Zotero…» `writing.zoteroSyncing`
- «Buscando en tus notas…» `writing.agentSearchingNotes`
- «Buscando evidencia en el corpus…» `writing.agentSearching`
- «Buscando…» `writing.corpusSearching, writing.notesSearching`
- «Buscar en el corpus» `writing.corpusSearch`
- «Buscar en las notas» `writing.notesSearch`
- «Buscar en tu biblioteca de Zotero» `writing.zoteroSearch`
- «Buscá el documento al que pertenece la nota» `writing.noteCreateTarget`
- «Buscá en tus colecciones para citar una fuente.» `writing.corpusStart`
- «Buscá entre tus notas para citarlas o vincularlas.» `writing.notesStart`
- «Búsqueda por significado no disponible: sin espacio semántico activo. Solo se muestran coincidencias de texto.» `writing.zoteroSemanticLexicalOnly`
- «Cambios pendientes» `writing.status.pending`

### Varios (2/8)

- «Cancelar» `writing.citeCancel, writing.discardCancel, writing.noteCreateCancel, writing.sectionDeleteCancel, writing.zoteroLibraryCancel`
- «Capítulo» `writing.locatorChapter`
- «Cerrar» `writing.exportClose, writing.noteDismiss, writing.sourceDismiss`
- «Cita insertada en el manuscrito.» `writing.corpusInserted, writing.zoteroCited`
- «Citar» `writing.zoteroCite`
- «Citas del corpus» `writing.exportCitations`
- «Clave para publicar en hlab.com.ar» `writing.publishKey`
- «Colecciones» `writing.zoteroDetailCollections`
- «Comentario» `writing.exportCiteComment`
- «Compartido» `writing.shared`
- «Compartido por {email}» `writing.shareFrom`
- «Compartir» `writing.shareAdd`
- «Compartir con otra cuenta» `writing.shareTitle`
- «Compartir y publicar» `writing.shareToggle`
- «Consultando al modelo…» `writing.agentAsking`
- «Copia local sin verificar ahora» `writing.zoteroDetailLocalCopy`
- «DOI» `writing.zoteroDetailDoi`
- «Dejar de recibir cambios» `writing.shareLeave`
- «Descargar» `writing.download`
- «Descargar como HTML» `writing.downloadHtml`
- «Descargar como Markdown» `writing.downloadMarkdown`
- «Descargar como Word (.docx)» `writing.downloadDocx`
- «Descartar» `writing.agentDiscard`
- «Descartar aviso» `writing.repairedDismiss`
- «Después de la cita» `writing.citeSuffix`
- «Documento nuevo» `writing.newDocument`
- «Editorial» `writing.zoteroDetailPublisher`
- «El documento no se modificó y la cita sigue en su lugar. Esto es lo que quedó registrado cuando la insertaste:» `writing.sourceMissingBody`
- «El espacio de escritura todavía se está preparando.» `writing.notReady`
- «El servidor de sincronización todavía no permite compartir.» `writing.shareUnavailable`
- «El texto en ese lugar ya no es el que citaste, así que no se resalta para no señalar la oración equivocada. Se abre la página igual. Lo que registraste fue:» `writing.sourceModifiedBody`
- «El texto queda independiente: si la nota cambia después, tu documento no.» `writing.notesCopyHelp`
- «El texto sobre el que se hizo esta propuesta cambió desde entonces. Revisala de nuevo antes de aplicarla; también podés descartarla.» `writing.agentTargetChanged`
- «El vínculo queda vivo: si la nota cambia, te lo avisamos sin tocar tu texto.» `writing.notesLinkHelp`
- «Elegí una acción» `writing.agentPick`
- «Elegí una versión para verla.» `writing.historyPick`
- «Eliminar» `writing.discardConfirm`
- «Eliminar la sección» `writing.sectionDeleteConfirm`
- «Eliminar la sección y su contenido» `writing.sectionDelete`
- «Eliminar «{title}»» `writing.discard`
- «Empezá a escribir…» `writing.placeholder`

### Varios (3/8)

- «En pausa: Zotero no responde. Se reintenta solo cuando conteste.» `writing.zoteroBibliographySyncWaiting`
- «En pausa: la API local de Zotero está apagada. Activala en Zotero y volvé a sincronizar.» `writing.zoteroBibliographySyncApiDisabled`
- «En pausa: {detail}» `writing.zoteroBibliographySyncPaused`
- «Enlace» `writing.zoteroDetailUrl`
- «Enviar a hlab.com.ar» `writing.publishSend, writing.publishTitle`
- «Esa biblioteca no se encontró en Zotero. Revisá el tipo y el ID.» `writing.zoteroLibraryNotFound`
- «Escribir sin perder el camino hasta las fuentes.» `writing.subtitle`
- «Escritura» `writing.eyebrow, writing.title`
- «Ese apartado no está en esta versión o en el manuscrito actual.» `writing.historyRestoreSectionMissing`
- «Esquema» `writing.outline`
- «Esta biblioteca todavía no está sincronizada en EntropIA: solo se muestran coincidencias de texto. «Sincronizar biblioteca» habilita también la búsqueda por significado.» `writing.zoteroSemanticNotSynced`
- «Esta ficha no trae una biblioteca válida para abrirla en Zotero.» `writing.zoteroDetailOpenInvalidLibrary`
- «Esta ficha no trae una clave válida para abrirla en Zotero.» `writing.zoteroDetailOpenInvalidKey`
- «Esta página todavía no fue leída, así que no hay texto para citar.» `writing.corpusNoText`
- «Esta sugerencia ya estaba resuelta, así que no se aplicó de nuevo.» `writing.agentAlreadyResolved`
- «Este documento todavía no tiene páginas procesadas.» `writing.corpusNoPages`
- «Estilo de cita» `writing.citeStyle`
- «Esto es lo que se envió» `writing.agentSent`
- «Etiquetas» `writing.zoteroDetailTags`
- «Exacto: {words}» `writing.searchFoundExact`
- «Exportando…» `writing.exportRunning`
- «Falta la credencial de OpenRouter. Cargala en Configuración y el agente queda disponible.» `writing.agentNoCredential`
- «Fecha» `writing.zoteroDetailDate`
- «Ficha de la obra» `writing.zoteroDetailTitle`
- «Fichas {worksDone}/{worksTotal} · Pasajes {passagesDone}/{passagesTotal}» `writing.zoteroBibliographySyncIndexingNoEta`
- «Fichas {worksDone}/{worksTotal} · Pasajes {passagesDone}/{passagesTotal} · {blocked} en espera: {reason}» `writing.zoteroBibliographySyncBlockedBacklog`
- «Fichas {worksDone}/{worksTotal} · Pasajes {passagesDone}/{passagesTotal} · ~{eta} restantes» `writing.zoteroBibliographySyncIndexing`
- «Fichas {worksDone}/{worksTotal} · Pasajes {passagesDone}/{passagesTotal} · ~{eta} restantes · {blocked} en espera: {reason}» `writing.zoteroBibliographySyncIndexingWaiting`
- «Folio» `writing.locatorFolio`
- «Formato de descarga» `writing.downloadFormats`
- «Fuentes citadas» `writing.exportSources`
- «Grupo» `writing.zoteroLibraryTypeGroup`
- «Guardado» `writing.status.saved`

### Varios (4/8)

- «Guardado automático» `writing.historyAuto`
- «Guardado en {path}» `writing.exportSaved`
- «Guardando» `writing.status.saving`
- «Guardar clave» `writing.publishSaveKey`
- «Guardar la selección como nota» `writing.noteCreate`
- «Historial de versiones» `writing.historyTitle`
- «ID» `writing.zoteroLibraryId`
- «ISBN» `writing.zoteroDetailIsbn`
- «Idioma» `writing.zoteroDetailLanguage`
- «Incluir bibliografía» `writing.exportWithBibliography`
- «Incluir coincidencias aproximadas» `writing.corpusFuzzy`
- «Ingresá un ID de biblioteca.» `writing.zoteroLibraryInvalid`
- «Insertar como cita» `writing.corpusInsert`
- «Insertar como texto» `writing.notesCopy`
- «Insertar como vínculo» `writing.notesLink`
- «Insertar debajo» `writing.agentInsertBelow`
- «Investigación» `writing.research`
- «La fuente cambió desde que citaste» `writing.sourceModifiedTitle`
- «La fuente de esta cita ya no está» `writing.sourceMissingTitle`
- «La nota cambió desde que la vinculaste» `writing.noteChangedTitle`
- «La nota vinculada ya no está» `writing.noteMissingTitle`
- «La otra persona lo recibe en su próxima sincronización y los cambios de cada uno le llegan al otro.» `writing.shareHint`
- «La sincronización se canceló.» `writing.zoteroBibliographySyncCancelled`
- «Las citas no se pudieron volver a generar: {message}. El archivo se exportó igual, con lo último que cada cita mostraba.» `writing.exportTrouble`
- «Leyendo la biblioteca…» `writing.zoteroLoading`
- «Leyendo la ficha…» `writing.zoteroDetailLoading`
- «Listo» `writing.citeDone`
- «Listo: el artículo está en hlab.com.ar.» `writing.publishDone`
- «Llega como borrador oculto. Si ya estaba publicado, el cambio sale al aire.» `writing.publishHint`
- «Lo seleccionado» `writing.agentWillSend`
- «Localizador» `writing.citeLocator`
- «Línea» `writing.locatorLine`
- «Mail de su cuenta de EntropIA» `writing.shareEmail`
- «Markdown no tiene comentarios: al descargar en Markdown, los comentarios salen como nota al pie, y se avisa.» `writing.exportCiteMarkdownComment`
- «Mi biblioteca» `writing.zoteroLibraryPersonal`
- «Mostrar tags automáticos ({count})» `writing.zoteroAutomaticTagsShow`
- «Mostrar u ocultar el esquema» `writing.toggleOutline`
- «Mostrar u ocultar el panel de investigación» `writing.toggleResearch`
- «Motivo: {reason}» `writing.zoteroDetailReason`
- «Nada responde en el puerto local de Zotero. No sabemos si está cerrado, si no está instalado o si algo bloquea el puerto: solo que no contesta.» `writing.zoteroStateUnavailable`
- «Ninguna de tus notas comparte palabras con este pasaje, así que no se consultó al modelo.» `writing.agentNoRelatedNotes`

### Varios (5/8)

- «Ninguna referencia coincide con esa búsqueda.» `writing.zoteroEmpty`
- «Ninguna versión cambió este apartado.» `writing.historySectionEmpty`
- «Ningún documento coincide con esa búsqueda.» `writing.noteCreateNoTargets`
- «No disponible» `writing.agentUnavailable`
- «No hay espacio para el esquema con este ancho.» `writing.outlineNoRoom`
- «No hay sugerencias pendientes.» `writing.agentNonePending`
- «No hay un modelo de chat configurado, así que el agente no puede proponer nada. Escribir a mano sigue funcionando igual.» `writing.agentNoModel`
- «No se exportó nada. DOCX debe conservar {elements}, y con esta configuración se perdería. Corregilo o elegí otro formato.» `writing.exportRefused`
- «No se pierde nada: lo que tenés ahora queda guardado como otra versión.» `writing.historyRestoreHint`
- «No se pudo comprobar que la fuente siga igual.» `writing.sourceUnverifiable`
- «No se pudo comprobar si la nota sigue igual.» `writing.noteUnverifiable`
- «No se pudo leer el estado de la sincronización: {detail}» `writing.zoteroBibliographySyncUnreadable`
- «No se pudo ubicar esa selección en el texto original. Probá seleccionando un poco distinto.» `writing.corpusSelectionUnmapped`
- «Nota» `writing.locatorNote`
- «Nota al pie» `writing.exportCiteFootnote`
- «Nota creada en «{item}».» `writing.noteCreated`
- «Nota insertada en el manuscrito.» `writing.notesInserted`
- «Obra no disponible en Zotero» `writing.zoteroDetailLost`
- «Ocultar tags automáticos» `writing.zoteroAutomaticTagsHide`
- «Otra ventana guardó este documento mientras escribías, así que reintentar pisaría ese trabajo. Lo que escribiste sigue acá y en el registro de recuperación: copialo antes de volver a abrir el documento.» `writing.saveConflict`
- «Para compartir hace falta tener la sincronización activa.» `writing.shareNeedsSync`
- «Para sincronizar, Zotero tiene que estar respondiendo en el puerto local.» `writing.zoteroBibliographySyncNeedsZotero`
- «Pedirle al agente» `writing.agentAsk`
- «Por contenido» `writing.zoteroContentTag`
- «Por qué» `writing.agentRationale`
- «Por significado» `writing.searchFoundMeaning, writing.zoteroSemanticTag`
- «Procesando «{title}»» `writing.zoteroBibliographyCurrent`
- «Procesando «{title}» · ~{eta} restantes» `writing.zoteroBibliographyCurrentEta`
- «Procesando «{title}»: página {done} de {total}» `writing.zoteroBibliographyCurrentPages`
- «Procesando «{title}»: página {done} de {total} · ~{eta} restantes» `writing.zoteroBibliographyCurrentPagesEta`
- «Publicación» `writing.zoteroDetailPublication`
- «Página» `writing.locatorPage`
- «Página {page}» `writing.corpusPage`
- «Párrafo» `writing.locatorParagraph`

### Varios (6/8)

- «Queda afuera: {count} pieza(s), por límite o por estar repetida(s).» `writing.agentOmitted`
- «Quita el autor de la cita y conserva el año y la página.» `writing.citeSuppressHelp`
- «Quitar» `writing.shareRemove`
- «Quitar esta obra de la cita» `writing.citeRemoveWork`
- «Recuperación disponible» `writing.status.recovery`
- «Reemplazar» `writing.agentReplace`
- «Referencia breve» `writing.exportCiteInline`
- «Reintentar» `writing.retrySave, writing.zoteroDetailRetry`
- «Renombrar la sección» `writing.sectionRename`
- «Restaurar esta versión» `writing.historyRestore`
- «Restaurar solo este apartado» `writing.historyRestoreSection`
- «Resumen» `writing.zoteroDetailAbstract`
- «Revisión {revision}» `writing.revision`
- «Se elimina «{title}» y todo lo que hay debajo: {words} palabra(s) en {headings} título(s). Se puede deshacer con Ctrl+Z, pero el autoguardado lo persiste enseguida.» `writing.sectionDeleteMessage`
- «Se quitaron {count} marcador(es) de nota al pie sin nota asociada, que impedían abrir el documento. El cambio se guarda con tu próxima edición.» `writing.repaired`
- «Sección» `writing.locatorSection`
- «Seleccioná el fragmento que querés citar.» `writing.corpusSelectFirst`
- «Seleccioná un pasaje del documento para guardarlo como nota.» `writing.noteCreateNoSelection`
- «Seleccioná un pasaje del documento para pedirle algo al agente.» `writing.agentNoSelection`
- «Sin copia de los metadatos.» `writing.zoteroDetailNoSnapshot`
- «Sin evidencia: es prosa del modelo.» `writing.agentNoEvidence`
- «Sin notas para esa búsqueda.» `writing.notesEmpty`
- «Sin número de página» `writing.corpusPageUnnumbered`
- «Sin recuperación sobre el corpus: las acciones que buscan evidencia no están disponibles en esta variante.» `writing.agentNoRetrieval`
- «Sin resultados para esa búsqueda.» `writing.corpusEmpty`
- «Sin título» `writing.newDocumentTitle`
- «Sin títulos todavía.» `writing.outlineEmpty`
- «Sincronización en cola…» `writing.zoteroBibliographySyncQueued`
- «Sincronizada, pero {failed} fichas o pasajes fallaron al procesarse.» `writing.zoteroBibliographySyncDrainedWithFailures`
- «Sincronizada. Obras nuevas o actualizadas: {works}. Adjuntos nuevos: {attachments}.» `writing.zoteroBibliographySyncDone`
- «Sincronizado el {date}» `writing.zoteroDetailSynced`
- «Sincronizando…» `writing.zoteroBibliographySyncRunning`
- «Sincronizando… {done} de {total} obras» `writing.zoteroBibliographySyncRunningOf`
- «Sincronizar biblioteca» `writing.zoteroBibliographySync`
- «Sobre qué se apoya» `writing.agentEvidence`

### Varios (7/8)

- «Solicita la sincronización del catálogo bibliográfico seleccionado en segundo plano. «Actualizar» solo renueva esta lista desde Zotero.» `writing.zoteroBibliographySyncHelp`
- «Solicitando la sincronización…» `writing.zoteroBibliographySyncRequesting`
- «Solicitud aceptada. Consultando el estado…» `writing.zoteroBibliographySyncRequested`
- «Solo las colecciones de este documento.» `writing.notesScoped`
- «Solo «{title}» vuelve a esta versión; el resto del manuscrito queda como está, y lo que cambia se guarda como otra versión.» `writing.historyRestoreSectionHint`
- «Subir la sección» `writing.sectionUp`
- «Sugerencias para revisar» `writing.agentPending`
- «Texto citado y nota» `writing.exportCiteQuote`
- «Texto original» `writing.agentOriginal`
- «Texto propuesto» `writing.agentProposed`
- «Tipo» `writing.zoteroDetailType, writing.zoteroLibraryType`
- «Tipo de localizador» `writing.citeLocatorKind`
- «Todavía no hay documentos.» `writing.empty`
- «Todavía no hay versiones guardadas. Se guarda una cada diez minutos de trabajo y otra antes de recibir cambios de otra cuenta.» `writing.historyEmpty`
- «Todos los apartados» `writing.historySectionAll`
- «Tu biblioteca aparece acá cuando Zotero responde.» `writing.zoteroStart`
- «Tu documento sigue diciendo lo que insertaste — no lo tocamos. Así dice la nota ahora:» `writing.noteChangedBody`
- «Tu texto no se modificó y el fragmento sigue acá. Esto es lo que quedó registrado cuando la vinculaste:» `writing.noteMissingBody`
- «Título del documento» `writing.titleLabel`
- «Título sin texto» `writing.outlineUntitled`
- «Una nota siempre pertenece a un documento real del corpus. Elegí cuál.» `writing.noteCreateWhy`
- «Usuario» `writing.zoteroLibraryTypeUser`
- «Ver ficha» `writing.zoteroDetails`
- «Versión guardada» `writing.historyOther`
- «Volumen» `writing.locatorVolume`
- «Volver a las notas» `writing.notesBack`
- «Volver a los documentos» `writing.backToList`
- «Volver a los resultados» `writing.corpusBack`
- «Volver a referencias» `writing.zoteroBack`
- «Vuelve a leer el catálogo de Zotero; no reinicia el procesamiento en curso» `writing.zoteroBibliographySyncReread`
- «Ya nombré al autor en mi frase» `writing.citeSuppress`
- «Zotero contestó algo que esta versión no sabe leer: {detail}» `writing.zoteroStateInvalid`
- «Zotero está ahí, pero su API local está apagada. Activala en Zotero: Editar → Configuración → Avanzado → «Permitir que otras aplicaciones se comuniquen con Zotero».» `writing.zoteroStateDisabled`
- «Zotero responde y la biblioteca está accesible.» `writing.zoteroStateAvailable`

### Varios (8/8)

- «Zotero tardó demasiado en responder. Está ahí, pero no llegó a contestar a tiempo.» `writing.zoteroStateTimeout`
- «configurá GLM-OCR en Configuración › OCR» `writing.zoteroBlockedReasonOcr`
- «configurá OpenRouter en Configuración» `writing.zoteroBlockedReasonEmbedding`
- «configurá OpenRouter en Configuración y GLM-OCR en Configuración › OCR» `writing.zoteroBlockedReasonEmbeddingAndOcr`
- «p. ej. 0, 6680944» `writing.zoteroLibraryIdPlaceholder`
- «página {page}» `writing.sourcePage`
- «ver, cf., según…» `writing.citePrefixPlaceholder`
- «y ss., passim…» `writing.citeSuffixPlaceholder`
- «{chars} caracteres en {pieces} pieza(s).» `writing.agentChars`
- «{count} documento(s)» `writing.documentCount`
- «{count} referencia(s) leída(s).» `writing.zoteroLoaded`
- «{element}: {count} caso(s) no se pueden representar en este formato.» `writing.exportDropped`
- «{element}: {count} caso(s) van representados de otra manera.» `writing.exportSubstituted`
- «{hours} h {minutes} min» `writing.zoteroEtaHoursMinutes`
- «{minutes} min» `writing.zoteroEtaMinutes`
- «{provider} · {model}» `writing.agentModel`
- ««{title}» pasa a la papelera, al pie de la lista. Su contenido, sus versiones y su historial se conservan, y desde ahí lo podés restaurar.» `writing.discardMessage`
- «¿Eliminar el documento?» `writing.discardTitle`
- «¿Eliminar esta sección?» `writing.sectionDeleteTitle`
