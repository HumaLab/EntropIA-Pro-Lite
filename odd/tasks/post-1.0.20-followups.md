# Post-1.0.20 follow-ups

Follow-ups left after shipping v1.0.20. Order chosen by the user: 2, 3, then 1.

- [x] 1. Release gate fetch timeout — `apps/desktop/scripts/check-runtime-bootstrap.mjs` fetches
      the manifest with no timeout (advisory of RDD `review-c8d8c5066ab8e3a0`, Phase 6.4b). A hung
      GitHub response stalls the release job until the job timeout. Bound it with
      `AbortSignal.timeout` (exit 2, like any fetch error) and a `--timeout-ms` option for tests.
- [x] 2. Runtime-unavailable message — `DependenciasTab.svelte` should tell "no published runtime
      for this version/platform" apart from "the manifest could not be read".
- [ ] 3. Pro on Linux — planned (see below): a linux-x86_64 runtime pack (no `materialize_linux_*`
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

## Plan for 3 (Pro on Linux)

The app side is about 90% portable already (`runtime/paths.rs`, `nlp/embeddings.rs` ort lookup,
`deps/install.rs` cfg(windows) shims); `publish_runtime_bootstrap.py` and the manifest already handle
several platforms; `release.yml` keeps the Linux deps step, the Pdfium fetch and
`tauri.pro.linux.conf.json`.

- [x] 3.0 Validate: `prepare_runtime_payload.py --fixture --platform linux-x86_64` plus
      `runtime-pack-smoke.py` on ubuntu-22.04; fix the executable bit of `python/bin/python3` and
      `uv/bin/uv` in the linux fixture.
- [ ] 3.1 Linux pack builder: `build-runtime-pack.yml` gains a linux-x86_64 job on ubuntu-22.04
      (relocatable python-build-standalone 3.11, uv linux tarball, `fetch-pdfium.sh linux-x64`,
      `libonnxruntime.so` matching `ort =2.0.0-rc.10`, the same four `uv pip install` specs, warmed
      caches, `build_runtime_pack.py --platform linux-x86_64 --require-release-payload`, smoke with
      `--install-probe`). Generalize `materialize_windows_runtime_payload.py` or add a linux sibling.
- [ ] 3.2 Publish: `publish-runtime-bootstrap.yml` takes a `platform` input (today
      `windows-x86_64` is hardcoded at lines 49-54); the manifest then holds both entries.
- [ ] 3.3 Release: re-add the ubuntu-22.04 Pro matrix entry with `runtime_platform: linux-x86_64`
      and the `tauri.pro.linux.conf.json` build args; smoke the `.deb` on a clean runner.
- [ ] 3.4 Rust on Linux: a Pro Linux CI job (clippy `--features local-ml`), clear the 9 known
      findings in `deps/uv.rs` and `llm/download.rs`, MNN-from-source toolchain on the runner.

Risks: `libonnxruntime.so` version vs `ort` rc.10; paddlepaddle GPU wheels are not on PyPI
(ship CPU first); MNN source build time; pack size (the split path already exists). Size: M,
about 1-2 weeks, mostly CI iteration.
