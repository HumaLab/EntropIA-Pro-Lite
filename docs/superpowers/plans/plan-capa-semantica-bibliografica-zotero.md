# Plan para una capa semántica bibliográfica de Zotero en EntropIA

## 1. Propósito

Incorporar en EntropIA una capa semántica destinada exclusivamente a bibliografía académica, separada del índice vectorial de fuentes primarias y articulada de manera obligatoria con Zotero.

La función central de esta capa será permitir que el usuario encuentre obras y fragmentos bibliográficos pertinentes para una investigación, los relacione con las fuentes documentales y los inserte como citas verificables en el editor académico de EntropIA.

Este documento propone posibilidades de diseño y una secuencia de trabajo. No prescribe una implementación cerrada. El agente deberá contrastar estas orientaciones con la arquitectura y el esquema de datos existentes antes de modificar el código.

## 2. Principio rector

> No puede existir bibliografía vectorizada en EntropIA sin un ítem bibliográfico previamente existente y vinculado en Zotero.

Zotero será la fuente de verdad para la identidad y la gestión bibliográfica. EntropIA no debe construir una biblioteca paralela, sino una capa semántica subordinada a Zotero.

Esto implica que:

- todo registro bibliográfico de EntropIA debe tener una clave o URI válida de Zotero
- todo resultado semántico debe conducir a un ítem citable
- un PDF arrastrado a EntropIA debe registrarse o vincularse primero en Zotero
- EntropIA no debe reconstruir referencias bibliográficas desde el texto extraído
- los estilos CSL, citas y bibliografías deben resolverse a partir de los datos de Zotero
- los registros que pierdan su vínculo con Zotero deben excluirse de la recuperación normal hasta resolver su estado

## 3. Alcance funcional

La capa debería permitir, de manera progresiva:

1. seleccionar una biblioteca, grupo o colección de Zotero
2. sincronizar sus ítems, metadatos, colecciones, etiquetas y adjuntos compatibles
3. extraer el texto de los adjuntos bibliográficos
4. crear una representación semántica global por obra
5. dividir el texto completo en fragmentos y generar embeddings por fragmento
6. buscar obras relevantes y luego pasajes dentro de ellas
7. filtrar por colección, autoría, fecha, tipo, etiqueta y otros metadatos
8. abrir el ítem o su adjunto en Zotero
9. insertar una cita desde el editor académico de EntropIA
10. combinar, cuando corresponda, resultados de fuentes y bibliografía sin confundir ambos dominios

## 4. Separación entre fuentes y bibliografía

Los dos dominios deben mantenerse independientes en almacenamiento, indexación, interfaz y contexto enviado a los modelos.

| Dominio | Objeto principal | Pregunta típica | Identidad |
| --- | --- | --- | --- |
| Fuentes | Documento histórico, entrevista, prensa, expediente, imagen o audio | ¿Qué dicen las fuentes? | ID interno del asset o documento |
| Bibliografía | Libro, artículo, capítulo, tesis u otra obra académica | ¿Qué trabajos ayudan a interpretar el problema? | Biblioteca de Zotero + `item_key` |

Una consulta mixta podrá ejecutar dos recuperaciones y presentar dos grupos claramente rotulados:

- **Fuentes documentales**
- **Bibliografía académica**

Los resultados no deberían mezclarse en una única lista ordenada únicamente por similitud vectorial.

## 5. Arquitectura conceptual

```text
Zotero
  ├── identidad bibliográfica
  ├── metadatos
  ├── colecciones y etiquetas
  ├── adjuntos
  └── estilos y producción de citas
          ↓ sincronización validada
EntropIA Bibliografía
  ├── réplica local de metadatos necesarios
  ├── perfil semántico de la obra
  ├── embedding global de la obra
  ├── texto extraído
  ├── chunks y embeddings de fragmentos
  ├── índice de búsqueda léxica
  └── estado de sincronización e indexación
          ↓
Recuperación académica
  ├── descubrimiento de obras
  ├── búsqueda de pasajes
  ├── apertura y trazabilidad
  └── inserción de citas mediante Zotero
```

La integración podría resolverse mediante la API web oficial de Zotero, el conector local de Zotero u otra vía compatible con la arquitectura actual. La elección deberá considerar autenticación, bibliotecas personales y grupales, funcionamiento local, paginación, límites de uso, adjuntos y distribución de EntropIA.

## 6. Identidad e integridad referencial

### 6.1. Identificador lógico

La identidad bibliográfica no debería depender del nombre del archivo, DOI, ISBN o una coincidencia de título. Como clave lógica conviene usar una identidad compuesta:

```text
zotero_library_type + zotero_library_id + zotero_item_key
```

El `item_key` por sí solo podría no ser suficiente si EntropIA admite varias bibliotecas. Los IDs internos de EntropIA pueden mantenerse como claves técnicas, siempre vinculados de manera obligatoria con la identidad compuesta de Zotero.

### 6.2. Restricciones recomendadas

- restricción única sobre biblioteca e `item_key`
- relación obligatoria entre cada obra semántica y su registro Zotero
- eliminación en cascada únicamente para derivados regenerables, nunca de manera automática sobre Zotero
- imposibilidad de indexar un adjunto sin resolver primero su ítem bibliográfico padre
- exclusión de ítems huérfanos, eliminados o inaccesibles de los resultados ordinarios
- registro explícito de la versión o fecha de modificación observada en Zotero

### 6.3. Estados posibles del vínculo

```text
UNLINKED
LINKED
SYNCED
STALE
ORPHANED
INACCESSIBLE
DELETED_UPSTREAM
```

`ORPHANED`, `INACCESSIBLE` y `DELETED_UPSTREAM` no deberían interpretarse automáticamente como autorización para borrar información local. La interfaz debe ofrecer revisar, revincular, conservar temporalmente o eliminar los derivados.

## 7. Modelo de datos orientativo

El agente deberá adaptar nombres y tipos al esquema existente. Una separación posible sería:

### `zotero_libraries`

- `id`
- `library_type`
- `zotero_library_id`
- `display_name`
- `sync_cursor` o versión observada
- `last_sync_at`
- `auth_scope` o referencia segura a la configuración, nunca el secreto en texto plano

### `bibliographic_items`

- `id`
- `zotero_library_id`
- `zotero_item_key`
- `zotero_version`
- `item_type`
- `title`
- `creators_json`
- `publication_title`
- `publisher`
- `date`
- `doi`
- `isbn`
- `abstract`
- `language`
- `url`
- `metadata_hash`
- `link_state`
- `synced_at`

### `bibliographic_collections` y tabla de relación

- identidad y nombre de la colección Zotero
- jerarquía de colecciones
- relación muchos a muchos entre ítems y colecciones

### `bibliographic_attachments`

- `id`
- ítem padre
- clave del adjunto en Zotero
- tipo MIME
- nombre y ubicación resoluble
- hash del archivo
- versión observada
- disponibilidad local
- estado de extracción

### `bibliographic_semantic_profiles`

- ítem bibliográfico
- texto canónico usado para el embedding global
- procedencia de cada campo
- modelo y versión de plantilla
- hash de entrada
- fecha de generación

### `bibliographic_chunks`

- ítem y adjunto de origen
- orden
- texto
- localización verificable, por ejemplo páginas o posiciones
- encabezado o sección, si puede determinarse
- hash del contenido
- versión del algoritmo de segmentación

### Embeddings

Se puede emplear una tabla diferenciada para embeddings globales y de chunks, o una tabla polimórfica con restricciones fuertes. En cualquier caso deberían conservarse:

- objeto embebido
- vector
- modelo y proveedor
- dimensión
- versión de la representación o chunking
- hash del texto de entrada
- fecha de generación

## 8. Representación semántica por obra

El embedding global no debería construirse únicamente con el título o la referencia formateada. Conviene formar un texto canónico con los campos disponibles y etiquetados, por ejemplo:

```text
Título: ...
Autores: ...
Año: ...
Tipo: ...
Publicación o editorial: ...
Resumen: ...
Palabras clave y etiquetas: ...
Notas seleccionadas: ...
Resumen del texto completo: ...
```

No todos los campos tienen que estar presentes. La plantilla debe evitar que los campos administrativos o repetitivos dominen la señal semántica.

El resumen del texto completo podría ser opcional y generarse solo cuando exista texto suficiente. Debe quedar identificado como contenido derivado por un modelo, no como resumen provisto por la publicación.

## 9. Ingesta controlada

### 9.1. Ingreso desde Zotero

Flujo preferente:

1. detectar cambios en Zotero
2. importar o actualizar metadatos
3. identificar adjuntos compatibles
4. verificar permisos y disponibilidad
5. calcular hashes y decidir qué derivados están desactualizados
6. extraer texto
7. construir o actualizar el perfil semántico
8. generar embedding global
9. segmentar el texto completo
10. generar embeddings de chunks
11. actualizar índices y estados

### 9.2. Ingreso desde EntropIA

Si el usuario arrastra un PDF a **Bibliografía**:

1. EntropIA intenta encontrar una coincidencia en Zotero por identificadores y metadatos
2. muestra coincidencias ambiguas para confirmación, sin vincular automáticamente por título aproximado
3. si existe el ítem, vincula o agrega el adjunto según la decisión del usuario
4. si no existe, crea primero un ítem válido en Zotero y adjunta el archivo
5. obtiene y confirma la identidad Zotero
6. solo entonces habilita la extracción y vectorización

Si Zotero no está disponible o falla la creación, el archivo puede quedar en una bandeja transitoria de **Pendientes de vinculación**, pero nunca en el índice bibliográfico.

## 10. Sincronización incremental e invalidación

No todo cambio debe provocar una vectorización completa.

| Cambio detectado | Acción sugerida |
| --- | --- |
| Colección o etiqueta | Actualizar metadatos y filtros. Recalcular el embedding global solo si esos campos forman parte de su representación |
| Corrección de autor, título o resumen | Actualizar metadatos y recalcular el embedding global |
| Cambio del estilo CSL | No recalcular embeddings |
| Sustitución o modificación del PDF | Invalidar extracción, chunks y embeddings derivados de ese adjunto |
| Nuevo adjunto | Extraer e indexar el nuevo contenido |
| Eliminación del adjunto | Marcar derivados y aplicar la política de conservación elegida |
| Eliminación o pérdida de acceso al ítem padre | Marcar el registro y excluirlo de la recuperación normal |
| Cambio de modelo de embeddings | Permitir reindexación versionada, preferentemente en segundo plano |

Estados de procesamiento posibles:

```text
PENDING → SYNCED → EXTRACTING → EXTRACTED → EMBEDDING → INDEXED
                  ↘ FAILED_RETRYABLE
                  ↘ FAILED_PERMANENT
```

La cola debería ser persistente, reanudable y tolerante a fallos individuales, siguiendo los mismos principios previstos para **Lotes**.

## 11. Recuperación jerárquica

Se propone una recuperación en dos niveles:

### Nivel 1: selección de obras

Combinar, según las capacidades ya presentes en EntropIA:

- similitud con el embedding global
- búsqueda léxica sobre título, autores, abstract, etiquetas y otros metadatos
- filtros estructurados
- coincidencias exactas de autor, DOI, ISBN o título
- eventual reranking

### Nivel 2: selección de fragmentos

Buscar los pasajes más pertinentes únicamente dentro de las obras candidatas. Esto reduce ruido y conserva la unidad bibliográfica como primer objeto de recuperación.

El sistema debería poder ampliar la búsqueda a todo el índice de chunks cuando la primera etapa no encuentre suficientes obras, pero registrar y mostrar esa decisión.

### Resultado mínimo trazable

Cada fragmento recuperado debe conservar:

- obra bibliográfica
- clave de Zotero
- adjunto de origen
- páginas o localización disponible
- texto del fragmento
- método de recuperación
- puntuaciones relevantes, sin presentarlas como certeza epistemológica

## 12. Recuperación combinada con fuentes

El agente o el usuario podrá elegir:

- buscar solo en fuentes
- buscar solo en bibliografía
- buscar en ambos dominios

En el modo combinado conviene ejecutar recuperaciones independientes y aplicar presupuestos separados de resultados y tokens. El contexto destinado al LLM debe usar delimitadores inequívocos y metadatos de procedencia para impedir que una interpretación historiográfica sea presentada como fuente primaria, o viceversa.

Una respuesta debería poder distinguir:

1. evidencia recuperada del corpus documental
2. interpretaciones o discusiones recuperadas de la bibliografía
3. síntesis producida por el modelo

## 13. Interfaz propuesta

### 13.1. Sección Bibliografía

Podría incorporar:

- selector de biblioteca o grupo Zotero
- árbol de colecciones sincronizadas
- filtros por autoría, año, tipo, etiqueta e idioma
- estado de sincronización e indexación
- búsqueda híbrida
- ficha de obra con metadatos y disponibilidad del texto
- número de fragmentos indexados
- acciones **Abrir en Zotero**, **Ver adjunto**, **Buscar dentro**, **Insertar cita** y **Reindexar**

La estética debe reutilizar componentes, variables y patrones ya existentes en EntropIA.

### 13.2. Estados comprensibles

Evitar exponer únicamente estados técnicos. Algunas etiquetas posibles:

- Sincronizado
- Falta texto completo
- Pendiente de indexación
- Indexado
- Actualización disponible
- Requiere vinculación con Zotero
- Ítem no disponible en Zotero
- Error de procesamiento

### 13.3. Editor académico

Desde un pasaje seleccionado del manuscrito, la acción **Buscar bibliografía relacionada** debería:

1. usar el texto seleccionado como consulta
2. recuperar obras relevantes
3. permitir examinar fragmentos y abrir los originales
4. insertar la cita mediante la identidad Zotero y el estilo configurado
5. registrar, si el sistema ya contempla trazabilidad, el vínculo entre el pasaje escrito, la obra y los fragmentos consultados

La inserción de citas debe integrarse en la solapa Zotero del editor y respetar el diseño compacto previsto para ella.

## 14. Citas y CSL

EntropIA debería delegar en Zotero, citeproc u otra implementación CSL compatible:

- estilos autor-fecha
- citas narrativas y parentéticas
- notas al pie
- localizadores, prefijos y sufijos
- múltiples obras en una cita
- bibliografía final

La capa semántica entrega el `item_key` y la evidencia que justifica la selección. La capa bibliográfica produce la cita. No conviene pedir al LLM que redacte referencias como mecanismo principal.

Antes de implementar, el agente debe verificar el mecanismo actualmente usado por el editor de EntropIA para evitar dos sistemas de citas paralelos.

## 15. Detección de duplicados y coincidencias

Zotero mantiene la autoridad sobre sus ítems, pero EntropIA puede asistir sin fusionar automáticamente. La comparación podría priorizar:

1. DOI normalizado
2. ISBN normalizado
3. identificadores propios del proveedor
4. combinación de título, autoría y año
5. similitud aproximada como señal secundaria

Una coincidencia dudosa debe requerir confirmación. Dos ediciones, traducciones o versiones de una obra no deben colapsarse solo por semejanza textual.

## 16. Privacidad, seguridad y funcionamiento local

- almacenar credenciales mediante el mecanismo seguro ya utilizado por EntropIA
- solicitar los permisos mínimos necesarios
- diferenciar bibliotecas personales y grupales
- informar antes de enviar texto bibliográfico a un proveedor externo de embeddings o LLM
- permitir proveedores locales cuando la arquitectura actual lo admita
- no registrar tokens ni contenido completo en logs
- respetar las políticas de acceso de Zotero y los derechos sobre los adjuntos
- permitir borrar todos los derivados locales sin modificar Zotero

## 17. Procesamiento en segundo plano

La extracción y los embeddings bibliográficos deberían integrarse con la infraestructura de tareas persistentes de **Lotes**, si esta ya existe o está en desarrollo. Conviene evitar un segundo sistema de colas.

Requisitos:

- procesamiento no bloqueante
- progreso por biblioteca, obra y adjunto
- reanudación después de cierre inesperado
- reintentos con límites y backoff
- cancelación segura
- errores individuales que no detengan el lote
- prioridad para una obra solicitada desde el editor
- actualización incremental
- registro suficiente para diagnóstico, sin datos sensibles

## 18. Estrategia de implementación por etapas

### Etapa 0. Relevamiento y decisiones técnicas

- inspeccionar el esquema, la indexación, la cola de tareas y la integración Zotero existentes
- identificar qué componentes pueden reutilizarse sin acoplar ambos índices
- decidir la vía de comunicación con Zotero
- definir la identidad compuesta, las políticas de borrado y los estados
- documentar compatibilidad y migraciones

### Etapa 1. Sincronización bibliográfica mínima

- conectar una biblioteca Zotero
- sincronizar metadatos, colecciones y etiquetas
- crear las tablas y restricciones de integridad
- mostrar los ítems en una sección Bibliografía
- abrir un ítem en Zotero
- impedir cualquier vectorización sin vínculo válido

### Etapa 2. Embedding global por obra

- construir perfiles semánticos reproducibles
- generar embeddings globales
- incorporar búsqueda híbrida y filtros
- exponer modelos, versiones, estados y errores

### Etapa 3. Texto completo y recuperación jerárquica

- resolver adjuntos
- extraer texto con localización de páginas cuando sea posible
- generar chunks y embeddings
- recuperar primero obras y luego fragmentos
- permitir verificar cada resultado en su contexto

### Etapa 4. Integración con el editor

- buscar bibliografía a partir de texto seleccionado
- revisar obras y fragmentos
- insertar y editar citas mediante Zotero/CSL
- conservar vínculos de trazabilidad

### Etapa 5. Consulta combinada y asistencia avanzada

- orquestar recuperación separada en fuentes y bibliografía
- componer contexto con procedencia explícita
- generar síntesis que distingan evidencia, interpretación bibliográfica y elaboración del modelo
- evaluar reranking, expansión de consultas y otras técnicas sobre un conjunto de prueba real

## 19. Migraciones y compatibilidad

Antes de aplicar migraciones, el agente debe:

- revisar si ya existen tablas o campos Zotero
- preservar los proyectos actuales y el índice de fuentes
- hacer migraciones versionadas y reversibles cuando la infraestructura lo permita
- no convertir automáticamente documentos existentes en bibliografía
- ofrecer vinculación asistida para contenidos que el usuario quiera reclasificar
- mantener operativa EntropIA aunque Zotero no esté configurado

La ausencia de Zotero debe desactivar o explicar las funciones bibliográficas, no afectar la gestión de fuentes.

## 20. Pruebas necesarias

### Integridad

- rechazar embeddings sin biblioteca e `item_key` válidos
- impedir duplicados de la identidad compuesta
- verificar el tratamiento de ítems borrados o inaccesibles
- comprobar que borrar derivados locales no borre el ítem Zotero

### Sincronización

- alta, modificación y eliminación de ítems
- cambios solo de etiquetas o colecciones
- reemplazo, incorporación y eliminación de adjuntos
- bibliotecas personales y grupales
- cortes de red, límites de uso y credenciales vencidas

### Procesamiento

- PDF con y sin texto
- PDF con OCR previo y documento escaneado
- varios adjuntos por obra
- archivos grandes, corruptos o protegidos
- reanudación tras cierre inesperado
- cambio de modelo de embeddings

### Recuperación

- consultas por tema, autor y obra conocida
- diferencias entre ranking de obras y ranking de chunks
- filtros combinados
- consultas mixtas de fuentes y bibliografía
- ausencia de resultados y resultados con baja pertinencia
- citas y localizadores asociados al fragmento correcto

### Editor

- cita simple y múltiple
- cita narrativa, parentética y nota
- páginas, prefijos y sufijos
- cambio de estilo CSL
- bibliografía final sin duplicados
- reapertura del proyecto con citas persistentes

## 21. Evaluación de calidad

Antes de ajustar algoritmos con intuiciones aisladas, conviene formar un pequeño conjunto de evaluación con consultas reales de investigación y juicios humanos sobre:

- obras relevantes esperadas
- pasajes relevantes dentro de esas obras
- falsos positivos graves
- utilidad de los filtros
- corrección de la procedencia y los localizadores

Se pueden comparar recuperación vectorial, léxica, híbrida y reranking mediante métricas como Recall@k, nDCG@k y MRR, complementadas por evaluación cualitativa. La elección final debería priorizar la utilidad para el trabajo académico, no una mejora mínima de una métrica aislada.

## 22. Criterios de aceptación del núcleo

El núcleo podrá considerarse funcional cuando:

- ningún registro vectorizado carezca de identidad Zotero válida
- la bibliografía y las fuentes mantengan índices y tipos de resultados separados
- una obra modificada se actualice sin reprocesar innecesariamente toda la biblioteca
- sea posible encontrar una obra por búsqueda semántica y revisar los fragmentos que sustentan su pertinencia
- cada fragmento conduzca al ítem, adjunto y localización de origen disponibles
- el usuario pueda insertar una cita sin que el LLM invente la referencia
- la cola sobreviva al cierre de la aplicación y gestione errores por ítem
- los ítems huérfanos o inaccesibles no aparezcan como resultados ordinarios
- el usuario pueda eliminar los derivados semánticos sin alterar Zotero

## 23. Decisiones que el agente debe presentar antes de implementar

Después del relevamiento, el agente debería informar:

1. mecanismo propuesto de conexión con Zotero y sus límites
2. tablas y componentes existentes que se reutilizarán
3. esquema final de identidad e integridad
4. estrategia de almacenamiento de adjuntos y derivados
5. mecanismo de sincronización incremental
6. plantilla del perfil semántico y política de invalidación
7. estrategia de chunking con preservación de páginas
8. método inicial de recuperación y evaluación
9. integración concreta con el editor y el sistema CSL existente
10. migraciones, riesgos y plan de pruebas

## 24. Restricciones explícitas

- no crear una biblioteca bibliográfica autónoma que compita con Zotero
- no vectorizar archivos pendientes de vinculación
- no usar nombres de archivo como identidad bibliográfica
- no fusionar automáticamente coincidencias ambiguas
- no mezclar fuentes y bibliografía en un índice indiferenciado
- no generar referencias finales confiando únicamente en el LLM
- no recalcular todos los embeddings ante cualquier cambio menor
- no ocultar la procedencia, el adjunto o la localización de un fragmento
- no borrar datos de Zotero como efecto colateral de limpiar el índice local
- no implementar una segunda cola si la infraestructura de Lotes puede ampliarse de forma segura

## 25. Resultado esperado

El resultado no será una copia de NotebookLM ni otro gestor bibliográfico. Será una capa semántica académica de Zotero dentro de EntropIA:

- Zotero administra qué es cada obra y cómo se cita
- EntropIA vuelve esa biblioteca buscable, interrogable y relacionable con el corpus
- el editor conecta escritura, evidencia documental y discusión bibliográfica
- el modelo ayuda a recuperar e interpretar, pero no sustituye la identidad bibliográfica ni la verificación de las fuentes
