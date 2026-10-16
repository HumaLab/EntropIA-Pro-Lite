# Post-1.0.20 follow-ups

Follow-ups left after shipping v1.0.20. Order chosen by the user: 2, 3, then 1.

- [x] 1. Release gate fetch timeout — `apps/desktop/scripts/check-runtime-bootstrap.mjs` fetches
      the manifest with no timeout (advisory of RDD `review-c8d8c5066ab8e3a0`, Phase 6.4b). A hung
      GitHub response stalls the release job until the job timeout. Bound it with
      `AbortSignal.timeout` (exit 2, like any fetch error) and a `--timeout-ms` option for tests.
- [x] 2. Runtime-unavailable message — `DependenciasTab.svelte` should tell "no published runtime
      for this version/platform" apart from "the manifest could not be read".
- [x] 3. Pro on Linux — planned (see below): a linux-x86_64 runtime pack (no `materialize_linux_*`
      exists; `build-runtime-pack.yml` is Windows-only) and re-adding the Pro Linux release leg.

## Evidence

- 1: RED 2 failed / 7 passed (`unknown argument: --timeout-ms`), GREEN 9 / 0 for
  `runtime-bootstrap-gate.test.ts` (21 / 0 with `runtime-packaging.test.ts`); lint 0, Lite
  typecheck 0; live gate against the published manifest still exits 0 for 1.0.20.
- 2: commit `4ed9e6b2`. Typed `BootstrapUnavailableReason` on the plan (`unavailableReason`) and
  the status (`blockedReason`); Dependencias shows "Esta versión de EntropIA Pro todavía no tiene su
  runtime publicado." for `no_matching_release`. RED 1 failed / 21 passed (vitest) plus Rust tests
  that did not compile on the base; GREEN vitest 32 / 0 / 1, Lite and Pro typecheck 0, lint 0, lean
  clippy clean, lean `cargo test --lib` 2580 / 0 / 14. Pro (`local-ml`) tests not run locally (MNN);
  CI Windows contract covers them. The 6 `db::` failures seen locally were a CRLF working copy of
  `0036_writing_journal.sql` (`core.autocrlf=true` in this clone), not this change.
- 3.0: `prepare_runtime_payload.py --fixture --platform linux-x86_64` plus `build_runtime_pack.py --payload-root` and the smoke pass on Linux, and the fixture-only Linux pack passes too. The real defect: the committed Linux fixture `python/bin/python3` and `uv/bin/uv` were `100644`, so the assembled pack shipped non-executable interpreters while the smoke still said ok. Fix: the smoke now reports `<key> is not executable: <path>` for the Linux interpreters on POSIX hosts, and both files are now `100755`. RED: the new smoke test (exit 0 on 0644 interpreters) failed, and once the check landed the bundled-fixture test failed with both contract errors. GREEN: `test_runtime_pack_scripts.py` 13/13, now also run by CI's `runtime-pack-smoke` step, which never ran it before; CI smoke Linux and Windows ok; `runtime-packaging.test.ts` plus `ci-supply-chain.test.ts` 26/26; `binaries:check` OK.
- 3.1/3.2: I generalized the materializer instead of adding a sibling. `materialize_windows_runtime_payload.py --platform linux-x86_64` uses a per-platform layout: `python/bin/python3`, `uv/bin/uv` (marked executable) and `libpdfium.so` plus `libonnxruntime.so`. Windows stays the default, so the existing job is unchanged. `build-runtime-pack.yml` takes a `platform` input (default `windows-x86_64`) and gains a `build-linux` job on ubuntu-22.04. That job uses uv's managed, relocatable CPython 3.11; the same four specs; warmed caches; the pinned `fetch-pdfium.sh linux-x64`; `libonnxruntime.so` from the venv's own onnxruntime wheel; and uv 0.6.14 checked against Astral's `.sha256` (`0aaf451c…`). It then runs `--require-release-payload`, smoke `--release --install-probe`, and uploads `runtime-archive-linux-x86_64`. `publish-runtime-bootstrap.yml` takes the same `platform` input and picks the matching artifact; the manifest upsert is keyed by (app, platform), so Windows entries are kept. The zip need not carry execute bits, because the app restores them from the manifest (`ensure_executable_bit`). RED: the new materializer tests (unknown `platform` kwarg and `platform_layout`) and the workflow contract test failed. GREEN: script tests 15/15, and `runtime-packaging.test.ts` plus `ci-supply-chain.test.ts` 27/27. Not yet run for real: the Linux build job, a manual dispatch of about 60–90 min.
- 3.4: The 9 Linux-only `clippy --features local-ml --all-targets` findings (RED: exit 101) had three causes. First, six `#[cfg(any(windows, test))]` items in `deps/uv.rs` that only Windows code uses: the download URLs, timeouts and attempt count, plus `download_url`. They are now `#[cfg(windows)]`. Second, a `mut` and a `return` in `UvBinary::command` and `download` that only Windows needed; they are now split per-cfg blocks. Third, a `panic!` followed by unreachable assertions in an `llm::download` test; the Windows assertions now sit in their own block. A new CI job `rust-pro-linux` (ubuntu-22.04, apt `cmake clang libclang-dev`, its own `pro-linux` rust-cache key, gated on Rust changes) runs that clippy so the gap cannot reopen. GREEN: local-ml clippy on Linux 0, Lite clippy 0, rustfmt 0, `deps::uv` plus `llm::download` tests with local-ml 22/22, and `runtime-packaging.test.ts` plus `ci-supply-chain.test.ts` 28/28 (new contract test RED first). The Windows side of the cfg split is proven by the Windows contract job in PR CI.
- 3.1 live and 3.3: Build Runtime Pack run 38828126885 (linux-x86_64, main@`0dad72af`) succeeded in 18 min. It produced Python 3.11.17 and uv 0.6.14, 4905 python files, 129 wheels, 95 cache files and 6 native assets. The release smoke ran its install probe for real: paddlepaddle 3.2.2, paddleocr 3.7.0, faster-whisper 1.2.1 and spaCy 3.8.16 installed offline from the wheelhouse and imported, and `es_core_news_md` loaded. The probe fetched that model wheel by URL, the same as it does on Windows. Archive: 2614720601 B (2.44 GB, split on publish), sha256 `37e82b64c5d21897cee0d12c0e6d915f0ef252cb33fe9cc2e4be5c0b9c5c12ed`, artifact `runtime-archive-linux-x86_64` id 12017245251. Publish Runtime Bootstrap run 38830678932 published (1.0.20, linux-x86_64). The signed manifest (generated 2026-10-16T00:43:00Z) still holds the five Windows pairs. Verified from outside: both 1.0.20 signatures validate against the baked public key, part000 is 1992294400 B and part001 622426201 B (sum = archive size), and `check-runtime-bootstrap.mjs` exits 0 for both linux-x86_64 and windows-x86_64. `release.yml` puts the Pro Linux leg back in the tag and `all` matrices (`runtime_platform: linux-x86_64`, `tauri.pro.linux.conf.json --features local-ml --bundles deb`), each leg behind its own fail-closed runtime gate with fail-fast off; the default dispatch stays Windows-only. The READMEs now list Pro Linux `.deb`. RED: the inverted matrix assertions in `runtime-packaging.test.ts`. GREEN: 28/28 with `ci-supply-chain.test.ts`. Still open: a clean-machine smoke of the Pro `.deb` happens on the next tag's draft release, as for every Pro build before 1.0.20.

## Plan for 3 (Pro on Linux)

The app side is about 90% portable already (`runtime/paths.rs`, `nlp/embeddings.rs` ort lookup,
`deps/install.rs` cfg(windows) shims); `publish_runtime_bootstrap.py` and the manifest already handle
several platforms; `release.yml` keeps the Linux deps step, the Pdfium fetch and
`tauri.pro.linux.conf.json`.

- [x] 3.0 Validate: `prepare_runtime_payload.py --fixture --platform linux-x86_64` plus
      `runtime-pack-smoke.py` on ubuntu-22.04; fix the executable bit of `python/bin/python3` and
      `uv/bin/uv` in the linux fixture.
- [x] 3.1 Linux pack builder: `build-runtime-pack.yml` gains a linux-x86_64 job on ubuntu-22.04
      (relocatable python-build-standalone 3.11, uv linux tarball, `fetch-pdfium.sh linux-x64`,
      `libonnxruntime.so` matching `ort =2.0.0-rc.10`, the same four `uv pip install` specs, warmed
      caches, `build_runtime_pack.py --platform linux-x86_64 --require-release-payload`, smoke with
      `--install-probe`). Generalize `materialize_windows_runtime_payload.py` or add a linux sibling.
- [x] 3.2 Publish: `publish-runtime-bootstrap.yml` takes a `platform` input (today
      `windows-x86_64` is hardcoded at lines 49-54); the manifest then holds both entries.
- [x] 3.3 Release: re-add the ubuntu-22.04 Pro matrix entry with `runtime_platform: linux-x86_64`
      and the `tauri.pro.linux.conf.json` build args; smoke the `.deb` on a clean runner.
- [x] 3.4 Rust on Linux: a Pro Linux CI job (clippy `--features local-ml`), clear the 9 known
      findings in `deps/uv.rs` and `llm/download.rs`, MNN-from-source toolchain on the runner.

Risks: `libonnxruntime.so` version vs `ort` rc.10; paddlepaddle GPU wheels are not on PyPI
(ship CPU first); MNN source build time; pack size (the split path already exists). Size: M,
about 1-2 weeks, mostly CI iteration.
