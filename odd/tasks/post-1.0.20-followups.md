# Post-1.0.20 follow-ups

Follow-ups left after shipping v1.0.20. Order chosen by the user: 2, 3, then 1.

- [x] 1. Release gate fetch timeout — `apps/desktop/scripts/check-runtime-bootstrap.mjs` fetches
      the manifest with no timeout (advisory of RDD `review-c8d8c5066ab8e3a0`, Phase 6.4b). A hung
      GitHub response stalls the release job until the job timeout. Bound it with
      `AbortSignal.timeout` (exit 2, like any fetch error) and a `--timeout-ms` option for tests.
- [ ] 2. Runtime-unavailable message — `DependenciasTab.svelte` should tell "no published runtime
      for this version/platform" apart from "the manifest could not be read".
- [ ] 3. Pro on Linux — plan only first: a linux-x86_64 runtime pack (no `materialize_linux_*`
      exists; `build-runtime-pack.yml` is Windows-only) and re-adding the Pro Linux release leg.

## Evidence

- 1: RED 2 failed / 7 passed (`unknown argument: --timeout-ms`), GREEN 9 / 0 for
  `runtime-bootstrap-gate.test.ts` (21 / 0 with `runtime-packaging.test.ts`); lint 0, Lite
  typecheck 0; live gate against the published manifest still exits 0 for 1.0.20.
