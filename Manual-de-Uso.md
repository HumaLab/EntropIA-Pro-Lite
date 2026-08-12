# Manual de uso — EntropIA Lite

Guía breve para trabajar con colecciones documentales, OCR, transcripción, notas y chat de investigación.

EntropIA Lite es una aplicación de escritorio. **Guarda tu archivo en tu computadora**. Las funciones de inteligencia artificial usan servicios en la nube que configurás vos (OpenRouter, AssemblyAI y GLM-OCR). Sin esas claves, podés organizar documentos y tomar notas; no podrás extraer texto, transcribir ni chatear con el corpus.

---

## 1. Primeros pasos

### Qué vas a ver

| Zona | Para qué sirve |
| --- | --- |
| **Barra superior** | Volver, buscar documentos, idioma, tema, chat, base de datos, configuración |
| **Panel lateral** | Colecciones y, dentro de una colección, el explorador de documentos |
| **Área central** | Lista de colecciones, documentos de una colección, o el documento abierto |

### Flujo habitual

1. Configurá las APIs (sección 2).
2. Creá una **colección** (por tema, fondo o proyecto).
3. **Importá** imágenes, PDF o audio.
4. Abrí un documento y extraé texto (OCR) o transcribí audio.
5. Agregá **notas** y **tópicos**.
6. Usá **Análisis**, el **mapa** o el **chat** según tu investigación.
7. Si querés, **exportá** la colección a JSON.

### Formatos que acepta

- **Imagen:** PNG, JPG/JPEG, WEBP, TIFF/TIF  
- **PDF**  
- **Audio:** WAV, MP3, FLAC, M4A, AAC, OGG  

---

## 2. Configuración básica (APIs y modelos)

Abrí **Configuración** (ícono de engranaje). Guardá siempre con **Guardar cambios**.

### Pestaña «APIs remotas»

#### OpenRouter (texto, resumen, entidades, chat)

Necesario para: corrección de OCR, resúmenes, entidades (NER), tripletes, embeddings y chat.

1. Creá una API key en [OpenRouter](https://openrouter.ai/settings/keys).
2. Pegala en **API Key** y usá **Probar conexión**.
3. Elegí el **modelo generativo** (por defecto: `google/gemma-4-26b-a4b-it`).
4. Dejá el **modelo de embeddings** en `baai/bge-m3`, salvo que sepas que tu cuenta usa otro compatible.

> Al usar OpenRouter, el texto necesario para cada tarea se envía a ese servicio.

#### AssemblyAI (voz a texto)

Necesario para: transcribir audios de la colección y el **dictado** en notas.

1. Creá una key en [AssemblyAI](https://www.assemblyai.com/app/account).
2. Pegala, probá la conexión y guardá.
3. Opción **Identificación de hablantes**: sirve solo para audio de la colección. El dictado de notas no la usa.

> El audio se envía a AssemblyAI para transcribirlo.

#### GLM-OCR / z.ai (lectura de documentos)

Necesario para: botones de OCR en imágenes y PDF.

1. Creá una key en [z.ai](https://z.ai/manage-apikey/apikey-list).
2. Pegala, probá la conexión y guardá.

> La imagen o el PDF se envían a GLM-OCR para leer el texto y el layout.

### Otras pestañas de configuración

| Pestaña | Qué hace |
| --- | --- |
| **Prompts** | Instrucciones que recibe la IA para corrección OCR, resumen, NER y tripletes. Podés validar o restaurar el valor por defecto. Debe existir el marcador `{text}`. |
| **Model Params** | Modelo y parámetros finos **por tarea** (corrección, resumen, NER, tripletes). Si no tocás nada, usan valores razonables. |
| **RAG Params** | Ajustes del **chat de investigación** (cuántos fragmentos usa, historial, temperatura, etc.). En Lite también podés elegir el modelo del chat y el del *reranker*. |
| **Sincronización** | Opcional: copiar datos entre dispositivos vía un servidor tuyo. Si no la activás, **nada sale** de tu equipo por este canal. |
| **Logs** | Registro técnico local para diagnóstico. |

### Idioma y tema

- **Idioma de la interfaz:** español o inglés (menú de la barra superior). No traduce tus documentos ni notas.
- **Tema:** claro/oscuro desde la barra superior.

---

## 3. Colecciones y documentos

### Colecciones

Son carpetas de trabajo (por ejemplo: *Prensa 1910*, *Entrevistas barrio X*).

- **Crear:** botón «Nueva colección» o el `+` del panel lateral.  
- **Buscar / filtrar** por nombre.  
- **Editar** nombre o descripción.  
- **Eliminar** borra la colección y todo lo asociado (no se puede deshacer).

### Dentro de una colección

- **Importar documento:** botón o arrastrar y soltar archivos.  
- **Buscar** documentos por nombre.  
- **Exportar JSON:** descarga un archivo con documentos, textos, notas, entidades, layout, etc.  
- **Análisis textual de la colección:** nube y ranking de palabras a partir del texto ya extraído o transcrito. Podés ajustar stopwords y bajar las visualizaciones.  
- **Eliminar** un documento o un archivo (asset) concreto.

Cada archivo importado se convierte en un **documento** con uno o más **assets** (páginas de un PDF, una imagen, un audio).

---

## 4. Trabajar con un documento

Al abrir un documento ves:

- **Izquierda:** visor del archivo (imagen, PDF o reproductor de audio) y herramientas de edición/anotación.  
- **Derecha:** pestañas de trabajo.

### Pestañas del panel derecho

| Pestaña | Uso |
| --- | --- |
| **Notas** | Tópicos del documento y notas de investigación |
| **Texto** | OCR / transcripción, edición del texto y resumen |
| **Análisis** | Indexar, embeddings, entidades, tripletes y mapa de lugares |
| **Búsquedas** | Búsqueda en texto y documentos parecidos |
| **Layout** | Bloques detectados por el OCR (títulos, texto, tablas…) |
| **Metadatos** | Datos del archivo y campos personalizados |

---

## 5. Notas y tópicos

### Tópicos (etiquetas del documento)

En **Notas**, arriba, el editor de **Tópicos** sirve para clasificar el documento (temas, series, palabras clave). Podés reutilizar tópicos ya usados en otros documentos.

> En la app se llaman **tópicos**, no «tags». Cumplen la función de etiquetas.

### Notas

- **Agregar nota** con el editor enriquecido: negrita, cursiva, títulos, listas, citas, enlaces.  
- Las notas quedan asociadas al **asset / página** activo cuando hay varias páginas.  
- **Editar** o **eliminar** desde la lista.  
- **Dictado:** botón de micrófono en el editor (requiere AssemblyAI). El texto se inserta en la nota.

---

## 6. OCR y texto de documentos

En la pestaña **Texto**, con una imagen o PDF seleccionado:

| Botón | Nombre corto | Qué hace |
| --- | --- | --- |
| Extraer texto | **OCRL** | Lectura de texto (vía GLM-OCR en Lite) |
| OCR de alta precisión | **OCRH** | Misma familia de proveedor; pensado para mejor layout y detalle |
| Corrección OCR | **OCRC** | Limpia errores de lectura y une columnas/líneas rotas (OpenRouter) |
| Generar resumen | **OCRR** / Resumen | Un párrafo de síntesis (OpenRouter) |

Después de extraer:

1. Revisá y **editá el texto** a mano si hace falta (se guarda solo).  
2. Usá **corrección** si el OCR salió ruidoso.  
3. Generá un **resumen** cuando el texto esté en condiciones.

Sin key de GLM-OCR no hay extracción. Sin OpenRouter no hay corrección ni resumen.

---

## 7. Layout (estructura del documento)

La pestaña **Layout** muestra los **bloques** que dejó el OCR: títulos, párrafos, tablas, figuras, notas, etc.

- Filtrá por tipo de bloque.  
- Al pasar el mouse o seleccionar un bloque, se resalta en el visor (si el overlay está activo).  
- El **inspector** muestra orden, página, posición y texto; podés copiar texto, bbox o JSON.  
- No aplica a archivos de **audio**.

En el visor, el botón de overlay muestra u oculta esas regiones sobre la imagen o el PDF.

---

## 8. Transcripción de audio

Con un asset de audio:

1. Reproducí el archivo en el visor (play, pausa, volumen, saltos).  
2. En **Texto**, usá **Transcribir** (**STT**).  
3. Esperá las etapas (subida → envío → respuesta).  
4. Editá la transcripción si hace falta y, si querés, generá un **resumen**.

Requiere AssemblyAI. La identificación de hablantes se controla en Configuración.

---

## 9. Herramientas sobre imagen y PDF

Barra del visor (según el tipo de archivo):

| Herramienta | Función |
| --- | --- |
| Mano | Mover / desplazar la vista |
| Zoom + / − | Acercar o alejar |
| Rectángulo | Anotación rectangular |
| Subrayado | Anotación de subrayado |
| Color | Color de la anotación |
| Recortar | Recorte a la selección |
| Borrar región | Relleno blanco en la zona elegida |
| Rotar 90° | Izquierda / derecha |
| Rotación fina | Grado a grado |
| Duplicar asset | Copia el archivo tal cual (mismo formato) |
| Deshacer / Rehacer | Historial de ediciones de imagen o PDF |
| Eliminar anotación | Quita la anotación seleccionada |

Las **anotaciones** se guardan con el documento. Los PDF multipágina se trabajan página a página (páginas editables separadas tras la importación).

---

## 10. Análisis (entidades, tripletes, búsqueda interna)

En **Análisis**, con texto ya disponible:

| Acción | Para qué |
| --- | --- |
| **INDEX** | Prepara la búsqueda de texto completo del documento |
| **EMBED** | Genera la “huella” semántica del asset (OpenRouter / BGE-M3) para similitud y chat |
| **NER** | Extrae entidades: personas, lugares, instituciones, fechas, etc. |
| **TRIPLET** | Extrae relaciones del tipo sujeto–predicado–objeto |

También podés:

- **Agregar o editar entidades a mano.**  
- Ver **tripletas** listadas.  
- Usar el **mapa**: lugares georreferenciados; arrastrá el marcador y guardá si la ubicación automática falló.

### Pestaña Búsquedas

- Búsqueda por palabras en el texto (tras indexar).  
- **Assets similares** (tras generar embeddings): documentos parecidos; podés abrir una vista previa.

---

## 11. Chat de investigación

Menú **Chat** en la barra superior.

- Hacé preguntas en lenguaje natural sobre lo ya **OCR-izado o transcrito** e **indexado con embeddings**.  
- Las respuestas citan **fuentes**; podés abrir el documento de origen.  
- Podés tener varias **conversaciones**, borrarlas o empezar una nueva.

Sin OpenRouter y sin texto/embeddings en el corpus, el chat no tiene material útil.

Ajustes finos: Configuración → **RAG Params**.

---

## 12. Metadatos

En **Metadatos**:

- Datos del archivo (nombre original, fechas de importación, etc.).  
- **Campos personalizados** clave–valor que agregues vos (fondo, signatura, año, etc.).

Se guardan con el documento.

---

## 13. Exportar y consultar la base

### Exportar colección

En la colección: **Exportar JSON**. Incluye estructura del corpus, textos, notas, tópicos, entidades, layout y resultados asociados. Sirve para respaldo o para llevar datos a otra herramienta.

### Base de datos (solo lectura)

**Base de datos** en la barra superior: explorá tablas internas, filtrá, paginá y, si hace falta, descargá una tabla en JSON. Es una herramienta de consulta; no reemplaza el trabajo diario en colecciones.

---

## 14. Sincronización (opcional)

En Configuración → **Sincronización**:

- Indicá la URL de un servidor de sync, registrate o iniciá sesión.  
- **Sincronizar ahora** o activar sync automática.  
- Ver dispositivos, uso de almacenamiento, conflictos y notificaciones.  
- Zona sensible: re-verificar archivos o borrar datos **del servidor** (lo local no se borra con esa acción).

Si no configurás sync, el trabajo permanece solo en tu máquina (salvo las APIs de IA que uses).

---

## 15. Atajos útiles

| Atajo | Acción |
| --- | --- |
| **Ctrl+B** | Mostrar u ocultar el panel lateral (fuera de un editor de texto) |
| **Escape** | Cierra modos/diálogos o vuelve atrás en la navegación |

En el editor de notas, Ctrl+B es **negrita**, no el panel.

---

## 16. Privacidad en pocas líneas (Lite)

| Qué | Dónde queda |
| --- | --- |
| Colecciones, archivos, notas, resultados | En tu computadora |
| OCR (GLM-OCR) | Se envía imagen/PDF al proveedor |
| Transcripción / dictado (AssemblyAI) | Se envía el audio al proveedor |
| Resumen, NER, tripletes, embeddings, chat (OpenRouter) | Se envía el texto necesario al proveedor |
| Sincronización | Solo si la activás, hacia el servidor que indiques |

No hay un servicio de analítica aparte en la app. Las API keys se guardan de forma protegida en el sistema; no las compartas.

---

## 17. Problemas frecuentes

| Situación | Qué revisar |
| --- | --- |
| No corre el OCR | Key de GLM-OCR y prueba de conexión; internet |
| No transcribe | Key de AssemblyAI; formato de audio soportado |
| No hay corrección / resumen / NER / chat | Key y modelo de OpenRouter |
| El chat “no encuentra nada” | ¿Hay texto extraído? ¿Corriste **EMBED** en los documentos relevantes? |
| La búsqueda no da resultados | ¿Corriste **INDEX**? ¿El texto existe en la pestaña Texto? |
| Importación rechaza archivos | Revisá la lista de formatos de la sección 1 |

---

## 18. Mapa mental del trabajo

```text
Colección
  └── Documento (ítem)
        ├── Asset(s): imagen / página PDF / audio
        ├── Texto (OCR o transcripción) + resumen
        ├── Layout y anotaciones
        ├── Notas + tópicos
        ├── Entidades, tripletes, mapa
        └── Embeddings → búsqueda similar y Chat
```

---

*Manual orientado a EntropIA Lite. La variante Pro añade motores locales; este texto describe solo Lite.*
