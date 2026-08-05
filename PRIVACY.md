# Aviso de privacidad de EntropIA Pro

**English:** [PRIVACY.en.md](./PRIVACY.en.md)

EntropIA Pro está diseñada como una app de escritorio local-first. Tus colecciones, archivos importados, texto extraído, notas, índices y salidas locales de IA se guardan en tu máquina salvo que configures o dispares explícitamente un proveedor remoto.

## Qué queda local por defecto

| Dato | Manejo por defecto |
| ---- | ------------------ |
| Colecciones y metadata | Se guardan en el directorio local de datos de EntropIA. |
| Assets importados | Se referencian o copian según el flujo de importación desktop. |
| OCR y texto extraído | Se guardan localmente en la base de datos de la app. |
| Índices FTS, embeddings, entidades, resúmenes | Se guardan localmente cuando se generan. |
| Archivos de modelos locales y dependencias de runtime | Se guardan localmente en directorios de app/runtime. |

## Actividad de red

EntropIA Pro puede contactar servicios externos solo para funciones que requieren descargas o proveedores cloud configurados por el usuario.

| Función | Destino | Qué puede enviarse o descargarse |
| ------- | ------- | -------------------------------- |
| Descarga del modelo local Gemma | URL de modelo Hugging Face configurada por la app | Descarga el archivo GGUF del modelo. |
| Bootstrap de dependencias/runtime | Fuentes configuradas de runtime y paquetes | Descarga archivos de runtime, paquetes Python o herramientas cuando no están ya bundleados. |
| Modo LLM OpenRouter | API de OpenRouter | Envía el texto necesario para la tarea LLM solicitada y la API key configurada. |
| Modo transcripción AssemblyAI | API de AssemblyAI | Sube el audio seleccionado para transcripción y usa la API key configurada. |
| OCRH con GLM OCR | API de GLM OCR (z.ai) | Sube las imágenes/PDF seleccionados para OCR de alta calidad y usa la API key configurada. |
| Links externos en la UI | Navegador/handler del sistema | Abre la URL seleccionada fuera de la app. |

El codebase actual no incluye un servicio separado de analytics o telemetría. Los logs operativos se escriben localmente para diagnóstico.

## API keys

Las API keys de OpenRouter, AssemblyAI y GLM OCR son configuraciones provistas por el usuario. Tratalas como secretos:

- no commitees datos de app ni archivos de configuración;
- no compartas logs que puedan contener nombres de proveedores, errores de request o detalles de configuración sin revisarlos primero;
- rotá una key si fue expuesta.

## Control del usuario

- Usá modos locales si no querés enviar contenido a un proveedor remoto de IA.
- Remové API keys de proveedores desde Settings para deshabilitar esos caminos remotos.
- Eliminá el directorio local de datos de la app si querés borrar bases locales, logs, archivos de runtime y salidas generadas.

## Limitaciones

Este aviso describe el comportamiento de la aplicación EntropIA Pro. Los proveedores remotos tienen sus propias políticas de privacidad, términos de retención y controles de cuenta.
