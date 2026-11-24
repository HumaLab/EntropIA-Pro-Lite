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

| ID  | Specs  | Route                       | Status      | Evidence                                                                                        |
| --- | ------ | --------------------------- | ----------- | ----------------------------------------------------------------------------------------------- |
| T1  | S3     | inline (spike on GPU boxes) | done        | 5090: 1.1 s predict, 5.9 GB peak; 4060 (8 GB): 1.7 s predict, 5.9 GB peak, 3/3 lines read       |
| T2  | S2, S4 | inline                      | done        | 142 runtime tests; CUDA activation/reuse tests mutation-checked                                 |
| T3  | S1, S5 | inline                      | in progress | run 45469847198 green: 7,399,271,027 B, smoke + offline install OK; staging publish 45473940682 |
| T4  | S3     | inline (remote)             | pending     |                                                                                                 |
| T5  | S3, S5 | inline (remote)             | pending     |                                                                                                 |
| T6  | S6     | inline                      | in progress | offer + switch + hardware-aware discovery; RED observed (3 Rust, 3 Vitest)                      |
| T7  | S2, S5 | inline                      | pending     |                                                                                                 |

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
- L9 T3 first CI run 45468055791 failed in warm-up: paddlepaddle-gpu cannot import without `libcuda.so.1` (runner has no GPU). Spike on 5090: the CUDA 12.9 driver stub (`cuda-driver-dev-12-9_12.9.79-1`, ubuntu2204, sha256 pinned) makes `import paddle` work, but `PaddleOCRVL(device='cpu')` still queries the GPU capability (`paddlex/.../_siglip.py:146`, CUDA error 34). Fix: stub only for import/install checks, weights warmed in a throwaway CPU venv, smoke install probe installs the Paddle build the wheelhouse ships.
- L10 T6 design: a healthy CPU runtime is never replaced by `ensure_ready` (pinned by `a_cpu_runtime_stays_on_a_cuda_machine_until_the_user_switches`). `runtime_get_gpu_offer` returns the CUDA release when the machine is CUDA, a CUDA pack is published and the active runtime is not it; `runtime_switch_to_gpu_start` runs the bootstrap worker with `RuntimeManager::for_gpu_switch()`, which plans past the healthy runtime; the after-ready sweep retires the CPU folder. Discovery sorts runtime folders and prefers the one matching the machine (`RuntimeManifest::ships_cuda_paddle`, the wheel the installer picks). The venv lives per runtime, and stale Python paths are already forgotten (`deps/mod.rs:459`).
- L11 Full Pro suite (bzw6lrv52) during heavy load: 3995 passed, 74 failed (automation server HTTP, subprocess and timing tests); to be rerun in isolation.
- L12 T1 4060: the first run stalled 33 min at "Fetching 20 files 19/20" (a stuck model download, GPU 0 %, 7.2 GB VRAM reserved). Rerun offline on the cached weights: pipeline ready 4.7 s, predict 1.7 s, 3/3 lines, peak 5,944 MB allocated (6.6 GB reserved) on the 8 GB card.
- L13 T3 run 45469847198 green: archive 7,399,271,027 bytes (6.89 GB, split into parts by the publisher), sha256 e9aefe87103185ed8cd6a99e536b57081f208ac121539bb935348f151e3f8289, smoke incl. offline install probe ok.
- L14 T6: the regular sweep keeps the CPU runtime after the switch (it still serves the running app). `sweep::retire_replaced_runtime` retires exactly the runtime the switch replaced, skipping one a running program uses.
