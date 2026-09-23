# Plan auditado: actualizaciones de Microsoft Store en EntropIA Lite

Implementar solo detección nativa y un aviso descartable que abra la ficha de Lite. No descargar, instalar, generar MSIX ni publicar. Este documento define el trabajo futuro; la auditoría no implementa la función.

## 1. Correcciones y simplificaciones

| Hallazgo                                                                            | Decisión                                                                                                                                                                                                                    |
| ----------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| El alcance decía Lite, pero no excluía Pro en el backend.                           | Validar variante, plataforma e identidad antes de consultar Store o recuperar la caché.                                                                                                                                     |
| Se proponía mostrar `Package.Id.Version` como `storeVersion`.                       | `StorePackageUpdate.Package` representa el paquete que tiene una actualización; su contrato no garantiza la versión de destino. Usar texto sin número y eliminar extracción, comparación y formato de versiones para la UI. |
| Booleanos, motivo y campos informativos podían contradecirse.                       | Un único estado serializable; sin versiones, `mandatory` ni URI repetida en cada respuesta.                                                                                                                                 |
| Guardar solo la fecha ocultaría un aviso conocido al reiniciar antes de seis horas. | Persistir fecha, resultado e identidad de la instalación en una entrada de `app_settings`.                                                                                                                                  |
| La búsqueda manual opcional añadía otro flujo completo.                             | Solo consulta al iniciar. Eliminar `force`, botón manual, sondeo y reintentos automáticos.                                                                                                                                  |
| Abstracciones y clasificación extensa de errores sin consumidores distintos.        | Un módulo concreto; función inyectable para pruebas. Agrupar fallos operativos y conservar su diagnóstico en logs.                                                                                                          |
| Plugins/permisos/fallback nuevos para abrir una ficha.                              | Reutilizar el lanzador existente con una excepción de URI exacta, sin ampliar el esquema completo.                                                                                                                          |

Se conservan HWND, hilo WinRT correcto, deduplicación, espera acotada y prueba con Store real: resuelven riesgos concretos, no son sobreingeniería.

## 2. Alcance y constantes

- **Solo Windows Lite:** Rust sin `local-ml`, frontend con `VITE_LOCAL_ML=0`. Pro, macOS y Linux no consultan Store ni muestran controles de esta función.
- **Product ID:** `9N328K9L95JD`.
- **URI exacta:** `ms-windows-store://pdp/?ProductId=9N328K9L95JD`.
- **Familia Store:** `CONICET.EntropIALite_b16na7gwepwme`. Contrastar con `apps/desktop/src-tauri/scripts/repack-store-msix.ps1` y `src-tauri/msix/README.md` dentro de `apps/desktop`; no usar la identidad de ejemplo del MSIX base.
- Detectar mediante `StoreContext::GetDefault()` y `GetAppAndOptionalStorePackageUpdatesAsync()`. El Product ID abre la ficha; no es un parámetro de esta consulta de la instalación actual.
- Fuera de alcance: HTML de Store, GitHub Releases, manifiestos remotos propios, updater de Tauri, descarga/instalación, actualizaciones obligatorias y publicación.

## 3. Archivos y reutilización

Rutas relativas a `apps/desktop/`:

| Archivo                                             | Cambio o reutilización                                                                                                                                        |
| --------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src-tauri/Cargo.toml`, `Cargo.lock`                | El lock fija Tauri `2.10.3` y contiene `windows 0.61.3` transitivamente. Declarar `windows` como dependencia directa de Windows; no actualizar crates ajenos. |
| `src-tauri/src/store_updates.rs` — nuevo            | Consulta, filtro de paquete, caché y estado de sesión; pruebas Rust en el mismo módulo. Sin capas de repositorio/servicio/proveedor.                          |
| `src-tauri/src/lib.rs`                              | Registrar comando/estado y admitir la URI exacta en `open_external_url`, solo en Windows Lite.                                                                |
| `src-tauri/src/settings.rs`                         | Reutilizar `get_setting`/`set_setting` sobre `app_settings`. Sin tabla, migración ni plugin nuevo.                                                            |
| `src/lib/store-updates.ts` — nuevo                  | Tipo y wrapper de `invoke`, siguiendo `settings.ts`; sin otro store global.                                                                                   |
| `src/App.svelte`                                    | Consultar después del intento de `dismissSplash`, solo tras inicialización exitosa. Mantener resultado/descarte en la raíz y pasarlos a `AppShell`.           |
| `src/layout/AppShell.svelte`                        | Aviso dentro de `<main>`, antes de la ruta; reutilizar componentes/tokens, sin alterar los avisos Pro.                                                        |
| `src/lib/external-links.ts`                         | Aceptar la misma URI exacta sin normalizarla a otra cadena; conservar las reglas HTTP(S).                                                                     |
| `src/lib/i18n.ts`                                   | Cadenas en español e inglés.                                                                                                                                  |
| `src/App.test.ts`, `src/lib/external-links.test.ts` | Ampliar pruebas de comportamiento afectadas.                                                                                                                  |

`NotificationBell.svelte` y `notification-store.ts` son notificaciones de sincronización ligadas a una cuenta. No conectar Store a ese subsistema ni exigir iniciar sesión en EntropIA. `instance_guard.rs` ya evita instancias simultáneas en la sesión: no agregar otro bloqueo entre procesos.

Antes de implementar, revisar estos puntos y sus dependencias necesarias, no todo el monorepo. Adaptarlos si cambiaron desde la auditoría; no refactorizar módulos ajenos.

## 4. Backend y contrato

Comando `check_microsoft_store_update`, sin argumentos. Wrapper `checkMicrosoftStoreUpdate(): Promise<StoreUpdateStatus>`:

```ts
export type StoreUpdateStatus = 'available' | 'up_to_date' | 'skipped' | 'unavailable'
```

Serializar el enum Rust en `snake_case`:

- `available`: actualización del paquete principal informada por Store o por caché válida de esa instalación.
- `up_to_date`: respuesta exitosa sin actualización principal, o ese resultado en caché válida. No implica una consulta nueva al servidor.
- `skipped`: variante/plataforma no aplicable, ausencia reconocida de identidad o familia ajena a Lite Store.
- `unavailable`: fallo de consulta, contexto nativo, lectura de identidad o timeout. Nunca equivale a estar actualizado.

### Recorrido nativo

1. Fuera de `#[cfg(all(target_os = "windows", not(feature = "local-ml")))]`, devolver `skipped` sin WinRT ni caché. Mantener el comando registrable en todas las variantes.
2. Obtener `Package::Current()` y comprobar la familia esperada. Distinguir ausencia reconocida de identidad de otros errores COM; no tratar todo error como aplicación desempaquetada.
3. Si no hay caché válida, crear `StoreContext`, asociar el HWND principal mediante `IInitializeWithWindow` e iniciar la operación en el hilo de interfaz con `run_on_main_thread`. Un comando Rust `async` no asegura ese hilo.
4. Esperar asincrónicamente, sin bloquear el hilo de interfaz ni mantener el lock de SQLite. No usar `.get()` bloqueante allí ni mover objetos COM entre hilos sin soporte del binding. Reducir el resultado a datos propios antes de cruzar IPC.
5. Hay actualización si algún `StorePackageUpdate.Package.Id.FamilyName` coincide con la familia actual. Ignorar opcionales de otra familia; no elegir el primer elemento, calcular máximos ni comparar versiones. Si falla una lectura necesaria para decidir, devolver `unavailable`, no un falso `up_to_date`.
6. Persistir solo tras interpretar correctamente una respuesta exitosa, positiva o negativa.

Microsoft documenta `0x80070578` cuando falta el hilo UI o el contexto de ventana. Registrar etapa y HRESULT mediante los logs existentes, sin datos personales ni códigos crudos en la interfaz. No introducir un logger ni una clasificación especulativa de red, servicios y Store dañada. Sin `unwrap()`/`expect()`; localizar y justificar el `unsafe` imprescindible para HWND/COM.

### Dependencias y timeout

Usar la línea `windows 0.61.3` ya resuelta y verificar firmas contra su código fuente al implementar. Los nombres reales de features incluyen `ApplicationModel`, `Services_Store`, `Foundation` y `Win32_System_WinRT`, **sin** prefijo `Windows_`. Activar solo lo necesario para los símbolos utilizados; no añadir crates de colecciones/asincronía que ya lleguen transitivamente y no se usen directamente.

Acotar a **20 segundos** la espera, incluido el despacho al hilo principal. Devolver `unavailable` sin persistir éxito al vencer. Usar cancelación WinRT cuando esté soportada; abandonar un future no demuestra cancelación nativa. No iniciar tarde un despacho vencido, ni permitir otra consulta o escrituras tardías en esa sesión.

## 5. Una consulta por inicio, caché de seis horas

Guardar una entrada `microsoft_store_update_cache` en `app_settings`:

```ts
type StoreUpdateCache = {
  packageFullName: string
  checkedAtUnixSeconds: number
  updateAvailable: boolean
}
```

| Situación                                                           | Comportamiento                                                                                                                                               |
| ------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Mismo `packageFullName` y `0 <= now - checkedAtUnixSeconds < 21600` | Devolver el resultado guardado, incluido el positivo, sin consultar Store.                                                                                   |
| Nombre completo distinto, caché ausente/corrupta o fecha futura     | Consultar una vez. La identidad completa incluye la versión instalada y evita conservar un aviso después de actualizar.                                      |
| Exactamente seis horas o más                                        | Consultar una vez. No hay sondeo mientras la app permanece abierta.                                                                                          |
| Respuesta Store exitosa                                             | Guardar resultado y segundos UTC desde Unix epoch al finalizar; timestamp entero no negativo.                                                                |
| Fallo o timeout                                                     | No renovar fecha ni mostrar una caché vencida. Otro inicio podrá intentar nuevamente.                                                                        |
| Store responde pero falla la escritura                              | Conservar resultado en memoria y registrar el fallo. No ocultar una actualización válida; informar que la persistencia entre reinicios no quedó garantizada. |

El descarte del aviso **no modifica la caché**. Al reiniciar dentro del intervalo puede reaparecer sin otra consulta.

Una única inicialización compartida en el estado Tauri, por ejemplo `tokio::sync::OnceCell<StoreUpdateStatus>`, debe deduplicar llamadas y memorizar el resultado terminal, también `unavailable`/`skipped`. No reiniciar automáticamente al devolver error ni permitir que la cancelación de un consumidor duplique la operación nativa. La política temporal pertenece al backend, no se replica en Svelte.

**Garantía precisa:** seis horas desde una respuesta exitosa persistida, para la misma identidad y versión instalada. Un fallo no renueva el plazo y permite otro intento en un inicio posterior. Esto corrige la contradicción original entre «no persistir fallos» y «como máximo una consulta cada seis horas». Store también tiene caché y límites propios: una respuesta exitosa no garantiza tráfico nuevo a sus servidores.

## 6. Aviso y apertura segura

La consulta queda fuera del `Promise.all` de inicialización y no demora la entrega de la ventana principal. Manejar también rechazos de IPC sin activar el error global de arranque.

Mostrar solo con `available` y sin descarte en esta sesión:

> **Actualización disponible**
>
> Hay una actualización de EntropIA Lite disponible en Microsoft Store.
>
> **Ver actualización** · Cerrar

- Reutilizar `Button`, `IconButton`, `ActionIcon` y tokens de `@entropia/ui` según corresponda; no crear un sistema genérico de banners.
- Aviso en el flujo del contenido, separado y adaptable a ventanas estrechas; sin overlay, colores nuevos ni spinner global.
- Texto con `role="status"`/`aria-live="polite"`, sin robar foco; botones etiquetados, foco visible y teclado. Al cerrar, no dejar foco en un elemento eliminado.
- Descarte en la raíz: sobrevive a navegación, no a otra sesión. Abrir Store no significa instalar ni descartar.
- Traducir español/inglés. Pro, otras plataformas y Lite sin identidad no muestran acciones de Store.

Conservar `openExternalUrl` → `open_external_url`. Ambas validaciones actuales solo admiten HTTP(S): agregar igualdad exacta para `ms-windows-store://pdp/?ProductId=9N328K9L95JD`, restringida a Windows Lite en el backend. Rechazar otros Product ID, parámetros extra y esquemas arbitrarios; no relajar HTTP(S).

El lanzador actual usa `rundll32.exe`: un `spawn()` exitoso no confirma que la ficha se abrió. Verificarlo manualmente. Ante un error detectable de apertura, mantener el aviso y mostrar un mensaje breve junto a la acción; sin anunciar éxito ni inventar un fallback automático basado en una detección inexistente.

## 7. Implementación y verificación

### A. Backend completo

Agregar módulo, dependencia, registro y caché. Aislar WinRT con una función/closure inyectable donde las pruebas lo necesiten; no una jerarquía de traits/adaptadores. Usar SQLite temporal/en memoria y conservar regresiones para:

- Principal mezclado con opcionales, solo opcionales y colección vacía: decide la familia, no el orden.
- Caché positiva recupera el aviso antes de seis horas; el límite exacto permite consultar.
- Cambio de instalación, fecha futura y JSON corrupto no reutilizan un aviso viejo ni inhiben indefinidamente consultas.
- Fallo/timeout no renueva fecha; éxito sí; escritura fallida no pierde el resultado válido.
- Dos llamadas concurrentes, incluso si fallan, realizan una consulta; un resultado tardío tras timeout no altera estado/caché.
- Variante/plataforma no aplicable, ausencia reconocida de identidad y familia ajena no llegan a Store. Ejecutar las ramas en los targets correspondientes o declarar cuáles no se verificaron.

### B. Interfaz y enlace

Integrar el resultado, aviso y allowlist exacta. Ampliar las pruebas existentes para descarte entre rutas, nueva sesión con caché positiva, consulta pendiente/rechazada sin bloquear la app y rechazo de variantes inseguras de la URI en frontend/backend. Mantener HTTP(S).

Verificar visualmente temas, idiomas, teclado, foco y ancho mínimo admitido: Happy DOM no demuestra ausencia de overflow. Usar mocks existentes o un harness temporal de UI, no flags persistentes de producción. Eliminar el harness después. No agregar tests de formato de versiones eliminado, copias de campos, texto literal o snapshots extensos sin riesgo concreto.

### C. Comandos y evidencia

Aplicar formatter solo a archivos modificados. Desde la raíz, usar Lite explícito y restaurar el entorno PowerShell:

```powershell
$previousLocalMl = $env:VITE_LOCAL_ML
try {
  $env:VITE_LOCAL_ML = '0'
  pnpm lint
  pnpm typecheck
  pnpm test
} finally {
  $env:VITE_LOCAL_ML = $previousLocalMl
}
```

Desde `apps/desktop/src-tauri`:

```powershell
cargo fmt --check
cargo check --locked --no-default-features
cargo test --locked --no-default-features store_updates
cargo test --locked --no-default-features validate_external_url
```

Registrar cada resultado por separado; el último comando exitoso no prueba los anteriores y los filtros no deben pasar con cero tests. Verificar frontend Pro con `VITE_LOCAL_ML=1`, restaurando después el entorno. Comprobar Rust Pro y macOS/Linux en sus entornos/CI; no inferirlo del check Lite ni disparar una compilación MNN completa solo para auditar.

Desde `apps/desktop`, con `VITE_LOCAL_ML=0`, ejecutar `pnpm exec tauri dev --config src-tauri/tauri.lite.conf.json --no-default-features`: no debe aparecer un error invasivo ni aviso sin identidad. Verificar aparte UI simulada y apertura real de la URI; ninguna prueba equivale a detección real en Store.

Entregar archivos/cambios, dependencias/features, comandos/resultados, evidencia visual y limitaciones. Distinguir pruebas automatizadas, UI simulada, ejecución sin identidad y consulta real.

## 8. Aceptación y prueba Store posterior

El cambio local está listo para revisión cuando cumple el alcance Windows Lite, filtrado, caché, deduplicación, descarte, arranque no bloqueante y apertura restringida; pasan las verificaciones ejecutadas y se identifican las no ejecutadas. No declarar otros targets aprobados sin evidencia.

**La integración Store solo queda validada con una instalación distribuida por Store o un Package Flight autorizado.** Un MSIX firmado cualquiera, mocks o `tauri dev` no bastan.

En esa prueba: instalar una versión anterior desde el canal pertinente; ofrecer una superior en ese canal y esperar su disponibilidad; verificar detección/ficha; cerrar y reabrir para comprobar caché/descarte; actualizar desde Store y confirmar que no reaparece el aviso anterior. Considerar propagación y caché de Microsoft.

No publicar ni generar un nuevo MSIX sin autorización expresa. Sin ella, entregar la implementación local con la validación Store pendiente, no afirmar que la integración completa está probada.

## Referencias

- [Microsoft: GetAppAndOptionalStorePackageUpdatesAsync](https://learn.microsoft.com/en-us/uwp/api/windows.services.store.storecontext.getappandoptionalstorepackageupdatesasync): opcionales, hilo UI, HWND y límites de Store.
- [Microsoft: StorePackageUpdate.Package](https://learn.microsoft.com/en-us/uwp/api/windows.services.store.storepackageupdate.package): paquete que tiene una actualización; no garantiza versión de destino.
- [Microsoft: actualizaciones desde Store](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/package-updates-from-store): identidad y distribución por Store.
- [Microsoft: URI de Microsoft Store](https://learn.microsoft.com/en-us/windows/apps/develop/launch/launch-store-app): ficha por Product ID.
- [windows 0.61.3: features](https://docs.rs/crate/windows/0.61.3/features): nombres reales. Confirmar firmas/requisitos exactos contra el código del crate al implementar; esta auditoría no compiló un adaptador WinRT.
