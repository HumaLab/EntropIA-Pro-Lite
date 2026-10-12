# Windows Pro Artifact-Only CI

## Authorization and scope
User authorized a narrow workflow change, tests, work-unit commit/push and a Windows Pro artifact build (Engram 615). No tags, releases, PRs, merge, Pro guest installation or security/licence decisions. Native Lite acceptance is separately closed and published at `4cd287e19f8684d584ab0a9e38934dc19ae90c03`.

Preserve dirty `.pi/subagents.json`, `odd/tasks/internet-archive.md` and untracked `odd/tasks/windows-validation-sandbox.md`. Preserve the Windows guest, public data, backups, all diagnostic media and burned review authority. Do not rebuild Lite or run Pro locally on the real archive.

## Design
Add isolated `.github/workflows/windows-pro-validation.yml`; do not change the release workflow. An exact `feat/internet-archive` push filter AND a path filter for this new workflow trigger the authorized build; later documentation-only pushes do not retrigger it. Default-branch registration prevents relying on dispatch of a newly added branch-only workflow. GitHub documents push support for workflows not merged into the default branch and conjunctive branch/path filters.

Windows only; contents read permission; Node 22 and pnpm 9; explicit `VITE_LOCAL_ML=1` and `--features local-ml`; NSIS/MSI bundles and upload-artifact only. Reuse the existing Pro bootstrap manifest/key literals, Windows resource overlay and helper preparation (Node/pnpm/Rust/cache). Wrapper unit tests run via the Windows overlay hook only; no DEBUG cargo double-builds in this workflow. Never pass release/tag parameters, grant contents-write permission or run Lite/Unix jobs. Cold MNN compilation may take 35–60 minutes; verify actual CI result rather than infer success from a watch timeout.

## Tasks
1. [in_progress] Implement the isolated workflow and deterministic wiring tests; observe RED before workflow creation, then GREEN. Delegate the bounded writer and independent verification. Keep tests and workflow together in one GREEN Conventional Commit; never push a failing RED checkpoint. Native review under the user-owned switch must precede the build-triggering push.
2. [pending] Push the approved work unit, observe the single authorized Windows Pro run, verify artifact identities/hashes and the no-release/no-Lite job graph. Do not install the artifacts without a separate user decision.
3. [pending] Record actual build/test outcomes and commit delivery evidence; keep overall W4/task 27 open until remaining IA/Pro acceptance is observed.

## Edit candidates and verification
- `.github/workflows/windows-pro-validation.yml`
- `apps/desktop/src/lib/windows-pro-validation.test.ts`
- This feature document only for progress/evidence.

Use the existing filesystem/string workflow test convention without adding dependencies or changing lockfiles. Focused desktop tests must cover both Pro and Lite test environments; applicable lint/typechecks and an independent structural/security readback remain required. Record every skipped/failed check. Existing global release behavior stays untouched.

## Current state
Task 1 still in_progress pending native review and commit; tasks 2-3 pending. Corrected GREEN independently observed: both `VITE_LOCAL_ML=1` and `0` targeted suites passed 28/28 tests (workflow 14, runtime packaging 5, WebView2 9); ESLint passed. Existing PyYAML parsed the exact trigger, read-only permission map, single Windows Pro job, explicit variant/build arguments and both installer guards before upload. Related release/config/hook files are unchanged. Git whitespace checks produced no diagnostics. Typechecks were not repeated after workflow/test-only corrections; earlier both environments passed with 0 errors and 8 existing warnings. Hosted build, Windows wrapper hook execution and artifact verification remain pending. Initial RED: 10 fail / 3 pass both envs. First GREEN attempt: 27 pass / 1 fail both envs — `buildRunLine` matched the `# Plain pnpm exec tauri build` comment instead of the executable `run:` step; now matches only non-comment `run:` lines. Lint 0; both frontend typechecks 0 errors with 8 preexisting warnings; no build yet. Material refinement: removed the `windows-feature-contract.ps1` step — lines 80-81 run DEBUG cargo builds for local-ml AND lean baseline (not unit tests), an extraneous double build before the 35-60 min MNN Pro build; wrapper unit tests still run fail-closed via the Windows overlay hook `prepare-webview2-repair.ps1:85-94` (`rustc --test`). Recovery: Engram `odd/windows-pro-artifact-ci/tasks`; architecture decision 634; native delivery 633.
