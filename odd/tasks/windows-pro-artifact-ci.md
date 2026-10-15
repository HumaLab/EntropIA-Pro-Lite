# Windows Pro Artifact-Only CI

## Authorization and scope
User authorized a narrow workflow change, tests, work-unit commit/push and a Windows Pro artifact build (Engram 615). No tags, releases, PRs, merge, Pro guest installation or security/licence decisions. Native Lite acceptance is separately closed and published at `4cd287e19f8684d584ab0a9e38934dc19ae90c03`.

Preserve dirty `.pi/subagents.json`, `odd/tasks/internet-archive.md` and untracked `odd/tasks/windows-validation-sandbox.md`. Preserve the Windows guest, public data, backups, all diagnostic media and burned review authority. Do not rebuild Lite or run Pro locally on the real archive.

## Design
Add isolated `.github/workflows/windows-pro-validation.yml`; do not change the release workflow. An exact `feat/internet-archive` push filter AND a path filter for this new workflow trigger the authorized build; later documentation-only pushes do not retrigger it. Default-branch registration prevents relying on dispatch of a newly added branch-only workflow. GitHub documents push support for workflows not merged into the default branch and conjunctive branch/path filters.

Windows only; contents read permission; Node 22 and pnpm 9; explicit `VITE_LOCAL_ML=1` and `--features local-ml`; NSIS/MSI bundles and upload-artifact only. Reuse the existing Pro bootstrap manifest/key literals, Windows resource overlay and helper preparation (Node/pnpm/Rust/cache). Wrapper unit tests run via the Windows overlay hook only; no DEBUG cargo double-builds in this workflow. Never pass release/tag parameters, grant contents-write permission or run Lite/Unix jobs. Cold MNN compilation may take 35–60 minutes; verify actual CI result rather than infer success from a watch timeout.

## Tasks
1. [done] Implement the isolated workflow and deterministic wiring tests; observe RED before workflow creation, then GREEN. Delegate the bounded writer and independent verification. Keep tests and workflow together in one GREEN Conventional Commit; never push a failing RED checkpoint. Native review under the user-owned switch must precede the build-triggering push.
2. [done] Push the approved work unit, observe the single authorized Windows Pro run, verify artifact identities/hashes and the no-release/no-Lite job graph. Do not install the artifacts without a separate user decision.
3. [done] Record actual build/test outcomes and commit delivery evidence; keep overall W4/task 27 open until remaining IA/Pro acceptance is observed.

## Edit candidates and verification
- `.github/workflows/windows-pro-validation.yml`
- `apps/desktop/src/lib/windows-pro-validation.test.ts`
- This feature document only for progress/evidence.

Use the existing filesystem/string workflow test convention without adding dependencies or changing lockfiles. Focused desktop tests must cover both Pro and Lite test environments; applicable lint/typechecks and an independent structural/security readback remain required. Record every skipped/failed check. Existing global release behavior stays untouched.

## Delivery and review checkpoint
Local work-unit commit: `5b870827` (`ci(windows): add isolated Pro artifact validation`), exactly the workflow, test and feature document (348 added lines). Source work-unit review approved and acknowledged. Published commit `5b8708278c83255e767bac545b5c1531896f7464`; remote branch independently confirmed. Push triggered [Windows Pro validation run 38216111644](https://github.com/HumaLab/EntropIA-Pro-Lite/actions/runs/38216111644), exactly one Windows Pro job `114701120796`. Latest observation (bounded follow-up, API log response timestamp `2026-10-12T07:15:35Z`): in progress, all preparation steps including dependency installation succeeded; `Build Pro installers (NSIS + MSI)` running since `2026-10-12T07:10:20Z`. Artifact API returned zero artifacts. Job-log API returned HTTP 404/BlobNotFound and CLI explicitly reported logs unavailable until completion; these are log availability limits, not a build failure. Build, dual-installer guard, artifact upload and hashes remain NOT VERIFIED; no failed or skipped steps reported. Commit-filtered API observed only this workflow run for this SHA, no Lite/Unix/release jobs in its graph. No installer or VM operation. Native committed-range review `review-130f0a60fc03591a` covers only this work unit over `4cd287e19f8684d584ab0a9e38934dc19ae90c03`. Target `sha256:a9bdd7eb3e76fc12ac998c0d5030df1a714db6c132a52a6012fbbc2cb265cd45`; candidate tree `14a2b397507a7b83b8e349efae9d23fab77fcf72`.

The initial group failed with xAI HTTP 403 and submitted zero reviewers. User authorized MiMo v2.6 replacement; only the active `review-resilience` model was changed to `opencode-go/mimo-v2.6-pro`, effort high. Fresh exact captures admitted risk, MiMo resilience, readability and reliability. Native closure APPROVED; exact acknowledgement consumed revision `sha256:ddfd9de86d570095c33729c2e4e365fc5d81146b473c6637a0eae134a71b49bd`, authority burned. Seven advisories are non-blocking separate follow-ups (action pinning, test readability/coverage overlap, redundant env mapping, historical doc/comment wording, artifact hash baseline). Candidate-view cleanup remains deferred (`candidate-view-git-failure`); no destructive cleanup or review reopening. Task 1 is done; task 2 is active. No authority reset, review switch change or VM operation was performed.

## First hosted run result and correction
Run 38216111644 completed with FAILURE (updated `2026-10-12T07:49:45Z`). The Pro Rust build itself PASSED: `release` profile with local-ml/MNN finished in 38m 47s and produced `entropia-pro-desktop.exe`. Tauri then ran `beforeBundleCommand` (`prepare-webview2-repair.ps1` under Windows PowerShell 5.1), which downloaded the bootstrapper and failed at `Get-AuthenticodeSignature`: `CouldNotAutoloadMatchingModule` for `Microsoft.PowerShell.Security`. Guard and upload were skipped; zero artifacts. Wrapper unit tests never ran (they follow the signature check).

Root cause: the build step declared no `shell`, so the runner used pwsh 7 (log line 563). A pwsh parent prepends PowerShell 7 module paths to `PSModulePath`; the child `powershell` 5.1 inherits it and cannot load its own Security module. The Lite release job ran the identical hook through `tauri-action` (a Node process, no pwsh-modified `PSModulePath`) and passed, so the hook itself is not at fault.

Correction (workflow-only, shared hook untouched): build step declares `shell: bash`. New test asserts the exact build step uses `shell: bash` and not pwsh. RED observed (1 failed / 14 passed), then GREEN both `VITE_LOCAL_ML=1` and `0`: 29/29 across workflow, runtime-packaging and WebView2 suites; ESLint, PyYAML step readback and whitespace checks pass. Hosted confirmation requires a new run.

## Second hosted run: PASS (2026-10-14)
Commit `18aafaaef1292e576d94cc5e53049b595743397b` pushed; [run 38524775653](https://github.com/HumaLab/EntropIA-Pro-Lite/actions/runs/38524775653) concluded SUCCESS (`2026-10-14T07:39:18Z`). Commit-filtered API: exactly one run for this SHA; one job `Build Windows Pro (NSIS + MSI)`; every step success; no Lite, Unix or release job.

- Build step ran under Git Bash; `pnpm exec tauri build` resolved.
- WebView2 hook: Microsoft bootstrapper SHA256 `BAA78C71C5615533A1EE0C4D2CA2EA3FD24DAE01965F9033AC530A61B5AA93A0`, signature Valid (CN=Microsoft Corporation, thumbprint `4028CAD637509D4744B17EC5B42AED8D7A31E6AF`), file version 1.3.287.9; wrapper unit tests `16 passed; 0 failed`; `repair.exe` SHA256 `F6CF9BD3CAC8C13262B1823835CC512B1FEAEEA28C121ED0AEFBD92FBF9BA64D`.
- Guard: NSIS `EntropIA Pro_1.0.20_x64-setup.exe` 61806220 bytes; MSI `EntropIA Pro_1.0.20_x64_en-US.msi` 82567168 bytes.
- Artifact `windows-pro-installers` id `11876854268`, 143689549 bytes, zip SHA256 `f34bec70a6d0ee85946ef62f916c38f28551e16289f25817121c94b35494995e` (upload log and API digest match), head SHA `18aafaae`, not expired.

NOT VERIFIED: per-installer file SHA256 and static PE/MSI inspection. The artifact was not downloaded because host free space was 123 GiB, below the 150 GiB floor the user set. No installation, execution or VM operation.

Observation, not attributable to this run: release `v1.0.20` (non-draft) exists, created `2026-10-14T04:52:22Z`, about two hours before this run started (`06:53`). This workflow holds only `contents: read` and has no release job, so it cannot have created it.

## Current state
Task 1 closed with reviewed local work-unit commit `5b870827`; task 2 active, task 3 pending. Corrected GREEN independently observed: both `VITE_LOCAL_ML=1` and `0` targeted suites passed 28/28 tests (workflow 14, runtime packaging 5, WebView2 9); ESLint passed. Existing PyYAML parsed the exact trigger, read-only permission map, single Windows Pro job, explicit variant/build arguments and both installer guards before upload. Related release/config/hook files are unchanged. Git whitespace checks produced no diagnostics. Typechecks were not repeated after workflow/test-only corrections; earlier both environments passed with 0 errors and 8 existing warnings. Hosted build, Windows wrapper hook execution and artifact verification remain pending. Initial RED: 10 fail / 3 pass both envs. First GREEN attempt: 27 pass / 1 fail both envs — `buildRunLine` matched the `# Plain pnpm exec tauri build` comment instead of the executable `run:` step; now matches only non-comment `run:` lines. Lint 0; both frontend typechecks 0 errors with 8 preexisting warnings; no build yet. Material refinement: removed the `windows-feature-contract.ps1` step — lines 80-81 run DEBUG cargo builds for local-ml AND lean baseline (not unit tests), an extraneous double build before the 35-60 min MNN Pro build; wrapper unit tests still run fail-closed via the Windows overlay hook `prepare-webview2-repair.ps1:85-94` (`rustc --test`). Recovery: Engram `odd/windows-pro-artifact-ci/tasks`; architecture decision 634; native delivery 633.
