# Plan de implementación del espacio de escritura académica de EntropIA

**Estado:** plan ajustado tras revisión arquitectónica; pendiente de validaciones de Fase 0  
**Alcance:** EntropIA Pro y EntropIA Lite desde el diseño inicial  
**Nombre funcional provisional:** Escritura  
**Ubicación acordada:** módulo interno de `EntropIA-Pro-Lite`; sin nuevo repositorio ni aplicación independiente.

## 1. Propósito

EntropIA incorporará un espacio de escritura académica asistida y trazable, integrado con el corpus documental, las notas de investigación, Zotero y las capacidades de recuperación y análisis de la aplicación.

La nueva sección no debe concebirse como un procesador de textos genérico ni como un reemplazo de Word. Su propósito es convertir el trabajo realizado dentro de EntropIA en escritura académica y mantener el vínculo entre el texto producido, las operaciones de investigación y las fuentes que lo sustentan.

El ciclo de trabajo que debe completar es:

> **fuente → procesamiento → análisis → recuperación → interpretación → escritura → fuente**

El principio rector de la implementación será:

> **EntropIA no incorpora simplemente un editor de texto. Incorpora un espacio de escritura académica en el que cada afirmación puede permanecer vinculada con el proceso de investigación y con las fuentes que permitieron construirla.**

En este espacio convergerán tres materiales diferentes, que deben mantenerse conceptualmente separados:

1. El texto redactado por el investigador.
2. Las fuentes documentales administradas por EntropIA.
3. La bibliografía académica administrada por Zotero.

El módulo podrá relacionar estas capas, pero no deberá confundirlas ni reducirlas a texto plano.

## 2. Alcance y supuestos

Este plan define la arquitectura funcional y técnica de Escritura dentro del monorepo existente. La revisión del código confirma Svelte 5, Rust/Tauri 2 y SQLite, con esquema, migraciones y repositorios en `packages/store`, controles compartidos en `packages/ui` e integración de escritorio en `apps/desktop`.

El corpus está organizado mediante colecciones, items y assets. Escritura consumirá las capacidades existentes de OCR, texto extraído, notas, metadatos, entidades, tripletas, búsqueda, recuperación y proveedores LLM; su disponibilidad exacta por flujo debe verificarse, no darse por supuesta.

### 2.1 Decisión de repositorio y responsabilidades

**Escritura se implementará como un módulo delimitado dentro de `EntropIA-Pro-Lite`.** No se creará un repositorio `EntropIA-Editor`, una aplicación separada ni una base paralela por defecto. La separación será de responsabilidades e interfaces, sin imponer un paquete por cada concepto.

| Responsabilidad | Ubicación y regla |
|---|---|
| Pantalla, paneles, selección y acciones | `apps/desktop`; integración con navegación y carga diferida existentes |
| Modelo documental, extensiones, anchors y transformaciones | Módulo interno de Escritura separado de las vistas y de los detalles de transporte |
| Esquema, migraciones y acceso persistente | Sistema existente de `packages/store` y puente Tauri; una sola autoridad de migraciones |
| Validación y operaciones persistentes atómicas, recuperación durable, archivos y conexión local con Zotero | Backend de escritorio cuando corresponda; reutilizar los mecanismos existentes |
| Controles visuales realmente compartidos | `packages/ui`; no alojar allí la orquestación específica de Escritura |
| Investigación y recuperación del agente | Consumir EntropIA-Agent; ampliar ese motor solo con capacidades propias de investigación |
| CSL y conversión documental | Lógica separada de las vistas; lugar de ejecución elegido mediante las pruebas de Fase 0 |

EntropIA-Agent es una biblioteca Rust con ejecutable de desarrollo/prueba: lee el corpus y mantiene su estado propio en `estado.sqlite`. Esa separación no se trasladará automáticamente al editor, que necesita evolucionar coordinadamente con navegación, persistencia y relaciones del corpus. El agente no será dueño del manuscrito canónico, undo/redo, CSL, guardado ni aplicación de sugerencias.

La extracción a otro repositorio solo se reconsiderará ante un consumidor adicional real, distribución independiente o mantenimiento mediante una interfaz ya estable. En ese caso se evaluará extraer el núcleo documental, no toda la integración de escritorio.

### 2.2 Compatibilidad Pro y Lite

Ambas variantes compartirán esquema documental, edición manual, citas, historial, exportación y conexión local con Zotero. Zotero Desktop no depende de `local-ml`; Zotero Web/OAuth no será un requisito para Lite.

Las acciones de IA y recuperación se habilitarán por capacidades efectivamente disponibles y proveedor configurado. La ausencia de una capacidad no impedirá abrir, editar, guardar ni exportar el mismo documento en la otra variante.

No se crearán servicios paralelos cuando EntropIA ya posea una solución equivalente. La Fase 0 validará los detalles pendientes y las pruebas técnicas; no reabrirá la decisión de repositorio sin un requisito nuevo explícito.

## 3. Objetivos funcionales

La sección **Escritura** deberá permitir:

- crear, abrir, duplicar, renombrar, archivar y eliminar documentos académicos
- asociar cada documento con una o varias colecciones, sin impedir búsquedas globales cuando el usuario las autorice
- redactar artículos, capítulos, ponencias, informes, proyectos y notas extensas
- organizar el texto mediante secciones y subsecciones
- consultar el corpus, las notas y Zotero sin abandonar el documento
- insertar fragmentos documentales conservando su procedencia
- insertar citas bibliográficas Zotero como objetos estructurados y cambiar posteriormente su estilo CSL
- generar una bibliografía dinámica con las obras efectivamente citadas
- ejecutar acciones del agente sobre una selección, una sección o el documento bajo control explícito del usuario
- distinguir y auditar texto manual, texto documental, texto sugerido por IA y citas bibliográficas
- guardar automáticamente el trabajo y recuperarlo después de un cierre inesperado
- consultar y restaurar versiones anteriores
- exportar a Markdown, HTML y DOCX

## 4. Límites del MVP

El MVP no debe intentar reproducir Word. Quedan fuera de la primera versión:

- paginación visual A4
- diseño editorial avanzado
- encabezados y pies de página complejos
- combinación de correspondencia
- colaboración simultánea multiusuario
- control de cambios equivalente al de Word
- edición completa de bibliotecas Zotero desde EntropIA
- sincronización Zotero Web mediante OAuth
- garantía de campos Zotero vivos y editables después de exportar a DOCX
- gestor bibliográfico propio
- importación arbitraria de todos los formatos de procesadores de texto
- escritura autónoma del agente sin revisión humana

Estas exclusiones reducen el riesgo de convertir la funcionalidad en un procesador de textos generalista y permiten concentrarse en su valor diferencial: escritura, trazabilidad y recuperación investigativa.

El MVP completo conserva todos los objetivos y criterios de aceptación de este plan. Es una entrega extensa, no un editor básico: se construirá mediante incrementos verticales verificables (§23). Ningún incremento aislado se presentará como el MVP terminado.

## 5. Auditoría obligatoria de la arquitectura existente

Antes de modificar código, el agente implementador debe documentar:

### 5.1 Frontend

- estructura de rutas y navegación principal
- gestión de estado global y local
- componentes de paneles, menús, diálogos, selectores y pestañas que puedan reutilizarse
- variables CSS, sistema de espaciado, tipografías e iconografía
- comportamiento de paneles plegables y responsive
- integración actual con comandos Tauri
- componentes del visor de assets y mecanismo de navegación hacia una página o fragmento
- UI existente de Chat, fuentes citadas, notas, búsquedas y documentos similares

### 5.2 Backend

- comandos Tauri y servicios Rust existentes
- separación actual entre repositorios, servicios, dominio y capa de transporte
- acceso a SQLite, transacciones y estrategia de migraciones
- gestión de errores y tipos compartidos con el frontend
- sistema de jobs, colas o procesos en segundo plano
- mecanismos de exportación
- integración con proveedores LLM locales y externos

### 5.3 Datos

- claves reales de colecciones, items y assets
- representación del OCR y texto extraído
- relación entre assets y páginas
- esquema de notas
- almacenamiento y búsqueda de embeddings
- esquema de entidades y tripletas
- persistencia de conversaciones y fuentes recuperadas
- políticas existentes de eliminación, archivado e integridad referencial

### 5.4 Resultado de la auditoría

La auditoría debe producir una tabla de correspondencias entre los conceptos usados en este plan y las entidades reales del proyecto. Toda dependencia nueva debe justificarse y todo componente reutilizable debe quedar identificado antes de comenzar la implementación.

### 5.5 Base comprobada y brechas pendientes

La revisión inicial identificó estas correspondencias. Deben actualizarse si el código cambia antes de implementar; no equivalen a una validación de ejecución.

| Concepto | Evidencia existente | Reutilización o brecha |
|---|---|---|
| Tiptap | `apps/desktop/package.json`; `packages/ui/src/components/NoteEditor/NoteEditor.svelte` | Tiptap 2.26.4 declarado e integración existente; notas utiliza HTML, no el nuevo contrato JSON |
| Navegación | `apps/desktop/src/lib/navigation.ts`; `route-loader.ts` | Agregar Escritura al mecanismo actual; la navegación a item admite `assetId`, pero debe probarse el recorrido hasta página/rango y regreso al manuscrito |
| Esquema y migraciones | `packages/store/src/schema.ts`; `runner.ts`; `migrations/` | Extender la autoridad existente, no crear un migrador Rust paralelo |
| Persistencia de escritorio | `apps/desktop/src/lib/db.ts`; `tauri-db-client.ts`; `src-tauri/src/db/commands.rs` | Existe `db_execute_transaction` parametrizado y atómico; falta definir validación documental y control de revisión |
| Identidad del corpus | `packages/store/src/schema.ts` | IDs de colecciones/items/assets; assets con `parentAssetId` y `pageNumber`; chunks con fuente, offsets y hash |
| Notas | `packages/store/src/repos/note.repo.ts`; `schema.ts` | Toda nota requiere `itemId`; búsqueda transversal y creación desde manuscrito requieren resolver alcance y destino |
| Motor de investigación | `apps/desktop/src-tauri/src/research.rs`; dependencia en `Cargo.toml` | Estado propio separado; el flujo inspeccionado instancia OpenRouter y exige su credencial |
| Exportación | `apps/desktop/src/lib/ocr-export.ts`; dependencia `html-docx-js` | Reutilizar lo que conserve fidelidad; la exportación existente no prueba notas al pie ni CSL |
| Variantes | `apps/desktop/src/lib/capabilities.ts` | `LOCAL_ML` diferencia capacidades; no justifica excluir Zotero local de Lite |

La auditoría también debe precisar el tratamiento de las tablas nuevas en backup, restauración, sincronización existente y borrado de colecciones/items/assets. Compartir base de datos no incorpora automáticamente esos comportamientos.

## 6. Arquitectura funcional

La nueva sección principal se denominará **Escritura**. Su pantalla de edición se organizará en tres áreas y admitirá un modo concentrado.

### 6.1 Panel izquierdo: estructura y documentos

El panel mostrará:

- selector o listado de documentos
- título del documento activo
- esquema derivado de sus encabezados
- secciones y subsecciones anidadas
- estado de guardado

Permitirá:

- crear y abrir documentos
- navegar a un encabezado
- crear, renombrar, mover y eliminar secciones
- plegar niveles del esquema
- reordenar encabezados mediante controles accesibles y, si no compromete estabilidad, drag and drop
- ocultar el panel

La estructura debe derivarse del documento, no mantenerse como una copia independiente susceptible de quedar desincronizada.

### 6.2 Panel central: editor

Se utilizará **Tiptap sobre ProseMirror**, mediante un wrapper Svelte pequeño y propio de Escritura. Ya existe integración Tiptap en `NoteEditor`; se reutilizarán patrones y controles adecuados, sin convertir ese editor de notas en el editor académico ni cambiar su contrato HTML por arrastre. La Fase 0 validará extensiones académicas, anchors, JSON versionado, rendimiento y exportación con las versiones instaladas; no será solamente una prueba de montaje de Tiptap.

El editor inicial deberá soportar:

- párrafos
- títulos y subtítulos
- negrita y cursiva
- listas ordenadas y no ordenadas
- citas en bloque
- enlaces
- tablas básicas
- notas al pie
- undo y redo
- búsqueda y reemplazo dentro del documento
- atajos de teclado convencionales
- selección de texto para acciones del agente

El contenido canónico será JSON estructurado de ProseMirror/Tiptap con una versión explícita de esquema. HTML, Markdown y DOCX serán formatos derivados.

### 6.3 Panel derecho: investigación

El panel derecho tendrá pestañas claramente diferenciadas:

- **Corpus**
- **Zotero**
- **Notas**
- **Agente**

Opcionalmente podrá incorporar accesos a Entidades y Tripletas dentro de Corpus, si ello evita saturar la navegación.

El panel podrá ocultarse. Con ambos laterales plegados, el editor ocupará el mayor ancho posible para sesiones de escritura concentrada.

## 7. Dos sistemas de procedencia

EntropIA debe representar de modo distinto la evidencia documental y la bibliografía académica.

| Dimensión | Corpus EntropIA | Zotero |
|---|---|---|
| Función | Evidencia documental o fuente primaria | Bibliografía académica |
| Unidad | Colección, item, asset, página o fragmento | Item bibliográfico |
| Identidad | IDs internos de EntropIA | Origen/instancia, biblioteca e `item_key` de Zotero |
| Localizador | Página, rango de texto, región u otro locator | Página, capítulo, sección, párrafo u otro locator CSL |
| Acción principal | Volver a la fuente original | Abrir el registro en Zotero |
| Representación | Cita documental trazable | Cita bibliográfica renderizada por CSL |
| Uso por el agente | Evidencia recuperada | Bibliografía citada o contexto bibliográfico |

Ambas clases pueden aparecer en un mismo párrafo, pero deben conservar tipos, metadatos y ciclos de vida independientes.

## 8. Modelo interno del documento

### 8.1 Documento canónico

Cada documento se almacenará como JSON de ProseMirror/Tiptap. No se almacenará únicamente como HTML ni se utilizará DOCX como formato interno.

El registro debe incluir:

- versión del esquema del editor
- contenido estructurado
- metadatos del documento
- configuración bibliográfica
- asociaciones con colecciones
- nodos y marcas con procedencia

### 8.2 Nodos y marcas personalizados

Se proponen, como mínimo:

- `entropiaDocumentCitation`: fragmento o referencia a una fuente del corpus
- `zoteroCitation`: cita bibliográfica estructurada
- `bibliography`: marcador de bibliografía dinámica
- `footnote` y `footnoteReference`: notas al pie
- `provenanceMark`: procedencia de una inserción o transformación relevante
- las sugerencias pendientes se gestionarán fuera del contenido canónico, con representación visual transitoria y persistencia propia (§9.7)

Los nombres definitivos deberán seguir las convenciones reales del proyecto.

### 8.3 Versionado del esquema

Cada contenido guardado tendrá un `schema_version`. Toda modificación incompatible requerirá:

1. migración determinista del JSON
2. copia de seguridad anterior a la migración
3. pruebas con documentos de todas las versiones conocidas
4. capacidad de informar un error sin sobrescribir el documento original

Las extensiones desconocidas no deben provocar pérdida silenciosa de contenido.

### 8.4 Autoridad del contenido y proyecciones

El JSON canónico contendrá la estructura, identidad de nodos, citas, vínculos y snapshots necesarios para interpretar el contenido sin depender de una biblioteca externa disponible. Las tablas normalizadas de citas (§9.4 y §9.5) serán proyecciones consultables de la revisión actual, no una segunda fuente editable de verdad. El estado de verificación externa es derivado; refrescar un snapshot requiere una operación explícita sobre el documento.

Contenido actual, revisión, proyecciones y eventos nuevos de procedencia se confirmarán en una misma transacción. No se editarán las tablas de citas por un camino que deje el JSON desactualizado. El historial de procedencia es un registro de operaciones, no una proyección que se borre al eliminar texto.

Toda operación estructural seguirá este contrato, incluidas copia/pegado, duplicación, eliminación, undo/redo y restauración. Copiar creará identidades nuevas para las ocurrencias insertadas sin perder la referencia a su fuente; mover dentro del documento conservará la identidad. Duplicar un documento creará identidad documental y ocurrencias propias y registrará su origen, sin compartir sugerencias pendientes.

Una versión debe conservar el contenido y la configuración necesarios para reproducir sus citas y bibliografía, no solo el texto. Restaurarla reconstruirá las proyecciones y creará una revisión nueva; no consultará Zotero para sustituir silenciosamente snapshots históricos.

### 8.5 Anchors, revisiones y concurrencia

Se distinguirán tres conceptos: revisión persistida del documento, identidad estable de nodos y anclaje de un rango dentro de ellos. Los rangos del manuscrito no se confundirán con offsets o regiones de la fuente documental.

La Fase 0 debe probar cómo se transforman anchors con transacciones ProseMirror, división/fusión de bloques, movimiento, borrado y undo/redo. Un hash detecta divergencia, pero no reubica una selección. Si un rango no puede resolverse sin ambigüedad, se marcará desactualizado y no se aplicará una propuesta sobre otra coincidencia textual.

Cada guardado llevará la revisión base esperada. La comprobación y la escritura serán atómicas: una revisión antigua no podrá sobrescribir silenciosamente una más reciente, incluso con ventanas o instancias concurrentes. Las colas de guardado preservarán el orden y la UI solo confirmará la revisión reconocida por persistencia.

## 9. Modelo de datos propuesto

Los nombres se ajustarán después de revisar el esquema real. Se recomienda no guardar grandes duplicados del contenido en varias tablas sin necesidad.

### 9.1 `research_documents`

- `id`
- `title`
- `document_type`
- `status`: activo, archivado o papelera
- `schema_version`
- `current_content_json`
- `revision`: secuencia monotónica para confirmación de guardado y control de concurrencia
- `plain_text_cache`, opcional y regenerable
- `citation_style_id`
- `citation_locale`
- `bibliography_enabled`
- `created_at`
- `updated_at`
- `last_opened_at`

El contenido actual permite abrir el documento sin reconstruirlo desde el historial.

### 9.2 `research_document_collections`

Tabla de asociación muchos a muchos:

- `document_id`
- `collection_id`
- `is_primary`
- `created_at`

### 9.3 `research_document_versions`

- `id`
- `document_id`
- `version_number`
- `content_json`
- `schema_version`
- `document_settings_json`: configuración bibliográfica y demás datos necesarios para reproducir la versión
- `reason`: automático, cierre, checkpoint, restauración o migración
- `created_at`
- `content_hash`

El historial no debe crear una versión completa por pulsación. Se aplicará retención configurable y compactación de snapshots automáticos.

### 9.4 `research_document_citations`

Proyección normalizada de citas documentales de la revisión actual (§8.4):

- `id`
- `document_id`
- `citation_node_id`
- `collection_id`
- `item_id`
- `asset_id`
- `page_number`
- `source_start`
- `source_end`
- `source_region_json`, opcional
- `quoted_text`
- `source_text_hash`
- `locator_json`
- `metadata_snapshot_json`
- `created_at`
- `updated_at`
- `integrity_status`: válida, fuente modificada, fuente ausente o no verificable

El `metadata_snapshot_json` permite interpretar una cita aunque la fuente cambie, pero no debe reemplazar el vínculo vivo con el asset.

### 9.5 `research_zotero_citations`

Proyección normalizada de clústeres e items citados de la revisión actual (§8.4):

- `id`
- `document_id`
- `citation_node_id`
- `citation_cluster_id`
- `item_position`: orden del item dentro del clúster
- `source_origin`: local o Web; Web queda fuera de la integración inicial
- `source_instance_id`: identidad de instancia o partición local según disponibilidad (§11.2)
- `library_type`
- `library_id`
- `item_key`
- `item_version`, si está disponible
- `locator_type`
- `locator`
- `prefix`
- `suffix`
- `suppress_author`
- `author_only`, si CSL lo admite en el flujo elegido
- `item_csl_json_snapshot`
- `created_at`
- `updated_at`
- `integrity_status`: válida, Zotero no disponible, item ausente o modificada

Una misma cita visual puede ser un clúster con varios items. El modelo no debe presuponer una relación uno a uno entre paréntesis e item bibliográfico.

### 9.6 `research_provenance_events`

Registro de intervenciones relevantes:

- `id`
- `document_id`
- `version_id`, cuando corresponda
- `range_anchor_json`
- `origin_type`: manual, corpus, nota, Zotero, agente o importación
- `operation_type`: insertar, reemplazar, reformular, restaurar u otra
- `source_reference_json`
- `model_provider`, opcional
- `model_name`, opcional
- `prompt_template_id`, opcional
- `created_at`

La procedencia no debe transformarse en vigilancia de cada tecla. Se registrarán inserciones estructuradas, operaciones del agente y acciones de importación o restauración que tengan valor de auditoría.

### 9.7 `research_agent_suggestions`

- `id`
- `document_id`
- `selection_anchor_json`
- `source_revision`
- `selected_content_hash`: hash del contenido estructurado objetivo, no del manuscrito completo
- `action_type`
- `original_text`
- `suggested_text`
- `rationale`, opcional
- `evidence_json`
- `status`: pendiente, aceptada, insertada debajo o descartada
- `provider`
- `model`
- `created_at`
- `resolved_at`

Antes de aplicar la sugerencia se resolverá `selection_anchor_json` y se contrastará el contenido objetivo con `selected_content_hash`. Cambios ajenos al rango no bastarán para rechazarla si el anclaje sigue siendo verificable; cambios en el objetivo o anclajes ambiguos exigirán revisión. La aplicación comprobará además la revisión persistente vigente dentro de la transacción de contenido, proyecciones, estado de sugerencia y procedencia.

## 10. Citas documentales trazables

### 10.1 Inserción

Desde un resultado del corpus, el usuario podrá:

- ver la fuente
- insertar una cita textual
- insertar una referencia documental sin transcribir el fragmento completo
- insertar el fragmento como nota de trabajo
- copiarlo
- enviarlo como contexto al agente

Al insertar una cita se creará simultáneamente:

1. un nodo o marca estructurada en el documento
2. un registro normalizado de procedencia
3. un snapshot mínimo de metadatos

La confirmación persistente será transaccional conforme a §8.4. La UI puede mostrar la edición pendiente, pero no declararla guardada hasta confirmar contenido, proyecciones y procedencia; ante un fallo conservará el borrador y el estado de error, sin dejar registros parcialmente confirmados.

### 10.2 Apertura de la fuente

Al pulsar la cita, EntropIA intentará:

1. resolver el asset por su ID
2. abrir el visor existente
3. navegar a la página
4. resaltar el rango de texto o región, cuando exista información suficiente
5. mostrar el fragmento y los metadatos registrados si el anclaje exacto ya no puede resolverse

### 10.3 Integridad

Si un asset citado se elimina, el sistema debe advertir sus dependencias antes de confirmar la eliminación. Se recomienda aplicar una de estas políticas, según la arquitectura actual:

- archivado lógico de assets referenciados
- restricción de borrado mientras existan citas
- borrado permitido con confirmación y preservación del snapshot

No se deben modificar IDs de fuentes referenciadas durante reorganizaciones ordinarias.

## 11. Integración con Zotero

### 11.1 Estrategia inicial

Para EntropIA Pro y Lite se utilizará la API local oficial de Zotero Desktop, disponible habitualmente en `http://localhost:23119/api/`. No se leerá directamente la base SQLite de Zotero. El acceso local es independiente de ML local y requiere habilitar la comunicación con otras aplicaciones en Zotero.

La primera integración será de solo lectura:

> **Zotero → EntropIA**

Permitirá consultar bibliotecas, colecciones, items y metadatos, pero no crear ni modificar referencias en Zotero. Esto reduce riesgos de permisos, sincronización y corrupción.

Documentación de referencia:

- [Zotero Local API](https://www.zotero.org/support/dev/web_api/v3/local_api)
- [Zotero Web API v3](https://www.zotero.org/support/dev/web_api/v3/start)
- [Citation Style Language](https://citationstyles.org/)
- [Zotero Word Processor Plugin](https://www.zotero.org/support/word_processor_plugin_usage)

### 11.2 Servicios propuestos

- `ZoteroConnector`: disponibilidad, solicitudes, timeout y manejo de errores
- `ZoteroLibraryService`: bibliotecas, colecciones, búsqueda e items
- `CitationManager`: clústeres, locators, edición y eliminación de citas
- `CSLProcessor`: renderizado de citas y bibliografía
- `BibliographyManager`: items citados, orden y actualización
- `ZoteroCache`: caché local regenerable de metadatos mínimos

Estos conceptos se agruparán en módulos con interfaces pequeñas; no requieren un archivo o servicio por nombre. El conector local pertenecerá a la integración de escritorio. El lugar de ejecución de CSL se elegirá en Fase 0 según compatibilidad, rendimiento y exportación, sin imponer Rust por defecto.

La caché distinguirá origen local/Web e instancia. Según la documentación oficial consultada, Zotero 10+ expone `Zotero-Server-ID` y versiones locales que no son comparables con las Web ni con otras instancias. Se particionará la caché por esa identidad cuando esté disponible. Para versiones anteriores se documentará una política conservadora de partición y revalidación, sin equiparar versiones entre orígenes.

Un cambio de instancia invalidará la caché correspondiente, no los snapshots del manuscrito. No se revincularán automáticamente citas a items de otra biblioteca por coincidencia de clave. Las consultas tendrán límites y paginación explícitos: la API local puede devolver todos los resultados si no se solicita un límite.

### 11.3 Detección y estados

La interfaz contemplará:

- Zotero disponible y biblioteca accesible
- endpoint local no disponible; sin afirmar que Zotero está instalado o cerrado si no existe evidencia adicional
- API local deshabilitada (`403`), con instrucciones para habilitarla
- timeout, respuesta inválida o cambio de instancia
- biblioteca accesible con item posteriormente eliminado

Los fallos de Zotero no deben bloquear la edición ni el guardado. Las citas existentes se renderizarán mediante sus snapshots cuando la biblioteca no esté disponible y se marcarán como pendientes de verificación.

### 11.4 Búsqueda

La pestaña Zotero permitirá buscar por:

- autor o autora
- título
- año
- publicación
- etiqueta
- colección
- DOI, ISBN u otro identificador disponible

Cada resultado mostrará acciones para:

- insertar cita
- abrir en Zotero
- usar como contexto del agente
- consultar metadatos

La búsqueda deberá utilizar debounce y cancelación de solicitudes obsoletas. No se consultará Zotero por cada pulsación sin control.

### 11.5 Inserción y edición de citas

El diálogo de cita permitirá:

- seleccionar uno o varios items
- elegir locator y tipo de locator
- agregar prefijo
- agregar sufijo
- suprimir autor cuando corresponda
- reorganizar items dentro del clúster
- previsualizar la cita

El documento almacenará una estructura equivalente a datos CSL, no solamente la cadena renderizada. Por ello, un cambio de APA a Chicago deberá volver a renderizar todas las citas sin intervención manual.

### 11.6 Estilos CSL y bibliografía dinámica

Cada documento tendrá:

- estilo CSL
- idioma bibliográfico
- modo autor-fecha o notas, según el estilo
- opción de bibliografía automática

El nodo `bibliography` incluirá únicamente los items efectivamente citados. La bibliografía renderizada será una vista derivada y nunca la fuente canónica.

Para el MVP se puede incluir un conjunto reducido de estilos frecuentes y una ruta segura para incorporar otros archivos CSL. Debe evitarse descargar o ejecutar contenido no validado sin control.

### 11.7 Zotero como contexto del agente

El agente podrá:

- buscar bibliografía relacionada dentro de la biblioteca del usuario
- identificar referencias ya citadas en el documento
- sugerir obras de Zotero pertinentes para un párrafo
- detectar afirmaciones sin respaldo bibliográfico aparente
- comparar un argumento con metadatos, notas o, en una fase posterior, adjuntos textuales disponibles

Insertar una referencia no equivale a afirmar que el agente leyó su contenido. La UI debe distinguir entre:

- coincidencia bibliográfica basada en metadatos
- consulta de notas Zotero
- consulta efectiva del texto de un adjunto

### 11.8 Evolución posterior

Quedan para fases posteriores:

- Zotero Web API y OAuth para acceso remoto o usuarios sin Zotero Desktop, tanto en Pro como en Lite
- sincronización de caché más sofisticada
- lectura e indexación opcional de adjuntos
- escritura controlada hacia Zotero
- anotaciones Zotero
- exportación experimental de campos Zotero vivos en DOCX

## 12. Integración con el corpus y la recuperación

La pestaña Corpus reutilizará los servicios existentes de:

- búsqueda textual
- búsqueda semántica
- documentos similares
- filtros por colección
- metadatos
- entidades
- tripletas

Una consulta desde Escritura deberá mostrar resultados con:

- título o identificación de la fuente
- colección
- fecha
- página o asset
- fragmento relevante
- tipo de recuperación
- score, cuando resulte útil y sea interpretable
- acciones de apertura, inserción y uso como contexto

La pantalla de Escritura no debe implementar un segundo motor de recuperación. Debe actuar como consumidor contextual de los servicios existentes.

## 13. Integración con notas

La pestaña Notas reutilizará el sistema actual y permitirá:

- buscar y filtrar notas
- abrir una nota completa
- insertar su contenido como texto editable
- insertar una referencia vinculada a la nota
- vincular una sección con varias notas
- crear una nota a partir de una selección del documento
- enviar una o más notas como contexto al agente

Debe definirse claramente la diferencia entre:

- copiar el contenido de una nota, que crea texto independiente
- vincular una nota, que mantiene una relación viva

Si una nota vinculada cambia, EntropIA informará la divergencia, pero no sobrescribirá automáticamente el artículo.

### 13.1 Destino de las notas creadas desde Escritura

El modelo actual exige `notes.itemId`. Crear una nota desde una selección requerirá elegir un item real de destino, y opcionalmente un asset perteneciente a él. Las colecciones asociadas al manuscrito ayudarán a filtrar el selector, pero no reemplazarán la identidad del item.

No se crearán items ficticios ni se volverá nullable `itemId` para sortear esta restricción. Las notas exclusivamente pertenecientes al manuscrito exigirían una ampliación explícita del dominio y no se introducirán implícitamente.

Los vínculos desde Escritura conservarán ID de nota, snapshot y hash del contenido insertado, además de la identidad del nodo o sección de destino. Si la nota se elimina se informará la ausencia sin borrar el texto ni el snapshot. Copiar una nota no creará una relación de actualización automática.

## 14. Integración con el agente

### 14.1 Acciones iniciales

Sobre una selección o sección, el agente podrá:

- corregir ortografía y gramática
- revisar redacción
- mejorar claridad
- mejorar argumentación
- acortar
- expandir
- resumir
- reformular
- detectar reiteraciones
- detectar contradicciones internas
- buscar evidencia en el corpus
- buscar evidencia contradictoria
- buscar contraejemplos
- recuperar notas relacionadas
- buscar bibliografía en Zotero
- sugerir citas documentales o bibliográficas
- detectar afirmaciones que requieren respaldo

### 14.2 Interacción segura

El agente nunca modificará silenciosamente el texto. Toda propuesta aparecerá como sugerencia con:

- texto original
- texto propuesto
- explicación breve, cuando corresponda
- fuentes del corpus utilizadas
- referencias Zotero utilizadas
- modelo y proveedor
- acciones **Reemplazar**, **Insertar debajo** y **Descartar**

Antes de aplicar una sugerencia se verificará el rango objetivo mediante anchors y hash de contenido, conforme a §8.5 y §9.7. Si cambió o resulta ambiguo, la aplicación pedirá revisar nuevamente la propuesta. La aplicación y el registro de aceptación serán una única operación persistente; repetir una solicitud ya confirmada no insertará dos veces el texto.

### 14.3 Construcción del contexto

No se enviará automáticamente todo el documento al LLM. El contexto se compondrá, según la acción, con:

- instrucción del usuario
- texto seleccionado
- encabezado y sección actual
- párrafos inmediatamente anteriores y posteriores
- resumen estructural del documento, si resulta necesario
- fragmentos RAG seleccionados
- citas documentales vinculadas
- notas seleccionadas
- metadatos o contenido Zotero realmente consultado

El constructor de contexto aplicará límites explícitos, deduplicación y registro de las piezas enviadas. La interfaz de acciones deberá admitir los proveedores locales y externos configurados que soporten el flujo; esta compatibilidad es trabajo a validar, no una propiedad ya demostrada del adaptador actual de investigación.

### 14.4 Privacidad y proveedores

Antes de enviar contenido a un proveedor externo se respetarán las configuraciones y advertencias existentes. El usuario debe poder saber qué porción del artículo y qué fuentes se enviarán. Con modelos locales se utilizará la misma interfaz de acciones, sin crear un flujo paralelo.

### 14.5 Contrato con EntropIA-Agent

El adaptador actual de investigación en `apps/desktop/src-tauri/src/research.rs` instancia `ClienteLlmOpenRouter` y exige su credencial. La Fase 0 inventariará qué acciones pueden consumir interfaces existentes y qué adaptación hace falta para otros proveedores. No se construirá otro motor RAG ni se expondrán como disponibles acciones sin implementación compatible.

EntropIA-Agent recibirá contexto acotado y devolverá propuestas y evidencia. Escritura conservará la autoridad sobre selección, contenido canónico, confirmación humana, validación del rango, guardado y procedencia. Las mejoras genéricas de investigación podrán realizarse en el repositorio del agente; el estado editorial no se trasladará a `estado.sqlite` del motor.

Los errores o cancelaciones del agente no iniciarán reintentos facturables silenciosos ni bloquearán la escritura manual. La misma interfaz visual representará capacidades locales y externas, indicando las que no estén disponibles.

## 15. Procedencia y autoría

El sistema mantendrá internamente cinco orígenes principales:

- texto manual
- texto procedente del corpus
- texto procedente de una nota
- cita bibliográfica Zotero
- texto generado o transformado por IA

No es necesario colorear permanentemente cada fragmento. Se recomienda una inspección contextual accesible mediante el panel de detalles o una acción **Ver procedencia**.

La aceptación de una sugerencia del agente no debe borrar su procedencia. Debe registrar:

- texto original
- texto aceptado
- acción solicitada
- fecha
- modelo y proveedor
- evidencia asociada

La procedencia debe acompañar la investigación sin volver la edición lenta ni visualmente intrusiva.

## 16. Autoguardado, recuperación e historial

### 16.1 Estrategia de guardado

Se propone:

- estado inmediato en memoria durante la escritura
- persistencia del contenido actual con debounce, por ejemplo entre 750 y 1500 ms después de la última edición
- guardado forzado al cambiar de documento, cerrar la vista o antes de operaciones sensibles
- journal local de recuperación para cambios aún no confirmados
- snapshot periódico y en checkpoints manuales

Los valores definitivos deberán validarse mediante pruebas de rendimiento. SQLite no se escribirá con cada tecla.

El debounce se complementará con un plazo máximo entre persistencias durante escritura continua: escribir sin pausas no puede posponer indefinidamente la recuperación. La Fase 0 fijará ese plazo y la ventana máxima de pérdida admitida mediante mediciones.

El journal será almacenamiento durable gestionado por el backend, no memoria ni un callback de cierre. Antes de implementar se elegirá entre tabla de recuperación en SQLite o archivo con protocolo de escritura atómica, identificando documento, revisión base, secuencia, versión de esquema y checksum. Se distinguirá la garantía ante terminación del proceso de la garantía ante caída del sistema o corte eléctrico.

Persistir el journal no equivale a confirmar el documento canónico. La UI mostrará «Guardado» solo tras el commit canónico y mantendrá «Cambios pendientes» si únicamente existe copia de recuperación. No se eliminará el journal que protege una secuencia hasta confirmar una versión canónica que la incluya.

### 16.2 Estados visibles

La UI mostrará estados discretos:

- Guardado
- Guardando
- Cambios pendientes
- Error de guardado
- Recuperación disponible

Un error de persistencia no debe desaparecer por sí solo. Debe conservarse el contenido en memoria, ofrecer reintento y evitar el cierre silencioso con cambios sin guardar.

### 16.3 Recuperación tras cierre inesperado

Al abrir un documento se compararán:

- versión confirmada en SQLite
- journal de recuperación
- hash o secuencia de edición

Si el journal contiene cambios posteriores, el usuario podrá recuperar, comparar de forma básica o descartar esa copia. El descarte requerirá confirmación y conservará temporalmente una copia recuperable.

La recuperación validará integridad, esquema y revisión base. Un journal truncado o incompatible no se aplicará parcialmente ni sobrescribirá el original; se conservará para diagnóstico o recuperación explícita. Si existe una revisión canónica concurrente incompatible, se ofrecerá recuperación separada o comparación, no un reemplazo automático.

La prueba obligatoria terminará el proceso durante escritura continua, antes y después de confirmar el journal, durante el guardado canónico y durante su compactación. Debe demostrar recuperación de toda secuencia confirmada como durable y medir la pérdida de cambios aún no confirmados dentro de la ventana declarada. Los hooks de cierre no cuentan como prueba de este comportamiento.

### 16.4 Versiones

El MVP mostrará:

- número o identificación de versión
- fecha y hora
- motivo
- opción de restaurar

Restaurar una versión creará una nueva versión actual. No eliminará el historial posterior.

## 17. Exportación

### 17.1 Formatos del MVP

- Markdown
- HTML
- DOCX

Las exportaciones deben conservar, en la medida admitida por cada formato:

- jerarquía de títulos
- énfasis
- listas
- citas en bloque
- enlaces
- tablas
- notas al pie
- citas documentales
- citas Zotero
- bibliografía

### 17.2 Citas del corpus

El usuario podrá elegir una representación de exportación:

- nota al pie con metadatos y locator
- referencia breve dentro del texto
- comentario, cuando el formato y la biblioteca lo permitan
- texto citado más nota de procedencia

La configuración de exportación no alterará el documento canónico.

### 17.3 Citas Zotero

En el MVP, DOCX contendrá citas y bibliografía correctamente formateadas como texto conforme al estilo CSL elegido.

La generación de campos Zotero vivos, editables posteriormente con el complemento de Word, se considerará experimental y no será criterio de aceptación inicial. Antes de prometerla deberá realizarse una investigación específica sobre el formato de campos, su estabilidad y su compatibilidad con Zotero y Word actuales.

### 17.4 Validación

Cada exportador tendrá pruebas de fidelidad con un documento patrón que incluya todos los nodos admitidos. Una exportación parcial debe advertir qué elementos no pudieron representarse.

La Fase 0 adelantará el documento patrón y una prueba real de CSL y DOCX con tablas, notas al pie, citas documentales, clústeres, cambio de estilo y bibliografía. Deben registrarse aplicaciones de lectura y versiones verificadas. La presencia de `html-docx-js` en exportación OCR no demuestra fidelidad para este contrato; se reutilizará solo si supera la prueba.

La matriz de fidelidad distinguirá soporte nativo, representación alternativa documentada y elementos no admitidos por formato. Una advertencia no permite declarar cumplido un elemento obligatorio que DOCX deba conservar. Los problemas críticos se resolverán o elevarán antes de fijar el esquema definitivo; no se postergarán hasta Fase 8.

## 18. Diseño visual y accesibilidad

La sección debe mantener estrictamente la identidad visual de EntropIA:

- tema oscuro monocromático
- variables CSS existentes
- tipografía, espaciado e iconografía compartidos
- bordes sutiles y fondos del sistema actual
- ausencia de controles azules y bordes blancos discordantes
- paneles redimensionables y plegables
- ancho máximo aprovechable
- modo de escritura concentrada

También deberá incluir:

- navegación completa por teclado
- foco visible y coherente
- etiquetas accesibles para botones con iconos
- menús que no dependan únicamente del color
- contraste suficiente
- comportamiento correcto con zoom de interfaz
- ausencia de desbordes horizontales

## 19. Rendimiento

La implementación se probará con:

- artículos de 10.000 a 20.000 palabras
- capítulos de aproximadamente 30.000 palabras
- documentos mayores como prueba de estrés
- centenares de citas
- bibliografías extensas

Reglas:

- no ejecutar RAG, CSL o llamadas LLM por cada tecla
- cachear representaciones derivadas regenerables
- cancelar búsquedas obsoletas
- cargar paneles secundarios de manera diferida
- evitar rerenderizar el documento completo por cambios periféricos
- aislar edición, persistencia, recuperación e IA
- medir tiempos de apertura, guardado, búsqueda y exportación

Si ProseMirror muestra límites con manuscritos muy grandes, se evaluará división por capítulos o secciones sin romper la experiencia unificada ni la exportación.

## 20. Manejo de errores

Cada operación deberá devolver errores tipados y mensajes accionables.

Casos mínimos:

- fallo al abrir o guardar un documento
- JSON incompatible o corrupto
- migración fallida
- referencia a asset inexistente
- Zotero cerrado o API deshabilitada
- item Zotero eliminado o modificado
- estilo CSL inválido
- llamada LLM cancelada o fallida
- sugerencia desactualizada respecto del texto
- exportación incompleta
- falta de espacio en disco
- cierre inesperado durante una transacción

Una falla en Zotero, el agente o la recuperación no debe bloquear la escritura manual. Una falla de guardado sí debe quedar claramente visible y activar mecanismos de recuperación.

## 21. Seguridad e integridad

- validar todo JSON recibido del frontend y de servicios externos
- utilizar consultas parametrizadas
- envolver cambios relacionados en transacciones SQLite
- no ejecutar HTML no sanitizado proveniente del documento, Zotero o el LLM
- validar archivos CSL antes de incorporarlos
- limitar tamaño de respuestas y timeouts de conectores
- no incluir claves API en documentos, versiones o logs
- evitar registrar texto completo sensible en logs ordinarios
- impedir que contenido recuperado del corpus se interprete como instrucción del sistema para el agente
- conservar copias anteriores durante migraciones

## 22. Estrategia de pruebas

### 22.1 Pruebas unitarias

- serialización y migración del JSON del editor
- creación y renderizado de citas documentales
- clústeres Zotero y locators
- renderizado CSL
- cálculo y validación de hashes
- políticas de snapshots
- construcción acotada del contexto LLM
- conversión a formatos de exportación

### 22.2 Pruebas de integración

- frontend ↔ comandos Tauri ↔ SQLite
- inserción transaccional de cita y procedencia
- Zotero disponible, cerrado y con errores
- apertura del visor en el asset y página correctos
- aceptación y descarte de sugerencias
- recuperación tras terminación forzada
- restauración de versiones
- copiar, mover, duplicar, eliminar, deshacer y restaurar citas sin desincronizar JSON, proyecciones y procedencia
- anchors ante división/fusión de bloques, cambios externos al rango, borrado del objetivo y selecciones ambiguas
- dos guardados con la misma revisión base; rechazo del obsoleto sin pérdida silenciosa
- aplicación repetida de una sugerencia sin duplicar contenido
- creación de nota con item de destino y conservación del snapshot ante borrado
- journal truncado, escritura continua y conflicto con revisión canónica posterior
- apertura del mismo documento en Pro y Lite, con Zotero y capacidades de IA disponibles y ausentes
- cambio de instancia Zotero sin revinculación silenciosa de citas

### 22.3 Pruebas end to end

1. crear un artículo
2. escribir y estructurar secciones
3. cerrar inesperadamente y recuperar cambios
4. buscar una fuente del corpus
5. insertar un fragmento y volver al asset original
6. buscar un libro en Zotero
7. insertar una cita con página
8. cambiar el estilo CSL
9. generar bibliografía
10. pedir al agente una revisión con evidencia
11. aceptar o descartar la sugerencia
12. exportar a los tres formatos

### 22.4 Pruebas de regresión

La incorporación de Escritura no debe alterar:

- importación y navegación de colecciones
- visor de assets
- OCR y texto extraído
- notas
- búsqueda textual y semántica
- NER y tripletas
- Chat/RAG
- exportaciones existentes

## 23. Fases de implementación

Las fases siguientes son áreas de trabajo, no autorizaciones para acumular capas sin un recorrido ejecutable. Se entregarán estos incrementos verticales, conservando todos los criterios de §25:

| Incremento | Fases implicadas | Evidencia de salida |
|---|---|---|
| Validación arquitectónica | 0 | Pruebas académicas Tiptap, Zotero, CSL/DOCX y contratos resueltos |
| Manuscrito durable | 1–3 | Crear, editar, guardar, matar el proceso, recuperar y restaurar en ambas variantes |
| Escritura con evidencia | 4–5 y procedencia de 7 | Buscar, citar, volver a la fuente, copiar/vincular notas y verificar undo/restauración |
| Bibliografía académica | 6 y conversión de 8 | Insertar clústeres, cambiar estilo, trabajar sin Zotero y exportar el patrón |
| Asistencia revisable | 7 | Propuesta con contexto acotado, aceptación atómica y rechazo de rangos desactualizados |
| Entrega completa | 8–9 | Todos los formatos, fidelidad, rendimiento, accesibilidad y regresión; totalidad de §25 |

Los exportadores evolucionarán con los nodos que incorpora cada incremento. La procedencia de corpus y notas se implementará junto con sus inserciones, no después de habilitar IA.

### Fase 0. Auditoría y decisiones técnicas

Entregables:

- mapa de arquitectura real
- tabla de reutilización de componentes y servicios
- prueba académica de Tiptap existente: JSON versionado, extensiones, anchors y manuscrito largo
- prueba de Zotero local para ambas variantes, con permisos, ausencia, identidad de instancia y límites de consulta
- elección del procesador CSL y su ejecución, respaldada por cambio de estilo y bibliografía
- documento patrón y prueba real de DOCX, incluyendo notas al pie y matriz de fidelidad
- definición de esquema, autoridad JSON/proyecciones y migraciones en el sistema existente
- contrato de revisión esperada, anchors y aplicación idempotente de sugerencias
- elección del almacenamiento durable del journal, plazo máximo y ventana de pérdida medible
- inventario de capacidades y adaptación de proveedores del agente
- política de notas con item de destino, borrado de fuentes y tratamiento en backup/sincronización

No avanzar si las pruebas revelan incompatibilidades críticas de edición, recuperación, CSL o DOCX. Documentar evidencia y decisiones pendientes; un resultado de compilación no sustituye los recorridos reales.

### Fase 1. Persistencia y dominio

- migraciones mediante la autoridad existente de `packages/store`
- acceso persistente y comandos específicos solo donde aporten validación o atomicidad
- asociaciones documento-colección
- esquema versionado, configuración histórica y proyecciones de citas
- revisión esperada comprobada atómicamente y errores tipados
- validación de transiciones de guardado e invariantes documentales

### Fase 2. Editor básico

- navegación Escritura
- listado y creación de documentos
- Tiptap integrado con la estética de EntropIA
- barra de herramientas mínima
- esquema por encabezados
- undo, redo y búsqueda
- paneles plegables y modo concentrado

### Fase 3. Guardado y recuperación

- debounce y guardado forzado
- journal durable con plazo máximo durante escritura continua y ventana de pérdida declarada
- snapshots
- historial básico
- restauración no destructiva
- pruebas de cierre inesperado

Esta fase debe completarse antes de incorporar operaciones de IA.

### Fase 4. Corpus y citas documentales

- reutilización de búsquedas existentes
- resultados en el panel derecho
- inserción estructurada
- navegación de regreso a la fuente
- verificación de integridad
- política de eliminación de assets referenciados
- procedencia desde la primera inserción y coherencia ante copia, movimiento, undo/redo y restauración

### Fase 5. Notas

- consulta y búsqueda
- inserción como copia o vínculo
- creación de notas desde el texto con elección de item real de destino
- notas como contexto del agente

### Fase 6. Zotero y CSL

- detección del endpoint local de Zotero en Pro y Lite, sin inferir instalación a partir de un fallo de conexión
- lectura de bibliotecas, colecciones e items
- búsqueda
- caché regenerable particionada por origen e instancia y consultas limitadas
- inserción y edición de clústeres
- locators, prefijos y sufijos
- estilos CSL
- bibliografía dinámica
- funcionamiento degradado sin Zotero

### Fase 7. Agente y procedencia

- acciones sobre selección o sección
- constructor de contexto
- resultados RAG y Zotero diferenciados
- sugerencias no destructivas
- validación de anchors y contenido objetivo, no solo hash del documento completo
- adaptación de proveedores según capacidades reales y procedencia de operaciones de IA
- aceptación, inserción y descarte

### Fase 8. Exportación

- Markdown
- HTML sanitizado
- DOCX
- notas al pie
- citas documentales configurables
- citas Zotero y bibliografía CSL
- completar la matriz y las pruebas de fidelidad iniciadas en Fase 0 y ampliadas con cada nodo

### Fase 9. Endurecimiento y publicación

- pruebas de rendimiento y estrés
- accesibilidad
- migraciones desde versiones de desarrollo
- regresión completa
- documentación de usuario
- telemetría local o logs diagnósticos respetuosos de la privacidad, si ya existe ese mecanismo

### Fases posteriores

- comparación visual de versiones
- control de cambios editorial
- Zotero Web API y OAuth
- adjuntos Zotero como fuente RAG
- exportación experimental con campos Zotero vivos
- importación DOCX
- LaTeX y PDF
- colaboración multiusuario
- plantillas de revistas

## 24. Riesgos y mitigaciones

| Riesgo | Mitigación |
|---|---|
| Pérdida de trabajo | Journal durable, plazo máximo en escritura continua, ventana de pérdida medida y pruebas de terminación forzada |
| Corrupción del JSON | Validación de esquema, transacciones, hashes, copias previas y migraciones probadas |
| Cambios incompatibles de Tiptap | Fijar versiones, usar extensiones propias pequeñas y mantener `schema_version` |
| Documentos demasiado grandes | Pruebas de estrés, renderizado controlado y posible segmentación futura |
| Citas documentales rotas | IDs estables, snapshot de metadatos, estado de integridad y política de borrado |
| Zotero no disponible | Caché local y snapshots CSL, edición no bloqueante y reintento explícito |
| Items Zotero modificados | Guardar versión/snapshot, comparar al refrescar y no sobrescribir sin informar |
| Renderizado CSL lento | Recalcular por eventos relevantes, cachear resultados y no hacerlo por tecla |
| Sugerencias aplicadas sobre texto cambiado | Anchors transformables, hash del objetivo y comprobación atómica de revisión al aplicar |
| Respuestas del LLM sin respaldo | Mostrar fuentes usadas y diferenciar inferencia, corpus y metadatos Zotero |
| Fuga de información a proveedores | Vista previa del contexto y respeto de la configuración local/externa |
| Exportación DOCX imperfecta | Documento patrón, advertencias y separación entre modelo interno y exportador |
| Expectativa de campos Zotero vivos | Declararlos fuera del MVP hasta demostrar compatibilidad real |
| Duplicación de servicios | Auditoría obligatoria y adaptadores sobre búsquedas, notas, RAG y proveedores existentes |
| UI sobrecargada | Paneles plegables, acciones contextuales y alcance limitado del MVP |
| JSON y tablas de citas divergentes | JSON canónico, proyecciones derivadas y commit conjunto en todas las operaciones estructurales |
| Sobrescritura entre ventanas o instancias | Revisión esperada y escritura en una misma transacción |
| Caché Zotero de otra instancia | Partición por origen/instancia; conservar snapshots sin revincular por coincidencia de clave |
| Separación prematura en repositorios | Módulo interno; extracción solo ante consumidor o distribución independiente reales |

## 25. Criterios de aceptación del MVP

El MVP se considerará completo cuando se verifiquen todos estos criterios en Pro y Lite. Las acciones de IA se comprobarán con cada proveedor declarado compatible y mostrarán indisponibilidad explícita cuando falte una capacidad; ninguna ausencia bloqueará la edición manual.

1. Se puede crear, abrir, duplicar, renombrar, archivar y eliminar un documento.
2. El editor admite la estructura y formatos definidos.
3. El esquema lateral navega correctamente por títulos y subtítulos.
4. El documento se autoguarda sin escribir en SQLite por cada tecla.
5. Un cierre inesperado permite recuperar las secuencias durables posteriores al último guardado canónico; la pérdida de cambios aún no confirmados respeta la ventana declarada, incluso durante escritura continua.
6. Se pueden consultar y restaurar versiones sin destruir el historial.
7. Se puede buscar el corpus sin abandonar Escritura.
8. Se puede insertar un fragmento con identidad y procedencia estructuradas.
9. Al pulsar una cita documental se abre el asset y la página correspondientes.
10. Una fuente ausente se informa sin borrar la cita ni impedir la edición.
11. Se pueden consultar e insertar notas como copia o vínculo, y crear notas desde una selección eligiendo un item real de destino.
12. EntropIA conecta con Zotero Desktop en Pro y Lite, informa permisos o indisponibilidad sin diagnósticos falsos y permite editar y renderizar citas existentes cuando Zotero está cerrado.
13. Se puede buscar una referencia en la biblioteca Zotero.
14. Se puede insertar y editar una cita Zotero con locator, prefijo y sufijo.
15. Una cita con varios items se conserva como clúster estructurado.
16. Se puede cambiar el estilo CSL y actualizar todas las citas.
17. La bibliografía automática contiene solo las obras citadas.
18. El agente puede actuar sobre una selección sin modificarla automáticamente.
19. Cada sugerencia permite reemplazar, insertar debajo o descartar.
20. Las sugerencias sustentadas en el corpus muestran las fuentes utilizadas.
21. Las referencias Zotero sugeridas se distinguen de evidencia textual realmente consultada.
22. La procedencia de inserciones y transformaciones importantes puede inspeccionarse.
23. Se puede exportar a Markdown, HTML y DOCX.
24. El DOCX conserva formatos, notas, citas y bibliografía dentro de las capacidades declaradas.
25. La incorporación del módulo no rompe las funcionalidades existentes de EntropIA.
26. Copia/pegado, duplicación, movimiento, eliminación, undo/redo y restauración conservan la coherencia del contenido, citas y procedencia.
27. Un guardado obsoleto no sobrescribe una revisión posterior; un journal incompatible o truncado no destruye el documento confirmado.
28. Una sugerencia nunca se aplica a un rango ambiguo o modificado; cambios ajenos al objetivo pueden conservar su validez y repetir una aceptación no duplica texto.
29. Cambiar de instancia Zotero no mezcla versiones ni revincula automáticamente las citas del manuscrito.
30. Un documento puede abrirse, editarse y exportarse en ambas variantes con el mismo esquema y sin requerir ML local.

## 26. Componentes y archivos previstos

Los puntos de integración comprobados están en §5.5. Las rutas nuevas y símbolos definitivos deben surgir de la Fase 0 y respetar la distribución de §2.1; la lista siguiente representa responsabilidades, no un archivo ni un servicio por línea.

### Frontend

- ruta o página principal de Escritura
- vista de administración de documentos
- shell de tres paneles
- wrapper del editor Tiptap
- barra de herramientas
- esquema del documento
- pestaña Corpus
- pestaña Zotero
- pestaña Notas
- pestaña Agente
- editor de citas documentales
- editor de clústeres Zotero
- selector de estilo CSL
- bibliografía dinámica
- historial y recuperación
- diálogo de exportación
- visor de procedencia

### Persistencia e integración de escritorio

- extensión de `packages/store` para esquema, repositorios y migraciones
- operaciones de guardado y versiones sobre el puente Tauri existente, con validación y revisión atómica
- recuperación durable y acceso a archivos en el backend
- conector y caché Zotero
- adaptación al motor de investigación y a proveedores compatibles, sin trasladar el estado editorial al agente

### Lógica documental separada de las vistas

- modelos, validación, migraciones JSON y transformaciones
- citas documentales, clústeres, vínculos de notas y anchors
- proyecciones e invariantes de procedencia
- renderizado CSL y exportadores, con lugar de ejecución elegido por evidencia
- navegación a assets mediante la interfaz existente del frontend

### Compartidos

- tipos serializables con versiones explícitas
- códigos de error
- validadores de esquema
- fixtures de documentos y citas

El implementador deberá enumerar los archivos reales a modificar y crear después de la Fase 0. No debe inventar rutas sin inspeccionar el repositorio.

## 27. Decisiones que deben resolverse en la Fase 0

1. Extensiones académicas y versiones Tiptap compatibles con el JSON canónico; reutilización concreta de controles sin alterar el contrato de notas.
2. Interfaces del módulo interno y división frontend/backend según §2.1; no reabrir la ubicación en este repositorio.
3. Biblioteca CSL, lugar de ejecución y evidencia de cambio de estilo y bibliografía.
4. Anchors y transformación de rangos ante edición, división/fusión, movimiento, borrado y undo/redo.
5. Almacenamiento durable del journal, plazo máximo en escritura continua, garantías de fallo, retención y compactación de versiones.
6. Política de borrado de assets, items y colecciones referenciados, incluida preservación de snapshots.
7. Caché Zotero por origen/instancia, compatibilidad con versiones sin ID de servidor y uso sin conexión.
8. Apertura de items Zotero según plataforma y estados de conexión que pueden diagnosticarse realmente.
9. Conversión a DOCX de notas al pie, citas y bibliografía con matriz de fidelidad y aplicaciones verificadas.
10. Representación de procedencia y proyecciones, autoridad JSON y comportamiento de todas las operaciones estructurales.
11. Validación del esquema común Pro/Lite y matriz de capacidades disponibles por variante y proveedor.
12. Alcance exacto del contexto externo y adaptación necesaria frente al flujo actual de investigación con OpenRouter.
13. Control atómico de revisión esperada, orden de guardados y aplicación idempotente de sugerencias.
14. Contrato de creación de notas con item de destino y representación durable de vínculos desde el manuscrito.
15. Integración de los datos nuevos con backup, restauración y sincronización existentes, sin confundirla con colaboración simultánea.

Cada decisión deberá registrarse mediante una nota arquitectónica breve con alternativa elegida, alternativas descartadas y consecuencias.

## 28. Definición final del producto

**Escritura** será el espacio donde converjan el texto del investigador, las fuentes documentales de EntropIA y la bibliografía académica de Zotero. Su valor no residirá en acumular funciones de edición, sino en conservar las relaciones entre redacción, evidencia, bibliografía, notas, recuperación y asistencia algorítmica.

La implementación deberá juzgarse siempre con una pregunta sencilla:

> ¿Esta decisión ayuda a escribir mejor sin perder el camino que conduce desde una afirmación hasta sus fuentes y operaciones de investigación?

Si la respuesta es negativa, la funcionalidad debe simplificarse, posponerse o descartarse.

## 29. Resultado de la Fase 0 y decisiones tomadas

La auditoría exigida por §5 está completa en `docs/escritura-phase-0-audit.md`, anclada al commit `65c9bb7`. Registra once brechas bloqueantes (G1–G11) y resuelve seis de las quince decisiones de §27. Lo que sigue depende de ese documento; no se repite aquí.

### 29.1 Decisiones resueltas

| Decisión | Resultado | Consecuencia para el plan |
|---|---|---|
| §27.2 — propiedad de la persistencia | Módulo Rust `writing/` posee lectura y escritura; `packages/store` conserva esquema, migraciones y fixture, igual que `processing_*` | G1 y G2 se resuelven reutilizando `processing/repository.rs:689-885` y `processing/scheduler.rs:104-131` en lugar de ampliar `db_execute_transaction`, comando compartido por toda la aplicación |
| §27.1 — extensiones Tiptap | Tablas y notas al pie alcanzables sobre el pin instalado `2.26.4`: `@tiptap/extension-table@2.26.4` y `tiptap-footnotes@2.0.4` | No se migra a Tiptap 3. `NoteEditor` y su contrato HTML quedan intactos |
| §27.3 — procesador CSL | `hayagriva` 0.10.1 en Rust (MIT OR Apache-2.0). citeproc-js es CPAL-1.0 OR AGPL-1.0 y `@citation-js/plugin-csl` depende de él pese a declararse MIT | S3 ejecutado: hayagriva cubre clústeres, locators, cambio de estilo y bibliografía, y consume CSL-JSON de Zotero sin conversión. No cubre afijos por item ni supresión de autor. §9.5 no cambia; la Unidad 6 incorpora un helper de afijos. Detalle en `docs/escritura-spikes.md` |
| §27.6 — borrado de fuentes referenciadas | Convención existente de instantánea sin FK viva (`entities.asset_id`, `triples.asset_id`, `processing_*`) | §9.4 y §9.5 se implementan con columnas nulables sin FK, más `metadata_snapshot_json` e `integrity_status` |
| §27.14 — notas con item de destino | Confirmado: `notes.item_id` es `NOT NULL` en todo el historial de migraciones | §13.1 se implementa tal cual está escrito |
| Ordinal y puntos de contacto | Próximo ordinal libre: `0035`. Una tabla nueva toca cinco lugares | Detallado en la Unidad 1 |

### 29.2 Nomenclatura adoptada

El prefijo `research_*` propuesto en §9 se reemplaza por **`writing_*`**. Razón: `research` ya nombra otra funcionalidad del repositorio (`apps/desktop/src-tauri/src/research.rs`, con su propio `estado.sqlite`), y el prefijo debe nombrar este módulo, no la actividad genérica. El módulo Rust correspondiente es `writing/`, en línea con `processing/`, `sync/` y `nlp/`.

Tablas: `writing_documents`, `writing_document_collections`, `writing_document_versions`, `writing_document_citations`, `writing_zotero_citations`, `writing_provenance_events`, `writing_agent_suggestions`. Los campos conservan los nombres de §9 salvo donde el esquema real ya tiene un sinónimo establecido: `source_start` y `source_end` se escriben `start_char` y `end_char`, como en `rag_chunks`.

### 29.3 Decisiones aún abiertas

- §27.7 y §27.15 — si el manuscrito entra en `SYNCED_TABLES`. Se decide en la Unidad 1 con el esquema a la vista, no antes.
- §27.4, §27.5, §27.9, §27.12 — dependen de los spikes de la Unidad 0.

## 30. Plan de implementación por unidades revisables

Orden obligatorio 0→1→2→3→4→5→6→7→8→9. La Unidad 0 no es opcional: §23 la llama «Validación arquitectónica» y su resultado puede modificar el esquema de las unidades siguientes. Ninguna unidad se considera entregada con datos simulados. Antes de modificar símbolos exportados, localizar referencias y migrar todos los productores identificados. Los archivos nuevos son propuestas; el resto son puntos existentes inspeccionados en `docs/escritura-phase-0-audit.md`.

### Unidad 0 — Validación arquitectónica

**Archivos:** crear `docs/escritura-spikes.md` como registro único de resultados. El código de prueba vive fuera del árbol de producción; ningún spike se integra sin pasar por la unidad que le corresponde.

**Consume:** la auditoría de Fase 0. **Produce:** evidencia medida para §27.3, §27.4, §27.5, §27.9 y §27.12, o una elevación explícita.

- [x] S1 Tiptap y JSON canónico — **ejecutado**. El pin 2.26.4 aguanta: tablas, notas al pie y nodo propio conviven sin conflicto de peers, y la ida y vuelta es idéntica byte a byte. 30.000 palabras son 0,32 MB de JSON, abren en 26 ms y una edición cuesta 5,5 ms. **Hallazgo grave: un nodo o una marca desconocidos vacían el documento entero**, no solo el elemento desconocido, y un autoguardado ingenuo persistiría ese vacío sobre el manuscrito real. La validación previa al montaje lo detecta con motivo utilizable. Consecuencias vinculantes para las Unidades 1, 2 y 3 en `docs/escritura-spikes.md`. StarterKit no trae `link` ni `underline`: agregarlos explícitamente en la Unidad 3.
- [x] S2 Anchors — **ejecutado**. El mapeo dentro de la sesión resuelve bien inserción, división y fusión de bloques. **`deletedAcross` es el predicado equivocado**: da `false` en tres de los cuatro casos destructivos, incluido el más simple. El hash del contenido objetivo discrimina exactamente como pide §9.7, conservando validez ante cambios ajenos al rango. **Undo no restaura el anclaje**: el texto vuelve pero la cadena de mapeo no, así que hay que re-resolver desde identidad estable. Y una posición absoluta persistida apunta a otro párrafo tras recargar, mientras que id de nodo más offset resuelve correcto: §9.6 y §9.7 deben guardar identidad, no `{from, to}`.
- [x] S3 CSL con `hayagriva` 0.10.1 — **ejecutado, aprobado con dos limitaciones**. Cubre clústeres, locator y tipo de locator, cambio de APA a Chicago y bibliografía solo con obras citadas, y consume CSL-JSON de Zotero sin capa de conversión. No expone afijos por item ni supresión de autor; los afijos se resuelven inyectándolos en el árbol renderizado, que es público. Ningún criterio de §25 queda sin cubrir. Resultados y consecuencias en `docs/escritura-spikes.md`.
- [x] S4 DOCX — **ejecutado y verificado en Word**. `html-docx-js` **no genera un DOCX**: su `document.xml` son 2 KB de espacios de nombres envolviendo un `altChunk`, y el contenido real es un MHTML en `afchunk.mht`, así que lo que se ve lo produce el importador de la aplicación que abre. Sin `footnotes.xml`, sin tabla, sin hipervínculo, sin estilos de título: **no puede cumplir §17.1**. **Se reemplaza por `docx` 9.7.1 (MIT)**, que cubre los doce elementos de la matriz sin una sola representación alternativa, incluidas **notas al pie que renumeran** al borrar la anterior y bibliografía con sangría francesa. Falta registrar la versión exacta de Word y agregar un lector no-Word en la Unidad 8. Alternativa Rust `docx-rs` 0.4.22 (MIT) a evaluar allí para mantener DOCX junto a CSL. Matriz completa en `docs/escritura-spikes.md`.
- [x] S5 Zotero local — **ejecutado contra una instalación real (9.0.3, ~5.000 items)**. Biblioteca accesible, `403` con cuerpo estable cuando la API está apagada, y `/connector/ping` responde aunque lo esté: separar sonda de vida de sonda de permiso elimina los diagnósticos falsos del criterio 12. Confirmado el peligro de §11.2: sin `limit`, una consulta devolvió **7,9 MB en una sola respuesta**. `Zotero-Server-ID` **no existe en esta versión**, así que §27.7 usa `Last-Modified-Version` y la política conservadora deja de ser alternativa. `format=csljson` alimenta hayagriva sin conversión. Detalle y dos tareas nuevas de normalización en `docs/escritura-spikes.md`.
- [x] S6 Journal — **ejecutado**. Elegida la **tabla de recuperación en SQLite con deltas**, no por velocidad sino porque comparte la transacción con el commit canónico, conserva la secuencia completa y no deja archivos temporales huérfanos. Un delta de 1 KB cuesta **p95 = 1,17 ms** con el `synchronous=FULL` que `db/open.rs` ya fija: **no hay que cambiar ningún PRAGMA**. El documento entero costaría 97,5 ms, así que el journal guarda deltas, nunca el documento completo. Terminación forzada: **ninguna secuencia confirmada se perdió** en 7 corridas, 0 registros corruptos. Plazo recomendado **≤ 500 ms** en escritura continua, con ventana declarada **≤ ~560 ms**; el journal sostiene ~2.800 escrituras durables por segundo, así que el plazo es decisión de producto y no límite técnico. La prueba cubre **muerte de proceso, no corte eléctrico**: esa distinción queda documentada en `docs/escritura-spikes.md`.

**Criterio de aceptación:** cada spike deja un resultado medido o una incompatibilidad elevada en `docs/escritura-spikes.md`. Un resultado de compilación no sustituye el recorrido real. Commit sugerido: `docs(writing): record phase 0 spike results`.

### Unidad 1 — Esquema durable y propiedad Rust del manuscrito

**Archivos:** crear `packages/store/src/migrations/0035_writing_documents.sql`; modificar `packages/store/src/runner.ts` (entrada en `MIGRATIONS` y, por llevar triggers o requerir registro atómico, la rama `BEGIN IMMEDIATE` de `runner.ts:1132-1147`), `packages/store/src/schema.ts`, `packages/store/src/schema-fixture.test.ts` (lista de tablas) y `packages/store/src/runner.test.ts`. Regenerar `apps/desktop/src-tauri/tests/fixtures/schema_full.sql` mediante `packages/store/scripts/export-schema.mjs`. Crear `apps/desktop/src-tauri/src/writing/{mod.rs,repository.rs,commands.rs,tests.rs}`; modificar `apps/desktop/src-tauri/src/lib.rs` para registrar el módulo y sus comandos manteniendo alineada la superficie entre variantes, según `AGENTS.md`. Evaluar `apps/desktop/src-tauri/src/sync/capture.rs` y `sync/cascade.rs`.

**Consume:** conexión y PRAGMAs de `db/open.rs`. **Produce:** las siete tablas de §9 con el prefijo de §29.2, y escritura con revisión esperada comprobada atómicamente.

- [x] Regresión de migración sobre SQLite real escrita **antes** de la migración: seis casos, incluido el corte entre el DDL y el registro en `_migrations`.
- [x] Siete tablas `writing_*` definidas en `0035_writing_workspace` con las convenciones de §4.2. Los ids de corpus en la proyección de citas van sin FK, como `entities.asset_id`; la asociación documento-colección sí cascadea, porque el borrado de colecciones está escrito a mano y un `RESTRICT` lo habría bloqueado.
- [x] Prueba de concurrencia escrita antes del repositorio: el guardado obsoleto falla con `revision_conflict`, la revisión no avanza y el contenido del ganador queda intacto.
- [x] `writing/repository.rs` implementado con `UPDATE ... WHERE id = ?1 AND revision = ?2` y verificación de `rows_affected`. Contenido, proyecciones y procedencia commitean en una sola transacción; un fallo no deja ni revisión avanzada ni filas de proyección.
- [x] Comandos expuestos y completos: crear, abrir, guardar con revisión esperada, sonda de esquema, renombrar, cambiar estado (archivar, papelera, restaurar) y duplicar. Devuelven `Result<T, WritingError>` con `{ code, message }` en vez de cadena opaca. Renombrar y cambiar estado **no** avanzan la revisión: son metadatos, y hacerlo convertiría una edición en vuelo en conflicto. Duplicar crea identidad y ocurrencias propias, copia asociaciones y ambas proyecciones con ids de fila nuevos, y registra su origen; no arrastra historial ni sugerencias pendientes.
- [x] Decidido: **ninguna tabla `writing_*` entra al sync por ahora**, igual que `rag_chunks` y `processing_*`. Subir `TRIGGERS_VERSION` obligaría a recrear los 48 triggers de todos los usuarios, incluidos los que nunca abran Escritura. Se vuelve a decidir con el MVP funcionando. Documentado en `writing/mod.rs`.
- [x] Verificados instalación nueva, reejecución idempotente y reinicio tras migración interrumpida. Fixture regenerado y coincidente byte a byte.

**Criterio de aceptación:** crear un documento, guardarlo, terminar el proceso y reabrir el archivo conserva la revisión confirmada; un guardado con revisión base vieja falla con código y no sobrescribe. Commit sugerido: `feat(writing): persist manuscripts with atomic revision control`.

### Unidad 2 — Journal durable y recuperación

**Archivos:** crear `apps/desktop/src-tauri/src/writing/journal.rs` y `recovery.rs`; ampliar `writing/repository.rs` y `writing/commands.rs`; crear `apps/desktop/src-tauri/tests/writing_recovery.rs` siguiendo el patrón de `tests/processing_recovery.rs`; marcar `pub mod writing` en `lib.rs` como se hizo para `processing` y `sync` en `lib.rs:22-24,32-33`.

**Consume:** la decisión medida de S6. **Produce:** persistencia durable de secuencias no confirmadas y recuperación verificada.

- [x] Pruebas de interrupción escritas antes del código. `tests/writing_recovery.rs` mata un escritor en plena escritura continua (`TerminateProcess`, sin Drop ni hooks) y comprueba que vuelve toda secuencia anunciada como durable. **Mutado para verificar que la aserción muerde**: borrar una entrada anunciada hace fallar el test.
- [x] Journal implementado en `writing/journal.rs` sobre la migración `0036`, con documento, revisión base, secuencia, versión de esquema y checksum. **El checksum lo calcula el backend**, nunca se acepta del llamador. `append()` devuelve secuencia, nunca revisión.
- [x] Plazo máximo implementado como política pura en `apps/desktop/src/lib/writing-scheduler.ts`: dos cadencias, journal cada 250 ms con techo de 500 y guardado canónico a los 1000. **Precondición del contrato descubierta por un test que falló**: quien lo use tiene que respetar `nextCheckInMs`; despertar solo con las teclas deja el techo en 600 ms y vuelve mentira la ventana declarada.
- [x] Comparación al abrir en `writing/recovery.rs`. Reproduce el prefijo verificado, se detiene en la primera entrada con checksum roto, informa dónde paró y cuántas quedaron atrás, y **no borra nada**. Un hueco en la secuencia no se trata como corrupción.
- [x] Snapshots, retención y restauración en `writing/versions.rs`. Una versión guarda contenido **y** configuración bibliográfica. La retención solo compacta `auto`. Restaurar avanza a una revisión nueva y conserva el historial posterior; un restore que pierde la carrera no deja snapshot huérfano.
- [x] Verificado en tres lugares: `append()` devuelve secuencia y no revisión, un test comprueba que journalear no avanza la revisión, y el planificador solo limpia el tramo pendiente con `onSaved()`.

**Criterio de aceptación:** una terminación forzada durante escritura continua recupera toda secuencia confirmada como durable, y la pérdida de lo no confirmado respeta la ventana declarada en S6. Los hooks de cierre no cuentan como prueba. Commit sugerido: `feat(writing): recover unsaved work after forced termination`.

### Unidad 3 — Navegación, shell de tres paneles y editor

**Archivos:** modificar `apps/desktop/src/lib/navigation.ts` (variante nueva en `View` y en `RootSectionView`), `apps/desktop/src/lib/route-loader.ts` y `apps/desktop/src/App.svelte`. Crear `apps/desktop/src/lib/writing.ts` como store de clase plana, siguiendo `rag-chat.ts:60`. Crear `apps/desktop/src/views/WritingView.svelte`, `WritingDocumentList.svelte`, `WritingOutlinePanel.svelte`, `WritingToolbar.svelte` y `WritingResearchPanel.svelte`. Crear en `packages/ui` los primitivos ausentes de G10 que resulten realmente compartidos: panel redimensionable, panel plegable, menú desplegable y contenedor modal. Crear `packages/ui/src/components/WritingEditor/` con el wrapper Tiptap propio. Modificar `apps/desktop/package.json` y `packages/ui/package.json` para las dos dependencias nuevas.

**Consume:** S1, S2 y los comandos de la Unidad 1. **Produce:** el recorrido de §6 con JSON canónico.

- [x] Registrar la sección en las tres coordenadas de navegación y comprobar la carga diferida.
- [x] Implementar el wrapper Tiptap con contrato JSON versionado, replicando el patrón de montaje de `NoteEditor.svelte:279-340,927-968` sin tocar `NoteEditor` ni su contrato HTML. **El contrato solo no alcanza**: un documento puede ser válido y aún así no renderizarse — un `footnoteReference` huérfano pasa el esquema y tumba el editor entero. Por eso `parseCanonical` valida **y** repara, y reporta la reparación en memoria sin persistirla (§8.3).
- [x] Extraer los primitivos de G10 a `packages/ui` solo si son genuinamente compartidos; la orquestación específica de Escritura no vive allí, conforme a §2.1. **Decidido: ninguno de los cuatro se extrae.** El contenedor modal ya existía (`ConfirmDialog`), las pestañas también (`TabList`, `TabButton`) y el campo de búsqueda también (`SearchBar`). El panel plegable se usa dos veces dentro de una misma vista, que es repetir dos líneas, no compartir; el redimensionable y el menú desplegable no tuvieron ningún consumidor. Construirlos por cumplir la lista habría dejado cuatro primitivos sin segundo usuario que los mantenga honestos.
- [x] Derivar el esquema lateral del documento, no mantener una copia paralela. Navegación por encabezados, plegado, creación, renombrado, movimiento y eliminación de secciones. Una **sección** es un encabezado más todo lo que sigue hasta el próximo encabezado de nivel igual o superior: un H2 arrastra sus H3, un H3 corta en el próximo H2 y una subsección no puede salir de su capítulo. El bloque de notas al pie nunca es parte de una sección. Las posiciones salen de ProseMirror y no de aritmética propia. **Solo eliminar pide confirmación**, y reporta cuántas palabras se van: que una acción sea reversible no alcanza si el error queda invisible hasta que el historial de deshacer ya avanzó.
- [x] Implementar barra de herramientas, undo, redo, búsqueda y reemplazo, atajos convencionales y modo concentrado con ambos laterales plegados. La búsqueda aplana el documento antes de comparar, porque un recorrido por nodos de texto no encuentra una frase partida por una negrita; usa decoraciones y no marcas, porque buscar no puede dejar rastro en el JSON canónico; y reemplazar todo va de atrás hacia adelante en una sola transacción.
- [x] Respetar los dos guardias de estética: iconos exclusivamente por `ActionIcon` y escala de control de `design-tokens.test.ts`. No replicar los botones a mano de `NoteEditor`. El guard rechazó un `input type="search"` escrito a mano y obligó a usar `SearchBar`: tenía razón.
- [x] Implementar los estados visibles de §16.2. Un error de guardado no desaparece solo y ofrece reintento. **El reintento se retiene ante `revision_conflict`**: otra ventana avanzó la revisión, así que la misma revisión esperada solo puede volver a fallar y forzarla pisaría trabajo ajeno. Ahí el mensaje explica y apunta al journal.
- [ ] Verificar visualmente con la aplicación real, en Pro y en Lite. **Parcial:** verificado en Lite a lo largo de la unidad. Falta Pro.

**Criterio de aceptación:** se crea, abre, edita y guarda un documento con JSON canónico; el esquema navega correctamente; ningún guardia de estética falla. Commit sugerido: `feat(writing): add the writing workspace and its editor`.

### Unidad 4 — Corpus, citas documentales y regreso a la fuente

**Archivos:** ampliar `apps/desktop/src/lib/navigation.ts` con página y rango en la variante de item (G4); modificar `apps/desktop/src/views/ItemView.svelte` para aceptar esa página en lugar de reiniciarla en `ItemView.svelte:2358`, y `ItemAssetPanel.svelte` para propagarla; ampliar `packages/ui/src/components/DocumentViewer/` con resaltado por rango (G3). Crear `apps/desktop/src/views/WritingCorpusTab.svelte`. Ampliar `writing/repository.rs` y `writing/commands.rs` con la proyección de citas documentales.

**Consume:** servicios de búsqueda existentes. **Produce:** inserción trazable y recorrido de ida y vuelta.

- [x] Reutilizar `FtsSearchController` y `similarAssets`; no implementar un segundo motor de recuperación. La pestaña Corpus usa `store.fts`, el mismo índice de la vista de item, con el mismo controlador. **Hallazgo:** el comando Tauri `fts_search` es un stub que devuelve `{note: "Use db_select..."}` y `nlp.ts` lo tipa como `FtsResult[]`; esa firma miente y no debe usarse.
- [x] Resolver la brecha de direccionamiento de G3: el visor superpone geometría en píxeles y la procedencia usa offsets de caracteres. Definir el mapeo o declarar explícitamente el alcance del resaltado. **Se declara el alcance; no se define un mapeo, porque no existe.** La cadena de direccionamiento es `asset_id` más `[start_char, end_char)` sobre `extractions.text_content`, y es exacta: `assets.page_number` (migración `0024`) hace que **un asset sea una página**, así que el asset ya identifica la página sin aritmética adicional. Lo que falta es geometría: la disposición que produce el OCR vive en `ocr/layout_onnx.rs` en tiempo de procesamiento y **ninguna migración la persiste** — no hay una sola columna de bounding box en el esquema. Sin cajas guardadas, un offset de caracteres no puede convertirse en píxeles, y derivarlo re-ejecutando OCR al abrir una cita sería no determinista frente al texto que la cita registró. En consecuencia, §10.2 queda así: los pasos 1, 2 y 3 se cumplen exactos; el **paso 4 se resuelve en el panel de texto extraído, no sobre la imagen**; el paso 5 no cambia. Si alguna vez se persiste la geometría del OCR, el resaltado sobre la imagen se agrega sin tocar el direccionamiento, porque el ancla ya es exacta.
- [x] Insertar cita creando nodo, proyección y evento de procedencia en una sola transacción. Ante fallo, conservar el borrador y el estado de error, sin registros parcialmente confirmados. **La proyección se deriva del documento**, nunca se mantiene al lado: `save_document` la reemplaza en la misma transacción que escribe el contenido, así que no puede desincronizarse. El evento de procedencia espera en el store hasta ese guardado y se limpia **por la cantidad enviada**, no vaciando la cola, para no perder una cita insertada mientras el guardado estaba en vuelo.
- [x] Implementar apertura de la fuente con los cinco pasos de §10.2, incluida la degradación a fragmento y metadatos cuando el anclaje ya no resuelve. Los cuatro desenlaces son el vocabulario que la columna `integrity_status` ya define. **`source_modified` es el caso que una posición guardada no detecta**: el rango sigue resolviendo, solo que a otras palabras, y solo el hash lo distingue; ahí se abre la página pero no se resalta. El fragmento se ubica en el panel **por su texto, no por los offsets**, porque `renderOcrHtml` reescribe la extracción y un offset crudo no nombra ninguna posición del DOM renderizado.
- [x] Probar copia, pegado, duplicación, movimiento, borrado, undo/redo y restauración sin desincronizar JSON, proyecciones y procedencia. Copiar crea identidades nuevas conservando la referencia a la fuente; mover conserva identidad. Derivar la proyección resuelve todos estos casos de una: ninguna de esas operaciones puede cambiarla sin cambiar el documento del que sale. **Copiar y mover se distinguen por lo que queda atrás**: ambos terminan en un pegado, pero tras un corte no hay segundo reclamante, así que no hay colisión y no se reemite nada. Cubierto por tests para copia, pegado, movimiento, borrado y undo; la duplicación de documentos ya la cubrían los tests Rust de la Unidad 1 y la restauración se sigue estructuralmente de la derivación.
- [x] Implementar la política de borrado de assets referenciados según §29.1 y advertir dependencias antes de confirmar. Advertencia, nunca veto: `writing_citations_for_asset` dice qué manuscritos citan el asset y la cita sobrevive con su fragmento y metadatos. Toda forma de fallar termina en "sin advertencia", jamás en un borrado que el usuario no pueda completar — esquema ausente responde vacío, y el cliente **comprueba la forma de la respuesta** en vez de asumirla.

**Criterio de aceptación:** se busca en el corpus sin salir de Escritura, se inserta un fragmento con procedencia, se vuelve al asset y la página correctos, y una fuente ausente se informa sin borrar la cita. Commit sugerido: `feat(writing): insert traceable corpus citations`.

### Unidad 5 — Notas

**Archivos:** crear `apps/desktop/src/views/WritingNotesTab.svelte`; ampliar `packages/store/src/repos/note.repo.ts` con búsqueda transversal (G11); ampliar `writing/repository.rs` con los vínculos a notas.

- [x] Agregar búsqueda de notas por alcance, sin relajar `notes.item_id`. `NoteRepo.search` con SQL propio, probada contra el esquema real: una búsqueda que devuelve una nota de la colección equivocada no es más lenta, es incorrecta. Una lista vacía de colecciones es un alcance (no busca en ninguna), y los comodines de LIKE van escapados.
- [x] Distinguir copiar de vincular: copiar crea texto independiente; vincular mantiene relación viva e informa divergencia sin sobrescribir. **Un vínculo guarda instantánea y hash, no una referencia para re-leer** — re-leer sería la sobreescritura automática que §13 prohíbe. `resolveNoteLink` informa y **ofrece** el texto nuevo; nunca lo aplica. Dos botones separados, cada uno con su consecuencia escrita al lado: un desplegable enterraría la decisión. Copiar no registra procedencia; vincular sí.
- [x] Implementar creación de nota desde una selección con elección de item real de destino, filtrado por las colecciones asociadas al manuscrito. El selector es el requisito, no una comodidad: `notes.item_id` sigue `NOT NULL` y no se acuña ningún item ficticio. **Pendiente de datos:** nada en la app escribe todavía `writing_document_collections`, así que el filtrado por colecciones está construido y probado pero no tiene de dónde alimentarse.
- [x] Conservar ID, snapshot y hash del contenido insertado. Si la nota se elimina, informar la ausencia sin borrar texto ni snapshot. Todo vive en el nodo: §8.1 dice que el JSON canónico contiene "identidad de nodos, citas, vínculos y snapshots", así que **no se agregó tabla ni migración**. Los cuatro desenlaces son el mismo vocabulario de integridad que las citas. **Borrada y no legible son respuestas distintas**: una lectura fallida reportaba la nota como eliminada, y eso era un defecto real corregido acá.

**Criterio de aceptación:** criterio 11 de §25 verificado en ambas variantes. Commit sugerido: `feat(writing): link and copy research notes`.

### Unidad 6 — Zotero y CSL

**Archivos:** crear `apps/desktop/src-tauri/src/writing/zotero/{mod.rs,connector.rs,cache.rs}` y `writing/csl.rs`; crear `apps/desktop/src/views/WritingZoteroTab.svelte` y `WritingCitationDialog.svelte`; modificar `apps/desktop/src-tauri/Cargo.toml`.

Su alcance definitivo depende de S3 y S5. Si S3 falla, esta unidad se detiene y se eleva antes de fijar §9.5.

- [x] Conector local de solo lectura con timeout, límites y paginación explícitos. `writing/zotero/connector.rs`: **no existe función que arme una petición sin `limit` y `start`** — S5 midió 7,9 MB en una sola respuesta sin límite, así que el tope es estructural y no un consejo. Un límite demasiado grande se recorta, no se rechaza. `format=csljson` porque S5 confirmó que alimenta a hayagriva sin capa de conversión. 15 tests.
- [x] Caché regenerable particionada por origen e instancia. Un cambio de instancia invalida la caché, no los snapshots del manuscrito, y nunca revincula citas por coincidencia de clave. **La regla filosa se volvió estructural, no recordada:** `get` recibe la instancia sobre la que se pregunta, así que **no se puede consultar nada sin nombrar la biblioteca** — revincular por coincidencia de clave no es algo que haya que evitar, es algo que no se puede escribir. Una clave Zotero es única *dentro* de una biblioteca; en otra, `ABCD1234` es otra obra. La identidad de instancia es tan débil como la dejó S5: sin `Zotero-Server-ID`, solo queda `Last-Modified-Version`, monótona dentro de una biblioteca y sin sentido entre bibliotecas. De ahí que la política conservadora de §27.7 sea la única disponible: **una versión que retrocede es otra biblioteca, y una versión ilegible no prueba nada — y no probar nada se trata como cambio.** Invalidar la caché **no es perder una cita**: el `metadata_snapshot_json` es del manuscrito y este módulo no puede alcanzarlo. Un test sostiene un snapshot a través de cada invalidación para decirlo. 11 tests.
- [x] Estados de §11.3 sin diagnósticos falsos: no afirmar que Zotero está cerrado o instalado sin evidencia. **Dos sondas, no una**: `/connector/ping` responde aunque la API local esté apagada (S5), así que vida y permiso son preguntas distintas — preguntar solo una es exactamente cómo una API deshabilitada se reporta como un programa que no está instalado. `ZoteroState` **no tiene palabra** para "cerrado" ni para "no instalado": cinco variantes, cada una algo observado. `diagnose` es función pura, así que se prueban todos los pares y no solo el reproducible.
- [x] Clústeres con locator, prefijo, sufijo, supresión de autor y reordenamiento; almacenar datos equivalentes a CSL, no la cadena renderizada. `writing/csl/render.rs` lee el CSL-JSON de Zotero **sin capa de conversión** (`citationberg::json::Item` implementa `EntryLike`). **Nada cachea su propio resultado**: un `ClusterItem` es item, locator, afijos y bandera de supresión, y el texto se deriva cada vez — por eso cambiar de estilo re-renderiza todo en vez de dejar cadenas viejas. Los dos huecos de hayagriva se aplican después de renderizar, **en ese orden**: primero la supresión, después los afijos, porque un prefijo va delante de lo que queda y no delante de un apellido que está por irse. **Supresión de autor resuelta por sustracción** (decidido por el usuario, 2026-09-16): se renderiza la cita y la versión solo-autor y se resta, **conservando la página** que las dos aproximaciones de hayagriva descartan. Si no se puede, `author_suppressed` vuelve en `false` y la cita completa queda — nunca una cita mutilada. El reordenamiento intra-clúster lo gobierna el estilo; S3 anotó que eso puede ser comportamiento correcto de CSL y no un defecto. 32 tests (9 afijos + 11 supresión + 12 renderizado).
- [x] Estilos CSL con conjunto reducido inicial y ruta segura para incorporar otros archivos, validados antes de usarse. El conjunto inicial es el archivo empaquetado de hayagriva (feature `archive`): una instalación nueva renderiza sin descargar nada. **Un estilo desconocido se rechaza por nombre en vez de caer a APA** — caer silenciosamente mal-renderizaría un manuscrito entero. Un `.csl` propio se valida **en la puerta**, porque un estilo que falla a mitad de un manuscrito es peor que uno rechazado antes de elegirse; un estilo dependiente se nombra como tal en vez de aceptarse, ya que apunta a otro y no renderiza por sí mismo. **Y se vuelve a validar al usarlo**: un estilo puede llegar al renderizador desde una preferencia escrita por un build viejo, y confiar en que se validó una vez es cómo un archivo ilegible se vuelve un crash en medio de una bibliografía. Con tope de tamaño.
- [x] Bibliografía dinámica solo con obras citadas, como vista derivada. `render_bibliography` la construye desde las citas que recibe, así que una obra que deja de citarse deja de aparecer sin que nada tenga que acordarse de sacarla. Probado en ambos sentidos.
- [x] Funcionamiento degradado con Zotero cerrado, renderizando desde snapshots y marcando pendiente de verificación. §11.3 cierra con la frase que gobierna toda la integración — un fallo de Zotero no puede bloquear la edición ni el guardado — y eso es posible porque la cita lleva su propio snapshot: **la biblioteca es una mejora sobre él, nunca una precondición**. Renderizar desde el snapshot **no es salida degradada**: es el mismo CSL-JSON que la biblioteca habría devuelto el día en que se citó, así que la cita se lee bien; lo que no se puede saber es si *sigue* coincidiendo, y eso se marca. **Una caída nunca se reporta como un borrado**: decirle a alguien que su obra se eliminó de Zotero porque el puerto estaba ocupado lo manda a buscar un problema inexistente. Solo una biblioteca que respondió y no la tiene es un borrado — el último estado de §11.3. 7 tests.

**Estado: completa y verificada contra una biblioteca real** (2026-09-16, ~662 referencias). Motor en Rust, cinco comandos, la pestaña Zotero, el diálogo de cita y la proyección escrita en la misma transacción que el contenido. Cuatro defectos encontrados y corregidos en esa verificación, todos por **suponer en vez de preguntar**: (1) `to_string()` sobre el árbol de hayagriva emite **VT100**, no texto plano — el plano está detrás de `{:#}`; (2) la paginación adivinaba por el largo de la página, pero `format=csljson` solo emite los citables, así que una biblioteca grande se leía como 662 y se daba por completa en silencio — ahora usa `Total-Results`; (3) la búsqueda filtraba la copia parcial en memoria en vez de preguntarle a Zotero (`q`, `qmode=everything`); (4) convertir la cita en clúster dejó huérfanas las citas ya escritas y `cluster[0]` hacía **panic** en el worker — ahora `worksOf` entiende la forma vieja como un clúster de una obra y un clúster vacío responde en vez de reventar. La desambiguación `2022a`/`2022b` funciona: lo que faltaba era impedir citar dos veces la misma obra en una cita, porque CSL no tiene con qué distinguir una obra de sí misma.

**Criterio de aceptación:** criterios 12 a 17 de §25, verificados en Pro y Lite. Commit sugerido: `feat(writing): integrate Zotero and CSL bibliography`.

### Unidad 7 — Agente y procedencia

**Archivos:** crear `apps/desktop/src/views/WritingAgentTab.svelte`; ampliar `writing/commands.rs`; modificar `apps/desktop/src-tauri/src/research.rs` únicamente en su punto de instanciación.

Su alcance está acotado por G9: el trait `ClienteLlm` permite sustituir el proveedor de chat en el sitio de llamada, pero `ClienteEmbeddings` y `ClienteRerank` son structs concretos atados a OpenRouter. No se declara compatible ninguna acción cuya recuperación dependa de ellos sin cambiar el crate `entropia-agent`.

- [x] Publicar la matriz real de capacidades por variante y proveedor antes de exponer acciones. Las no disponibles se muestran como tales; ninguna ausencia bloquea la edición manual. `writing/agent_actions.rs`: las catorce acciones de §14.1 contra lo que cada una necesita. G9 fija el límite — `ClienteLlm` es trait, pero `ClienteEmbeddings` y `ClienteRerank` son tipos concretos atados a OpenRouter — así que las cuatro de recuperación **se listan como no disponibles con su motivo**, ni ocultas ni ofrecidas para fallar cuando el autor ya eligió un pasaje. Una matriz ilegible no ofrece nada en vez de todo. 6 tests.
- [x] Constructor de contexto con límites explícitos, deduplicación, vista previa de lo que se envía y registro de las piezas enviadas. `agent-context.ts`: **la vista previa es el contexto ensamblado**, no una descripción de él — §14.4 solo se cumple mientras lo mostrado y lo enviado sean el mismo objeto, y el registro es esa misma lista. Lo que no entra **se reporta**, nunca se descarta en silencio. El orden de recorte es el que la acción necesita: la selección y la instrucción son lo que la acción *es*, así que nunca se sacan para dejar entrar evidencia. La deduplicación corre antes del límite, para que una copia jamás desplace a una pieza real. 16 tests.
- [x] Sugerencias no destructivas con las tres acciones de §14.2 y distinción entre evidencia del corpus, metadatos Zotero y consulta efectiva de texto. `evidenceOf` reporta **solo lo que efectivamente se envió**: evidencia que el límite dejó afuera no se envió y por lo tanto no es evidencia. Y **un metadato de Zotero no es un texto consultado** — una propuesta que vio un título y un autor no leyó nada.
- [x] Validar el rango objetivo con anchors y hash del contenido objetivo, no del manuscrito completo. Aplicación y registro de aceptación en una sola operación persistente; repetir una aceptación no duplica texto. **La aceptación es una transición de estado condicional** — `UPDATE ... WHERE status = 'pending'` con verificación de filas afectadas, el mismo patrón que el guard de revisión — así que la decisión y el registro son la misma escritura y no hay ventana entre una y otra. La segunda aceptación no recibe texto. Verificado por mutación. El hash es **del pasaje**: hashear el manuscrito invalidaría toda sugerencia pendiente apenas alguien escriba en cualquier parte, que es cómo se enseña a ignorar una advertencia. Una sugerencia cuyo objetivo se movió **igual se puede descartar**, porque descartarla es lo que uno hace al respecto.
- [x] Registrar procedencia de cada operación de IA con texto original, texto aceptado, acción, fecha, modelo, proveedor y evidencia. El evento se escribe **solo si el texto efectivamente entró** — un registro que afirma una edición que no ocurrió es peor que no tener registro — y apunta a la fila de la sugerencia en vez de copiar su evidencia, porque la fila es el registro y dos copias terminan discrepando.

**Estado:** los cinco ítems cerrados. El comando propio se escribió (`writing_agent_ask`): entra un pasaje con su contexto ya ensamblado y mostrado, sale una propuesta pendiente; **sin herramientas**, para que el agente no alcance material que el autor no vio en la vista previa. La construcción del prompt vive en `writing/agent_prompt.rs` como módulo puro y testeado, porque un prompt armado dentro de un comando es un prompt que nadie vuelve a leer.

Tres decisiones que conviene dejar dichas:

1. **El objetivo se localiza por su texto, nunca por una posición guardada.** Entre proponer y aceptar, el autor sigue escribiendo; S2 ya había medido que un `{from, to}` persistido apunta a otro párrafo después de recargar. Un pasaje que desapareció es un pasaje que cambió, y uno que ahora aparece dos veces **se rechaza en vez de adivinarse**: elegir la primera ocurrencia editaría un párrafo que el autor no estaba mirando, y se enteraría mucho después.
2. **El guardián del hash estaba muerto y ahora no lo está.** La pestaña enviaba `row.selected_content_hash` como "el hash actual", o sea comparaba un valor contra sí mismo. Ahora se recalcula sobre el documento como está.
3. **La respuesta se pide en JSON pero una respuesta plana no se descarta.** Un modelo que ignoró el formato igual escribió algo que una persona puede leer y juzgar; tirarlo convertiría un capricho de formato en una acción fallida.

Un guardián nuevo (`agent-actions-vocabulary.test.ts`) mantiene alineadas las tres listas que no tienen nada que las una en tiempo de compilación: la matriz de capacidades, las instrucciones del prompt y las etiquetas de los dos idiomas. La peor de las derivas es la instrucción faltante, porque la acción igual corre y le pide al modelo el texto genérico en vez de lo que el botón prometía — parece que funcionó. Verificado por mutación.

**Queda sin conectar, y dicho en pantalla:** las cuatro acciones de recuperación siguen listadas como no disponibles. G9 no se movió.

**Criterio de aceptación:** criterios 18 a 22 y 28 de §25. Commit sugerido: `feat(writing): add reviewable agent assistance`.

### Unidad 8 — Exportación

**Archivos:** crear `apps/desktop/src/lib/writing-export.ts` y `apps/desktop/src/views/WritingExportDialog.svelte`; reutilizar el guardado de archivos de `ocr-export.ts:1-2,265-276`. El motor DOCX depende del resultado de S4.

- [x] Completar la matriz de fidelidad iniciada en S4 y ampliarla con cada nodo incorporado desde la Unidad 3. **La matriz es código, no un documento**: `export-fidelity.ts` la lee la exportación para construir sus advertencias y la leen sus pruebas para rechazar un DOCX que degrade algo obligatorio. Un guardián la compara contra el esquema mismo, en las dos direcciones: un nodo agregado al manuscrito y olvidado acá exportaría como silencio, que es la pérdida de datos más callada que existe.
- [x] Markdown, HTML sanitizado y DOCX con notas al pie, citas documentales configurables, citas Zotero y bibliografía CSL. Las pruebas del DOCX **abren el zip**, porque esa es la verificación que habría cazado al exportador anterior: S4 encontró que `html-docx-js` produce un archivo que Word muestra bien y que no contiene modelo de documento alguno. «Sanitizado» acá significa que **no pasa marcado**: cada nodo se construye y cada texto se escapa, así que la única superficie es el destino de un enlace, y ahí el esquema se verifica contra una lista — un destino rechazado conserva sus palabras y pierde el enlace.
- [x] La configuración de exportación no altera el documento canónico. Por construcción: se lee la configuración, no se escribe el documento, y las citas renderizadas viven en un objeto que se descarta. Hay una prueba que compara el JSON antes y después de exportar en los tres formatos.
- [x] Una exportación parcial advierte qué elementos no pudieron representarse. Una advertencia no permite declarar cumplido un elemento obligatorio. **Las advertencias cuentan ocurrencias**, no listan tipos: «una nota al pie no se pudo representar» y «cuarenta» son hechos distintos, y el segundo suele cambiarle el formato al autor. Y la última oración es literal: un DOCX que perdería un elemento obligatorio **no se escribe**. La negativa llega *antes* de pedir el nombre del archivo, porque después se lee como una falla al guardar y no como lo que es.

**Estado:** los cuatro ítems cerrados. Tres decisiones que conviene dejar dichas:

1. **El documento patrón revela lo que las pruebas por construcción no.** Dos defectos reales aparecieron recién al exportar el manuscrito entero: las notas de cita no se escapaban como el resto del texto (un título con un asterisco abría cursiva dentro de la nota) y el título salía dos veces cuando el documento ya empezaba con un H1.
2. **Una opción que el formato no tiene se deshabilita, no se oculta ni se sustituye.** Markdown no tiene comentario. Ocultarlo dejaría al autor buscando algo que usó la vez anterior; ofrecerlo y convertirlo en silencio en una nota al pie le daría un documento que no pidió.
3. **El mapeo de clústers CSL se extrajo a `citation-clusters.ts`** en vez de copiarse. Dos copias derivarían, y derivarían de la peor manera: el manuscrito en pantalla y el archivo exportado citando las mismas obras distinto, sin que nada lo reporte.

**Verificado por el usuario (2026-09-16), que es lo que §17.4 pide expresamente:** *"Deben registrarse aplicaciones de lectura y versiones verificadas."*

- **Microsoft Word** (Microsoft 365, versión 2608, compilación 16.0.20326.20072, 64 bits, Canal actual): toda la matriz renderiza y **las notas al pie renumeran** — borrada la primera, la segunda pasó de ² a ¹ junto con su marcador en el cuerpo. Eso solo lo hace un `footnotes.xml` real, y es exactamente lo que el exportador anterior no podía hacer.
- **LibreOffice Writer 26.8.0.3** (X86_64, Windows 10 build 19045): abre el mismo archivo sin degradar nada, **nota al pie incluida**. Era el punto del ejercicio: dos implementaciones independientes rinden el mismo documento igual, así que lo que el lector ve sale del archivo y no del importador de la aplicación que lo abre.

**Esa revisión encontró un defecto que las pruebas no veían.** El hipervínculo salía **vivo pero sin estilo**: `docx` emite el `w:hyperlink` con un run pelado salvo que se pida el estilo de carácter `Hyperlink` por nombre, y la prueba solo verificaba que el `w:hyperlink` existiera. La matriz de S4 decía «azul, subrayado, vivo», así que el estilo era parte de lo verificado y no un adorno. Corregido; la prueba ahora exige `<w:rStyle w:val="Hyperlink"/>`. **La lección se repite: una prueba que verifica que algo existe no verifica que sirva.**

Los tres archivos del documento patrón se reescriben en `docs/escritura-export-pattern/` en cada corrida de pruebas — así no pueden quedar desactualizados — con su lista de revisión en `LEEME.md`.

**Criterio de aceptación:** criterios 23 y 24 de §25, con aplicaciones y versiones registradas. Commit sugerido: `feat(writing): export manuscripts to markdown, html and docx`.

### Unidad 9 — Endurecimiento y entrega

- [x] Pruebas de rendimiento con los tamaños de §19 y medición de apertura, guardado, búsqueda y exportación. `writing-performance.test.ts`, con los tamaños que §19 nombra: artículo de 10.000, capítulo de 30.000 y un documento de estrés de 60.000 palabras, con centenares de citas. **Son presupuestos, no benchmarks**: holgados a propósito, porque lo que hay que cazar es un cambio de *orden* y no una tarde lenta en CI. Un O(n²) accidental en un recorrido del documento es invisible con una fixture de doscientas palabras y ruinoso con treinta mil, así que hay además una prueba de *forma*: el triple de palabras no puede costar nada parecido al cuádruple de tiempo.

  Medido en esta máquina: validar un capítulo **22 ms**, buscar en él **16 ms**, exportarlo a Markdown **23 ms**, a DOCX **139 ms**, y validar y exportar el documento de estrés **18 ms**. Dos pruebas más fijan que el motor CSL se consulta **una vez** por exportación y una vez para la bibliografía, cualesquiera sean las citas — §11.5 lo exige por corrección, y con centenares de citas también lo exigiría el reloj.

- [~] Accesibilidad de §18. **La mitad que es un hecho sobre el código está cerrada y guardada**: `writing-accessibility.test.ts` exige que todo botón con icono de las vistas de escritura tenga nombre accesible y que ese nombre venga de las traducciones, y que los mensajes de error lleven `role="alert"`. El peor de esos defectos es el botón sin rótulo: un lector de pantalla lo anuncia como «botón», quien depende de uno no distingue el esquema de la exportación, y a simple vista la interfaz se ve perfecta. Verificado por mutación.

  **Verificado en la aplicación (2026-09-17):**

  - **Teclado:** el recorrido con Tab funciona y **no hay trampa de foco** en el editor. Era lo más grave de la lista: si Tab insertara una tabulación en vez de mover el foco, quien navega por teclado quedaría encerrado en la superficie de escritura sin salida.
  - **Zoom y desbordes:** ninguno, en el peor caso que la aplicación permite — ventana en su mínimo de 900 px, zoom en su techo de 125 %, los dos paneles abiertos. Llegar ahí costó tres correcciones y las tres valen como registro:
    1. Los paneles tenían ancho fijo y **no cedían**, así que el manuscrito quedaba más angosto que cualquiera de los dos. Se hicieron redimensionables (§18 pedía «redimensionables y plegables»; plegables ya eran).
    2. Hacerlos redimensionables movió el problema: `min-width: 0` en el editor —la forma habitual de decirle a un hijo flex que tome lo que sobre— lo convierte en **lo primero que flexbox comprime**, así que los paneles nunca llegaban a ceder. La prosa terminó envolviendo a una palabra por línea. Se le dio piso propio al manuscrito.
    3. Y eso produjo scroll horizontal, porque **el test invariante contaba las columnas y no la página**: 20 px de padding por lado, 12 px entre cada par de hijos y los bordes suman 94 px que no estaban en la cuenta. Se agregó un segundo piso a los paneles, `squeeze`, distinto de `min`: `min` es lo que un arrastre no puede pasar, `squeeze` es lo que la ventana puede forzar cuando la alternativa es desbordar.

  **Queda para mirar:** el contraste, que es lo único de §18 que no se puede afirmar desde el código ni desde el comportamiento.

- [x] Regresión completa de §22.4, recorrida en la aplicación (2026-09-17). Las suites automáticas pasan enteras —Rust 957, desktop 1423, ui 420, `svelte-check` sin errores ni advertencias en los dos paquetes— pero §22.4 pide recorrer la aplicación, y eso es uso, no ejecución de pruebas.

  **Ordenada por riesgo medido, no alfabéticamente.** De todo lo que la Unidad tocó, solo dos archivos viven fuera del módulo de escritura: `ItemView.svelte`, al que se le cambió la lista de assets, y `AppShell.svelte`, cuya barra de estado se ve en todas las pantallas. El resto —`settings.ts`, `i18n.ts`, los `index.ts` de `@entropia/ui`— son agregados. Así que el recorrido empezó por visor, OCR y notas, y el resto se confirmó después.

  **Verificado:** visor de un PDF de varias páginas sin páginas fantasma ni faltantes, texto extraído y OCR por página, una nota creada desde el visor que queda en su página; importación y navegación de colecciones; búsqueda textual y semántica; NER y tripletas; Chat/RAG; y la exportación OCR a DOCX, que era el único punto con alguna sombra: la Unidad 8 agregó `docx` al proyecto y esa exportación usa `html-docx-js`. `ocr-export.ts` no tiene un solo cambio y `html-docx-js` sigue instalado, así que en teoría no se tocan — y una regresión existe precisamente para no quedarse en la teoría.
- [ ] Apertura del mismo documento en Pro y Lite, con y sin Zotero y capacidades de IA.
- [ ] Migraciones desde versiones de desarrollo y documentación de usuario.

**Estado:** rendimiento cerrado con números; accesibilidad cerrada en lo automatizable y abierta en lo visual; los tres ítems restantes **no los puede cerrar una prueba**, porque son verificación en la aplicación corriendo y en las dos variantes. Están enumerados como pendientes en vez de declarados cumplidos.

**Criterio de aceptación:** la totalidad de §25. Commit sugerido: `feat(writing): harden and document the writing workspace`.

## 31. Matriz de aceptación y comandos de verificación

Los comandos siguientes son para la implementación futura. No se ejecutan ni se declaran aprobados por escribir este plan.

| Escenario | Evidencia exigida |
|---|---|
| Dos guardados con la misma revisión base | El obsoleto se rechaza con código; el reciente queda intacto y no hay pérdida silenciosa |
| Terminación forzada durante escritura continua | Recupera toda secuencia durable; la pérdida no confirmada respeta la ventana declarada en S6 |
| Journal truncado o incompatible | No se aplica parcialmente, no destruye lo confirmado y queda disponible para diagnóstico |
| Revisión canónica concurrente frente a journal | Ofrece recuperación separada o comparación, nunca reemplazo automático |
| Copia, movimiento, duplicación, borrado, undo/redo y restauración | JSON, proyecciones y procedencia permanecen coherentes; copiar crea identidades nuevas, mover las conserva |
| Anchor ante división y fusión de bloques, borrado del objetivo y cambio externo al rango | Cambios ajenos al objetivo conservan validez; objetivo modificado o anclaje ambiguo exigen revisión |
| Aplicación repetida de una sugerencia ya aceptada | No duplica contenido |
| Asset citado eliminado | Se advierten dependencias antes de confirmar; la cita sobrevive con su snapshot y estado de integridad |
| Zotero disponible, deshabilitado con `403`, ausente y con timeout | Cada estado se informa sin diagnósticos falsos; la edición y el guardado nunca se bloquean |
| Cambio de instancia Zotero | Invalida la caché correspondiente, no los snapshots; no revincula citas por coincidencia de clave |
| Cambio de estilo CSL sobre un manuscrito con clústeres | Todas las citas y la bibliografía se vuelven a renderizar sin intervención manual |
| Documento patrón exportado a los tres formatos | Matriz de fidelidad completa con aplicaciones y versiones verificadas |
| Mismo documento abierto en Pro y en Lite | Idéntico esquema, sin requerir ML local; capacidades ausentes se muestran como tales |
| Instalación nueva, actualización y migración interrumpida | Esquema listo, migración atómica e idempotente, fixture coincidente byte a byte |
| Manuscrito de 30.000 palabras con centenares de citas | Apertura, edición y guardado dentro de los tiempos medidos en S1; sin RAG, CSL ni LLM por tecla |

### Comandos concretos

Desde la raíz, una vez implementados los archivos:

~~~powershell
pnpm --filter @entropia/store test -- src/runner.test.ts
pnpm --filter @entropia/store test -- src/schema-fixture.test.ts
pnpm --filter @entropia/store export-schema
pnpm --filter @entropia-pro/desktop test -- src/views/WritingView.test.ts
pnpm --filter @entropia-pro/desktop test -- src/design-tokens.test.ts
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --no-default-features writing
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --features local-ml writing
pnpm lint
pnpm typecheck
pnpm test
~~~

Para typecheck y pruebas Lite, usar `VITE_LOCAL_ML=0` en el proceso o guardar y restaurar el valor de PowerShell en `try/finally`; para Pro, usar `1` explícitamente. Los filtros Rust exigen que las pruebas nuevas lleven `writing` en su ruta o nombre; verificar que se ejecutaron casos y no aceptar una salida con cero pruebas como evidencia. El script de exportación de esquema declara requerir Node 24+; usar ese runtime solo para el export.

Prueba de terminación forzada en entorno desechable, instrumentando los puntos de la Unidad 2. Un `kill` demuestra muerte de proceso, no comportamiento del dispositivo ante pérdida eléctrica; registrar esa diferencia en el informe de verificación.

## 32. Condiciones de entrega

La funcionalidad no se considera terminada si solo existen la sección, un editor que guarda HTML, un historial sin restauración o citas renderizadas como texto plano. Deben quedar unidos: JSON canónico versionado, revisión esperada comprobada atómicamente, journal durable con ventana de pérdida medida, citas documentales con regreso a la fuente, clústeres Zotero con cambio de estilo, sugerencias revisables con anclaje validado y exportación con matriz de fidelidad.

Puntos no negociables de revisión: ningún guardado que declare «Guardado» antes del commit canónico; ninguna revisión vieja que sobrescriba una posterior; ninguna proyección de citas editada por un camino que deje el JSON desactualizado; ninguna sugerencia aplicada sobre un rango ambiguo; ninguna cita revinculada automáticamente a otra biblioteca; ningún motor de recuperación paralelo al existente; ninguna acción de IA declarada disponible sin implementación compatible; ninguna dependencia copyleft incompatible con la licencia MIT del proyecto.
