# EntropIA Pro Privacy Notice

**Español:** [PRIVACY.md](./PRIVACY.md)

EntropIA Pro is designed as a local-first desktop app. Your collections, imported files, extracted text, notes, indexes, and local AI outputs are stored on your machine unless you explicitly configure or trigger a remote provider.

## What stays local by default

| Data | Default handling |
| ---- | ---------------- |
| Collections and metadata | Stored in the local EntropIA app data directory. |
| Imported assets | Referenced or copied according to the desktop import flow. |
| OCR and extracted text | Stored locally in the app database. |
| FTS indexes, embeddings, entities, summaries | Stored locally when generated. |
| Local model files and runtime dependencies | Stored locally in app/runtime directories. |

## Network activity

EntropIA Pro can contact external services only for features that require downloads or user-configured cloud providers.

| Feature | Destination | What may be sent or downloaded |
| ------- | ----------- | ------------------------------ |
| Gemma local model download | Hugging Face model URL configured by the app | Downloads the GGUF model file. |
| Dependency/runtime bootstrap | Configured runtime and package sources | Downloads runtime archives, Python packages, or tools when not already bundled. |
| OpenRouter LLM mode | OpenRouter API | Sends the text needed for the requested LLM task and the configured API key. |
| AssemblyAI transcription mode | AssemblyAI API | Uploads the audio selected for transcription and uses the configured API key. |
| GLM OCR high-quality OCR | GLM OCR API (z.ai) | Uploads the selected images/PDFs for high-quality OCR and uses the configured API key. |
| External links in the UI | Browser/system handler | Opens the selected URL outside the app. |

The current codebase does not include a separate analytics or telemetry service. Operational logs are written locally for diagnostics.

## API keys

OpenRouter, AssemblyAI, and GLM OCR API keys are user-provided settings. Treat them as secrets:

- do not commit app data or settings files;
- do not share logs that may contain provider names, request errors, or configuration details without reviewing them first;
- rotate a key if it was exposed.

## User control

- Use local modes when you do not want content sent to a remote AI provider.
- Remove provider API keys from Settings to disable those remote paths.
- Delete the local app data directory if you want to remove local databases, logs, runtime files, and generated outputs.

## Limitations

This notice describes the EntropIA Pro application behavior. Remote providers have their own privacy policies, retention terms, and account controls.
