# Aviso de privacidad de EntropIA Pro

**English:** [PRIVACY.en.md](./PRIVACY.en.md)

EntropIA Pro está diseñada como una app de escritorio local-first. Tus colecciones, archivos importados, texto extraído, notas, índices y salidas locales de IA se guardan en tu máquina. El contenido solo se envía cuando configurás o disparás un proveedor remoto, o cuando activás la sincronización.

Tres caminos de red se disparan **automáticamente**, sin que los pidas explícitamente en ese momento: las teselas al abrir el mapa, la geocodificación al guardar una entidad de tipo lugar, y el título automático de una conversación nueva de chat. Están detallados abajo.

## Qué queda local por defecto

| Dato                                                  | Manejo por defecto                                             |
| ----------------------------------------------------- | -------------------------------------------------------------- |
| Colecciones y metadata                                | Se guardan en el directorio local de datos de EntropIA.        |
| Assets importados                                     | Se referencian o copian según el flujo de importación desktop. |
| OCR y texto extraído                                  | Se guardan localmente en la base de datos de la app.           |
| Índices FTS, embeddings, entidades, resúmenes         | Se guardan localmente cuando se generan.                       |
| Archivos de modelos locales y dependencias de runtime | Se guardan localmente en directorios de app/runtime.           |
| Tokens de sesión de sincronización                    | Se guardan en el gestor de credenciales del sistema operativo, no en la base de la app. |

## Actividad de red

EntropIA Pro puede contactar servicios externos para descargas, proveedores cloud configurados por el usuario, sincronización entre dispositivos y servicios de OpenStreetMap.

| Función                           | Destino                                                           | Qué puede enviarse o descargarse                                                                                                                                      |
| --------------------------------- | ----------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Descarga del modelo local Gemma   | URL de modelo Hugging Face configurada por la app                 | Descarga el archivo GGUF del modelo.                                                                                                                                  |
| Bootstrap de dependencias/runtime | Fuentes configuradas de runtime y paquetes                        | Descarga archivos de runtime, paquetes Python o herramientas cuando no están ya bundleados.                                                                           |
| Modo LLM OpenRouter               | API de OpenRouter                                                 | Envía el texto necesario para la tarea LLM solicitada y la API key configurada.                                                                                       |
| Modo transcripción AssemblyAI     | API de AssemblyAI                                                 | Sube el audio seleccionado para transcripción y usa la API key configurada.                                                                                           |
| OCRH con GLM OCR                  | API de GLM OCR (z.ai)                                             | Sube las imágenes/PDF seleccionados para OCR de alta calidad y usa la API key configurada.                                                                            |
| Título automático de conversación | API de OpenRouter                                                 | Al responderse la primera pregunta de una conversación nueva de chat, envía esa pregunta y esa respuesta (recortadas a 600 caracteres cada una) para que el modelo proponga un título. Ocurre sin acción explícita del usuario. |
| Geocodificación de lugares        | Nominatim de OpenStreetMap (`nominatim.openstreetmap.org`)        | Al crear o editar una entidad de tipo lugar, envía el texto del lugar para resolver sus coordenadas. Ocurre aunque nunca abras el mapa; el servicio recibe la dirección IP y el texto consultado. |
| Mapa de ubicaciones               | Servidores de teselas de OpenStreetMap (`tile.openstreetmap.org`) | Al abrir el mapa, solicita las teselas visibles; el servicio recibe la dirección IP, metadata de la solicitud y las coordenadas/zoom codificadas en las URLs pedidas. |
| Sincronización entre dispositivos | El servidor de sincronización que configures                      | Envía credenciales al registrar la cuenta o iniciar sesión, y las filas y blobs de lo que se sincroniza. El destino no es un servicio de EntropIA: es el servidor que elijas. La app exige HTTPS y rechaza un `server_url` sin TLS salvo que apunte a loopback. |
| Links externos en la UI           | Navegador/handler del sistema                                     | Abre la URL seleccionada fuera de la app.                                                                                                                             |

El codebase actual no incluye un servicio separado de analytics o telemetría. Los logs operativos se escriben localmente para diagnóstico.

## Sincronización entre dispositivos

La sincronización está **desactivada hasta que la configurás**. Cuando la activás:

- el servidor de destino lo elegís vos; EntropIA no opera ninguno;
- la conexión exige HTTPS: un `server_url` sin TLS se rechaza, salvo que apunte a loopback;
- el token de sesión se guarda en el gestor de credenciales del sistema operativo — llavero en Linux, Administrador de credenciales en Windows — y no en la base de datos de la app;
- ese servidor recibe las filas y blobs de lo que sincronices, con la misma sensibilidad que tengan tus colecciones.

Para cortar el camino remoto por completo, cerrá la sesión de sincronización y no configures ningún servidor.

## API keys

Las API keys de OpenRouter, AssemblyAI y GLM OCR son configuraciones provistas por el usuario. Tratalas como secretos:

- no commitees datos de app ni archivos de configuración;
- no compartas logs que puedan contener nombres de proveedores, errores de request o detalles de configuración sin revisarlos primero;
- rotá una key si fue expuesta.

## Control del usuario

- Usá modos locales si no querés enviar contenido a un proveedor remoto de IA.
- Remové API keys de proveedores desde Settings para deshabilitar esos caminos remotos. Sin API key de OpenRouter tampoco se genera el título automático de conversaciones.
- No marques entidades como lugar, o no las guardes, si no querés que su texto se consulte contra Nominatim.
- Cerrá la sesión de sincronización y dejá el servidor sin configurar para que nada salga por ese camino.
- Eliminá el directorio local de datos de la app si querés borrar bases locales, logs, archivos de runtime y salidas generadas. El token de sincronización no vive ahí: se quita cerrando la sesión.

## Limitaciones

Este aviso describe el comportamiento de la aplicación EntropIA Pro. Los proveedores remotos tienen sus propias políticas de privacidad, términos de retención y controles de cuenta.
