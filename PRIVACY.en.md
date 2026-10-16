# EntropIA Pro and Lite privacy notice

**Español:** [PRIVACY.md](./PRIVACY.md)

EntropIA is a local-first desktop app in two variants: **Pro** (local AI plus optional remote providers) and **Lite** (100% remote AI). Your collections, imported files, extracted text, notes, manuscripts, indexes, AI outputs, and backups are stored on your machine. Content is only sent when you configure or trigger a remote provider, when you sync a Zotero library into the Biblioteca, or when you turn on sync.

**Lite** has no local engines: OCR, transcription, correction, summaries, entities, relations, embeddings, Chat, Research, and the Writing agent send the content they need to the remote providers you configure. In **Pro** those tasks can run on the machine; what is sent depends on the mode chosen in Settings.

Some network paths fire **automatically**, without you asking for them at that moment: the map tiles when the map opens, geocoding when a place entity is saved, the automatic title of a new chat conversation, the background processing of an already-synced Zotero library, the Microsoft Store update check (Lite on Windows) and, in Pro, the AI runtime download at startup. They are detailed below.

## What stays local by default

| Data                                                  | Default handling                                               |
| ----------------------------------------------------- | -------------------------------------------------------------- |
| Collections and metadata                              | Stored in the local EntropIA data directory.                   |
| Imported assets                                       | Copied into the data directory on import.                      |
| OCR and extracted text                                | Stored locally in the app database.                            |
| FTS indexes, embeddings, entities, summaries          | Stored locally when generated.                                 |
| Writing manuscripts and their version history         | Stored locally in the app database.                            |
| Biblioteca (catalog, text and embeddings of Zotero works) | Stored locally; it does not travel with sync.              |
| Backups                                               | Written only to the folder you choose.                         |
| Local model files and runtime dependencies (Pro)      | Stored locally in app/runtime directories.                     |
| Provider API keys and the sync session token          | Stored in the operating system's credential manager, not in the app database nor in backups. |

## Network activity

| Feature                           | Destination                                                       | What may be sent or downloaded                                                                                                                                        |
| --------------------------------- | ----------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| AI runtime (Pro)                  | Signed releases of the GitHub repository                          | At startup, downloads the AI runtime, verifying its signature and sha256; then installs the dependencies from its offline wheelhouse.                                 |
| Local Gemma model download (Pro)  | Hugging Face model URL configured by the app                      | Downloads the model GGUF file.                                                                                                                                        |
| Dependency bootstrap (Pro)        | Configured runtime and package sources                            | Downloads runtime files, Python packages, or tools when they are not already bundled.                                                                                |
| LLM and embeddings with OpenRouter | OpenRouter API                                                   | Sends the text needed for the requested task (correction, summary, entities, relations, custom schemas, Chat, Research, Writing agent, embeddings, and passage reranking) and the configured API key. |
| Vercel AI Gateway fallback        | Vercel AI Gateway API (`ai-gateway.vercel.sh`)                    | Only if you saved a key and OpenRouter fails: resends that same text request (correction, summary, entities, relations, custom schemas, or Chat) with the same model. |
| Transcription with AssemblyAI     | AssemblyAI API                                                    | Uploads the selected audio for transcription or dictation and uses the configured API key.                                                                          |
| OCRH with GLM-OCR                 | GLM-OCR API (z.ai)                                                | Uploads the selected images/PDFs for high-quality OCR and uses the configured API key.                                                                               |
| Automatic conversation title      | OpenRouter API                                                    | When the first question of a new chat conversation is answered, sends that question and that answer (each trimmed to 600 characters) so the model can propose a title. It happens without an explicit user action. |
| Local Zotero                      | Zotero's local API on the same machine (`127.0.0.1:23119`)        | Reads references, attachments, and libraries for citing and for the Biblioteca. It does not leave the machine.                                                       |
| Biblioteca (syncing a Zotero library) | GLM-OCR and OpenRouter                                        | After you ask to sync a library, its works are processed in the background: scanned or unreadable PDF pages go to GLM-OCR, and the text of the works and their passages goes to OpenRouter to compute embeddings (in Pro, depending on the chosen OCR and embedding modes). It continues after the app reopens until it finishes. |
| Meaning search in the Biblioteca  | OpenRouter API                                                    | Sends the query (or the text selected with "Buscar desde la selección") to compute its embedding.                                                                    |
| Research bibliographic queries    | Local Zotero and OpenAlex (`api.openalex.org`)                    | When you approve a plan with bibliographic queries, sends those queries to fetch work metadata.                                                                      |
| Internet Archive import           | `archive.org` and its subdomains                                  | Sends the searches, identifiers, or links you type and downloads the PDFs you pick. The service receives the IP address and the queries.                             |
| Place geocoding                   | OpenStreetMap Nominatim (`nominatim.openstreetmap.org`)           | When a place entity is created or edited, sends the place text to resolve its coordinates. It happens even if the map is never opened; the service receives the IP address and the queried text. |
| Location map                      | OpenStreetMap tile servers (`tile.openstreetmap.org`)             | When the map opens, requests the visible tiles; the service receives the IP address, request metadata, and the coordinates/zoom encoded in the requested URLs.       |
| Cross-device sync                 | EntropIA's sync service, operated by HLab (`entropia-cloud.app.hlab.com.ar`) | Sends credentials when registering or signing in, and the rows and files of what is synced (see below).                                                 |
| Sharing a manuscript              | The same sync service                                             | The shared manuscript, with its images, reaches the account you shared it with.                                                                                      |
| Send to hlab.com.ar               | `hlab.com.ar`                                                     | Only when you ask: the manuscript's title and text, with the publishing key you saved.                                                                               |
| Zotero Web API key                | `api.zotero.org`                                                  | Only if you save a key: verifies it and shows its permissions. Today only an experimental feature, not enabled in published builds, uses it.                         |
| OpenRouter model list             | OpenRouter API                                                    | When testing the connection or opening a model picker, requests the model catalog with your key.                                                                     |
| Update notice (Lite on Windows)   | Microsoft Store                                                   | Asks the Store whether a newer version is published. It downloads and installs nothing.                                                                              |
| External links in the UI          | System browser/handler                                            | Opens the selected URL outside the app.                                                                                                                              |

The current codebase does not include a separate analytics or telemetry service. Operational logs are written locally for diagnostics.

## Cross-device sync

Sync is **off until you sign in**. When you turn it on:

- the destination is EntropIA's sync service, operated by HLab; the app does not let you change its address;
- the connection requires HTTPS;
- the session token is stored in the operating system's credential manager — the keyring on Linux, Credential Manager on Windows, Keychain on macOS — and not in the app database;
- the service receives collections, documents and their files, texts, notes, topics, annotations, entities, relations, AI results, embeddings, Chat conversations, manuscripts with their images, and finished or failed investigations, with the same sensitivity as your collections;
- it does not receive the Biblioteca, custom schemas, preferences, sample collections, or API keys.

To cut the remote path entirely, sign out of sync. **Borrar mis datos del servidor**, in Settings → Sync, deletes your account's remote data and keeps the local data.

## API keys

The OpenRouter, Vercel AI Gateway, AssemblyAI, GLM-OCR, and Zotero API keys and the hlab.com.ar publishing key are user-supplied settings. Treat them as secrets:

- do not commit app data or configuration files;
- do not share logs that may contain provider names, request errors, or configuration details without reviewing them first;
- rotate a key if it was exposed.

## User control

- In Pro, use local modes if you do not want to send content to a remote AI provider. In Lite, every AI task uses a remote provider: do not run it on material you do not want to send.
- Remove provider API keys from Settings (the **Quitar clave** button) to disable those remote paths. Without an OpenRouter API key no automatic conversation title is generated and the Biblioteca is not processed for meaning search; without a Vercel key there is no fallback.
- Do not sync Zotero libraries into the Biblioteca if you do not want the text of their works sent to OpenRouter and GLM-OCR.
- Do not mark entities as places, or do not save them, if you do not want their text queried against Nominatim.
- Sign out of sync so nothing leaves through that path.
- Backups contain your whole archive: keep them in a protected folder.
- Delete the local app data directory if you want to remove local databases, logs, runtime files, and generated outputs. API keys and the sync token do not live there: remove them from Settings or by signing out.

## Limitations

This notice describes the behavior of the EntropIA Pro and Lite application. Remote providers (OpenRouter, Vercel, AssemblyAI, z.ai, OpenStreetMap, Internet Archive, OpenAlex, Zotero, Microsoft) have their own privacy policies, retention terms, and account controls.
