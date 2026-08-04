# Política de firma de código de EntropIA

**English:** [CODE_SIGNING.en.md](./CODE_SIGNING.en.md)

La firma de instaladores no está integrada en el repositorio. Los instaladores y paquetes producidos actualmente pueden estar sin firmar; la firma Ed25519 del bootstrap remoto de Pro protege otro límite de confianza y no constituye firma Authenticode del instalador.

## Estado actual

| Área | Pro | Lite | Estado |
| ---- | --- | ---- | ------ |
| Windows NSIS/MSI | Sí | Sí | No hay proveedor ni paso Authenticode integrado; pueden estar sin firmar. |
| Microsoft Store MSIX | No | Sí | CI lo produce deliberadamente sin firma; se espera que Microsoft Store lo firme después del envío. |
| Linux DEB | Sí | No | `Release` puede construirlo; no hay firma de paquetes integrada. |
| macOS arm64 | Sí, mediante ejecución manual con `release_platform=all` | No | No hay firma de código ni notarización integrada. |
| Bootstrap remoto del runtime | Sí, para instaladores lean | No | Manifest/catálogo firmado con Ed25519 y archivo validado con SHA-256. |
| GitHub Release | `Release` crea un draft con los assets de Pro aplicables. | NSIS/MSI se adjuntan en tags; el MSIX queda como artefacto de Actions. | La publicación del draft es manual y externa al workflow. |
| Controles de cadena de suministro | Ambas variantes | Ambas variantes | No hay attestation de procedencia, SBOM generado, checksums de instalador publicados ni aprobación manual de firma exigida. |

## Límites de confianza

### 1. Firma de instaladores y paquetes

No existe una integración para Authenticode, firma de paquetes Linux ni firma/notarización de Apple. Tampoco se ha seleccionado formalmente un proveedor de firma. Hasta integrar y aprobar esos controles, ningún asset debe presentarse como instalador firmado por el proyecto.

### 2. Firma e integridad del bootstrap remoto de Pro

`Publish Runtime Bootstrap` publica un manifest/catálogo firmado con Ed25519 y el digest SHA-256 del archivo de runtime. La aplicación verifica la firma con la clave pública incorporada y exige una entrada cuyo `app_version` y plataforma coincidan exactamente antes de descargar y validar el archivo.

Estos controles protegen el runtime remoto, no el instalador. Las condiciones `payload_profile=release`, `release_injection_required=false` y `external_artifacts_required` vacío describen únicamente un payload de release autocontenido; no son condiciones generales de firma para los instaladores lean actuales ni para Lite.

### 3. Draft de GitHub Release y publicación manual

En un tag, `Release` crea la release de Pro como draft y adjunta los assets aplicables. Los instaladores NSIS/MSI de Lite también se adjuntan en tags; el MSIX permanece como artefacto separado para Microsoft Store. Publicar el draft es una acción manual fuera del workflow y no equivale a una aprobación de firma exigida por CI.

## Camino actual de release

Este es el orden operativo para una release Pro lean; no es una dependencia automática entre todos los workflows:

1. Ejecutar `Build Runtime Pack` para la versión de aplicación prevista. Actualmente construye el pack `windows-x86_64`, produce los artefactos `runtime-archive` y `runtime-payloads` y ejecuta el smoke test del runtime-pack.
2. Ejecutar `Publish Runtime Bootstrap` con `runtime_pack_run_id` de esa corrida. Consume `runtime-archive`, publica el archivo o sus partes y actualiza el manifest/catálogo firmado.
3. Verificar obligatoriamente que exista una entrada compatible y activa del catálogo firmado para cada artefacto Pro lean, con coincidencia exacta de `app_version` y plataforma.
4. Disparar `Release` mediante un tag o manualmente. Sus únicos inputs manuales son `release_platform` y `lite_only`; no recibe `runtime_payload_artifact` ni `runtime_payload_run_id`.
5. Revisar el draft y publicarlo manualmente cuando correspondan los artefactos y controles aplicables.

`Release` incorpora intencionalmente fixture runtime-packs en los instaladores Pro lean y compila la URL del manifest bootstrap y su clave pública. La construcción de instaladores no está condicionada al resultado del workflow de smoke; por eso la verificación del catálogo activo es un control previo obligatorio.

## Checklist previo a publicar

- [ ] Registrar el tag y commit exactos y la corrida de `Release`.
- [ ] Para Pro lean, registrar las corridas de `Build Runtime Pack` y `Publish Runtime Bootstrap`, la entrada compatible del manifest/catálogo firmado y el digest SHA-256 del archivo de runtime.
- [ ] Confirmar la coincidencia exacta de `app_version` y plataforma contra el catálogo firmado activo, sin depender de contenidos efímeros documentados aquí.
- [ ] Revisar los assets del draft y confirmar qué formatos y plataformas están sin firmar.
- [ ] Completar la revisión aplicable de licencias, avisos de terceros y componentes redistribuibles; el gate actual de avisos/SBOM/licencias aún es incompleto.
- [ ] Antes de cualquier firma futura, generar y revisar el digest del instalador exacto. Actualmente no se publica un checksum explícito del instalador ni procedencia equivalente en los assets o notas de release.

## Requisitos antes de integrar firma de instaladores

- Seleccionar formalmente un proveedor y definir identidades, custodia de claves, permisos y política de aprobación.
- Firmar solo artefactos de CI trazables al tag y commit revisados; no firmar builds locales ad hoc.
- Mantener certificados, claves y tokens fuera del repositorio y aplicar rotación y mínimo privilegio.
- Agregar verificación posterior a la firma y publicación explícita de checksums y procedencia.
- Completar el gate de licencias, avisos de terceros y SBOM para los componentes realmente distribuidos.
- Definir controles específicos por plataforma antes de declarar soporte: Authenticode para Windows, firma de paquetes para Linux y firma/notarización para macOS.

## Procedencia requerida

Una futura firma de instalador debe poder vincularse con:

- el tag y commit exactos;
- la corrida de `Release` que produjo el asset;
- para Pro lean, las corridas de `Build Runtime Pack` y `Publish Runtime Bootstrap`;
- el manifest/catálogo firmado, su entrada exacta de `app_version` y plataforma, y el digest del archivo de runtime;
- el digest generado y revisado del instalador antes de firmarlo.

El repositorio todavía no genera ni publica el último digest como asset o checksum de notas de release; los digests expuestos por la API de GitHub no sustituyen esa publicación explícita ni una attestation de procedencia.

## Respuesta ante incidentes

Si se sospecha que un futuro instalador firmado o un manifest/runtime firmado está comprometido:

1. Detener la publicación o retirar los assets afectados y advertir en la GitHub Release.
2. Identificar tag, commit, corridas, entradas de catálogo y digests afectados.
3. Publicar las versiones, plataformas y digests afectados con instrucciones de mitigación.
4. Revocar o rotar las credenciales correspondientes; para la clave del runtime, coordinar el nuevo manifest con la clave pública incorporada en las aplicaciones compatibles.
5. Reconstruir desde corridas limpias, repetir las verificaciones y publicar artefactos corregidos.
