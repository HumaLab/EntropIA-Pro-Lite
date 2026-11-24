# PaddleOCR-VL on NVIDIA GPUs: CUDA runtime pack (Linux first)

Objective: Pro builds on Linux with an NVIDIA GPU download a CUDA runtime pack and run PaddleOCR-VL on the GPU; every other machine keeps today's CPU pack untouched.
Branch: `feat/paddle-cuda-runtime-pack` · Delivery: `ask-on-risk` · Tests: `cargo test --features local-ml`, Vitest, `scripts/test_runtime_pack_scripts.py`, GPU checks on pop-os (RTX 5090) and pop-os-notebook (RTX 4060).

## Specs

- S1. Quoted: "PaddleOCR-VL en GPU NVIDIA en los builds publicados. ... dos packs, uno CPU y otro CUDA, y que la app elija según `detect_nvidia_gpu()` antes de descargar. Así nadie baja lo que no usa."
- S2. Quoted: "es muy importante no romper nada de lo que ya funciona": machines without NVIDIA, or with an old driver, and already-installed apps keep selecting and running the CPU pack exactly as today. Old apps must never pick the CUDA pack (published as a distinct platform `linux-x86_64-cuda`, own `pack_version`; CPU entries and their signed bytes unchanged).
- S3. Quoted: "muy importante que paddle corra en gpu": verified on a real NVIDIA GPU (pop-os RTX 5090 and pop-os-notebook RTX 4060), with `nvidia-smi` showing the process and PaddleOCR-VL output text.
- S4. The app picks CUDA only when NVIDIA is detected, the driver is new enough for the pack's CUDA build, and a CUDA release exists for its app version; otherwise CPU.
- S5. Packs are tested from a staging manifest (separate release tag) via the existing `runtime_bootstrap_manifest_url` setting before the production manifest is touched.
- S6. Existing CPU runtimes are not replaced silently; Dependencias offers the switch to the GPU runtime (default decided 2026-11-24, user approved "dale, arrancá con Linux").
- S7. Windows CUDA pack is out of scope for this feature (no Windows NVIDIA machine to verify S3).

## Tasks

| ID  | Specs  | Route                       | Status                  | Evidence                                                                                   |
| --- | ------ | --------------------------- | ----------------------- | ------------------------------------------------------------------------------------------ |
| T1  | S3     | inline (spike on GPU boxes) | 5090 done, 4060 running | 5090: paddle 3.2.2 cu129, "works well on 1 GPU", OCR text OK, 1.1 s/page, 5.9 GB VRAM peak |
| T2  | S2, S4 | inline                      | done                    | 142 runtime tests; CUDA activation/reuse tests mutation-checked                            |
| T3  | S1, S5 | inline                      | in progress             | workflows edited; CI run pending                                                           |
| T4  | S3     | inline (remote)             | pending                 |                                                                                            |
| T5  | S3, S5 | inline (remote)             | pending                 |                                                                                            |
| T6  | S6     | inline                      | pending                 |                                                                                            |
| T7  | S2, S5 | inline                      | pending                 |                                                                                            |

T1 spike: paddlepaddle-gpu (cu129) + paddleocr-vl install and run on both GPUs. T2 manifest/bootstrap variant selection with tests pinning the CPU path. T3 CI builds the Linux CUDA pack and publishes to a staging manifest. T4 pack-level GPU run on both machines. T5 Pro Linux build against staging. T6 Dependencias switch. T7 PR, merge, publish to production.

## Log

- L1 (2026-11-24, user): "no, antes hagamos esto: 6. PaddleOCR-VL en GPU NVIDIA: dos packs, uno para CPU y otro para CUDA."
- L2 (2026-11-24, user): "proba todo y testea todo las veces que sea necesario, es muy importante no romper nada d elo que ya funcion ay muy importante que paddle corra en gpu"
- L3 (2026-11-24, user): "podes usar la pop os via ssh o tailscale la de rtx 4060" / "tambien via tailscale". pop-os 100.95.192.61 agusnieto77 is an RTX 5090 (Blackwell), pop-os-notebook 100.123.213.11 agustin is the RTX 4060 Laptop; both driver 580.
- L4 (2026-11-24, user): "dale, arrancá con Linux".
- L5 Mapping: manifest selects by exact (app_version, platform) (`runtime/manifest.rs:133`); wheelhouse is a repack of the build venv (`materialize_windows_runtime_payload.py:237`); managed install is offline (`deps/install.rs:977`); publish upserts by (app_version, platform) (`publish_runtime_bootstrap.py:174`); runtime dir keyed by pack_version (`paths.rs:11`).
- L6 T1 first try on 5090: uv refused to mix the Paddle cu129 index with PyPI; retrying with `--index-strategy unsafe-best-match`.
- L7 T1 on 5090 (pop-os): paddlepaddle-gpu 3.2.2 (cu129) pulls 15 nvidia-*-cu12 wheels from pip (cuDNN 9.9, cuBLAS 12.9...); `paddle.utils.run_check()` passes on Blackwell (compute 12.0); PaddleOCR-VL read the 3 test lines; venv 8.2 GB installed.
- L8 T2 design: CUDA release platform `linux-x86_64-cuda` (signed), inner runtime manifest keeps `linux-x86_64`; `BootstrapDownloadPlan.release_platform` carries the signed platform; driver >= 575 required (`runtime/accelerator.rs`); `ENTROPIA_RUNTIME_ACCELERATOR=cpu|cuda` forces the choice.
