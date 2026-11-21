# EntropIA Pro and Lite feature inventory

**EntropIA version:** 1.7.1 · **Updated:** 14 November 2026 · **Español:** [Inventario de funciones](inventario-funciones-manual.md)

**Companion document:** [User manual](manual-usuario.en.md)

## Scope

This list gathers the features of both EntropIA variants —**Lite** and **Pro**— and says which manual chapter explains each one. Almost all are common; when one changes between variants, the middle column says so ("In Lite…", "In Pro…"), and the engine differences are gathered in the **Differences between Lite and Pro** section below and explained in [chapter 2 of the manual](manual-usuario.en.md#lite-y-pro). We state whether a feature needs an Internet connection, an account or a key. Features marked **(next version)** are already in the manual but arrive in the version after 1.7.1.

The manual combines real screenshots of EntropIA Lite, details cropped from those screenshots and process diagrams labelled as such. Each image has a description and can be opened full size from the web edition. The screenshots show the Spanish interface.

**How to read it:**

- **Available:** you can use this option in both variants.
- **Pro only / Lite only:** the option exists in only one variant.
- **Needs a condition:** it requires the Internet, an account, a key, a local model or recognised text; the table says which.
- **Not available:** there is no option to do this task in any current variant.

## General navigation and preferences

| Visible feature | Availability / scope | Manual section |
|---|---|---|
| **Home → Workspace**: Continue, Corpus status, Quick access and Recent activity | Available. Indicators show archive processing, not quality or topical coverage. **View batch** opens the active process when there is one. | [Ch. 1](manual-usuario.en.md#capitulo-1-inicio-rapido) |
| **Home → Import**, **Retrieve**, **Write** | Import lets you choose or create a collection, or bring files **From Internet Archive**; Retrieve starts a new Chat, it does not restore files; Write creates or reuses an empty manuscript. | [Ch. 1](manual-usuario.en.md#capitulo-1-inicio-rapido) |
| **Collections**; name filter | Available from Home or the top bar; the filter helps find collections but does not change their order. | [Ch. 1](manual-usuario.en.md#capitulo-1-inicio-rapido), [ch. 4](manual-usuario.en.md#capitulo-4-crear-y-organizar-un-corpus) |
| Top-bar shortcuts: **Home**, **Collections**, **Library**, **Browser**, **Research chat**, **Research agent**, **Writing**, **Split view**, **Database**, **Settings** | Available; the views are called **Investigate** and **DB Browser**. | [Ch. 1](manual-usuario.en.md#capitulo-1-inicio-rapido), [ch. 4](manual-usuario.en.md#navegador), [ch. 10](manual-usuario.en.md#capitulo-10-investigacion), [ch. 13](manual-usuario.en.md#capitulo-13-chat-y-agentes-de-entropia) |
| Work tabs, up to 4; open, choose and close | Available. Each tab keeps its route. The last one cannot be closed. | [Ch. 1](manual-usuario.en.md#pestanas-y-vista-dividida) |
| **Split view** | Available with enough width: each pane needs 640 px and at least 40 % of the space. When narrowed, only the active tab stays visible; they do not stack. | [Ch. 1](manual-usuario.en.md#pestanas-y-vista-dividida) |
| Explorer in split view | Available as a panel over the active area; it starts closed. In a single pane it stays on the left and **Ctrl+B** hides it. | [Ch. 1](manual-usuario.en.md#pestanas-y-vista-dividida), [ch. 4](manual-usuario.en.md#capitulo-4-crear-y-organizar-un-corpus) |
| Each tab's route; previous/next document; **Delete active page** | Available on that tab's route. Deleting a cited page or file may leave the citation unable to open the source, and removes it from the other tabs. | [Ch. 5](manual-usuario.en.md#capitulo-5-trabajar-con-documentos-y-assets) |
| Top-bar search | Up to 20 documents by title, data or indexed text; may include spelling variants. Below, a **Library** group with up to 5 synced Zotero works. It does not answer topical questions with AI. | [Ch. 1](manual-usuario.en.md#capitulo-1-inicio-rapido), [ch. 7](manual-usuario.en.md#capitulo-7-buscar-informacion) |
| **Settings → Appearance**: themes **Oscuro**, **Cálido**, **Claro**, **Lite**, **Papel**, **Bosque**, **Vibrante**; soft/normal/high contrast | Available. **Lite** is also the name of a visual theme; it does not change the installed variant. They are no longer set on the top bar. | [Ch. 16](manual-usuario.en.md#capitulo-16-configuracion-y-herramientas-generales) |
| **Appearance → Zoom** of the interface from 75 to 125 %; Ctrl+/Ctrl−/Ctrl0 shortcuts | Available. Independent from the document viewer's zoom. | [Ch. 16](manual-usuario.en.md#capitulo-16-configuracion-y-herramientas-generales) |
| **Appearance → Typography**: **Academic**, **Modern**, **Editorial**, **Archive**; Español/English language | Available. The language changes the interface texts, not the documents. | [Ch. 16](manual-usuario.en.md#capitulo-16-configuracion-y-herramientas-generales) |
| Version, GitHub/HLab links, sync and batch state, notifications | Available; some indications depend on having signed in. | [Ch. 15](manual-usuario.en.md#capitulo-15-sincronizacion-y-nube), [ch. 16](manual-usuario.en.md#capitulo-16-configuracion-y-herramientas-generales) |
| **View update** notice | May open the Microsoft Store page; it does not install the update. | [Ch. 16](manual-usuario.en.md#capitulo-16-configuracion-y-herramientas-generales) |
| **Help bubble** above the status bar: help chat with local frequent questions, optional diagnostics, optional search in your documents, actions with approval, lookups in the repository and the HLab page, local history **(next version)** | Frequent questions, first steps, greetings and the time are answered on the computer, without a key. The rest uses the OpenRouter key and is charged. Diagnostics and documents travel only if you tick their box for each question. Same in Lite and Pro. | [Ch. 13](manual-usuario.en.md#ayuda-en-la-app) |
| **Optimizing the archive…** on close | Automatic when reclaiming database space is worthwhile; the window closes by itself when it finishes. | [Ch. 1](manual-usuario.en.md#capitulo-1-inicio-rapido) |
| Sample collections; **Try it with sample documents** | Three collections with analysis done in a new archive; they stay on this computer and are not synced. | [Ch. 1](manual-usuario.en.md#capitulo-1-inicio-rapido) |

## Collections, documents and viewer

| Visible feature | Availability / scope | Manual section |
|---|---|---|
| **Browser**: browse with up to 4 tabs; **Capture page** or **Capture selection**; save verified PDFs; **Saved sources**; **Copy to collection** or **Copy to Zotero** | Needs the Internet. Web sources do not belong to any collection and copies are independent. It keeps no cookies or sessions between uses of the application. | [Ch. 4](manual-usuario.en.md#navegador) |
| Create, edit and delete collections | Available; deleting a collection also deletes its associated content. | [Ch. 4](manual-usuario.en.md#capitulo-4-crear-y-organizar-un-corpus) |
| Filter collections and search documents | Available; there is no option to sort collections by hand. | [Ch. 4](manual-usuario.en.md#capitulo-4-crear-y-organizar-un-corpus) |
| Import one or more files with the picker or by dragging them | Available for the formats listed in the manual; importing does not recognise text or transcribe audio automatically. | [Ch. 1](manual-usuario.en.md#capitulo-1-inicio-rapido), [ch. 4](manual-usuario.en.md#capitulo-4-crear-y-organizar-un-corpus) |
| **Home → Import sources**: existing destination or **+ New collection**, **Choose files** and summary | Available. Cancelling the picker does not create a new collection. If there are skips or errors, you can review the summary before **Go to collection**. | [Ch. 1](manual-usuario.en.md#capitulo-1-inicio-rapido), [ch. 4](manual-usuario.en.md#capitulo-4-crear-y-organizar-un-corpus) |
| Import PDF, PNG/JPG/JPEG, WEBP, TIFF/TIF and WAV/MP3/FLAC/M4A/AAC/OGG | Available. Not every importable format is suitable for text recognition. A PDF's legible digital text is stored at import as **native text**; photos are shown upright according to their EXIF. | [Ch. 4](manual-usuario.en.md#capitulo-4-crear-y-organizar-un-corpus), [ch. 6](manual-usuario.en.md#capitulo-6-obtener-texto-de-documentos) |
| Each imported file creates a document and is stored as a copy on this computer | Available; a multi-page PDF is browsed page by page. If you import the same unchanged file again, it may be skipped. | [Ch. 4](manual-usuario.en.md#capitulo-4-crear-y-organizar-un-corpus) |
| **Import from Internet Archive**: search, browse a collection or paste a link; choose files, destination and duplicates | Needs the Internet; no account or key required. Public PDFs; one import at a time; an interrupted import can be resumed. It does not run OCR. | [Ch. 4](manual-usuario.en.md#internet-archive) |
| Internet Archive: **File type** (PDF, PDF + text, Images), **Results per page**, whole records, whole page and **Select all N results** with batch import **(next version)** | Up to 10,000 results per search. The queue pauses on close and resumes with **Continue the queue**. JPEG, PNG and TIFF images are imported as originals. | [Ch. 4](manual-usuario.en.md#internet-archive) |
| **Download the collection text** as TXT, Markdown, PDF, Word or CSV | Available. One file with the text of all documents, marking native PDF text. It cannot rebuild the collection. | [Ch. 4](manual-usuario.en.md#capitulo-4-crear-y-organizar-un-corpus), [ch. 17](manual-usuario.en.md#capitulo-17-exportar-y-descargar) |
| A collection's **Context for the AI** | Available when editing the collection. Read by the Chat, Investigate, OCR correction and the Writing agent; it is synced and travels to the AI provider with the prompt. | [Ch. 4](manual-usuario.en.md#contexto-ia) |
| Export a collection's data as JSON (a data file) | Available. It does not include the original PDFs, images or audio and cannot rebuild the collection in EntropIA. | [Ch. 4](manual-usuario.en.md#capitulo-4-crear-y-organizar-un-corpus), [ch. 17](manual-usuario.en.md#capitulo-17-exportar-y-descargar) |
| Open collections, documents and their pages or files | Available; you can browse them but not edit or reorder their pages from the explorer. | [Ch. 4](manual-usuario.en.md#capitulo-4-crear-y-organizar-un-corpus), [ch. 5](manual-usuario.en.md#capitulo-5-trabajar-con-documentos-y-assets) |
| PDF and image viewer: zoom, hand, annotation, crop, cover region, rotation, duplicate, undo/redo | Available for the file types the viewer opens. | [Ch. 5](manual-usuario.en.md#capitulo-5-trabajar-con-documentos-y-assets) |
| Audio player | Available: playback, position, skips and volume. | [Ch. 5](manual-usuario.en.md#capitulo-5-trabajar-con-documentos-y-assets) |
| Delete the open document or page/file | Available; the citation may keep the passage but stop opening the original document. | [Ch. 5](manual-usuario.en.md#capitulo-5-trabajar-con-documentos-y-assets) |
| Document tabs: **Document**, **Extracted text**, **Notes**, **Text**, **Analysis**, **Map**, **Search**, **Layout**, **Metadata** | They appear depending on the page or file type and the available data. | [Ch. 5](manual-usuario.en.md#capitulo-5-trabajar-con-documentos-y-assets) |
| **Layout**: show detected areas, filter them and copy their data | The tab may be empty if no sections were detected. It is not an audio tool. | [Ch. 5](manual-usuario.en.md#capitulo-5-trabajar-con-documentos-y-assets), [ch. 8](manual-usuario.en.md#capitulo-8-analizar-documentos-y-colecciones) |
| **Layout → Fix box**: **Adjust box**, **New box after this one**, **Recognize box text** | Needs the Internet and the GLM-OCR key (in Pro too); each box is one request. The block keeps its place in the reading order and the text enters that same place in **Extracted text**. Nothing is saved if the layout changed while you were editing. | [Ch. 5](manual-usuario.en.md#corregir-cajas) |
| **Metadata → Image versions**: **Original**, **Version N**, **Current**, **Matches the layout/text**, **Use this version** | Available on edited images; no file is deleted. Notices **This layout/text belongs to another version of the image** with **Run the layout/OCR again**. | [Ch. 5](manual-usuario.en.md#versiones-de-imagen) |
| **Metadata → Process history**: process, tool, model, versions, parameters and SHA-256 fingerprints; **Reload** and **Export JSON** | Available. It stays on this computer and is not synced. It records processes since version 1.4.0; manual editing is grouped by session. | [Ch. 5](manual-usuario.en.md#historial-de-procesos) |
| File data and custom data | File data can be consulted; you can add, change or remove custom data. There is no option to rename the document from this panel. | [Ch. 5](manual-usuario.en.md#capitulo-5-trabajar-con-documentos-y-assets) |

## OCR, transcription, text and search

| Visible feature | Availability / scope | Manual section |
|---|---|---|
| **OCRH** | PDF, PNG and JPG/JPEG. It always runs the page through the engine and replaces the native text. In Lite it uses GLM-OCR over the Internet (WEBP/TIFF can be imported but not sent to GLM-OCR). In Pro, according to **OCRH mode**: PaddleOCR-VL or local GLM-OCR (NVIDIA GPU, or AMD, Intel or Apple with 4 GB), remote GLM-OCR or Automatic. It asks for confirmation if there are manual corrections. | [Ch. 1](manual-usuario.en.md#capitulo-1-inicio-rapido), [ch. 6](manual-usuario.en.md#capitulo-6-obtener-texto-de-documentos) |
| **OCRL** | Pro only. Fast local OCR with PaddleOCR, offline; with layout and reading order (next version). | [Ch. 6](manual-usuario.en.md#capitulo-6-obtener-texto-de-documentos) |
| Edit or restore the recognised text | Edit in **Text**, right panel. **Restore original OCR**, in **Extracted text** above the viewer, requires a saved original and discards the corrections. | [Ch. 6](manual-usuario.en.md#capitulo-6-obtener-texto-de-documentos) |
| **OCRC**, **OCRR/Summary** | They need text and OpenRouter (in Pro, also the local model or the local Qwen3-VL corrector, next version). OCRC replaces the text when it finishes, with no later acceptance. Check correction and summary against the source. | [Ch. 6](manual-usuario.en.md#capitulo-6-obtener-texto-de-documentos) |
| **STT**, edit and summarise a transcription | In Lite, STT needs the Internet and AssemblyAI; in Pro, according to **STT mode**: local (Whisper; Parakeet with speaker detection, next version) or AssemblyAI. Editing the text is local; **Transcribe again** asks for confirmation if there are manual corrections. Summarising requires OpenRouter or, in Pro, the local model. | [Ch. 6](manual-usuario.en.md#capitulo-6-obtener-texto-de-documentos) |
| **Rename speakers** **(next version)** | Replaces "Speaker A"… with names throughout the transcription. | [Ch. 6](manual-usuario.en.md#capitulo-6-obtener-texto-de-documentos) |
| **Collection audio speaker identification** | AssemblyAI selector in Remote APIs. It affects STT of imported audio, not dictation. Check the attributions by listening to the source. | [Ch. 6](manual-usuario.en.md#capitulo-6-obtener-texto-de-documentos), [ch. 16](manual-usuario.en.md#capitulo-16-configuracion-y-herramientas-generales) |
| Copy and download text as Markdown, PDF or Word | In **Extracted text** above the viewer of PDFs/images with OCR. Audio has no such menu: copy the editable field by hand. | [Ch. 6](manual-usuario.en.md#capitulo-6-obtener-texto-de-documentos), [ch. 17](manual-usuario.en.md#capitulo-17-exportar-y-descargar) |
| **INDEX** | Prepares the document's text for lexical search, which may include spelling variants; it does not search by meaning. | [Ch. 7](manual-usuario.en.md#capitulo-7-buscar-informacion), [ch. 8](manual-usuario.en.md#capitulo-8-analizar-documentos-y-colecciones) |
| **Search → Search by Similar Text (FTS)** | Up to 10 documents through terms and close variants; searching content needs indexed text. | [Ch. 7](manual-usuario.en.md#capitulo-7-buscar-informacion) |
| **EMBED** and **Similar Pages** | EMBED prepares the selected file/page with text; in Lite with OpenRouter and the Internet, in Pro also with local BGE-M3. **Similar Pages** loads when you open Search: up to 5 results with similarity and preview. | [Ch. 7](manual-usuario.en.md#capitulo-7-buscar-informacion), [ch. 8](manual-usuario.en.md#capitulo-8-analizar-documentos-y-colecciones) |
| **NER** and **TRIPLET** | They propose names and relations; review or correct each result. In Lite they need OpenRouter; in Pro, spaCy and the local model. Each relation keeps its source sentence, highlighted when you open it. | [Ch. 8](manual-usuario.en.md#capitulo-8-analizar-documentos-y-colecciones) |
| **Same as…** and **Settings → Aliases** | Available. Joins forms of a name without changing the mentions; **Suggestions** proposes groups you accept or discard. It is synced. | [Ch. 8](manual-usuario.en.md#alias-de-entidades) |
| **Map** | Lets you review and change places associated with a document. Viewing the map and searching places requires the Internet. | [Ch. 8](manual-usuario.en.md#capitulo-8-analizar-documentos-y-colecciones) |
| Word analysis of a collection: cloud, chart, excluded words and PNG download | Counts words from stored recognised texts and transcriptions; it does not summarise the documents. | [Ch. 8](manual-usuario.en.md#capitulo-8-analizar-documentos-y-colecciones) |
| A collection's **3D relations** | Graph of the stored triples; a click on a name or a line highlights its links and opens the passage in the text. Settings for what is shown, appearance and layout (Forces, Communities, Radial hierarchy). **Save the graph for Gephi (.gexf)** (next version). Needs triples extracted beforehand. | [Ch. 8](manual-usuario.en.md#relaciones-3d) |
| **Phrases**: frequent phrases, associated words (G²) and word tree | Available, local and without keys. | [Ch. 8](manual-usuario.en.md#frases) |

## Notes, Chat and Research

| Visible feature | Availability / scope | Manual section |
|---|---|---|
| Topics and notes attached to a document or a page | Available; you can edit and delete notes, with confirmation before deleting. | [Ch. 9](manual-usuario.en.md#capitulo-9-notas-y-topicos) |
| Dictation in notes | Needs microphone permission; in Lite, AssemblyAI and the Internet; in Pro, the **STT mode** engine. | [Ch. 9](manual-usuario.en.md#capitulo-9-notas-y-topicos), [ch. 16](manual-usuario.en.md#capitulo-16-configuracion-y-herramientas-generales) |
| **Research chat** | Answers questions about your documents, your Library or both (**Search in**) and can show the sources; opening a source takes you to the cited fragment. Needs OpenRouter and the Internet (in Pro, or the local model according to **LLM mode**); it does not search the Web. With **Corpus** and **Both**, the **Corpus collections** menu lets you search all or some. | [Ch. 13](manual-usuario.en.md#capitulo-13-chat-y-agentes-de-entropia) |
| Chat history: search, rename, start or delete conversations; download a conversation as PDF | The PDF is saved directly to **Downloads**, with no location picker. Deleting a conversation asks for confirmation and cannot be undone. | [Ch. 13](manual-usuario.en.md#capitulo-13-chat-y-agentes-de-entropia), [ch. 17](manual-usuario.en.md#capitulo-17-exportar-y-descargar) |
| **Investigate with the Agent** from the Chat | Carries the last question and the loaded messages to the **Investigate** form. It lets you review the context; it does not start the work automatically. | [Ch. 13](manual-usuario.en.md#capitulo-13-chat-y-agentes-de-entropia) |
| Create an investigation: model, **Search in** (Corpus, Library or Both) and collections | Needs processed material within the scope, OpenRouter and the Internet (also in Pro). Zero fragments in the corpus, or no synced library chosen, prevents starting. The model is searched by typing across the whole OpenRouter catalogue and stays fixed for the whole work. An investigation stopped by an error shows the message and **Retry**. | [Ch. 10](manual-usuario.en.md#capitulo-10-investigacion) |
| Review questions, the research plan and the searches | The corpus respects the chosen collections. **Bibliography queries** may look up metadata in local Zotero and OpenAlex over the Internet; they do not bring full texts. You can empty that field before approving. | [Ch. 10](manual-usuario.en.md#capitulo-10-investigacion) |
| Pause, continue or cancel; adjust call and cost limits | Available depending on the work's state; adjust the limits while it is paused. | [Ch. 10](manual-usuario.en.md#capitulo-10-investigacion) |
| Report with sections, sources and citations; edit or rewrite | Editing marks the text as unverified. **Rewrite** replaces the section directly when it finishes and adds cost even with the budget used up. Readable warnings name the step that degraded, with **Technical detail**. Check the source. | [Ch. 10](manual-usuario.en.md#capitulo-10-investigacion) |
| Investigation history and deletion with confirmation | Available; deleting also removes the report and its associated information. With sync, finished or failed investigations travel to your other computers. | [Ch. 10](manual-usuario.en.md#capitulo-10-investigacion) |
| Download a report as Markdown, HTML or DOCX | Available; there is no PDF option on the report screen. | [Ch. 10](manual-usuario.en.md#capitulo-10-investigacion), [ch. 17](manual-usuario.en.md#capitulo-17-exportar-y-descargar) |

## Writing, Zotero and Agent

| Visible feature | Availability / scope | Manual section |
|---|---|---|
| Create a manuscript, edit the title, autosave, state, change counter and confirmed deletion | **New document** creates another. Only **Home → Write** reuses the open one if it is still empty and untitled. Writing takes a single tab. It does not import DOCX/Markdown or recover deleted manuscripts. | [Ch. 1](manual-usuario.en.md#pestanas-y-vista-dividida), [ch. 11](manual-usuario.en.md#capitulo-11-escritura) |
| A manuscript's **Version history** | One version every ten minutes of work and before receiving changes or restoring; 50 are kept. **Restore this version** saves the current one as another version. **Section** filters by section and **Restore only this section** recovers just one. | [Ch. 11](manual-usuario.en.md#historial-de-versiones) |
| Editor: formatting, tables, search/replace, headings, PNG/JPG/GIF images | Picker, paste or drag for images; alignment, size, alt text and caption. **More tools** holds controls that do not fit in the bar. | [Ch. 11](manual-usuario.en.md#capitulo-11-escritura) |
| Dictation in Writing | Needs a microphone; in Lite, AssemblyAI and the Internet; in Pro, the **STT mode** engine. | [Ch. 11](manual-usuario.en.md#capitulo-11-escritura) |
| **Corpus**, **Zotero**, **Works**, **Notes**, **Agent**, **Export** panel | With a manuscript open. Corpus searches recognised text and transcriptions locally: up to 20 results, optional variants and a citation linked to a selected passage; no collection filter. | [Ch. 11](manual-usuario.en.md#capitulo-11-escritura) |
| Insert notes as text or a link; create a note from a selection | Available; you cannot limit the search to the manuscript's collections. If the note changes or disappears, the manuscript keeps its text. | [Ch. 9](manual-usuario.en.md#capitulo-9-notas-y-topicos), [ch. 11](manual-usuario.en.md#capitulo-11-escritura) |
| Writing agent: 14 actions and proposal review | Clicking an action sends the selection to OpenRouter (in Pro too). Applying requires **Replace** or **Insert below**; it is refused if the selection changed or is ambiguous. The **Notes** action searches your notes by words in common (not by meaning) and does not call the model if it finds none. | [Ch. 13](manual-usuario.en.md#capitulo-13-chat-y-agentes-de-entropia) |
| Connect Zotero on the same computer; choose or **Add library** (personal or group) | Open a manuscript and the Zotero tab. Zotero must be open and allow local communication; the earlier metadata copy may remain visible. No Zotero web key required. | [Ch. 12](manual-usuario.en.md#capitulo-12-zotero) |
| Search and cite references; **View details**; adjust locator | Typing filters the local copy; Enter searches Zotero and, with the library synced, adds **By meaning** and **By content** results. **Cite** inserts immediately; cancelling the adjustments does not delete the citation. APA is the style and there is no style selector. Refresh does not rewrite inserted citations. Automatic tags are hidden by default. | [Ch. 12](manual-usuario.en.md#capitulo-12-zotero) |
| **Synchronize library** from Zotero | Copies the catalogue and prepares the text of PDFs and web snapshots. It works in the background and resumes when reopened. In Lite it uses GLM-OCR for scanned pages and OpenRouter for embeddings: it may cost; in Pro, the engines you chose. It does not modify Zotero. | [Ch. 12](manual-usuario.en.md#biblioteca) |
| **Library** section: works by library, order, search; a work with **Original**, **Text** and **Metadata** | Records are edited in Zotero; only PDFs are annotated. It stays on this computer and does not travel with sync; the paid OCR and embedding results are shared between the account's computers so they are not paid twice. | [Ch. 12](manual-usuario.en.md#biblioteca) |
| **Annotations** on Library PDFs: highlight, underline, note, area, text, comment and colour; states, conflicts and Zotero bin | Needs the Internet and a Zotero Web API key with write access; personal library only, not groups. They sync both ways with Zotero, not with EntropIA's sync. | [Ch. 12](manual-usuario.en.md#anotaciones-zotero) |
| Writing's **Works** tab: search works and **Passages**; **Open passage**, **Cite passage** | Needs a synced library with its indexing finished and the embedding provider. It cites with the page as locator. | [Ch. 12](manual-usuario.en.md#capitulo-12-zotero) |
| Library **Reprocess text** | Finds attachments with damaged text, shows pages and estimated cost before queuing; progress is in **Settings → Batches**. | [Ch. 12](manual-usuario.en.md#reprocesar-texto) |
| Citation and bibliography preferences | Available when downloading; you can choose footnotes, short reference, comment or quoted text, and include the Zotero bibliography. | [Ch. 17](manual-usuario.en.md#capitulo-17-exportar-y-descargar) |
| Download a manuscript as Markdown, HTML or DOCX | Available. Writing offers no PDF download; the citation format may vary with the chosen file. | [Ch. 11](manual-usuario.en.md#capitulo-11-escritura), [ch. 17](manual-usuario.en.md#capitulo-17-exportar-y-descargar) |

## Batches, sync and settings

| Visible feature | Availability / scope | Manual section |
|---|---|---|
| Batch OCR, embeddings, entities, triples, OCR correction or summaries | From **Settings → Batches**, with prior review. **Include native text** also recognises pages with native text; the **Document** column links each unit; a batch never overwrites text corrected by hand. Entities, triples, correction and summaries are off by default; in Lite each document is a paid OpenRouter request and correction is paid in Pro too. **Analyze selection** asks for the OpenRouter key first when needed. It writes one summary per page, not a general summary of the collection, and does not transcribe audio. | [Ch. 14](manual-usuario.en.md#capitulo-14-procesamiento-por-lotes) |
| **Custom schema**: define fields, search the model in the OpenRouter catalogue, extract a table by batch, see results and download CSV | Available in **Settings → Batches**. A field can repeat (one row with several values). The model catalogue needs the saved key; it also accepts an identifier typed by hand. In Lite each document is a paid OpenRouter request. Schemas and results are not synced. | [Ch. 14](manual-usuario.en.md#esquema-propio) |
| **Saved flow** | Remembers which operations to tick; it does not save collections. It stays on this computer. | [Ch. 14](manual-usuario.en.md#flujos-guardados) |
| **Settings → Assistants**: allow external assistants (Claude Code, Codex, OpenCode, Pi or another MCP assistant); one-click **Conectar Claude Code/Codex/OpenCode/Pi**; manual connection; **Copy setup instruction for any assistant**; `--check` diagnostics; revoke; **Preparar procesamiento y editar el corpus** and **Pedir procesamiento pago** permissions; **Carpetas de entrada** | Off by default; only on this computer (`127.0.0.1`, and on Windows a named pipe for your user only) and with EntropIA open. A new assistant can only read. Claude Code and Codex connect with a terminal command; OpenCode and Pi with a configuration snippet (Pi needs an MCP extension); others, with the copied instruction. When reconnecting, the earlier entry is replaced. Spending, importing, replacing text or deleting requires your approval in an EntropIA dialog; the request expires after ten minutes. What the assistant reads goes to its AI provider. | [Ch. 13](manual-usuario.en.md#asistentes-externos) |
| Progress, pause, resume, cancel, **Priority**, failed tasks and history | Closing stops processing; on return, the recovery notice offers **Keep paused** or **Resume**. Cancelling keeps finished results. Priority (**Background**, **High**, **Interactive**) is changed in an unfinished batch's details. | [Ch. 14](manual-usuario.en.md#capitulo-14-procesamiento-por-lotes) |
| Optional sync: registration, sign-in, mode and interval, computers, storage and conflicts | Registering does not sign in. Once in, you can sync and review differences; you cannot select collections or change the server. **Request plan change** opens a plan request. It does not sync the Library (except paid OCR and embedding results), schemas, flows, paired assistants, help conversations, process history or preferences. | [Ch. 15](manual-usuario.en.md#capitulo-15-sincronizacion-y-nube) |
| Re-verify files and delete server data | Deleting remote data requires the password and confirmation; it does not delete local data. It is not a full backup. | [Ch. 15](manual-usuario.en.md#capitulo-15-sincronizacion-y-nube) |
| Writing manuscripts in general sync; **Sync now** button in Writing | They sync between your account's computers, with their images; simultaneous editing keeps the other version as a separate manuscript (**Conflict copy**). The button appears in the list and on the manuscript bar when sync is on. | [Ch. 11](manual-usuario.en.md#capitulo-11-escritura), [ch. 15](manual-usuario.en.md#capitulo-15-sincronizacion-y-nube) |
| **Share and publish**: share a manuscript with another account | Needs sync on in both accounts and a server that supports it. The recipient can leave and keeps their copy. | [Ch. 11](manual-usuario.en.md#compartir-manuscrito) |
| **Share and publish**: send to hlab.com.ar | For the HLab team; needs the publishing key. It arrives as a hidden draft; resending updates the article. | [Ch. 11](manual-usuario.en.md#enviar-a-hlab) |
| **Remote APIs**, **Prompts**, **Model Params**, **RAG Params**, **Agent**, **Sync**, **Assistants**, **Help** (next version), **Batches**, **Aliases**, **Logs**, **Backups**, **Maintenance**, **Appearance** | Common tabs. **AI dependencies** appears only in Pro. | [Ch. 16](manual-usuario.en.md#capitulo-16-configuracion-y-herramientas-generales) |
| **Settings → Maintenance → Review image versions** | Available. It first lists what it finds and changes nothing until **Apply**; it deletes no files. | [Ch. 16](manual-usuario.en.md#mantenimiento) |
| Local engines: **LLM mode**, **Local model (Gemma)**, **BGE-M3 embeddings**, **Local RAG reranker**, **STT mode**, **OCRH mode**, **Local corrector (Qwen3-VL)**; **AI dependencies** tab | Pro only. It downloads the AI base at start-up and the models from each card. Local GLM-OCR needs an NVIDIA GPU. | [Ch. 16](manual-usuario.en.md#motores-locales-pro) |
| **OpenAI-compatible server (optional)** **(next version)** | On the OpenRouter card: text generation goes to your own server (vLLM, Ollama) with no key and no Vercel fallback. | [Ch. 16](manual-usuario.en.md#capitulo-16-configuracion-y-herramientas-generales) |
| **Settings → Agent**: Research Agent parameters | Available. Five groups (model and output, network and retries, archive, retrieval, clarification and queries), with defaults and validation. They are frozen when each investigation is created. | [Ch. 16](manual-usuario.en.md#capitulo-16-configuracion-y-herramientas-generales) |
| OpenRouter, AssemblyAI, GLM-OCR/z.ai; get key, test connection, save and **Remove key** | OpenRouter has separate generative and embedding models; AssemblyAI transcribes audio and enables dictation; GLM-OCR recognises text in supported PDFs and images. Keys are kept in the system store; removing one asks for confirmation. | [Ch. 6](manual-usuario.en.md#capitulo-6-obtener-texto-de-documentos), [ch. 16](manual-usuario.en.md#capitulo-16-configuracion-y-herramientas-generales) |
| **Fallback: Vercel AI Gateway** | Optional. If OpenRouter fails, correction, summaries, entities, triples, schemas and the Chat go to Vercel with the same model. It does not cover Investigate, the Writing agent or embeddings. Vercel requires a card on the account. | [Ch. 16](manual-usuario.en.md#capitulo-16-configuracion-y-herramientas-generales) |
| **Zotero** Web API key | Optional; it is verified and shows its permissions. Used by Library annotations and the Browser's **Copy to Zotero** (to fill in empty fields). Not needed to cite or to sync the Library. | [Ch. 16](manual-usuario.en.md#capitulo-16-configuracion-y-herramientas-generales) |
| **Settings → Backups**: folder, daily or weekly automatic backup, number to keep, **Back up now** | Saves the database and all files in a new folder; it is recovered by copying it back with the application closed. It does not include keys or the sync session. | [Ch. 17](manual-usuario.en.md#copias-de-seguridad) |
| **Database → DB Browser** | Query by table, search, order, page size and cell copy. JSON/CSV exports all filtered rows, not only the visible page; it is not a full backup. | [Ch. 16](manual-usuario.en.md#capitulo-16-configuracion-y-herramientas-generales), [ch. 17](manual-usuario.en.md#capitulo-17-exportar-y-descargar) |

## Differences between Lite and Pro

| Feature | In Lite | In Pro | Reference |
|---|---|---|---|
| Fast **OCRL** OCR | Not available. | Local PaddleOCR. | [Ch. 6](manual-usuario.en.md#capitulo-6-obtener-texto-de-documentos) |
| **OCRH** | Remote GLM-OCR. | Local, remote or Automatic. | [Ch. 6](manual-usuario.en.md#capitulo-6-obtener-texto-de-documentos) |
| Transcription and dictation | AssemblyAI. | Local or AssemblyAI. | [Ch. 6](manual-usuario.en.md#capitulo-6-obtener-texto-de-documentos) |
| Correction, summaries, entities, relations, schemas and Chat | OpenRouter (or a compatible server), Vercel as fallback. | Local model, OpenRouter or Automatic. | [Ch. 2](manual-usuario.en.md#lite-y-pro) |
| Investigate, Writing agent, box correction, help chat | Remote services. | Remote services (same as Lite). | [Ch. 2](manual-usuario.en.md#lite-y-pro) |
| Embeddings and reranking | OpenRouter. | Local BGE-M3 and reranker, or OpenRouter. | [Ch. 16](manual-usuario.en.md#motores-locales-pro) |
| Platforms | Windows (including Microsoft Store), macOS and Linux. | Windows and Linux. | [Ch. 2](manual-usuario.en.md#capitulo-2-introduccion-a-entropia-lite) |

## Features neither variant includes

| Feature | What to know | Related reference |
|---|---|---|
| Automatic text recognition or transcription on import | Importing adds the files and stores the native text of digital PDFs; you start OCR and transcription yourself. | [Ch. 4](manual-usuario.en.md#capitulo-4-crear-y-organizar-un-corpus) |
| Import `.txt`, Word, Excel or other Office files | There is no option to import those formats. From Internet Archive you can import PDFs (and images, next version). | [Ch. 4](manual-usuario.en.md#capitulo-4-crear-y-organizar-un-corpus) |
| Rebuild a collection from **Export JSON** | You cannot import that file back into EntropIA. | [Ch. 4](manual-usuario.en.md#capitulo-4-crear-y-organizar-un-corpus) |
| Restore a backup with a button | The backup is made from **Settings → Backups**, but recovered by hand, copying the folder back with EntropIA closed. | [Ch. 17](manual-usuario.en.md#copias-de-seguridad) |
| Free search by meaning or synonyms in text search | Lexical search may include close spellings, not semantic equivalences. **Similar Pages** compares material prepared with embeddings. | [Ch. 7](manual-usuario.en.md#capitulo-7-buscar-informacion) |
| General web browsing from the Chat or Investigate | The Chat does not browse the Web. Investigate may look up bibliographic metadata in OpenAlex/Zotero according to the plan; it does not download full texts. To open pages and save them as sources, use the **Browser**. | [Ch. 4](manual-usuario.en.md#navegador), [ch. 10](manual-usuario.en.md#capitulo-10-investigacion), [ch. 12](manual-usuario.en.md#capitulo-12-zotero), [ch. 13](manual-usuario.en.md#capitulo-13-chat-y-agentes-de-entropia) |
| Choose collections for the Chat | The Chat uses the available documents unless you narrow the corpus in **Corpus collections**, with **Corpus** or **Both** as scope; **Investigate** also has its own selector. | [Ch. 10](manual-usuario.en.md#capitulo-10-investigacion), [ch. 13](manual-usuario.en.md#capitulo-13-chat-y-agentes-de-entropia) |
| Change the sync service address or sync collections separately | There is no option to change the service address or choose individual collections. | [Ch. 15](manual-usuario.en.md#capitulo-15-sincronizacion-y-nube) |
| Add a page to an existing document, move documents between collections or reorder pages | There are no options for those tasks. | [Ch. 4](manual-usuario.en.md#capitulo-4-crear-y-organizar-un-corpus), [ch. 5](manual-usuario.en.md#capitulo-5-trabajar-con-documentos-y-assets) |
| Import a file as a new manuscript | There is no option to import files as manuscripts. | [Ch. 11](manual-usuario.en.md#capitulo-11-escritura), [ch. 15](manual-usuario.en.md#capitulo-15-sincronizacion-y-nube) |
| Recover deleted manuscripts | There is no bin: the version history is deleted with the manuscript. Only an earlier backup keeps it. | [Ch. 11](manual-usuario.en.md#capitulo-11-escritura) |
| Choose another citation style or edit Zotero works from EntropIA | APA is the style; works, collections and tags are edited in Zotero. In the Library only PDF annotations are edited. | [Ch. 12](manual-usuario.en.md#capitulo-12-zotero) |
| Download PDF from Writing or from an Investigate report | There is no PDF on those screens. There is for OCR of images/PDFs and for Chat conversations, not for audio transcriptions. | [Ch. 10](manual-usuario.en.md#capitulo-10-investigacion), [ch. 17](manual-usuario.en.md#capitulo-17-exportar-y-descargar) |
| Write your own instruction for the Writing agent | There is no field to type instructions; you choose among the available actions for the selected text. | [Ch. 13](manual-usuario.en.md#capitulo-13-chat-y-agentes-de-entropia) |
| A general summary of a whole collection | Batches write one summary per page; they do not produce a single text summarising the collection. | [Ch. 14](manual-usuario.en.md#capitulo-14-procesamiento-por-lotes) |
| Sync the Library, custom schemas, flows, process history or help conversations between computers | They stay on each computer. Each computer syncs its Library from its own Zotero. | [Ch. 15](manual-usuario.en.md#capitulo-15-sincronizacion-y-nube) |

## How to use this inventory

The **Manual section** column links to the corresponding steps and explanations. Access conditions —such as needing the Internet or a key— appear in the middle column.

## Screenshots and provenance

The screenshots show EntropIA Lite in Spanish; in Pro, some screens add the local engines' controls. The original screenshots (PNG) are not versioned in the repository; their web copies are published in `manual/manual-usuario/images/` as lossless WebP, keeping the original dimensions. No controls, data or results were altered. The examples belong to the screenshots' corpus, not to a fresh installation.

The screenshots were taken with version 1.3.0 on the sample collections, in an isolated development profile. The Library work is *Facundo* (Sarmiento, 1845), in the public domain, loaded with sample annotations. The screenshots of the Chat, Investigate and its report, the Browser and the Zotero detail are still from 1.1.1: they show AI answers, web pages or a real Zotero library, which the sample corpus does not reproduce.

| Source screenshot | Use in the manual |
|---|---|
| `EntropIA-Colecciones.png` | Create and locate collections. |
| `EntropIA-Coleccion.png` | Cover; documents and collection indicators. Its side panel detail illustrates text analysis. |
| `EntropIA-Documento.png` | Viewer and panels. Its right panel detail illustrates notes and topics. |
| `EntropIA-Dividida.png` | Split view. Its left pane, labelled as a detail, illustrates Home. |
| `EntropIA-Agente.png` | The Investigate form and history; not the Writing agent. |
| `EntropIA-Investigacion.png` | Report, coverage and consulting the original source. |
| `EntropIA-Chat-RAG.png` | Chat, citations, sources and history. |
| `EntropIA-Escritura.png` | Manuscript list. |
| `EntropIA-Escrito.png` | Manuscript editor and the research panel with the Corpus tab. |
| `EntropIA-Configuracion.png` | Remote providers and Settings navigation. |
| `EntropIA-Base.png` | DB Browser, filters, data and export. |
| `EntropIA-Navegador.png` | Browser with one tab open (1.1.1). |
| `EntropIA-Biblioteca-anotaciones.png` | A Library work with annotations and the **Annotations** panel. |
| `EntropIA-Asistentes.png` | Settings → Assistants: pairing, permissions, import folders and connection lines. |

The four crops are published as `EntropIA-Inicio-detalle.webp`, `EntropIA-Analisis-detalle.webp`, `EntropIA-Notas-detalle.webp` and `EntropIA-Zotero-detalle.webp` (the last one from 1.1.1). They only remove areas outside the panel explained.

The remaining diagrams illustrate OCR, organisation, batch, sync, example and troubleshooting flows; they do not replace screenshots or claim those tasks were run with the corpus shown.

The favicon `images/entropia.ico` is a copy of the application icon (`apps/desktop/src-tauri/icons/icon.ico`). It is used in the manual, this inventory and the site entry page.

---

**Manual:** [Read the user manual](manual-usuario.en.md) · [Español](manual-usuario.md).\
**Screenshots and diagrams:** included next to the corresponding steps of the [manual](manual-usuario.en.md).
