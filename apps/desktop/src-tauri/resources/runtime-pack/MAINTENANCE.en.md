# Runtime-pack maintenance contract

**Español:** [MAINTENANCE.md](./MAINTENANCE.md)

## What this repository covers

- Versioned manifests by platform.
- Bundleable structure for `windows-x86_64` and `linux-x86_64`.
- Assembly and smoke scripts (`scripts/build_runtime_pack.py`, `scripts/runtime-pack-smoke.py`).
- Small fixtures to validate wiring without uploading heavy payloads to the repository.

## What the real runtime pack must contain

The in-repo fixture only validates wiring; the real runtime is assembled outside the repo by **Build Runtime Pack** with an external payload and hosted via **Publish Runtime Bootstrap**. A release pack MUST include:

1. Redistributable relocatable Python per platform.
2. Audited `uv` if it changes from the fixture.
3. Real offline wheelhouse for OCR/transcription.
4. Seeded caches/models (HF and PaddleX) required by core flows. Embeddings: OpenRouter `baai/bge-m3` or local BGE-M3 via ONNX; local NER (Pro): spaCy `es_core_news_md` with Gemma/OpenRouter fallback. There is no `scripts/embed.py`; spaCy IS part of the managed runtime.
5. Audited Linux shared libraries (`libpdfium.so`, `libonnxruntime.so`, and any additional dependency that becomes mandatory).

## External payload contract

- The `scripts/build_runtime_pack.py` script accepts `--payload-root`.
- That directory can use a direct layout (`python/`, `uv/`, `wheelhouse/`, `caches/`, `resources/lib/`) or `<payload-root>/<platform>/...`.
- If `manifest.overrides.json` exists, the script applies those overrides to the final manifest and **recalculates** listings/checksums/sizes from the files that were actually assembled.
- Current release pipeline: **Build Runtime Pack** (manual) assembles the pack from an external payload and uploads the `runtime-archive` artifact; **Publish Runtime Bootstrap** splits it under 2 GiB per asset, uploads the parts to the `runtime-bootstrap` tag, and publishes an ed25519-signed `manifest.json`; **Release** (tag `v*`) builds the lean installer with the manifest URL and public key baked in and ships the fixture as-is — the app detects “runtime not ready” and bootstraps from the signed source. `build.rs` fails closed if a release build embeds the fixture without a baked source.
- Heavy payload injection no longer happens in Release; assembling with an external payload (below) is how the runtime-pack artifact is produced. Assembly and smoke run inside Build Runtime Pack, never in Release, and never replace the in-repo fixture.

### Accepted layouts for `--payload-root`

Direct layout:

```text
runtime-payloads/
├── manifest.overrides.json
├── python/
├── uv/
├── wheelhouse/
├── caches/
└── resources/lib/
```

Per-platform layout:

```text
runtime-payloads/
├── windows-x86_64/
│   ├── manifest.overrides.json
│   ├── python/
│   ├── uv/
│   ├── wheelhouse/
│   ├── caches/
│   └── resources/lib/
└── linux-x86_64/
    ├── manifest.overrides.json
    ├── python/
    ├── uv/
    ├── wheelhouse/
    ├── caches/
    └── resources/lib/
```

### Real handoff by platform

| Platform | Expected `python_relpath` | Expected `uv_relpath` | Minimum native assets | Minimum external artifacts |
| -------- | ------------------------- | --------------------- | --------------------- | -------------------------- |
| `windows-x86_64` | `python/python.exe` | `uv/uv.exe` | `resources/lib/pdfium.dll`, `resources/lib/onnxruntime.dll` | `relocatable-python-windows-x86_64`, `offline-wheelhouse-core`, `seeded-model-caches` |
| `linux-x86_64` | `python/bin/python3` | `uv/bin/uv` | `resources/lib/libpdfium.so`, `resources/lib/libonnxruntime.so` | `relocatable-python-linux-x86_64`, `offline-wheelhouse-core`, `seeded-model-caches`, `linux-native-libs` |

### Verifiable assembly output

- Each `build_runtime_pack.py` run leaves `target/runtime-pack/<platform>/assembly-summary.json` with the resolved `payload_root`, final profile, and list of assembled files.
- `runtime-pack-smoke.py` accepts both the parent directory (`target/runtime-pack/`) and the specific platform directory (`target/runtime-pack/<platform>`) as `--root`.
- The useful validation for real handoff is: **assemble with external payload → review `assembly-summary.json` → run smoke against that output**.

Quick readiness diagnostic without faking payloads:

```bash
python3 apps/desktop/src-tauri/scripts/build_runtime_pack.py --platform windows-x86_64 --output-dir apps/desktop/src-tauri/target/runtime-pack --require-release-payload
```

If `--payload-root` was not passed, this command must fail with `--require-release-payload requires --payload-root`. That failure is correct: it confirms the real release still needs an external artifact and prevents publishing fixtures as a self-contained runtime.

Manual validation examples with a real payload:

```bash
python3 apps/desktop/src-tauri/scripts/build_runtime_pack.py --platform windows-x86_64 --payload-root /abs/path/runtime-payloads --output-dir apps/desktop/src-tauri/target/runtime-pack
python3 apps/desktop/src-tauri/scripts/runtime-pack-smoke.py --platform windows-x86_64 --root apps/desktop/src-tauri/target/runtime-pack

python3 apps/desktop/src-tauri/scripts/build_runtime_pack.py --platform linux-x86_64 --payload-root /abs/path/runtime-payloads --output-dir apps/desktop/src-tauri/target/runtime-pack
python3 apps/desktop/src-tauri/scripts/runtime-pack-smoke.py --platform linux-x86_64 --root apps/desktop/src-tauri/target/runtime-pack
```

### Windows x86_64 from the local managed venv

When a Windows machine already has a working `managed_venv`, a reproducible release payload can be materialized from that environment without uploading heavy binaries to git:

```powershell
python apps/desktop/src-tauri/scripts/materialize_windows_runtime_payload.py `
  --pack-version 2026.05.0 `
  --app-version 0.1.1 `
  --output-dir apps/desktop/src-tauri/target/runtime-payloads

python apps/desktop/src-tauri/scripts/build_runtime_pack.py `
  --platform windows-x86_64 `
  --payload-root apps/desktop/src-tauri/target/runtime-payloads `
  --output-dir apps/desktop/src-tauri/target/runtime-pack `
  --require-release-payload

python apps/desktop/src-tauri/scripts/runtime-pack-smoke.py `
  --platform windows-x86_64 `
  --root apps/desktop/src-tauri/target/runtime-pack `
  --release `
  --install-probe
```

That output stays under `target/`: it is a release artifact, not committable source.

### Windows x86_64 closure criteria

Windows is considered closed when these conditions are met:

1. `materialize_windows_runtime_payload.py` generates `target/runtime-payloads/windows-x86_64` from a working `managed_venv`.
2. `build_runtime_pack.py --require-release-payload` generates `target/runtime-pack/windows-x86_64` with `payload_profile=release`, `release_injection_required=false`, and `external_artifacts_required=[]`.
3. `runtime-pack-smoke.py --release --install-probe` passes on Windows.
4. The release pack does not contain any `CACHE_NOT_SEEDED.txt`; if that marker appears, release smoke must fail.
5. In dev, `ENTROPIA_RUNTIME_PACK_ROOT` can point to `target/runtime-pack` to validate the app without copying the heavy payload to `resources/`.

### OCRH / PaddleOCR-VL CPU policy

On Windows without an NVIDIA GPU, PaddleOCR-VL can use CPU. The 900s timeout is accepted: by itself it is not a sign of a broken runtime. If it expires, OCRH must fail in a controlled way and fall back to plain OCR; do not lower this timeout unless a new product decision is made.

To test it in dev without copying 3GB to `resources/`, start Tauri with:

```powershell
$env:ENTROPIA_RUNTIME_PACK_ROOT = "<repo>\apps\desktop\src-tauri\target\runtime-pack"
pnpm --filter @entropia-pro/desktop tauri dev
```

The override accepts both the parent directory (`target/runtime-pack`) and the direct pack (`target/runtime-pack/windows-x86_64`).

Minimal `manifest.overrides.json` example for a complete injection:

```json
{
  "payload_profile": "release",
  "release_injection_required": false,
  "external_artifacts_required": []
}
```

## Rule of truth

If `payload_profile != release` or `release_injection_required = true`, the runtime MUST NOT be presented as ready for the core offline flow.
Also, a `release` pack cannot keep declaring `external_artifacts_required`.

## Suggested ownership

- Product/app: defines which capabilities are part of “core offline”.
- Release engineering: injects artifacts, recalculates checksums, and publishes installers.
- OCR/NLP maintainers: validate licenses, size, and compatibility of included models/caches.
