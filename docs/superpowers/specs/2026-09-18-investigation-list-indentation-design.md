# Sangría de listas en Investigación

## Problema

Las listas de contenido del informe de Investigación heredan la presentación del navegador sin una sangría horizontal explícita. Los marcadores quedan demasiado cerca del borde izquierdo del panel y pierden separación respecto del texto.

## Alcance

La corrección se limita al contenido textual renderizado dentro de `.investigation-chat__report` en `apps/desktop/src/views/InvestigationView.svelte`:

- listas desordenadas y numeradas de informes estructurados;
- listas producidas por Markdown/HTML sanitizado, incluido el fallback del informe completo;
- listas anidadas y elementos de varias líneas;
- contenido guardado desde la edición manual una vez renderizado nuevamente.

Quedan fuera las listas funcionales sin marcador de citas, fuentes y rutas. No se modifican estilos globales ni otras vistas.

## Diseño

Agregar una regla descendente scoped para `ul` y `ol` de contenido dentro de `.investigation-chat__report`, excluyendo explícitamente `.report__quotes` y `.report__sources-list`.

La regla usará:

- `padding-inline-start: var(--space-6)` para una sangría de 24 px, dentro del rango visual solicitado y alineada con la escala de 4 px de EntropIA;
- `list-style-position: outside` para conservar el marcador fuera del bloque de texto y la sangría colgante de elementos multilínea;
- propiedades lógicas, de modo que la presentación siga la dirección del texto.

No se agregarán márgenes verticales. Las listas anidadas recibirán la misma regla en cada nivel y acumularán una jerarquía clara sin introducir un espaciado nuevo entre bloques.

## Alternativas descartadas

1. Envolver cada fragmento Markdown con una clase adicional: más markup y riesgo de omitir alguna ruta de renderizado.
2. Crear un estilo rich-text compartido: ampliaría innecesariamente el alcance a otras superficies de la aplicación.

## Verificación

- Una comprobación enfocada debe cubrir `ul`, `ol`, listas anidadas y elementos multilínea dentro del informe.
- La inspección visual debe confirmar una sangría de 24 px, marcadores externos, alineación colgante y ausencia de cambios en citas/fuentes funcionales.
- El componente debe pasar el autofixer de Svelte y las comprobaciones enfocadas del paquete desktop.
