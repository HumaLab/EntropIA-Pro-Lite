# 1.8.7 — findings from testing EntropIA Pro 1.8.6

Tested on the maintainer's Windows machine, Store build (`install=msix`), updated from 1.8.5.

**Status: closed on 2026-11-21.** Testing is finished: 29 items to fix (numbered 1–31; 22 and 25 unused), the improvements list, and the open checks below. Fixes have not started.

Not covered by this round: the Navegador page area (see «To verify»), the Lite variant, and sync between two devices (the Sync E2E suite covers that).

## Verified working

- Runtime cleanup end to end: after the 2026.11.10 runtime activated, `runtime/` went from 21 GB to 3.9 GB. Only `2026.11.10` remains, `.downloads` is empty, and no "Could not remove stale runtime" warning appears.
- The AI dependencies reinstalled automatically against the new runtime and finished without errors.

## To fix

1. **Sync write blocked at startup.** 38 s after launch, while the runtime downloads: `Error de sync: [sync] failed to write sync_meta['last_sync_at']: database is locked`. The sync engine retries, but it should not contend with startup writes.
2. **Dependencies reported complete before the runtime exists.** At startup, with the runtime still "pendiente de descarga", the log reads `Verificación completada: 5 dependencias, 0 faltantes, 0 fallidas`. Check what the Dependencias tab shows in that window; it must not report everything as installed.
3. **Second launch does not focus the open window.** Launching EntropIA while it is open shows the correct "EntropIA ya está abierta" notice, but the existing window stays behind. Bring it to the front.

4. **Embeddings counts disagree on Inicio.** "Estado del corpus" shows `Embeddings 1.377 / 1.378 · con embeddings · 100 %`, and right below `17 pendientes de embeddings`. One document is missing, yet it reads 100 % and 17 pending. The archive has exactly 1 asset with a pending embedding revision, so "17" counts something else; label it for what it counts, and round 99.9 % down.
5. **New OCR text reaches embeddings only after a delay, with no sign of it.** Right after OCRL on an image, the log shows only the FTS update and Chat says `Pasajes indexados en 1378 de 1379 documentos`. Some minutes later Inicio shows `1.379 / 1.379`, so the embedding does arrive. Show that the document is queued for embeddings, so a user asking the Chat right away knows why it is not found.
6. **GLM-OCR layout markup is stored, indexed and shown.** Chat source previews print `<div align="center"> # Los Fileteros: ... </div>` literally. The cause is upstream: the stored OCR text keeps GLM's layout markup. In the archive, 606 of 1,364 `extractions` rows contain `<div`, 546 contain image references like `![](page=0,bbox=[27, 43, 325, 364])`, and 866 of 7,860 `rag_chunks` contain `<div`. That markup goes into FTS and embeddings, and into the previews. Keep the rich text for display, but derive a plain-text form (no HTML, no image/bbox references, no Markdown markers) for indexing, chunking and previews; re-chunk existing rows.
7. **Help tooltip names a renamed option.** The «Mis documentos» tooltip says `Marcá «Usar OpenRouter» para esta pregunta`, but since 1.8.5 the checkbox is labelled **OpenRouter**.
8. **Status badges are in English and do not reflect stored work.** The extraction card shows `idle`, `running`, `done` (the STT card `Idle`) in an otherwise Spanish UI. Worse, in a document's Análisis tab `INDEX`, `EMBED`, `NER` and `TRIPLET` all read `idle` for Voces › SOIP1, which has an embedding (1 row in `vec_assets`, 27 `rag_chunks`). The badges show this session's job state, so `idle` reads as "not done". Derive them from the stored state (done / pending / not run) and translate them.
9. **Vision models installed but unusable without NVIDIA.** On this AMD machine, Configuración › Modelos locales shows `Modelos GLM-OCR instalados` and `Modelos Qwen instalados` (about 4 GB) next to `Falta llama-server: se descarga solo en equipos con GPU NVIDIA`. No way to delete them from there. With #373 (Vulkan) the models become usable; the card text then needs updating.
10. **Second instance draws an empty window.** Besides not focusing the open window (item 3), the refused second launch paints a large empty black window behind the notice, and a minimized original stays minimized.
11. **Help button covers content.** The floating help button sits over the Chat input, the last line of Modelos locales cards and the llama-server row in Dependencias de IA.
12. **OCR log mislabels the engine.** An OCRL (light) run logs `[OCRH] PaddleOCR-VL usará Python del runtime administrado`, though it ran plain Paddle.

13. **CRITICAL — Parakeet translates Spanish audio into English.** Re-transcribing a 5:58 Spanish interview (Voces › `HACIENDO SOCIALES MICRO 2 MES 5 FINAL.mp3`) with Parakeet `tdt-0.6b-v3` on 1.8.6: the first sentence is Spanish, then the output switches to English for most of the text (`The control obrero of the production, since the empresarial patron burguese, has numerous references historically...`), and back to Spanish near the end. The record is tagged `language: es`. 4,055 characters and 3 segments, against 4,446 in the AssemblyAI transcript it replaced. Force the Spanish language/prompt in the Parakeet decoder, or detect the language drift and fall back; add a regression test on a Spanish sample.
    - **Reproduced on a second audio.** A copy of `HACIENDO SOCIALES MICRO 1 MES 5 FINAL.mp3` (4:11), imported into a test collection, gives 81 common English words against 61 Spanish: `Los procesos of recuperation of empresas for visibilization in Argentina during the década de los 90…`. The original's AssemblyAI transcript is all Spanish.
    - **Root cause: the language setting never reaches the model.** `ParakeetEngine` stores `language` (`transcription/parakeet.rs:119`), but `transcribe_samples(chunk, SAMPLE_RATE, 1, Some(mode))` (`parakeet.rs:194`) takes no language. The value is only copied into the result (`parakeet.rs:234`), so `es` is a label, not a constraint. Parakeet TDT v3 picks the language per window on its own. Since it cannot be forced, the fix is a drift check (share of English stop-words per segment), with a fallback to Whisper with `language=es` for the drifting windows or the whole file. Pin it with a test on a Spanish sample.
14. **Re-transcribing replaces the existing transcript without warning or history.** Pressing STT on an audio that already has a transcript starts at once and overwrites it, including speaker names renamed by hand (`Entrevistador` / `Entrevistado`). Then the embeddings are rebuilt from the new text. Ask for confirmation when a transcript exists, and keep the previous version so it can be restored. (During this test the AssemblyAI transcript was lost this way; it survived only in the local backup `backup-pre-1.5.0-20261110`, saved to `.tmp/transcript-68fbf48c-assemblyai.json`.)

15. **CRITICAL — OCRL silently drops a whole line.** A clean, printed test image of five lines (Times 40 pt, black on white, 1400×700) imported into `prueba` and run through OCRL (Paddle, 9 s) stores only four. Line 3, `convoco a una asamblea el 12 de marzo de 1965`, is missing from `extractions.text_content`, and nothing warns. Repro image: `S:\Descargas\entropia-import-test\prueba-importacion.png` (asset `d6c0fb14-11e7-4865-960e-e5237307e1bb`). Check whether detection misses the line, a confidence filter drops it (it is the only line with numbers), or region merging loses it; add this image as a regression fixture. Narrowed down: the document's Layout tab lists `paddle · 4 bloques · 4 regiones`, and the overlay draws boxes around lines 1, 2, 4 and 5 only. Line 3 never gets a region, so detection misses it; no later filter drops it.
16. **The help recommends OCRH without saying it needs a GPU.** Asked "¿Cómo hago OCR de un documento?", the help says `Pulsá OCRH y esperá a que termine`. On a machine without an NVIDIA GPU, OCRH then warns it takes about 15 minutes per page. The help should say OCRL is the fast option without a GPU.
17. **Collection header undercounts embeddings.** FACA shows `156 documentos · 9 OCR · 0 Embed`, but the archive has 9 FACA items in `vec_assets` (8 with embedded `rag_chunks`). Other collections agree (`prueba`: 4 OCR · 4 Embed; Movimiento Obrero: 13 · 13). Find which source the chip reads and why FACA's embeddings fall outside it.
18. **Mantenimiento finds 34 images to re-run but offers nothing.** "Revisar versiones de imágenes" reports `La imagen es más nueva que su layout: hay que volver a correr el layout — 34`, then `No hay imágenes para reparar` and no action. Either offer to re-run those 34 layouts or say how.
19. **Batch analysis never shows its result.** In Lotes, selecting `prueba` and pressing "Analizar selección" stays on `Analizando selección… 7/7` with no summary. The draft batch in the archive is complete: 2 admitted for OCR, 4 already done, 1 PDF container skipped. Show that breakdown. The draft also counts in the status bar as `Procesando 1`, and `7/7` counts the PDF container on top of the 6 documents the collection card shows.
20. **A discarded draft batch lands in Historial.** "Descartar borrador" leaves a `Cancelado` row with 0 documents in the batch history; an older one from 11/11 shows the same. Drop discarded drafts from the history, or label them as drafts.
21. **"Sincronizar ahora" gives no feedback.** The sync runs (`sync_meta.last_sync_at` matches the click), but the panel shows nothing: no spinner, no result, no updated time.
23. **Global search sorts its hits by title, not relevance.** `ItemRepo.searchGlobal` (`packages/store/src/repos/item.repo.ts:1656`) takes FTS's top results and re-reads them `orderBy(asc(items.title))`, discarding the BM25 order. For `petitorio patronal`, the freshly imported test PDF ranks first by BM25 (−13.75), but the dropdown lists a run of `DSC0…` images first, alphabetically. This also explains why `Huelga Filet` listed the exact-title match fourth. Keep the FTS rank order.
24. **PDF pages get no OCRL button.** On an image the Texto tab offers OCRL, OCRH, OCRC and OCRR; on a PDF page only OCRH is active (OCRL is image-only by design, `ItemView.test.ts:6013`). Without an NVIDIA GPU the user must click OCRH and read the warning to discover "Usar OCRL", which then works (`pdf_ocr`, a 4-line page read perfectly). Offer OCRL on PDF pages directly.
26. **Deleted manuscripts cannot be recovered.** Deleting a manuscript says `Su contenido, sus versiones y su historial se conservan`, and the row is kept as `status = trashed` (`lib/writing.ts:499`), but no screen lists or restores trashed manuscripts. This archive holds 18. Add a trash view with restore, or say plainly that the document disappears.
27. **Alias suggestions pick a poor canonical name.** Suggestions read `ex Dodero ← Dodero`, `NESTOR RAMIREZ ← Néstor Ramírez, Ramirez`, `CAYO AYALA ← AYALA`. Prefer the accented, mixed-case form without prefixes, and let the user pick the canonical name when accepting.
28. **Motores settings are lost without a warning when not saved.** Picking OpenRouter under Modo LLM marks it as selected. Leaving the tab without pressing the save button drops the choice silently: `llm_mode` stays `local`, and the next Chat question runs on local Gemma. Save on selection, or block leaving with unsaved changes and mark the pending state.
29. **AssemblyAI transcripts lose their timestamps and speakers.** `transcription/assemblyai/mod.rs:237-246` stores the whole result as one segment, `start: 0.0` to the end with `speaker: None`. The utterances AssemblyAI returns, with their times and speakers, are dropped. A test audio of 3,386 characters came back as a single segment. Map each utterance to a segment.
30. **Investigación citation offsets do not match the source text.** In a fresh report, quote [4] reads `chars 2154–2228`, but that item's extracted text is 2,220 characters long. The «Fuentes citadas» list shows other ranges for the same sources (`[4] chars 2100–2220`). Make both use one offset base and keep them inside the text.
31. **Deleting an investigation leaves the confirm dialog open.** After «Borrar», the investigation leaves the list, but the dialog stays on screen with its «Borrar» button active. The dialog also does not name the investigation it deletes. Close it on success, and name the investigation.
32. **A deleted investigation leaves a research outbox entry that never syncs.** Minutes after deleting the test investigation, every sync cycle logs `research push stays pending (UnprovableOutbox) for job-37104-0: research outbox entries have unprovable snapshots or report files; they stay pending` (`sync/research_cycle.rs:988`). A delete should leave a `D` tombstone the push can send, not an upsert whose files are gone.

## Scope of 1.8.7 (decided 2026-11-21)

Fixed in 1.8.7: 6 (FTS and chat previews only), 7, 8, 12, 13, 14 (with a transcript history), 15, 16, 21, 23, 24, 28, 29, 31. Commits are listed in `odd/tasks/release-1.8.7-fixes.md`.

Deferred to 1.8.8:

- 1–5, 9, 10, 17–20, 26, 27, 30 and 32 from the list above.
- 6, the rest: re-chunk the existing `rag_chunks` and re-embed them from plain text. This needs a bump of the chunking contract and a slow background sweep.
- 11: the floating help button covers content; decide where it sits (the status bar is one option, at the cost of dragging).
- The «Improvements» list below.
- Advisories from the transcript-history review: restoring can hit `SQLITE_BUSY_SNAPSHOT` when a worker commits at the same moment (it rolls back cleanly and the user retries), and `transcription_versions` has no size cap.

## Improvements

- **Exports finish without a notice.** Saving the extracted text as Word writes a correct file (17 KB, 68 paragraphs, no markup) but the app shows nothing after the dialog closes.
- **Actividad reciente misses OCR and transcription work.** After running OCR and STT on two documents today, Inicio › Actividad reciente still lists documents from two days ago.

- **Rejoin hyphenated line breaks.** OCRL text keeps newspaper hyphenation (`aconteci-` / `miento`, `perma-` / `necer`), so a search for `acontecimiento` misses it. Join `-\n` + lowercase before indexing, keep the original for display.
- **Writing › Corpus search shows titles only.** Results for `fileteros` list document names with no snippet, so choosing what to cite means opening each one.
- **Library title sort ignores nothing.** Sorting Biblioteca by title puts every title that starts with a quote mark first (`"As a Scab"`, `"cultura woke"`). Ignore leading punctuation when sorting.
- **Document FTS search shows file names and raw ranks.** In a document's Búsquedas tab, `asamblea` returns rows like `DSC02448 — rank -6.672`: camera file names, a negative BM25 score, no snippet, no collection. Show title, collection and a snippet; hide the raw rank or turn it into a relevance label.
- **Word cloud keeps spoken-Spanish fillers.** On Voces (interviews) the top words include `qué`, `cómo`, `viste`, `creo`, `tenés`, `mirá`, `cosas`. Add oral fillers to the stop-word list, or offer a toggle for oral sources.
- **Collocations list the word itself.** "Una palabra en contexto" for `huelga` lists `huelga` among its associated words (`6 veces cerca`). Exclude the node word.
- **"Incluir bibliografía" ignores corpus citations.** The bibliography is built only from Zotero clusters (`lib/writing-export.ts:136`). A manuscript citing only corpus documents exports with the box ticked and no bibliography. Label it as Zotero works, or add a list of cited corpus sources.
- **A corpus footnote carries only the document title.** The exported note reads `1965-04-19 - lunes - STIA delegados planta  levantan huelga filet` (with the title's double space), with no collection or archive. Add the collection and, for PDFs, the page.
- **A corpus quote looks like plain text in the editor.** The inserted `«…»` has no marker for its source, so it reads like typed text; the footnote appears only on export.
- **A PDF has no "OCR all pages" in the document view.** The Texto tab runs one page at a time; the whole document needs Lotes. Add a document-level action.
- **The side panel resets to Notas on page change.** With Texto open on page 1 of a PDF, moving to page 2 switches the panel back to Notas. Keep the open tab.
- **OCR line breaks differ by path.** The same PDF read by a manual run stores lines separated by blank lines (`\n\n`); the page read by a batch stores single `\n`.
- **Empty collection has no import button.** It says `Importá archivos para empezar a trabajar`, but the import action is an unlabeled icon in the header.
- **Creating a collection with no name does nothing.** "Crear colección" with an empty name shows no message and is not disabled. Without a description, the card repeats the name as its subtitle.
- **Delete confirmation is an icon.** "Eliminar colección" confirms with a trash icon and no word; a destructive confirm should say `Eliminar`.
- **The breadcrumb shows the internal page file.** A PDF page reads `prueba-pdf-2paginas_page_1.pdf`; show `prueba-pdf-2paginas · página 1`.
- **Backup folder shows the `\\?\` prefix.** Configuración › Copias displays `\\?\E:\EntropIA-BK`. Strip the extended-length prefix for display.
- **Collection cards cut the relative date.** Cards read `hace 5 m…`, `hace 1…`, losing the unit. Give the date its own line or shorten the format.
- **Lotes collection grid hides its scroll.** The grid scrolls on its own, and its visible area ends exactly on a row boundary. Five of the 29 collections are hidden, with nothing showing that more exist. Show a scrollbar or a partial row.
- **Logs tab does not refresh and hides its last line.** New lines appear only after "Refrescar", and the help button covers the last row (see item 11).
- **Local chat is slow on CPU.** A corpus question took about 2.5 minutes with Gemma local and logged `Failed to apply model chat template: ffi error -1; using the Gemma 4 turn format` twice. Check the template error; show progress instead of a static "Buscando en la base de conocimiento…".
- **Investigación scope "select all" shows a stale state.** With all 30 collections selected, the toggle still offered «select all», and the first click did nothing visible. Derive its state from the current selection.
- **The Investigación header does not show the scope.** The header shows `Corpus` and the model, but not which collections the run covers.
- **«Responder y seguir» with every question empty does nothing.** The form says the questions may be left unanswered, but with all four empty the button gives no response. Answering one makes it continue.
- **The report repeats the «Limitaciones» heading.** One report showed a «Limitaciones» paragraph followed at once by a «Limitaciones» list. Merge them.

## Verified in 1.8.6

- Help panel: the four options (OpenRouter, Diagnóstico, Ver input, Mis documentos) are visible; a local answer arrives in seconds with sources.
- Dependencias de IA: all green, pointing at runtime `2026.11.10`; the llama-server row reads "No aplica en este equipo" with the reason (#374).
- Global search: instant results; opening one shows the document.
- OCRL on a newspaper image: 12,793 characters in 29 s via Paddle on the new runtime.
- Chat on the corpus: correct answer with three cited sources.
- Investigación: an existing report renders with its coverage table and agreed framing.
- Escritura: an existing manuscript opens, the editor and toolbar render, corpus search returns results.
- Biblioteca: Zotero works list loads.
- Base de datos: read-only table viewer with paging works.
- Export: extracted text to Word (`.docx`) is valid, keeps line breaks as `w:br`, and carries no markup.
- Import: a PNG into an existing collection lands with its thumbnail.
- OCRH without a GPU warns about ~15 minutes per page and offers OCRL instead.
- Copias: backup folder, weekly schedule, keep 2, last copy listed.
- Sincronización: devices, server usage (7.1 of 100 GB) and the danger zone render; a manual sync runs.
- Apariencia: switching to Claro and back to Oscuro applies at once.
- Split view: Configuración and Inicio side by side.
- Notes: a note on a document saves and lists with its date.
- Mapa: OpenStreetMap tiles load and the NER place (Mar del Plata) gets its pin.
- Búsquedas › Páginas similares: relevant neighbours (1965 fish-filleting strike documents).
- Layout tab with overlay draws the detected regions on the image.
- Collection Análisis textual: word cloud and top-20 chart, frequent phrases (`mar del plata`, `peronismo de base`), keyword in context with G² collocations and a word tree, 3D relations graph (300 names, 280 relations, the hidden count explained). Collection export menu offers txt, md, pdf, docx and csv.
- Audio import: several files at once; a file already in the collection is skipped and reported (`Importados 1 · Omitidos 1 · Errores 0`). Local STT of a 4:11 MP3 in under a minute, with two speakers labelled.
- Escritura: new manuscript, title, heading, outline, autosave (`Guardado`); corpus search, preview with highlights, a selected fragment inserted as a quote and stored as a `documentCitation` with offsets and hash (`integrity_status: valid`); DOCX export with a real footnote, and the app reports where it saved.
- Collection lifecycle: create, import a 2-page PDF (split into pages, opens on page 1), OCRL via `pdf_ocr` (exact text), a batch with OCR + embeddings on the rest (about 30 s, no errors), FTS indexed, then delete. The delete removes items, assets, extractions, vectors, chunks, the FTS row and the files on disk; only the batch history keeps its snapshot.
- Chat with OpenRouter (once saved): a cautious answer in under 30 s, with sources cited.
- AssemblyAI STT on a copy of a Spanish interview: correct Spanish text, 3,386 characters, in about 11 s (see item 29 for the segments).
- Investigación, new run with OpenRouter on one collection: research design, clarifying questions, the search plan shown before spending, then the report. The coverage table matches the archive (7 items, 2 with text, 9 chunks), and the report states its limits instead of inventing content.

## To verify

- Navegador: loading `es.wikipedia.org/wiki/Mar_del_Plata` set the tab title, but the page area stayed black in a screen capture. The child webview may simply not be captured; the maintainer should look at it directly.

- Parakeet diarization: an earlier log (1.8.2) has `Parakeet diarization failed: ONNX Runtime error: Array has a non-contiguous layout`, with fallback to faster-whisper (no speakers). On 1.8.6 diarization ran without that error (two speakers), so ort rc.15 (#354) seems to fix it; confirm on more audios.
- Old Lite build on the shared archive: the log has a launch of `variant=lite version=1.8.1 install=msix` on 2026-11-21 against the same `com.entropia.shared` archive the Pro 1.8.6 uses. Check that the Store Lite updates, and what a 1.8.1 build does with a newer schema.
- Stale settings: after the cleanup, `deps` logs `Forgot missing Python path ... runtime\2026.11.9\...` three times. That is expected self-healing, but the settings pointed at a deleted runtime until then.

## Carried over

- PR #373 (local GLM-OCR and Qwen on AMD, Intel and Apple GPUs via Vulkan) merged into main as 63b777d5. The GPU check now accepts NVIDIA, or an AMD, Intel or Apple GPU with 4 GB of video memory for GLM-OCR and 6 GB for Qwen. Re-check item 9 on this machine with a 1.8.7 build.
- PR #375 (Windows CI job timeout from 90 to 120 min).
