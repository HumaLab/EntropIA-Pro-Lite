# Preguntas frecuentes de la ayuda en la app

Respuestas deterministas que la ayuda responde en este equipo, sin consultar a OpenRouter (S10).
Cada respuesta está tomada del manual del usuario, con los mismos nombres de botones y menús.

## Primeros pasos

### ¿Cómo creo mi primera colección?
Variantes: crear colección; nueva colección; armar una colección
Fuente: Capítulo 1. Inicio rápido › 1.3. Crear o seleccionar una colección

1. En **Colecciones**, pulsá el botón **Nueva colección** (o el botón con forma de carpeta y signo más del explorador lateral).
2. Escribí un **Nombre de la colección**, por ejemplo `Prensa local, 1930–1940`. La **Descripción (opcional)** puede quedar vacía.
3. Pulsá **Crear colección** y comprobá que aparece la tarjeta nueva; pulsá su nombre para abrirla.

### ¿Cómo importo un PDF?
Variantes: importar pdf; subir un pdf; agregar archivos a la colección
Fuente: Capítulo 4. Crear y organizar una colección de documentos › 4.2. Importar archivos

1. Dentro de la colección, pulsá **Importar documento** o arrastrá los archivos desde el Explorador de Windows sobre la colección.
2. Elegí uno o varios PDF, imágenes o audios admitidos y confirmá.
3. Esperá a que termine: **Resumen de importación** indica cuántos se importaron, omitieron o fallaron.
4. La importación no ejecuta OCR ni transcribe: para obtener texto seguí con **Texto**.

### ¿Cómo abro un documento?
Variantes: abrir documento; ver un documento; entrar a un documento
Fuente: Capítulo 5. Trabajar con documentos, páginas y archivos › 5.1. Abrir y recorrer un documento

1. En la colección, seleccioná la tarjeta del documento.
2. Movete con **Página anterior** y **Página siguiente**, o desde el explorador lateral.
3. Acercá o alejate con los controles de zoom del visor. Si toda la interfaz se ve pequeña, ajustá **Configuración → Apariencia → Zoom**.

### ¿Cómo obtengo el texto de un PDF escaneado?
Variantes: texto de pdf escaneado; leer pdf escaneado; extraer texto de escaneo
Fuente: Capítulo 18. Ejemplos paso a paso › 18.1. Tengo un PDF escaneado y quiero buscar su contenido

1. Abrí el PDF, elegí la página y abrí **Texto** en el panel derecho.
2. Pulsá **OCRH** y esperá. Necesitás conexión y GLM-OCR configurado en **Configuración → APIs remotas**.
3. Compará el texto con la imagen y corregí nombres o cifras. Si falta un párrafo entero, corregí esa caja en **Layout** en vez de escribirlo a mano.
4. Buscá una palabra; si no aparece, usá **Análisis → INDEX** y repetí la búsqueda.

### ¿Cómo hago OCR de un documento?
Variantes: ocr; reconocer texto; sacar texto de imagen
Fuente: Capítulo 6. Obtener texto de documentos › 6.1. OCR de PDF o imagen

1. Seleccioná un PDF o una imagen compatible y abrí **Texto** en el panel derecho.
2. Pulsá **OCRH** y esperá a que termine.
3. Leé el resultado contra la página original y corregí a mano los errores que importen.
4. Convertí a PNG o JPG las imágenes WebP, TIFF o TIF antes de reconocerlas: GLM-OCR no acepta esos formatos.

### ¿Cómo transcribo un audio?
Variantes: transcribir audio; pasar audio a texto; stt
Fuente: Capítulo 6. Obtener texto de documentos › 6.3. Transcribir audio

1. Abrí el audio, comprobá que se pueda reproducir y escuchá un fragmento.
2. En el panel **Texto**, elegí **STT** y esperá la transcripción. Importar o reproducir no la inicia.
3. Revisá el texto junto al audio y corregí nombres, fechas y turnos de habla.
4. Para conservarla fuera de EntropIA, copiá el campo editable a un archivo propio: el audio no tiene menú **Descargar**.

### ¿Cómo elimino una imagen?
Variantes: eliminar imagen; borrar página; eliminar página activa
Fuente: Capítulo 5. Trabajar con documentos, páginas y archivos › 5.6. Eliminar un documento o una página

1. Para quitar solo la página o archivo abierto, usá **Eliminar página activa** junto a la ruta de esa pestaña.
2. Para quitar el documento completo con sus archivos y datos, usá la papelera de su tarjeta.
3. Revisá el nombre y la confirmación: no hay deshacer; solo una copia de seguridad anterior permite recuperarlo.

### ¿Cómo cargo mi clave de OpenRouter?
Variantes: clave openrouter; api key openrouter; configurar openrouter
Fuente: Capítulo 16. Configuración y herramientas generales › 16.1. Configurar proveedores

1. Abrí **Configuración → APIs remotas** y buscá **OpenRouter**.
2. Pegá la clave en **API Key** y pulsá **Probar conexión**.
3. Cuando la prueba sea correcta, pulsá **Guardar cambios**: probar no guarda la clave.
4. Conservá un **Modelo generativo** y un **Modelo de embeddings** válidos para tu cuenta.

### ¿Cómo busco una palabra en mis documentos?
Variantes: buscar palabra; ubicar una palabra; buscar en mis documentos
Fuente: Capítulo 7. Buscar información › 7.1. Buscar una palabra o varios términos

1. Escribí el término en la búsqueda de la barra superior o en **Búsquedas**.
2. Abrí el resultado y comprobá el fragmento en el documento original.
3. Si el texto recién reconocido no aparece, abrí **Análisis** en ese documento, pulsá **INDEX** y volvé a buscar.
4. Probá otra grafía o una parte distintiva de la frase si el OCR produjo una variante.

### ¿Cómo exporto el texto de un documento?
Variantes: exportar texto; descargar texto; copiar texto extraído
Fuente: Capítulo 6. Obtener texto de documentos › 6.4. Comprobar, copiar y descargar

1. Abrí la página y desplegá **Texto extraído** sobre el visor (imágenes y PDF con OCR).
2. Usá **Copiar** o **Descargar → Markdown / PDF / Word (.docx)**.
3. La transcripción de un audio no tiene ese menú: copiá manualmente su campo editable.
4. Para guardar todo (base y archivos), usá las copias de seguridad: las descargas sueltas no permiten recuperar EntropIA.

## Preguntas frecuentes

### Importé un documento pero no veo ningún texto, ¿qué hago?
Variantes: no hay texto después de importar; importar sin texto; texto vacío tras importar
Fuente: Capítulo 19. ¿Qué hago si…? › Capítulo 19. ¿Qué hago si…?

1. Es normal: la importación no reconoce texto automáticamente.
2. Abrí **Texto** y usá **OCRH** para una imagen o un PDF compatible, o **STT** para pasar el audio a texto.
3. Si el texto ya existe y no lo encontrás, preparalo con **Análisis → INDEX**.

### El reconocimiento confunde letras y palabras, ¿cómo lo corrijo?
Variantes: errores de reconocimiento; letras mal reconocidas; corregir palabras mal leídas
Fuente: Capítulo 19. ¿Qué hago si…? › Capítulo 19. ¿Qué hago si…?

1. Compará el resultado con la imagen y corregí lo necesario a mano en **Texto extraído**.
2. En fotos, usá PNG o JPG y revisá que la imagen no esté borrosa, inclinada u oscura.
3. Si falta un párrafo entero o el final de un bloque, corregí esa caja en **Layout** en vez de repetir el OCR.

### OCRH no está disponible o da error, ¿cómo lo reviso?
Variantes: ocrh no funciona; ocrh da error; error de reconocimiento remoto
Fuente: Capítulo 19. ¿Qué hago si…? › Capítulo 19. ¿Qué hago si…?

1. En Lite el reconocimiento necesita Internet: revisá la conexión.
2. Abrí **Configuración → APIs remotas**, revisá GLM-OCR, probá la conexión y pulsá **Guardar cambios**.
3. Convertí a PNG o JPG las imágenes WebP, TIFF o TIF antes de reconocerlas.

### OpenRouter se quedó sin crédito o no responde, ¿qué hago?
Variantes: sin crédito openrouter; openrouter no responde; error de crédito
Fuente: Capítulo 19. ¿Qué hago si…? › Capítulo 19. ¿Qué hago si…?

1. Revisá tu cuenta de OpenRouter: la clave no equivale a crédito ni a disponibilidad de cada modelo.
2. Si configuraste el respaldo de **Vercel AI Gateway**, la corrección, los resúmenes, las entidades, los tripletes, los esquemas y el Chat siguen por Vercel; Investigar, el Agente de Escritura y los embeddings no.
3. Reintentá cuando el problema se haya resuelto.

### La primera transcripción con AssemblyAI falla, ¿qué reviso?
Variantes: falló transcripción; error 401 assemblyai; guardar cambios assemblyai
Fuente: Capítulo 16. Configuración y herramientas generales › 16.1. Configurar proveedores

1. Abrí **Configuración → APIs remotas → AssemblyAI** y revisá que la clave sea la correcta.
2. Pulsá **Probar conexión** y después **Guardar cambios**: probar no guarda la clave, y sin guardar la transcripción falla.
3. Reintentá la transcripción con **STT** en **Texto**.

### Los PDF no se abren en mi Mac, ¿qué hago?
Variantes: pdf no abre mac; macos viejo pdf; safari pdf no abre
Fuente: Capítulo 2. Introducción a EntropIA Pro y Lite › 2.1. Qué necesitás para cada tarea

1. El visor de PDF necesita Safari 16.4, que llega con **macOS 13.3 Ventura o posterior**: actualizá el sistema.
2. En versiones anteriores los PDF no abren; no es un problema de tus archivos.
3. Después de actualizar, volvé a abrir el documento.

### Un documento no me aparece, ¿cómo lo localizo?
Variantes: documento perdido; no aparece documento; localizar documento
Fuente: Capítulo 19. ¿Qué hago si…? › Capítulo 19. ¿Qué hago si…?

1. Abrí la colección correcta y limpiá los filtros.
2. Buscá por nombre en **Buscar documentos...** o usá la búsqueda superior.
3. Si lo eliminaste, solo una copia de seguridad anterior permite recuperarlo.

### Una búsqueda no devuelve resultados, ¿qué hago?
Variantes: búsqueda sin resultados; palabra sin resultados; indexar texto
Fuente: Capítulo 19. ¿Qué hago si…? › Capítulo 19. ¿Qué hago si…?

1. Comprobá que la página correcta tenga texto reconocido.
2. En **Análisis**, pulsá **INDEX** y probá una palabra más breve o escrita de otra manera.
3. Revisá que estés buscando en la colección correcta.

### Assets similares no muestra resultados, ¿qué falta?
Variantes: assets similares vacío; sin materiales parecidos; embed sin resultados
Fuente: Capítulo 19. ¿Qué hago si…? › Capítulo 19. ¿Qué hago si…?

1. Comprobá que la página o archivo tenga texto y que **EMBED** haya terminado en **Análisis**.
2. En Lite, revisá la conexión con OpenRouter en **Configuración → APIs remotas**.
3. Prepará también los otros materiales que querés comparar: con una sola página con embeddings no alcanza.

### Mi lote quedó pausado, ¿cómo lo reanudo?
Variantes: lote pausado; reanudar lote; lote detenido
Fuente: Capítulo 14. Procesamiento por lotes › Capítulo 14. Procesamiento por lotes

1. Abrí **Configuración → Lotes** y entrá en **Lotes activos y recuperados → Ver detalle**.
2. Revisá pendientes, interrumpidos y fallidos antes de continuar.
3. Usá **Reanudar** cuando el estado lo permite; al reabrir la app, el aviso de recuperación permite **Mantener pausados** o **Reanudar**.

### ¿Cómo hago una copia de seguridad de todo?
Variantes: copia de seguridad; respaldar entropia; backup
Fuente: Capítulo 17. Exportar, descargar y respaldar › 17.3. Copias de seguridad

1. Abrí **Configuración → Copias** y en **Carpeta de las copias** pulsá **Elegir carpeta** (idealmente en otro disco).
2. Elegí la **Copia automática** y cuántas conservar en **Copias a guardar**.
3. Pulsá **Hacer una copia ahora**; al terminar se muestra **Copia guardada en** con la ruta.
4. Para recuperar no hay botón: se copia de vuelta a mano con EntropIA cerrada.

### ¿Cómo conecto Zotero con EntropIA?
Variantes: conectar zotero; zotero conexión; vincular zotero
Fuente: Capítulo 12. Zotero y Biblioteca › 12.1. Conectar Zotero

1. Abrí Zotero y activá **Permitir que otras aplicaciones se comuniquen con Zotero** en **Editar → Configuración → Avanzado**.
2. En EntropIA, creá o abrí un manuscrito en **Escritura**, mostrá el panel de investigación y elegí **Zotero**.
3. Cuando Zotero responda, la pestaña muestra las referencias y habilita **Actualizar**.

### Zotero no aparece en Escritura, ¿qué reviso?
Variantes: zotero no aparece; escritura sin zotero; permitir comunicación zotero
Fuente: Capítulo 19. ¿Qué hago si…? › Capítulo 19. ¿Qué hago si…?

1. Abrí Zotero y habilitá **Permitir que otras aplicaciones se comuniquen con Zotero** en **Editar → Configuración → Avanzado**.
2. Volvé a **Escritura → Zotero** y usá **Actualizar** si está disponible.
3. Si abriste Zotero después, cambiá de pestaña del panel y volvé a **Zotero** para comprobar la conexión.

### Mis anotaciones no llegan a Zotero, ¿qué falta?
Variantes: anotaciones no sincronizan; permiso escritura zotero; verificar clave zotero
Fuente: Capítulo 19. ¿Qué hago si…? › Capítulo 19. ¿Qué hago si…?

1. Leé el aviso del panel **Anotaciones**.
2. Hace falta una clave de la Web API con permiso de escritura en la biblioteca personal: revisala en **Configuración → APIs remotas → Zotero → Verificar clave**, de la misma cuenta que tenés abierta en Zotero. En los grupos no se puede anotar.
3. Sin conexión quedan **Pendiente**; pulsá **Sincronizar con Zotero** cuando vuelva.

### Me registré en la nube pero no sincroniza, ¿qué hago?
Variantes: registro no sincroniza; iniciar sesión sincronización; no puedo sincronizar
Fuente: Capítulo 15. Sincronización y nube › 15.1. Iniciar sesión y sincronizar

1. Después de **Registrar cuenta**, pulsá **Iniciar sesión**: el registro no inicia la sincronización.
2. Usá **Sincronizar ahora** o activá la **Sincronización automática** con un intervalo en minutos.
3. Revisá el indicador inferior, la conexión y la sesión en **Configuración → Sincronización**; no borres datos como prueba.

### Internet Archive dice restringido o se cortó la descarga, ¿qué hago?
Variantes: archive restringido; descarga cortada archive; retomar importación archive
Fuente: Capítulo 19. ¿Qué hago si…? › Capítulo 19. ¿Qué hago si…?

1. Si dice **restringido**, **cifrado** o **privado**, Internet Archive no permite descargarlo: buscá otra edición o un derivado disponible.
2. Si se cortó, abrí de nuevo el diálogo de Internet Archive y usá **Importaciones en curso → Retomar importación**.
3. La importación no ejecuta OCR: después usá **OCRH** o un lote.

### No encuentro Descargar en una transcripción, ¿cómo la guardo?
Variantes: descargar transcripción; guardar transcripción; copiar transcripción
Fuente: Capítulo 19. ¿Qué hago si…? › Capítulo 19. ¿Qué hago si…?

1. Ese menú solo existe para OCR de imágenes y PDF en **Texto extraído** sobre el visor.
2. Seleccioná y copiá manualmente el texto del campo editable de la transcripción a un archivo propio.
3. Conservá también el audio original.

### OCRC cambió mi texto sin preguntar, ¿cómo vuelvo atrás?
Variantes: restaurar texto original; corrección automática; deshacer corrección
Fuente: Capítulo 19. ¿Qué hago si…? › Capítulo 19. ¿Qué hago si…?

1. Es así: **OCRC** aplica la corrección al terminar, sin pedir aceptación.
2. En **Texto extraído** sobre el visor usá **Restaurar OCR original** para volver al texto anterior.
3. Si editaste el texto a mano después de corregir, conservá aparte lo que quieras mantener.

### Guardar cambios falla después de pegar una clave, ¿qué reviso?
Variantes: no se guarda la clave; error al guardar clave; validación de prompts
Fuente: Capítulo 19. ¿Qué hago si…? › Capítulo 19. ¿Qué hago si…?

1. Revisá el mensaje y la pestaña señalada: también se validan Prompts, Model Params y RAG Params.
2. Si el guardado te lleva a otra pestaña, corregí el parámetro señalado.
3. Probá de nuevo después de corregir.

### Analizar selección está deshabilitado en Lotes, ¿qué falta?
Variantes: analizar selección bloqueado; botón de lote deshabilitado; falta clave para el lote
Fuente: Capítulo 19. ¿Qué hago si…? › Capítulo 19. ¿Qué hago si…?

1. Elegí al menos una colección y una operación.
2. Si marcaste Embeddings, Entidades, Tripletes o un esquema, primero guardá la clave de OpenRouter en **Configuración → APIs remotas → OpenRouter**.
3. Revisá también que haya texto: sin texto, esas operaciones esperan al OCR.
