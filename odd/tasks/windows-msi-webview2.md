# Windows MSI WebView2 repair

## Intent and authorization

Fix the verified Lite MSI failure without removing the forced repair needed for Windows Sandbox registry false positives. The user authorized tests, commits and push on `feat/internet-archive`, and another Windows Lite-only CI build. No tags, releases, Pro build, archive reset, or backup restoration.

The failing MSI returned `0x80040828` / `-2147219416` from `ForceInvokeWebView2Bootstrapper` and rolled back with error 1722. External Microsoft EdgeUpdate logs identify this code as already installed. Preserve failure for every other nonzero code.

## Scope and design

- Keep the forced bootstrapper action, deferred execution, `Return="check"`, and `NOT REMOVE`.
- Use a small standalone native Rust wrapper with the verified Microsoft bootstrapper embedded. Normalize only the documented already-installed result to success.
- Prepare it through a Windows-only before-bundle hook using the existing Rust toolchain, static CRT, HTTPS and valid Microsoft Authenticode verification. No installer-time scripts, policy bypass or dependencies.
- Extract exclusively into a private temporary directory; fail on extraction, launch or wait errors; protect executable integrity until launch; clean up only owned files.
- Tests and installer wiring belong to one work-unit commit. Existing `.pi/subagents.json` and other task documents are excluded.

## Delivery and routing

- Branch: `feat/internet-archive`; starting boundary: `ad684a7a2ae6189c43cd2164fb8942a1924b6618`.
- Strategy: `feature-branch-chain` (user-selected for the 755-line fix; replaces `ask-on-risk` forecast). One honest semantic slicing pass, no cosmetic minification.
- Single writer: this worker prepares source work-units only. No commits/push/CI/guest/PR by this worker. Parent handles candidate boundaries/delivery and commits separate units on the work branch with immutable boundaries for later authorized chain preparation. No claim that an actual PR chain already exists.
- Feature tracker: `feat/internet-archive`; remote default `main` verified via gh API. Future PR boundaries are planning-only; PR/remote child creation NOT authorized.
- Slice (source units, actual new lines recorded in Evidence after implementation):
  - MSI-1a — secure staging module with own tests: `windows/webview2_stage.rs` (extracted cohesive native staging/security unit; standalone `--test` + imported via `#[path]`).
  - MSI-1b — checked runner / only-already-installed exit policy with own tests: `windows/webview2-repair.rs` (mapper/argv/run via `run_in_owned_stage`; guard-through-spawn/wait then dir-release-before-cleanup; 9 own Windows / 8 Linux).
  - MSI-1c — Windows pre-bundle/MSI wiring + TS regression: `scripts/prepare-webview2-repair.ps1`, `windows/fragments/force-webview2-bootstrapper.wxs`, `tauri.windows.conf.json`, `src/lib/force-webview2-bootstrapper.test.ts`.
  - MSI-2 — Lite CI provenance (parent-owned, not this worker).
  - MSI-3 — guest acceptance (parent-owned, not this worker).
- Old MSI-1 umbrella remains OPEN until sub-units verified. No task done without commits observed. Existing auth (fix/tests/commit+push/new Lite CI; not Pro/tags/releases) retained. Exclude existing unrelated `.pi/` and old task docs.

## Tasks

- [ ] MSI-1 (umbrella) — OPEN until MSI-1a/1b/1c all verified; no task done without commits observed.
- [ ] MSI-1a — Secure staging module with own tests (`windows/webview2_stage.rs`). Source COMMITTED `cf5a17d3349d9496d60344d1515b28f3ccd8702f` on base `ad684a7a2ae6189c43cd2164fb8942a1924b6618` (copied existing full base id from doc) + APPROVED native MEDIUM consolidated reliability `review-7cd79104cbc70ada` (ACK authority burned target `95ce3db22cfe8c403ea2caa1f4f8f9964fc76ba9eccb6e339e752fe584f8facf`). Actual unit 530 lines (stage 463 + doc 67), one honest cohesive oversize, cached selected feature-branch-chain, no repeated split. Checkbox remains OPEN; Windows verification delegated to MSI-2.
- [ ] MSI-1b — Checked runner / only-already-installed exit policy with own tests (`windows/webview2-repair.rs`). Source COMMITTED `6116014b31aba07610b7700c96faea361b1ed9be` on base `cf5a17d3349d9496d60344d1515b28f3ccd8702f` + APPROVED native HIGH consolidated risk/resilience/readability/reliability `review-f8a9b867c26d990c` (ACK authority burned target `b34569c17042da5935a005c73a7088bf29f2b300fe353f36100c7d4751df2adb`). Actual unit 298 lines (runner 290 + doc delta 8: 294 adds/4 deletes), cached selected feature-branch-chain, one honest slicing pass, no minification. Advisory `R2-001` WARNING (115-121) / `R2-002` SUGGESTION (151-153) / `R2-003` SUGGESTION (243) / `R2-004` SUGGESTION (253-262) / `R2-005` SUGGESTION (doc 52) non-blocking later work. Checkbox remains OPEN; Windows verification delegated to MSI-2.
- [ ] MSI-1c — Windows pre-bundle/MSI wiring + TS regression (hook path fix + structural test, fragment, conf, TS tests). Package native pending; native review and commit pending (parent-owned, COMMIT not yet).
- [ ] MSI-2 — Push the verified commit; dispatch and verify Windows Lite-only CI and installer provenance.
- [ ] MSI-3 — Install the matching MSI in the authorized guest and verify version, startup and preserved public test data.

## Acceptance and checks

- Observe meaningful RED for the currently fatal already-installed result, then GREEN for success, that result, adjacent/unrelated nonzero codes and operational failures.
- Preserve Sandbox forced execution; registry presence alone must not skip repair.
- Structural checks verify the hook, binary source and checked action wiring. Run focused and applicable package tests.
- Windows CI must compile/test the helper, build Lite NSIS/MSI, and create no tag or release.
- Guest UUID: `d9e70790-1faa-488b-a23a-a622d99cb202`. Keep all existing media and the verified guest-only backup `EMSI-Verified-20261011-051221` unchanged.
- Guest acceptance: installed MSI identity/version, functioning existing WebView2, 4 collections/14 documents, selected IA PDF and read-only SQLite integrity. Stop at unexpected consent, reboot, security decisions or data inconsistency.
- No claim of bit-identical pre/post uninstall data: previous baseline captured file sizes, not source hashes. The coherent backup is independently verified.

## Evidence and next step

MSI-1b source COMMITTED+APPROVED milestone (commit `6116014b31aba07610b7700c96faea361b1ed9be`; review `review-f8a9b867c26d990c`), but MSI-1 umbrella remains OPEN until MSI-1a/1b/1c all Windows-verified via MSI-2. Running authored 530+298=828 after 2 commits. Current reviewed boundary 6116 for 1c range only; exclude unrelated `.pi`/older docs; all future PR mechanics deferred, PR creation unauthorized. MSI-1c commit/review PENDING; all units Windows native checks await MSI-2 so checkboxes stay OPEN. Independent local: full Lite 3553 passed / 28 skipped; stage 6 / helper 14 Linux; Windows native 16 EXPECTED unrun. Post-ACK cache cleanup deferred (read-only inspection valid clean registered view, tree HEAD 6116, no active OS lock); preserve, no reset/remove. Advisory IDs `R3-001` WARNING (stage 229-320) / `R3-002` SUGGESTION (60-70) / `R3-003` SUGGESTION (131-136) NOT blocking / NOT correction / reopen. Post-ACK: candidate cache cleanup deferred (RO incident); valid clean registered tree `b6c..`, directories 555, no Locks, HEAD unchanged; preserve view fresh INSPECT before next START; not authority corruption, no reset/cleanup permission. Independent FINAL local checks: stage 6 / runner 14 / full Lite 3553 passed, 28 skipped. Windows native 16 EXPECTED not observed until MSI-2 CI. No commits/push/CI/guest/PR by this worker; parent owns boundaries/delivery. Source work-units only on feat current branch for later authorized feature-chain PR preparation; no actual PR created/permission implied. Document immutable reviewed boundaries, not accumulated branch. No PR chain claimed. Exclude `.pi/` and old task docs. Prior baseline 710+45=755 retained for sizing; forecast ~360 failed (honest oversize, no minification).

Focused verification (foreground, observed):
- `rustc --edition=2021 --test apps/desktop/src-tauri/windows/webview2_stage.rs -o /tmp/entropia-webview2-stage-tests && /tmp/entropia-webview2-stage-tests`: 6/6 PASS (Linux portable; 2 Windows-only lock/hold tests cfg-gated for CI).
- `rustc --edition=2021 --test apps/desktop/src-tauri/windows/webview2-repair.rs -o /tmp/entropia-webview2-repair-tests && /tmp/entropia-webview2-repair-tests`: 14/14 PASS Linux (8 own + 6 stage); Windows EXPECTED 16 (9 own + 7 stage, unix 0700 excluded). Direct-run: portable invalid-exe 1603 + dir gone + sibling kept; Windows-only self-exe `--list` code 0 + dir gone. Native RED unavailable on Linux (POSIX removes held dir); source ordering `drop(dir_hold)` before `cleanup_owned_stage` is Windows proof.
- `VITE_LOCAL_ML=0 pnpm --filter @entropia-pro/desktop test -- src/lib/force-webview2-bootstrapper.test.ts`: RED 1 failed/7 passed (frontend-relative -File), then GREEN 8/8 after src-tauri/scripts fix.
- `git diff --check`: clean.
- No full Tauri build locally; Windows CI pending (explicit). Full Lite 3552-pass suite not rerun here (not claimed).

Units (one honest semantic split):
- MSI-1a (463 lines new): `windows/webview2_stage.rs` — cohesive staging/security + own tests; CreateDirectoryW with SA from creation, SDDL SY/BA/OW/CO protected, dir-hold no-delete, POSIX 0700 fallback, owned-only cleanup. Smallest cohesive >400; best reported without artificial split.
- MSI-1b (290 lines, was 232): `windows/webview2-repair.rs` — #[path] stage import, 0x80040828-only->0 preserves 3010/others, `run()` + private `run_in_owned_stage()` with payload-then-dir release before owned cleanup, 8 own Linux / 9 own Windows + direct-run regression.
- MSI-1c (TS 147 lines +21, conf 1-line fix): `tauri.windows.conf.json` frontend-relative hook, `force-webview2-bootstrapper.test.ts` structural resolve+existsSync+PSScriptRoot, plus unchanged hook `prepare-webview2-repair.ps1` (116) and `force-webview2-bootstrapper.wxs` (71).
- Diff-hunk partition for parent commits: hunk1 MSI-1a new stage file; hunk2 MSI-1b repair rewrite+import; hunk3 MSI-1c conf+TS (ps1/wxs provenance unchanged). Immutable boundaries for later authorized chain prep only.

Blocking fixed: hook cfg now `src-tauri/scripts/...`; structural test proves existence from frontend CWD. Security tests added: real staging+exact bytes, collision-keeps, trailing-byte, owned-only, POSIX 0700; Windows read-lock deny write/delete + dir-hold rename cfg(windows) pending live run. Post-create ACL race unproven — hardened via creation-time SA, review point. No new crates/lockfiles, no install scripts/cert overrides/secrets; cfg(test) synthetic payloads only.
