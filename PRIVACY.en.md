# EntropIA Pro Privacy Notice

**Español:** [PRIVACY.md](./PRIVACY.md)

EntropIA Pro is designed as a local-first desktop app. Your collections, imported files, extracted text, notes, indexes, and local AI outputs are stored on your machine. Content is sent only when you configure or trigger a remote provider, or when you turn on sync.

Three network paths fire **automatically**, without you asking for them at that moment: map tiles when the map opens, geocoding when you save a place entity, and the automatic title for a new chat conversation. All three are detailed below.

## What stays local by default

| Data                                         | Default handling                                           |
| -------------------------------------------- | ---------------------------------------------------------- |
| Collections and metadata                     | Stored in the local EntropIA app data directory.           |
| Imported assets                              | Referenced or copied according to the desktop import flow. |
| OCR and extracted text                       | Stored locally in the app database.                        |
| FTS indexes, embeddings, entities, summaries | Stored locally when generated.                             |
| Local model files and runtime dependencies   | Stored locally in app/runtime directories.                 |
| Sync session tokens                          | Stored in the operating system credential manager, not in the app database. |

## Network activity

EntropIA Pro can contact external services for downloads, user-configured cloud providers, cross-device sync, and OpenStreetMap services.

| Feature                       | Destination                                           | What may be sent or downloaded                                                                                                                                     |
| ----------------------------- | ----------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Gemma local model download    | Hugging Face model URL configured by the app          | Downloads the GGUF model file.                                                                                                                                     |
| Dependency/runtime bootstrap  | Configured runtime and package sources                | Downloads runtime archives, Python packages, or tools when not already bundled.                                                                                    |
| OpenRouter LLM mode           | OpenRouter API                                        | Sends the text needed for the requested LLM task and the configured API key.                                                                                       |
| AssemblyAI transcription mode | AssemblyAI API                                        | Uploads the audio selected for transcription and uses the configured API key.                                                                                      |
| GLM OCR high-quality OCR      | GLM OCR API (z.ai)                                    | Uploads the selected images/PDFs for high-quality OCR and uses the configured API key.                                                                             |
| Conversation auto-title       | OpenRouter API                                        | When the first question in a new chat conversation is answered, sends that question and that answer (each truncated to 600 characters) so the model can propose a title. This happens without an explicit user action. |
| Place geocoding               | OpenStreetMap Nominatim (`nominatim.openstreetmap.org`) | When you create or edit a place-typed entity, sends the place text to resolve its coordinates. This happens even if you never open the map; the service receives the IP address and the queried text. |
| Location map                  | OpenStreetMap tile servers (`tile.openstreetmap.org`) | When the map opens, it requests the visible tiles; the service receives the IP address, request metadata, and tile coordinates/zoom encoded in the requested URLs. |
| Cross-device sync             | The sync server you configure                         | Sends credentials when registering an account or signing in, plus the rows and blobs of whatever is synced. The destination is not an EntropIA service: it is the server you choose. The app requires HTTPS and rejects a non-TLS `server_url` unless it points at loopback. |
| External links in the UI      | Browser/system handler                                | Opens the selected URL outside the app.                                                                                                                            |

The current codebase does not include a separate analytics or telemetry service. Operational logs are written locally for diagnostics.

## Cross-device sync

Sync is **off until you configure it**. Once you turn it on:

- you choose the destination server; EntropIA operates none;
- the connection requires HTTPS: a non-TLS `server_url` is rejected unless it points at loopback;
- the session token is stored in the operating system credential manager — the keyring on Linux, Credential Manager on Windows — not in the app database;
- that server receives the rows and blobs of whatever you sync, carrying whatever sensitivity your collections carry.

To close the remote path entirely, sign out of sync and leave no server configured.

## API keys

OpenRouter, AssemblyAI, and GLM OCR API keys are user-provided settings. Treat them as secrets:

- do not commit app data or settings files;
- do not share logs that may contain provider names, request errors, or configuration details without reviewing them first;
- rotate a key if it was exposed.

## User control

- Use local modes when you do not want content sent to a remote AI provider.
- Remove provider API keys from Settings to disable those remote paths. Without an OpenRouter API key, conversation auto-titling does not run either.
- Do not mark entities as places, or do not save them, if you do not want their text queried against Nominatim.
- Sign out of sync and leave the server unconfigured so nothing leaves through that path.
- Delete the local app data directory if you want to remove local databases, logs, runtime files, and generated outputs. The sync token does not live there: remove it by signing out.

## Limitations

This notice describes the EntropIA Pro application behavior. Remote providers have their own privacy policies, retention terms, and account controls.
