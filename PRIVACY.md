# Aviso de privacidad de EntropIA Pro y Lite

**English:** [PRIVACY.en.md](./PRIVACY.en.md)

EntropIA es una app de escritorio local-first, en dos variantes: **Pro** (IA local más proveedores remotos opcionales) y **Lite** (IA 100% remota). Tus colecciones, archivos importados, texto extraído, notas, manuscritos, índices, salidas de IA y copias de seguridad se guardan en tu máquina. El contenido solo se envía cuando configurás o disparás un proveedor remoto, cuando sincronizás una biblioteca de Zotero en la Biblioteca, o cuando activás la sincronización.

En **Lite** no hay motores locales: OCR, transcripción, corrección, resúmenes, entidades, relaciones, embeddings, Chat, Investigar y el agente de Escritura envían el contenido necesario a los proveedores remotos que configures. En **Pro** esas tareas pueden correr en la máquina; el envío depende del modo elegido en Configuración.

Algunos caminos de red se disparan **automáticamente**, sin que los pidas explícitamente en ese momento: las teselas al abrir el mapa, la geocodificación al guardar una entidad de tipo lugar, el título automático de una conversación nueva de chat, el procesamiento en segundo plano de una biblioteca de Zotero ya sincronizada, la comprobación de actualizaciones de Microsoft Store (Lite en Windows) y, en Pro, la descarga del runtime de IA al iniciar. Están detallados abajo.

## Qué queda local por defecto

| Dato                                                  | Manejo por defecto                                             |
| ----------------------------------------------------- | -------------------------------------------------------------- |
| Colecciones y metadata                                | Se guardan en el directorio local de datos de EntropIA.        |
| Assets importados                                     | Se copian al directorio de datos durante la importación.       |
| OCR y texto extraído                                  | Se guardan localmente en la base de datos de la app.           |
| Índices FTS, embeddings, entidades, resúmenes         | Se guardan localmente cuando se generan.                       |
| Manuscritos de Escritura y su historial de versiones  | Se guardan localmente en la base de datos de la app.           |
| Biblioteca (catálogo, texto y embeddings de obras de Zotero) | Se guarda localmente; no viaja con la sincronización.   |
| Copias de seguridad                                   | Se escriben solo en la carpeta que elijas.                     |
| Archivos de modelos locales y dependencias de runtime (Pro) | Se guardan localmente en directorios de app/runtime.     |
| API keys de proveedores y token de sesión de sincronización | Se guardan en el gestor de credenciales del sistema operativo, no en la base de la app ni en las copias de seguridad. |

## Actividad de red

| Función                           | Destino                                                           | Qué puede enviarse o descargarse                                                                                                                                      |
| --------------------------------- | ----------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Runtime de IA (Pro)               | Releases firmados del repositorio en GitHub                       | Al iniciar, descarga el runtime de IA verificando firma y sha256; luego instala las dependencias desde su wheelhouse offline.                                          |
| Descarga del modelo local Gemma (Pro) | URL de modelo Hugging Face configurada por la app             | Descarga el archivo GGUF del modelo.                                                                                                                                  |
| Bootstrap de dependencias (Pro)   | Fuentes configuradas de runtime y paquetes                        | Descarga archivos de runtime, paquetes Python o herramientas cuando no están ya bundleados.                                                                           |
| LLM y embeddings con OpenRouter   | API de OpenRouter                                                 | Envía el texto necesario para la tarea solicitada (corrección, resumen, entidades, relaciones, esquemas propios, Chat, Investigar, agente de Escritura, embeddings y reordenamiento de pasajes) y la API key configurada. |
| Respaldo Vercel AI Gateway        | API de Vercel AI Gateway (`ai-gateway.vercel.sh`)                 | Solo si guardaste una clave y OpenRouter falla: reenvía ese mismo pedido de texto (corrección, resumen, entidades, relaciones, esquemas propios o Chat) con el mismo modelo. |
| Transcripción con AssemblyAI      | API de AssemblyAI                                                 | Sube el audio seleccionado para transcripción o dictado y usa la API key configurada.                                                                                 |
| OCRH con GLM-OCR                  | API de GLM-OCR (z.ai)                                             | Sube las imágenes/PDF seleccionados para OCR de alta calidad y usa la API key configurada.                                                                            |
| Título automático de conversación | API de OpenRouter                                                 | Al responderse la primera pregunta de una conversación nueva de chat, envía esa pregunta y esa respuesta (recortadas a 600 caracteres cada una) para que el modelo proponga un título. Ocurre sin acción explícita del usuario. |
| Zotero local                      | La API local de Zotero en el mismo equipo (`127.0.0.1:23119`)     | Lee referencias, adjuntos y bibliotecas para citar y para la Biblioteca. No sale de la máquina.                                                                        |
| Biblioteca (sincronización de una biblioteca de Zotero) | GLM-OCR y OpenRouter                         | Después de que pedís sincronizar una biblioteca, procesa en segundo plano sus obras: envía a GLM-OCR las páginas escaneadas o con texto ilegible de sus PDF, y a OpenRouter el texto de las obras y sus pasajes para calcular embeddings (en Pro, según los modos de OCR y embeddings elegidos). Sigue al reabrir la app hasta terminar. |
| Búsqueda por significado en la Biblioteca | API de OpenRouter                                         | Envía la consulta (o el texto seleccionado con «Buscar desde la selección») para calcular su embedding.                                                              |
| Consultas bibliográficas de Investigar | Zotero local y OpenAlex (`api.openalex.org`)                 | Cuando aprobás un plan con consultas bibliográficas, envía esas consultas para obtener metadatos de obras.                                                            |
| Importar desde Internet Archive   | `archive.org` y sus subdominios                                   | Envía las búsquedas, identificadores o enlaces que escribas y descarga los PDF que elijas. El servicio recibe la dirección IP y las consultas.                         |
| Geocodificación de lugares        | Nominatim de OpenStreetMap (`nominatim.openstreetmap.org`)        | Al crear o editar una entidad de tipo lugar, envía el texto del lugar para resolver sus coordenadas. Ocurre aunque nunca abras el mapa; el servicio recibe la dirección IP y el texto consultado. |
| Mapa de ubicaciones               | Servidores de teselas de OpenStreetMap (`tile.openstreetmap.org`) | Al abrir el mapa, solicita las teselas visibles; el servicio recibe la dirección IP, metadata de la solicitud y las coordenadas/zoom codificadas en las URLs pedidas. |
| Sincronización entre dispositivos | Servicio de sincronización de EntropIA, operado por HLab (`entropia-cloud.app.hlab.com.ar`) | Envía credenciales al registrar la cuenta o iniciar sesión, y las filas y archivos de lo que se sincroniza (ver abajo). |
| Compartir un manuscrito           | El mismo servicio de sincronización                               | El manuscrito compartido, con sus imágenes, llega a la cuenta con la que lo compartiste.                                                                              |
| Enviar a hlab.com.ar              | `hlab.com.ar`                                                     | Solo cuando lo pedís: el título y el texto del manuscrito, con la clave de publicación que guardaste.                                                                 |
| Clave de la Web API de Zotero     | `api.zotero.org`                                                  | Solo si guardás una clave: la verifica y muestra sus permisos. Hoy solo la usa una función experimental que no viene activada en las versiones publicadas.            |
| Lista de modelos de OpenRouter    | API de OpenRouter                                                 | Al probar la conexión o abrir un selector de modelo, pide el catálogo de modelos con tu clave.                                                                       |
| Aviso de actualización (Lite en Windows) | Microsoft Store                                            | Consulta a la Store si hay una versión más nueva publicada. No descarga ni instala nada.                                                                              |
| Links externos en la UI           | Navegador/handler del sistema                                     | Abre la URL seleccionada fuera de la app.                                                                                                                             |

El codebase actual no incluye un servicio separado de analytics o telemetría. Los logs operativos se escriben localmente para diagnóstico.

## Sincronización entre dispositivos

La sincronización está **desactivada hasta que iniciás sesión**. Cuando la activás:

- el destino es el servicio de sincronización de EntropIA, operado por HLab; la app no permite cambiar su dirección;
- la conexión exige HTTPS;
- el token de sesión se guarda en el gestor de credenciales del sistema operativo — llavero en Linux, Administrador de credenciales en Windows, Llavero en macOS — y no en la base de datos de la app;
- el servicio recibe colecciones, documentos y sus archivos, textos, notas, tópicos, anotaciones, entidades, relaciones, resultados de IA, embeddings, conversaciones del Chat, manuscritos con sus imágenes y las investigaciones terminadas o fallidas, con la misma sensibilidad que tengan tus colecciones;
- no recibe la Biblioteca, los esquemas propios, las preferencias, las colecciones de ejemplo ni las API keys.

Para cortar el camino remoto por completo, cerrá la sesión de sincronización. **Borrar mis datos del servidor**, en Configuración → Sincronización, elimina los datos remotos de tu cuenta y conserva los locales.

## API keys

Las API keys de OpenRouter, Vercel AI Gateway, AssemblyAI, GLM-OCR, Zotero y la clave de publicación de hlab.com.ar son configuraciones provistas por el usuario. Tratalas como secretos:

- no commitees datos de app ni archivos de configuración;
- no compartas logs que puedan contener nombres de proveedores, errores de request o detalles de configuración sin revisarlos primero;
- rotá una key si fue expuesta.

## Control del usuario

- En Pro, usá modos locales si no querés enviar contenido a un proveedor remoto de IA. En Lite, cada tarea de IA usa un proveedor remoto: no la ejecutes sobre material que no quieras enviar.
- Quitá API keys de proveedores desde Configuración (botón **Quitar clave**) para deshabilitar esos caminos remotos. Sin API key de OpenRouter tampoco se genera el título automático de conversaciones ni se procesa la Biblioteca por significado; sin clave de Vercel no hay respaldo.
- No sincronices bibliotecas de Zotero en la Biblioteca si no querés que el texto de sus obras se envíe a OpenRouter y GLM-OCR.
- No marques entidades como lugar, o no las guardes, si no querés que su texto se consulte contra Nominatim.
- Cerrá la sesión de sincronización para que nada salga por ese camino.
- Las copias de seguridad contienen todo tu archivo: guardalas en una carpeta protegida.
- Eliminá el directorio local de datos de la app si querés borrar bases locales, logs, archivos de runtime y salidas generadas. Las API keys y el token de sincronización no viven ahí: se quitan desde Configuración o cerrando la sesión.

## Limitaciones

Este aviso describe el comportamiento de la aplicación EntropIA Pro y Lite. Los proveedores remotos (OpenRouter, Vercel, AssemblyAI, z.ai, OpenStreetMap, Internet Archive, OpenAlex, Zotero, Microsoft) tienen sus propias políticas de privacidad, términos de retención y controles de cuenta.
