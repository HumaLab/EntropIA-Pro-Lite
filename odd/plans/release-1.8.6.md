# Release 1.8.6

## Part 1 — Release notes (draft for the GitHub release body)

## Español

Esta versión corrige dos problemas que seguían en **EntropIA Pro 1.8.5**: los runtimes de IA viejos que quedaban ocupando espacio y la opción **Mis documentos**, que no se veía en el panel de ayuda. También evita volver a descargar el runtime en las próximas versiones y mejora la transcripción con hablantes y las fuentes de las respuestas.

No agrega migraciones: se puede actualizar en cualquier orden entre equipos con 1.8.5.

### Pro y Lite

- **Panel de ayuda.** Todas las opciones debajo de la pregunta se ven siempre. En el panel chico pasan a una segunda línea; con el panel maximizado entran en una sola. Antes, en el panel chico **Mis documentos** quedaba oculta detrás de un desplazamiento lateral.

### Pro

- **Runtimes viejos.** Cuando el runtime nuevo queda listo, la app aparta los runtimes que ya no usa y los borra. Si un programa todavía usa uno, lo deja intacto y lo borra en un arranque posterior. También borra lo que dejó un borrado interrumpido en 1.8.5 (hasta 2 GB) y el archivo descargado de cada runtime instalado, que ocupaba unos 2,3 GB por versión y no se usaba.
- **Runtime en uso.** La app ya no carga partes de un runtime viejo cuando hay uno de su versión. Esto era lo que impedía borrarlo.
- **llama-server en Windows.** La descarga de llama-server, que necesitan GLM-OCR y el corrector Qwen locales, fallaba siempre al final con «Acceso denegado (os error 5)». Ahora se instala.
- **Sin volver a descargar el runtime.** A partir del runtime de esta versión, una actualización que no cambia el runtime lo sigue usando, sin descargar varios GB otra vez. Esta actualización todavía descarga uno nuevo.
- **Hablantes en todas las palabras.** En una transcripción con identificación de hablantes, las palabras que caían entre dos turnos quedaban sin hablante. Ahora toman el turno más cercano.
- **Puntuación en las transcripciones.** Las comas y los puntos van pegados a la palabra anterior («bueno, la» en lugar de «bueno , la»).
- **Fuentes de las respuestas.** Cuando un modelo local cita varias fuentes juntas, como [1, 2, 3] o [4-6], la respuesta ahora las muestra. Antes quedaba sin fuentes.

## English

This release fixes two problems still present in **EntropIA Pro 1.8.5**: old AI runtimes that kept taking up space, and the **My documents** option, which was hidden in the help panel. It also stops future releases from downloading the runtime again, and improves transcription with speakers and the sources of answers.

It adds no migrations: devices on 1.8.5 can update in any order.

### Pro and Lite

- **Help panel.** Every option below the question is always visible. In the small panel they wrap to a second line; with the panel maximized they fit on one. Before, in the small panel **My documents** was hidden behind a sideways scroll.

### Pro

- **Old runtimes.** Once the new runtime is ready, the app sets aside the runtimes it no longer uses and deletes them. If a program still uses one, the app leaves it whole and deletes it on a later start. It also deletes what an interrupted deletion left behind in 1.8.5 (up to 2 GB) and the downloaded archive of every installed runtime, about 2.3 GB per version that nothing used.
- **Runtime in use.** The app no longer loads parts of an old runtime when one for its own version is present. That was what kept the old one from being deleted.
- **llama-server on Windows.** The llama-server download, which local GLM-OCR and the local Qwen corrector need, always failed at the end with "Access denied (os error 5)". It now installs.
- **No more runtime re-downloads.** From this version's runtime on, an update that leaves the runtime unchanged keeps using it instead of downloading several GB again. This update still downloads a new one.
- **Speakers on every word.** In a transcription with speaker labels, words that fell between two turns had no speaker. They now take the nearest turn.
- **Punctuation in transcriptions.** Commas and periods attach to the word before them ("bueno, la" instead of "bueno , la").
- **Sources of answers.** When a local model cites several sources together, such as [1, 2, 3] or [4-6], the answer now lists them. Before, it showed no sources.

---

## Part 2 — Release checklist

- [x] Version bump — `pnpm version:bump 1.8.6`; `pnpm version:check` passes.
- [ ] PR to main, CI green, merge.
- [ ] On main: Build Runtime Pack for **windows-x86_64** and **linux-x86_64** (`app_version 1.8.6`, new `pack_version 2026.11.10`, `native_source_release_tag runtime-source-0.1.0`). Needed this time even with runtime reuse (#369): the installed `2026.11.9` predates `runtime_abi`, so it only serves 1.8.5. From `2026.11.10` on, a release that leaves the runtime unchanged can reuse it.
- [ ] Publish the runtime bootstrap for **app_version 1.8.6** for both platforms (`publish-runtime-bootstrap.yml`: `platform`, `app_version 1.8.6`, `pack_version 2026.11.10`, `runtime_pack_run_id`, `release_tag runtime-bootstrap`) — before tagging.
- [ ] Push tag `v1.8.6` on main (Pro installers + Lite MSIX + draft release).
- [ ] Release draft: paste Part 1 as the body, publish.
- [ ] Build the Pro Store MSIX 1.8.6.0 (`build-pro-store-msix.ps1`).
- [ ] End-to-end check on a machine with 1.8.5 (runtime `2026.11.9` and the orphan `2026.11.8`): after updating, only `2026.11.10` remains under `runtime/`, `runtime/.downloads` holds no finished archive (today 2026.11.4 to 2026.11.9, about 14 GB), and the help panel shows all four options.
- [ ] Submit both MSIX packages in Partner Center.
