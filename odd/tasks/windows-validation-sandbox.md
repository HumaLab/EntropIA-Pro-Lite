# Isolated Windows validation sandbox

Provision a real Windows guest for today's EntropIA Windows acceptance without
exposing the shared archive. User authorized tools, evaluation registration,
new-guest disk deletion and the displayed Windows licence separately.

## Scope and constraints

- Use existing direct QEMU/KVM, UEFI Secure Boot and emulated TPM2. No Wine-based
  acceptance substitution; no unnecessary virt-manager/libvirt installation.
- VM/media live outside the repository under `~/.local/share/entropia-windows-test/`.
- Official Microsoft media only; no fabricated registration, activation/TPM bypass
  or licence acceptance without explicit human decision. Registration personal
  data must not enter the repository, logs, screenshots or Engram.
- No host home/archive mounts, shared clipboard or public management ports. NAT.
- 8 vCPUs / 16 GiB RAM, 200 GiB sparse guest disk; preserve 150 GiB host-free.
- System privilege authentication uses the system dialog; never log credentials.
- Source/lockfiles, existing worktrees and native review authority stay untouched.
- No commits, pushes or publishing without explicit user request.

## Tasks

- [x] W1. Install and verify QEMU/KVM, OVMF, TPM and graphical tooling.
  Route: delegated provisioner plus independent read-only verifier. No meaningful
  unit RED for host package installation; verify real binaries, firmware and KVM.
- [x] W2. Obtain official Windows media and verify its published checksum.
  Route: delegated registration/download plus independent integrity verifier.
  User-authorized registration submitted once; no marketing or partner sharing.
- [x] W3. Install and boot isolated Windows guest to the desktop.
  Status: functional checks passed; tracking uncommitted per user constraint. Installation/copy/reboots succeeded after explicit licence
  acceptance. Argentina / Spanish (Spain) selected, second keyboard omitted.
  First guest halted at unexpected SAP branding. Authorized second blank guest
  with unique identities installed successfully and shows no SAP branding.
  User authorized disposable local EntropiaTest and random private password.
  Name/password and three explicitly authorized random recovery answers accepted.
  Optional privacy disabled, diagnostics Required only. Desktop/local-account/
  domain/network/runtime isolation independently verified; evidence below.
- [ ] W4. Validate a current-source Lite EXE/MSI first; cover Pro separately.
  Status: in progress; W4 NOT done and Windows as a whole is NOT marked complete. Current 2026-10-11: corrected Lite MSI 335c functional PASS observed (full UI install, 1.0.20, startup verified, 4 collections and 14 documents preserved); evidence committed `e4dc60a1981f738c6d6ca025917752ed11ce7f7e` on unchanged source 3b (doc commits local UNPUSHED). MSI feature PASS; W4 still OPEN. Historical pre-fix MSI FAILED (1722-type) is preserved below as history. Still OPEN: Pro with Rust Pdfium integration, additional IA cases, and the child-asset DB size follow-up (DB reads 0 vs 1373 bytes on disk). Native review cleanup stays deferred with read-only views.
  Historical routing: Lite-only CI run 38094631750 matches current HEAD. After a bounded 1200-second watch, NSIS/MSI
  build still running; installer artifact absent at that time. Update 2026-10-11: expected run 38094631750 success + artifact EntropIALite-installers ID 11686085054 (51,453,521 bytes, unexpired) to be re-queried before download; bounded unit covers download/provenance/read-only transfer/NSIS install/startup, STOP before full IA acceptance for independent verifier. Windows application not tested.
  IA is not gated by local-ml, so Lite suffices for today's IA/WebView2/installer/
  Pdfium acceptance. Require installer source provenance and actual results.
  Old unsigned MSIX 1.0.5 does not prove current 1.0.20 IA behavior. Pro/local-ML
  acceptance remains separate; avoid lengthy MSVC/MNN setup until required.

## Host and installed-tool evidence

Pop!_OS 22.04 / amd64; Ryzen 9 9950X (32 threads), 123 GiB RAM, initially 89 GiB
available and approximately 406 GiB disk-free. Installed via pkexec/apt:
`qemu-system-x86 qemu-utils ovmf swtpm swtpm-tools qemu-system-gui`.
Final transaction: 25 new packages, no upgrades/removals. Provisioner and independent
verifier confirmed six packages installed, QEMU 6.2.0, swtpm 0.6.3, OVMF Secure Boot
code/Microsoft-enrolled variable templates and TPM2 support. KVM API 12 and empty-VM
creation passed. GTK listed; initial `ui-ui-none` display-list warning did not stop
later actual graphical guest boot. No product-source/lockfile changes from setup.

## W2 evidence

Single authorized registration POST returned HTTP 200, formId and no error; both
commercial preferences rejected, no retry. No personal registration values recorded.
Official selection: https://www.microsoft.com/en-us/evalcenter/download-windows-11-enterprise
English x64 link: https://go.microsoft.com/fwlink/?LinkId=2382600&clcid=0x409&culture=en-us&country=us
Official CDN ISO: `26300.9457.260913-1737.26h2_ge_release_svc_refresh_CLIENTENTERPRISEEVAL_OEMRET_x64FRE_en-us.iso`.
Size: 8,225,329,152 bytes. SHA-256:
`bc3f24086ebadc94489066b5ad78089e2cf5c3491e90e790bb81a2b199c10e38`.
Media saved under the external `media/` directory. Independent verifier confirmed
matching Microsoft's exact Enterprise Eval x64 EN-US row in saved
`VerifyWindowsISO-KB5130867.html`, plus bootable ISO signature. Suspected saved-page
mutation was retracted after repeated stable byte/hash/stat checks.

## W3 evidence and current state

VM: `~/.local/share/entropia-windows-test/vm-20261010-163943-w3/`.
Owner-only launcher/state/sockets. Q35/SMM, host CPU, SATA disk/DVD, e1000e NAT,
read-only `OVMF_CODE_4M.secboot.fd` and private copy of `OVMF_VARS_4M.ms.fd`, TPM2.
QMP confirmed KVM enabled/present and TPM emulator. Only guest disk and readonly
verified ISO attached; no host disks/archive shares. Q35 required qemu-xhci keyboard.
User confirmed Install Windows 11 and new-disk deletion, then explicitly accepted
Microsoft Software License Terms / Windows Operating System, updated April 2024.
Licence screenshots `shots/license-p1.png` and `shots/lp-*`; imperfect OCR in
`logs/license-terms-ocr.txt`. Parent inspected setup and first licence screenshots.
Worker pressed Accept once, selected sole Drive 0 Unallocated 200 GB and observed
recap Windows 11 Enterprise Evaluation / Keep nothing. Copy and automatic reboots
completed; Windows Boot Manager booted from guest disk without another CD keypress.
Disk grew from approximately 393 KB to 16.1 GB physical; host-free approximately
384 GiB at handoff, reserve maintained.
QEMU PID 1648596 and swtpm PID 1648601 running at handoff. User selected Argentina,
Spanish (Spain) and Skip second keyboard. Current `shots/oobe-account.png` shows
SAP-branded work/school sign-in and SAP IT-support text. No account/domain/privacy/
PIN decisions or credentials entered; no desktop yet. No product key requested.
Independent verifier matched saved and live QMP screen to the owned guest, and
matched ISO checksum against fresh official Microsoft publication. QMP UUID is
all-zero; launcher uses generic MAC/disk identities with no explicit UUID/SMBIOS.
No backing image or external provisioning files found. Cause unknown: hardware
identity/Autopilot/cached responses are hypotheses, not established enrollment.
Live guest filesystem/logs have not been inspected. No security/activation bypass.

## Delivery and next step

Delivery strategy: focused tracking document, forecast below 150 authored lines,
no product code planned, no commit requested. Native assessment was unavailable
for the new untracked tracking document; independent environment checks passed.
No new review START attempted. Existing Internet Archive dialog/Pdfium/cleanup
follow-ups remain separately pending in `odd/tasks/internet-archive.md`.

Keep the unexpected corporate sign-in halted; never enter registration or account
credentials there. User explicitly authorized a second diagnostic VM with unique
UUID/device identities, preserving the current guest. Create a genuinely blank
disk, private fresh firmware variables and TPM state; reuse verified media only,
not installed guest state. A new VM is a diagnostic option, not a proven fix. Complete desktop/organization-independence/isolation
verification before closing W3. Obtain a current Lite installer with provenance
before claiming W4 acceptance; Pro remains pending.

## Second diagnostic guest evidence (2026-10-10 ~19:00 UTC)

New dir: `~/.local/share/entropia-windows-test/vm-20261010-211936-unique/`.
QEMU PID 1656972, swtpm PID 1656977, both running alongside the preserved first
guest (1648596/1648601, disk/state untouched). UUID
`d9e70790-1faa-488b-a23a-a622d99cb202` (QMP query-uuid match, nonzero v4 vs
first guest all-zero); MAC `02:51:45:48:8c:ae`; SMBIOS
`EntropIATest2-0AD08449`/`ET2B-8C1D43`; disk serial `ET2DISK419475`. KVM
enabled+present, TPM emulator, fresh TPM state, private OVMF_VARS copy matching
the MS template hash. Blank 200 GiB sparse qcow2, no backing/clone. query-block
shows only the new disk plus the readonly ISO, whose SHA-256 was re-verified
(`bc3f...c10e38`) before boot. Host-free approximately 366 GiB.

Install (same authorized decisions): CD-boot keypress, en-US defaults, then an
accidental Tab onto the Previous-Version-of-Setup link continued the install via
the legacy setup UI on the same media. Its license screen shows the identical
April 2024 Microsoft Software License Terms / Windows Operating System text
(binding arbitration section 11), visually compared against the first guest's
`shots/license-p1.png`; checkbox Accept applied once under the existing
authorization. Custom install on the sole Drive 0 Unallocated 200 GB; copy and
reboots completed with no CD keypress (disk 196 KiB to approximately 18 GB).
OOBE: Argentina via 11xDown, exact Spanish (Spanish Variation and Latin American
visibly unselected), Skip second layout, update check passed.

Diagnostic endpoint (`shots/oobe-account.png`): the blank unique-identity guest
stops at generic "Let's set things up for your work or school" Microsoft
sign-in (`someone@example.com`, Sign-in options, Next). No SAP branding. SAP did
NOT reproduce; nothing entered, Next untouched. This is one hypothesis-test data
point, not causal proof. W3 remains partial: no desktop boot yet on either guest.
Independent verifier confirmed second guest UUID/KVM/TPM/block devices and no
host shares; host-free about 365 GiB. Both guests remain running. No historical
byte-for-byte preservation proof was supplied for the first guest; it was not
intentionally modified. Parent inspected and adopted the passive appended evidence
(worker wrote it despite no-taskdoc-edit instruction); no source/lockfile changes.

Inspected actual Sign-in options: Face, fingerprint, PIN or security key (passkey),
then Domain join instead. Opening the latter for inspection displays Who's going
to use this device?, empty Name field, Even better, use an online account link and
Next. Inspection evidence: `shots/independent-domain-inspection-screen.png`.
This is an advertised Windows UI route, not a registry/OOBE bypass. User subsequently
authorized EntropiaTest plus a random private password; name and masked password /
confirmation were accepted. Credential file under the second VM:
`privatecredentials/EntropiaTest-password.txt` (0600), containing directory 0700.
No password printed or recorded in repository/Engram; no real identity reused.
User explicitly authorized random disposable recovery answers, never personal
facts. First-pet, birth-city and childhood-nickname questions accepted with three
independent CSPRNG 22-character alphanumeric answers. Stored once in private
`privatecredentials/EntropiaTest-recovery.txt` (0600); password unchanged. Answer
screens saved redacted; worker reports transient screenshot PPM removed.
User authorized disabling all five optional privacy choices and continuing.
Before Accept: Location No, Find my device No, Diagnostic data Required only,
Inking & typing No, Personalized offers No. Accepted and booted desktop.
Independent live verification: matching QMP UUID/running/KVM, 8 CPUs/16 GiB,
200 GiB qcow2/no backing, read-only ISO; credentials 0600 and directory/TPM 0700,
no symlinks, host-free 363.29 GiB. EntropiaTest PrincipalSource Local; WORKGROUP,
PartOfDomain False, AzureAd/Enterprise/Domain/WorkplaceJoined NO. Public TCP 443
and HTTPS 200 passed without disabling TLS verification. Process confirms NAT
without forwards or host shares/clipboard/public VM-management listener.
Evidence: `shots/verifier-desktop-runtime.png`, `verifier-account-result.png`,
`verifier-domain-result.png`, `verifier-management-upper.png`,
`verifier-network-tcp-result.png`, `verifier-network-https-result.png`.
NAT is not stronger host-network isolation. Existing host SSH listener untouched,
not a VM port forward. Historical TPM freshness/exactly-one Accept not independently
reconstructed. Actual desktop watermark Build 26100.ge_release.240331-1435 differs
from media filename 26300; correlation unresolved, do not infer guest version from
filename. No additional account/PIN/service choice appeared. Entropia application,
installer and WebView2 acceptance remain pending; no source edits or commits.

## W4 current-source artifact routing

Local HEAD `ad684a7a2ae6189c43cd2164fb8942a1924b6618`, version 1.0.20. Working-tree
changes are harness/tracking only; no application-source delta listed. No matching
HEAD among latest ten release.yml runs (not a proof across entire history).
Successful v1.0.19 run 36978416910 has unexpired EntropIALite-installers artifact
11215376080, but SHA 5084e05ef59d0f0e53152234641dd8ef250a23b1 is not current.
Remote GET for feat/internet-archive returned 404; no push or dispatch performed.
Manual dispatch with lite_only=true / release_platform=windows skips Pro/draft
release job; Lite tauri-action has no release params, release attachment is
refs/tags-only. Explorer's draft-release inference was corrected by actual gates.
User explicitly authorized pushing existing feature commits and running Lite-only
CI artifacts, without new commits, uncommitted changes, tags or releases.
Initial HTTPS push failed (missing credential helper); separate read-only incident
check confirmed existing gh authentication. Invocation-only gh credential helper
push of pinned ad684a7a succeeded; persistent Git config unchanged. Dispatched
release.yml on feat/internet-archive with lite_only=true / release_platform=windows.
Run: https://github.com/HumaLab/EntropIA-Pro-Lite/actions/runs/38094631750.
Independent verifier confirms remote/run HEAD matches pinned SHA and event
workflow_dispatch. After one 1200-second bounded watch, Lite job remains in_progress
building installers, setup/dependency steps succeeded, artifact query empty.
Pro matrix/unix/release-attachment jobs skipped. No build success, installer
availability or Windows Entropia acceptance claimed. Next: fresh run status,
verify EntropIALite-installers provenance/hash, transfer without host sharing,
then actual NSIS/MSI/WebView2/IA/Pdfium acceptance. No rerun/extra dispatch performed.

## W4 bounded unit 2026-10-11 (partial; STOP before full IA; W4 NOT done)

Provenance re-queried: run 38094631750 success, headSha ad684a7a2ae6189c43cd2164fb8942a1924b6618, branch feat/internet-archive, event workflow_dispatch. Artifact EntropIALite-installers ID 11686085054 unexpired, 51453521 bytes, expires 2027-01-08, digest sha256:52ddeaf2447b67f14faab5e8ac08471fd1579132e1fbec7df08b56cb8353f57f. Downloaded via gh run download -n EntropIALite-installers to NEW owner-only $VM/w4-lite-20261011 (0700). Contents: exactly 2 files, no traversal/extra exes/symlinks. EXE `EntropIA Lite_1.0.20_x64-setup.exe` 23310752 bytes SHA256 6dba2d2c78ed0329754b69b6aba665dc149f38694d434539203242f4d612dcc5 (PE32 NSIS). MSI `EntropIA Lite_1.0.20_x64_en-US.msi` 28499968 bytes SHA256 8948e11b450d16ae174429ff3d5dd0287df0414cc149a9f3abe30c886edcd769 (MSI WiX 3.14.1.8722). Host hashes do NOT prove Authenticode; signing distinct from provenance, no host Authenticode tool (osslsigncode absent).

Transfer: host genisoimage/xorriso absent, pycdlib absent globally; used ephemeral external venv $VM/.venv-w4-transfer (pycdlib 1.22.0, no global/repo install, no host apt). Authored installer-only ISO $VM/w4-lite-installers-20261011.iso 51877888 bytes SHA256 dbf2470d3a8a5faa3b6ec8c64c6a1b9d9d5f8338e7e4c8a5e9257a57702d40cf, vol ENTROPIA_LITE, Joliet names match Lite 1.0.20 + SHA256SUMS.txt (738 bytes). QMP eject Windows ISO then blockdev-change-medium to installer ISO; tray events observed. query-block: cdrom0=new ISO ro true, sysdisk unchanged; original Windows ISO preserved under media/ (bc3f...c10e38 re-verified, NOT deleted). Guest Get-Volume: D ENTROPIA_LITE CD-ROM 49.47 MB; Get-ChildItem sizes match host (23310752/28499968/738); Get-FileHash match host (setup 6DBA...DCC5, msi 8948...D769 case-insensitive). Live UUID d9e70790-1faa-488b-a23a-a622d99cb202 running; desktop verified (EntropiaTest, WORKGROUP, 5:07-5:23 PM). First guest dir mtime 2026-10-10 16:56 preserved, processes 1648596/1648601 running; disk mtime reflects running-OS background writes, no worker files. Host-free 360 GiB. NAT only, no shares/clipboard/forwards/host HTTP. Credentials/recovery NEVER read/printed.

Install/startup (NSIS only; MSI NOT installed over NSIS, pending clean-scope): Start-Process setup.exe -> Welcome (Next/Cancel, no license) -> Choose Install Location default guest-only `C:\Users\EntropiaTest\AppData\Local\EntropIA Lite`, 58.9 MB req, 163.4 GB avail, NSIS v3.11 -> Installing (WebView2 silent Execute shown, progress full, no license/privacy/SmartScreen/AV prompt, no UAC dialog observed) -> Installation Complete -> Completing (Run EntropIA Lite + Create desktop shortcut both checked) -> Finish autostarted Lite. No new license/privacy/SmartScreen Run Anyway/AV/policy decision encountered; none accepted. App window `w4-lite-window-20261011.png`: ENTROPIA LITE Inicio Espacio de trabajo, 3 colecciones 13 docs, OCR 12/13. Get-Process msedgewebview2: 17 processes (1460/1524/1872/2024/3676/4596/5876/6116/6388/6584/6632/7504/7964/8220/8394/8776/9396/11344). Registry uninstall entry, installed exe ProductVersion, WebView2 version, VC/Pdfium resources, %APPDATA% com.entropia.shared blank-archive check NOT completed (colon/dollar/star keymap flaky, HKCU nav failed, EntropIA process quotes failed); STOP here for independent verifier. No Entropia login/sync/real archive import. Full IA/Pdfium acceptance NOT done. W4 remains open; no commits.

## W4 independent verification 2026-10-11 (NSIS installed + startup verified; W4 NOT done)

Independent live QMP UUID d9e70790-1faa-488b-a23a-a622d99cb202 running, KVM, 8 CPU; sysdisk second VM, CDROM readonly w4-lite-installers-20261011.iso. Host EXE/MSI/ISO hashes independently match worker record. Guest registry EntropIA Lite 1.0.20, default LocalAppData/EntropIA Lite; installed entropia-pro-desktop.exe ProductName Lite / ProductVersion + FileVersion 1.0.20. Actual WebView2 file version 155.0.4283.45; sampled process paths resolve it but Entropia ownership not established. Fresh shots/independent-w4-live-app.png: app works, 3 collections / 13 docs / 12 text; matches public bundled demo Actas 2 / Casas 6 / La Capital 5, documents.json 21/89/272 one empty text 67, and sample-collection.ts 9-21/128-168. Guest APPDATA shared path filename-only assets/research SQLite WAL/SHM processing lock; no contents read, not proof history empty. No host share/sync account/new identity configured; release DEV_PROFILE ignored. Installer Authenticode NotSigned; matching CI hashes are not signing. VC/Pdfium DLL presence observed, NOT functionality. MSI/IA/native Pdfium pending; no Pro. Evidence: independent-w4-*-result.png. ASSESS: unassessable untracked task doc; independent verifier required and completed; no native code closure claim. Historical failed typing checks preserved above; these are superseding observations.

## W4 IA + native Pdfium actual 2026-10-11 (W4 NOT done; MSI/Pro pending)

Second VM UUID d9e70790-1faa-488b-a23a-a622d99cb202 running, QMP query-uuid/status/block verified before input. Lite 1.0.20 NSIS, public demo 3/13 preserved; fresh ASCII collection IA Windows Test. First guest untouched. No shares/clipboard/forwards/remote-debug/cloud/security changes. No credentials read.

IA real UI PASS. Home Importar -> Desde Internet Archive -> Enlace -> dummy-pdf_2 -> Abrir -> select single PDF -> Continuar -> new IA Windows Test -> Preparar -> Iniciar. First: Importados 1, Ya estaban 0, Con error 0. Preparation showed 1 file, 7.3 KB, vol 14.6 KB. Opened doc dummy-pdf_2_page_1.pdf, PDF displayed Dummy PDF download (pdf.js viewer, not a native claim). Home then 4 collections 14 docs. Repeat same file/target: preparation 0 files, Duplicados dummy-pdf_2.pdf Ya importado, vol 0 B; result 0/1/0. Same collection 62af3181-da91-4a8d-a746-84db0ff4990d, same item e929e121-aa95-4700-b028-358580b680ac, same hashes. No network refusal, no overflow, no bypass. UI blocked never; no spurious display-equals-native claim.

Fixture public only: identifier dummy-pdf_2, URL https://archive.org/details/dummy-pdf_2, file dummy-pdf_2.pdf 7478 bytes 1 page. Parent SHA 29550d778741b8ba2ed38d4e21b7bbc97d84925eec24422a55f8a4d5b1eb9a43 match in job.json and asset hash. Child dummy-pdf_2_page_1.pdf 1373 bytes SHA 756a1f4cd5dcf449a897b40688f46edb48fd14998f6ba9f73f8871305ed7a87c match (no platform difference observed). Jobs completed: 4e1a2a97-e5c6-42a4-9237-0dce2f519ce3 (first) and ceeb30ad-d652-468f-bee1-521fcb8a2321 (dedup). Job files show Text PDF, original, expected 7478, md5 f45a5a93dbd16785565f30e87021e32c, sha1 d73218ded66fc2eb2b1dd1fc9f26168272a3a238, state imported, page_count 1, staging null. Assets 62af.../e929.../b64901f3-4ffd-4de3-bea9-a2ffae3290e1_dummy-pdf_2.pdf + dummy-pdf_2_page_1.pdf. Guest staging path %APPDATA% com.entropia.shared/staging/internet-archive/<job>/job.json. SQLite CLI absent; entropia.sqlite 1339392 + wal 4120032 seen via dir only, integrity/provenance DB NOTVERIFIED.

Native Pdfium identity PASS, API NOTVERIFIED, app integration NOT tested. Actual path C:\Users\EntropiaTest\AppData\Local\EntropIA Lite\resources\lib\pdfium.dll, size 5787136, SHA256 6B963C2BE9CACBAA0C0C7F4BF6D20D2FD16729EBDAA9989978B0F7B119C1C1CB, MZ 77/90, PE offset at 0x3C 120 (0x78), PE 80/69/0/0, machine 100/134 = 0x8664 x64. LICENSE 11777. VC DLLs present in install root; entropia-pro-desktop.exe 50671616 PID 6012 running. Session PATH change not applied; no persistent env change. FPDF_Init/LoadDocument/GetLastError/GetPageCount/Close/Destroy count probe not executed: brackets/quotes/colons not safely typable via QMP ES/US mapping without risking unexpected code, long Base64 typing fragile; no loader error, no fallback/system DLL, no tool install. Source ocr/commands.rs generate_pdf_thumbnail -> ocr/pdf.rs render_pdf_thumbnail uses Pdfium, IA split uses lopdf, viewer uses pdf.js; thumbnail/log/cache not verified without debug port/new code.

Screenshots/logs second VM only: shots/w4-ia-*.png, w4-lib, w4-pdfium-hash, w4-mz, w4-machine, w4-parent-hash, w4-child-hash, w4-job1-*, w4-job2-*, w4-proc-ent2, w4-final. Keymap notes: ES slash gives hyphen, Shift+slash underscore; dollar/letters/space/dir/cd/type/Get-*/Tab work; colon/semicolon/brackets unreliable; Format-Hex has no Count; Alt+F4 hit shutdown dialog, Esc cancelled, no shutdown; Win+Space standard layout toggle ES/US only. W4 remains open; MSI over NSIS not done; Pro pending; no commits.

## W4 bounded unit 2026-10-11 (planned): SQLite readonly + Rust thumbnail integration

Planned BEFORE any guest mutation. Target second VM UUID
d9e70790-1faa-488b-a23a-a622d99cb202 (running, verified via QMP
query-status/query-uuid). CDROM currently read-only
w4-native-probe-20261011.iso (SHA 350e9f64..., preserved). Guest visually
verified awake at PowerShell prompt (assets dir, probe ISO as D). No login
needed; stop if login required. No credentials read.

SQLite native read-only plan: FIRST File.Exists winsqlite3 check inside C#
(blocked, no install/copy if absent); guest absolute winsqlite3.dll P/Invoke
cdecl open_v2 flag 0x1, db_readonly(main)==1, busy_timeout 2000, UTF8
prepare/step/column/finalize/close/errmsg; IntPtr.Size 8; DB ONLY
%APPDATA% com.entropia.shared/entropia.sqlite; read-only SELECT/PRAGMA only
(quick_check, foreign_key_check, table_info items/assets, target collection
62af3181-da91-4a8d-a746-84db0ff4990d / item
e929e121-aa95-4700-b028-358580b680ac + its 2 assets + metadata
__entropia_internet_archive); parent 7478
29550d778741b8ba2ed38d4e21b7bbc97d84925eec24422a55f8a4d5b1eb9a43 vs child
1373 756a1f4cd5dcf449a897b40688f46edb48fd14998f6ba9f73f8871305ed7a87c
hash compare on disk; WAL presence informational, locked -> NOTVERIFIED (no
force/copy/process action). New audited sqlite-only C# source + hash ISO
created on host, CDROM swapped readonly, old ISOs preserved. Single SHORT
UTF16LE -enc entry `Add-Type -Path 'D:\SqliteProbe.cs';[SqliteProbe]::Run(); `
(len 57, alnum-only base64, no manual special chars); Ensure-Path process-only
if System32 missing from PATH. Enter once after visual verification.

Rust thumbnail plan: source chain CollectionView card queue -> .pdf ->
file-import.generatePdfThumbnail -> generate_pdf_thumbnail (ocr/commands.rs) ->
render_pdf_thumbnail 400px (ocr/pdf.rs). Readonly-first: derive root PDF asset
ID from SQLite, inspect %LOCALAPPDATA% com.entropia.shared/thumbnails/<id>.png
(mtime/size/PNG dims/content). Fresh native op only via existing UI (new ASCII
collection reimport of public dummy-pdf_2, record new asset ID + cache absent
BEFORE -> PNG AFTER); dedup-reuse -> guest-temp local copy
NativeRustPdfium.pdf fresh collection, never delete cache. Module proof:
app PID Get-Process entropia-pro-desktop Modules pdfium.dll FileName/hash MUST
match installed resource pin (not probe process). pdf.js viewer / lopdf split
are NOT proof. Report cached-only limits explicitly.

W4 NOT done after this unit (MSI/Pro/other cases remain).

## W4 bounded unit 2026-10-11 (actual SQLite PASS; Rust thumbnail NOT proven; W4 NOT done)

Second VM d9e70790-1faa-488b-a23a-a622d99cb202 running throughout (QMP
query-status/uuid/block at close). CDROM ends readonly
w4-sqlite2-probe-20261011.iso (SHA
8d2c412ca452c638d59bcd683e3adc52774344a02575f37c8b3b2df89a8db1c3, 92160 B,
vol ENTROPIA_SQL2, guest Get-Volume confirmed). Old ISOs preserved
(lite-installers dbf2470d..., native-probe 350e9f64..., sqlite-v1 same size).
Host C# under VM/w4-sqlite2-src/S.cs (SHA
d8001b86..., class S/entry R(), same checks as audited v1 d4862942...).
Entry deviation from plan: short names + `-Path` named (positional does NOT
bind -Path; first attempt proved it: Add-Type compiled literal `D:\S.cs` and
failed). No credentials read; no login needed; reboot/shutdown avoided
(shutdown dialog once via Alt+F4 misfocus, Esc cancelled, no harm).

SQLite native readonly PASS (guest winsqlite3.dll P/Invoke cdecl, open_v2
flag 0x1, single SHORT -enc entries, typed chunked+verified, no writes):
winsqlite_exists=True C:\Windows\System32\winsqlite3.dll, PATH already
has system32, IntPtr proven 8 (run passed gate), DB ONLY %APPDATA%
com.entropia.shared\entropia.sqlite size 1339392, WAL+SHM present (normal),
sqlite_version=3.51.1, open_v2_readonly ok, db_readonly_main=1,
busy_timeout_rc=0, quick_check 1 row `ok`, foreign_key_check 0 rows,
items/assets columns match schema.ts, target_item_count=1,
item e929e121-aa95-4700-b028-358580b680ac title dummy-pdf_2 metadata 1040 B
in collection 62af3181-da91-4a8d-a746-84db0ff4990d (IA Windows Test),
provenance key+hint True, 2 assets: parent daedde35-dabb-459a-9a46-9e003bde473f
pdf 7478 B on disk SHA full-match `valid`, child
3c0c4e00-d290-4450-bf69-e6de5f95338f page 1 on disk 1373 B SHA full-match
`valid` (DB size column reads 0 for child; on-disk authoritative per plan),
close ok, `PASS sqlite readonly integrity + public IA provenance valid`.
Shots: shots/w4-sqlite-20261011/w4-r1..r5 (tail/head), w4-mod, w4-vol3/4.

Rust thumbnail integration NOT proven: `(gps entropia-pro-desktop).Modules`
filter pdfium.dll returned EMPTY in live app PID 6012 (same PID as native
probe era, app idle 0.2% CPU) -> `Get-FileHash : Cannot bind ... Path ...
null` at char 99. So pdfium (lazy get_pdfium OnceLock-style) never
initialized in this process life; card-queue render did not fire or failed
silently. Thumbnail cache state UNKNOWN (Line T dir-query blocked by
transport losses; NO cache deletion performed). Fresh UI reimport NOT
attempted. pdf.js viewer / lopdf split remain NOT proof (unchanged).
Direct-DLL native API PASS stands as prior independent evidence (not Rust).

Transport incidents (preserved, no blind retry): one U/7 char-flood after a
168-char burst at 0.06 s/char (guest CPU hog ~83% host-core, guest TM 22%
with all-visible ~0%; WT 904 s + 2 orphan powershells killed 1888/11860;
hog root cause unresolved); recovered by closing flooded console (Alt+F4 on
focused window; one shutdown-dialog misfocus, Esc, no shutdown). Slow
chunked typing (0.4-0.5 s/char, <=12-char chunks, verify-before-Enter, no
cursor keys) then delivered 3 exact lines (88/312-blob PASS runs + module
query). One malformed 82-char line failed clean `not properly encoded`
(no execution, no DB touch). Stuck cmd avoided via new window; orphan
consoles left idle (no reboot: would stop at password login).

W4 NOT done: MSI install, Pro variant, fresh Rust-native thumbnail op, other
cases remain. No commits. Main archive untouched; first guest untouched.

## W4 independent native verification 2026-10-11 (IA confirmed; Pdfium direct-API render PASS; W4 still open)

Independent IA verifier confirmed the worker images: first 1/0/0, repeat 0/1/0, same collection 62af3181-da91-4a8d-a746-84db0ff4990d, same item e929e121-aa95-4700-b028-358580b680ac and jobs, parent 7478 / child 1373 hashes matching known fixtures, viewer Dummy PDF. No new imports by the verifier.

Native incident reconciled (worker API NOTVERIFIED above preserved as resolved history): worker ENTER never sent; the black screen was benign DPMS, mouse woke it, no lock/credentials. Audited C# on read-only probe ISO: /NativeProbe.cs 18099 bytes SHA b0b0932e975b2c89a356891103e84676a100d70a9a4a6d8d2ebe74e3f1b842ce; ISO w4-native-probe-20261011.iso SHA 350e9f64b98e12a0c0fcb03f5b35894387e6271e8d1ab9706d9a6915c8948e2b. Entry Add-Type -Path 'D:\NativeProbe.cs'; [NativeProbe]::Run() executed ONCE by independent gentle-ai-verify. Observed: installed DLL 5787136 SHA 6b963c2be9cacbaa0c0c7f4bf6d20d2fd16729ebdaa9989978b0f7b119c1c1cb, IntPtr 8, PE 0x8664; FPDF_InitLibrary ok; LoadDocument and LoadPage handles nonzero; PageCount 1; page 595.219970703125 x 842; bitmap 300x425 FillRect white ok, RenderPageBitmap ok, total 510000 bytes, nonwhite 227, PASS; Bitmap_Destroy/ClosePage/CloseDocument/DestroyLibrary ok; prompt returned, VM running. Public job PDF parent 29550d778741b8ba2ed38d4e21b7bbc97d84925eec24422a55f8a4d5b1eb9a43 verified. Evidence shots/verifier-native-wake-continuation.png, verifier-native-command-ready.png, verifier-native-result-header2.png, verifier-native-result-2s.png, verifier-native-result-5s.png. This proves installed-DLL native API render, NOT Rust app integration/pdf.js. Transitive VC deps normal Windows resolution, not pinned. ASSESS: unassessable untracked task doc; independent verifiers completed; no native approval claim. W4 substeps IA and Pdfium direct API verified; MSI/Pro/Rust app native integration/SQLite integrity/other cases remain open; no all-feature completion claimed.

## W4 bounded unit 2026-10-11 (actual thumbnail cache readonly + temp copy + single-instance guard; fresh native NOT proven; W4 NOT done)

Second VM d9e70790-1faa-488b-a23a-a622d99cb202 running (QMP query-uuid/status match before input). Benign DPMS black woke with one rel-x mouse move; no lock, no creds. Input via QMP input-send-event paired down/up same batch, 2-3 chars/batch at ~0.4 s/char; '-' on ES guest is qcode 'slash' (qcode 'minus' yields "'"), Esc clears line, Shift+letter yields uppercase. No hold-for-text, no layout change, no host shares/clipboard/forwards.

Source corrected: Lite PDF thumbnail chain SUPPORTED ungated (CollectionView.svelte 308-314 queue .pdf -> generatePdfThumbnail; file-import.ts 391; Rust lib.rs 1412 unconditional command; ocr/commands.rs 390-437 cache-then-render; ocr/pdf.rs render_pdf_thumbnail 400px). Prior module-absent never proved lazy-init never fires; only snapshot absence.

Readonly cache: `%LOCALAPPDATA%\com.entropia.shared\thumbnails` holds ONE file `daedde35-dabb-459a-9a46-9e003bde473f.png` 7425 B 10/10/2026 6:03 PM (matches IA parent asset id from SQLite PASS). No cache deletion. Controlled guest temp copy from existing public bytes only: `Copy-Item $env:APPDATA\...\*\*\*dummy-pdf_2.pdf $env:TEMP\NativeRustPdfium.pdf` -> `$env:TEMP\NativeRustPdfium.pdf` 7478 B present. No new network, no DB write, no prior-record alter/delete; demo/IA imports preserved.

UI focus attempt: Win + `EntropIA Lite` + Enter hit real single-instance guard `EntropIA ya esta abierta / Ya hay una EntropIA abierta (Lite, Pro o de desarrollo)` + `Inicializando...` behind; Enter dismissed OK, no second instance, no DB damage. Existing app window stays behind PowerShell; normal gallery/cards view NOT reached. New ASCII collection `Rust Native Thumbnail Test` + local-PDF import + before-absent/after-PNG + dimensions + card-vs-placeholder NOT attempted (would exceed ~30-call UI budget; file-picker path typing unreliable). Fresh UI->Rust call NOT proven.

Modules: `(Get-Process entropia-pro-desktop).Modules` tail shows system DLLs only in current viewport (PgUp did not scroll console); prior live filtered `pdfium.dll` query in app PID 6012 returned EMPTY so `Get-FileHash Path null` at char 99. No associated loaded-module + no fresh PNG => Rust integration NOTVERIFIED (cached-only). Installed-DLL direct-API PASS stands as separate evidence, not app integration. Screenshots second VM only: w4-thumb-*.png, w4-copy-*.png, w4-verify-copy, w4-desktop, w4-appfocus/after (guard), w4-mods/up. W4 NOT done; MSI/Pro/fresh-native remain.

## W4 clarification 2026-10-11 (independent verifier; no overclaim)

User authorized EXE uninstall + MSI preserving data, but uninstall NOT invoked and MSI NOT executed: MSI NOT TESTED, not failed. After ONE normal visible-main X, app PID 6012 observed alive (QMP second UUID running, readonly SQLite, 2 media); native handle execution / click delivery NOT proven by cursor/X screens or no-close log, cause unknown: say 'no exit observed', not 'ignores X'. Guest %TEMP%\EMSI20261011 backup copied live WAL: CONSISTENCY UNVERIFIED, not proven incoherent/corrupt; size-equal is not snapshot proof. No forced exit/reset/second close; first guest and host archive untouched. Preinstalled NSIS still presumed current version from stored query, not fresh uninstall proof. Independent stored-proof read stop gate respected. ASSESS: unassessable untracked tracking doc; independent verifier completed, no review closure claim. Next PROPOSED only (not implemented/authorized): guest-only SQLite ONLINE backup API snapshot + quick_check/FK/assets-manifest verification, then explicit force-exit consent if needed. W4 open (Pro/Rust-native/DB child-size/MSI). History above preserved.

## W4 reconciliation 2026-10-11 (history preserved; supersedes overstrong phrasing only)

- SQLite S.cs d8001b86a6c8fc2af5e55ae5ccde92bc925c7ae760dd6d63891fb11c81152cbc, shots w4-sqlite-20261011/w4-r1..r5: readonly_main=1, 3.51.1, quick_check one `ok`, fk 0, IA id count 1, parent 7478 / child 1373 on-disk SHA match; child DB size 0 vs disk 1373 WARN (disk authoritative); provenance substring only, not JSON; close/finalize codes ignored so prior `close ok` means prompt-returned, not verified handle closure; no full metadata/global-dedup claim.
- Rust native integration NOTVERIFIED (supported ungated chain noted); no fresh cache-miss/native op; module absence snapshot-only, not lazy-init diagnosis; existing 7425 B PNG filename-only, not PDF-correlation proof; direct-DLL render PASS historical unchanged.
- Temp NativeRustPdfium.pdf copy only, no new collection/import; single-instance guard dismissed, no product fix; MSI awaits explicit EXE uninstall preserving guest data (none done); Pro needs matching build + fresh CI decision (not run); W4 in progress, ASSESS unassessable, independent verification done.

## W4 bounded unit 2026-10-11 (planned authorized MSI replacement, pre-backup/app-closure plan; NO mutation yet)

User authorized uninstallLiteEXE in second VM d9e70790-1faa-488b-a23a-a622d99cb202 PRESERVING collections/testdata, then install current MSI. Bounded unit, no source fix/build/Pro/commits/push/tags/releases. Host-free 359 GiB. QEMU 1656972/swtpm 1656977 running; UUID/status/block verified; CDROM readonly w4-sqlite2-probe ISO; installer ISO dbf2470d preserved. First guest untouched. Screen w4-msi-wake: benign DPMS wake via rel-x, PowerShell at system32 prompt, no login/creds.
Preflight: repo windows/hooks.nsh PREUNINSTALL deletes only VC DLLs under $INSTDIR, never APPDATA com.entropia.shared. Never check delete-user-data. Quit Lite via normal UI Exit/CloseMainWindow after IA jobs complete; if tray use Exit NOT kill; if DB/WAL backup unsafe DO NOT uninstall. Preservation: NEW guest-only backup under local TEMP with APPDATA assets/research/staging/entropia.sqlite (+wal/shm) + nonsecret metadata; no host copy, no keyring. Baseline manifest/SHA + counts via probe after close. No restore authorized. Uninstall via exact registry NSIS uninstaller; verify entry/binary gone + data unchanged before MSI. Swap to installer ISO, verify MSI SHA 8948e11b, read ProductCode/UpgradeCode/Version/ALLUSERS readonly, run MSI FULL UI, pin exit 0/3010. Verify MSI 1.0.20/path/Lite/WindowsInstaller vs NSIS removed, start MSI binary, verify demo+IA Test+dummy PDF+viewer, guest-only APPDATA, readonly probe repeat. Compare BEFORE launch; after-launch WAL change as logical counts.

## W4 bounded unit 2026-10-11 (actual STOP BEFORE UNINSTALL; MSI NOT tested; W4 open)

Second VM d9e70790-1faa-488b-a23a-a622d99cb202 running (query-uuid/status match). Screens w4-msi-wake/type-test/clear/smoke/proc/reg/appdata/quit/gps2/mkdir/copy1/copywalshm/backup-verify. Host-free 359 GiB. First guest 1648596/1648601 untouched. CDROM stays readonly w4-sqlite2-probe ISO; installer ISO dbf2470d + MSI 8948e11b preserved, verified on host.
Preflight: repo windows/hooks.nsh PREUNINSTALL deletes only VC DLLs under $INSTDIR, never APPDATA com.entropia.shared. Guest reg HKCU Uninstall EntropIA Lite: DisplayName EntropIA Lite, InstallLocation Local EntropIA Lite, UninstallString uninstall.exe, MainBinary entropia-pro-desktop.exe, Publisher entropia. Live PID 6012 path Local EntropIA Lite entropia-pro-desktop.exe. APPDATA Roaming com.entropia.shared: assets/models/research/staging, lock 0, sqlite 1339392 3:57 AM, shm 32768, wal 4120032 4:16 AM.
App closure: (gps).CloseMainWindow returned True but gps still 6012 (430 handles, 114s CPU). Tray-Exit menu not attempted within safe bounded budget; no kill per rule. WAL mtime advanced 4:16 to 4:23 during backup, same lengths, so consistent backup not assured.
Preservation: NEW guest-only $env:TEMP\EMSI20261011 with sqlite 1339392 + shm 32768 + wal 4120032 (lengths match live at copy). No host copy, no keyring, no restore. Assets/research/staging hashes and IA counts (parent 7478/child 1373, 3/13 + IA Test) not re-proven after close; SQLite probe not rerun.
Uninstall NOT invoked, CDROM NOT swapped, MSI ProductCode/UpgradeCode/ALLUSERS NOT read, msiexec NOT run, NSIS entry still present, no reboot/license/SmartScreen prompt encountered. STOP per hardgate. Backup preserved for verifier. W4 open for Pro/Rust integration/DB child-size; no commits.

## W4 bounded unit 2026-10-11 (actual ONE normal X; process ALIVE; STOP no uninstall/MSI; W4 open)

Second VM d9e70790-1faa-488b-a23a-a622d99cb202 running (query-status/uuid/block match; CDROM readonly w4-sqlite2-probe ISO). Pre-X verified actual visible main Lite Espacio de trabajo 4 collections 14 docs, no modal/closing/compaction overlay (shots oneX-windown2/oneX-aim-20261011.png; prior nonclosing-state-close-log.png same). PowerShell minimized via 2x Win+Down (keyboard only) to reveal main; no credentials/login.
ONE standard visible-main-window X only: absolute move 31680,1493 aimed (cursor on X verified), single btn down/up, no second X/WM_CLOSE/kill/Taskkill/tray-Exit/Rust-force/backend close. After 3s main still 4/14, no closing overlay/phase (oneX-after3s.png). Bounded >60s read-only PID check via cmd powershell -enc gps-Id-6012: PID 6012 entropia-pro-desktop ALIVE 444 handles 123.81s CPU (up from 430/114s), main still behind PowerShell (oneX-pidcheck3-typed.png). No close observed. Per hardgate STOP: no NEW closed-app backup (old $env:TEMP\EMSI20261011 LIVE WAL stays NOT coherent, keep don't use), no uninstall, no CDROM swap, no MSI read/run, no reboot/license/SmartScreen prompt. NSIS entry still present. First guest untouched. W4 still open (Pro/Rust/DB-size). No commits.

## W4 bounded unit 2026-10-11 (planned guest-only CONSISTENT backup, STOP AFTER BACKUP VERIFY; NO force/uninstall/MSI in this unit)
Target second VM d9e70790-1faa-488b-a23a-a622d99cb202 only, Lite PID 6012 last, guest 4 collections 14 docs public demo+IAWindowsTest. Old TEMP/EMSI20261011 live WAL copy UNVERIFIED preserved, not valid snapshot. No host archive/shared/clipboard/servers/forwards/first VM/credentials/keyring/sync secrets. Benign mouse wake allowed, stop if lock. No kill/close/reboot/reset/uninstall/restore/overwrite in THIS unit.
Plan: builtin guest C:\Windows\System32\winsqlite3.dll online backup API (no raw DB+WAL copy), source READONLY flag 1 + db_readonly(main)==1, NEW guest TEMP/EMSI-Verified-<timestamp>/entropia.sqlite (require absent, no overwrite), dest RW|CREATE 6, backup_init/step 128 pages until DONE 101 (OK continues, BUSY/LOCKED bounded <=30s, others fail), backup_finish OK required, close/finalize in finally with checked codes. No source writes/vacuum/checkpoint/WAL copy. Then DEST readonly verify (quick_check one ok, FK 0, 4/14 or actual baseline, IA 62af3181/e929e121 provenance JSON or marker, parent 7478 29550d77.../child 1373 756a1f4c... on-disk, child DB size 0 WARN preserved). Copy public assets/research/staging only (no DB/host copy/keyring), manifest lengths/SHA before/after stable (DB excluded, WAL allowed change), dest hash AFTER close/validation, IA hashes independent, backup-report.txt+manifest JSON public only. Delivery: NEW readonly helper ISO with auditable C# B.cs+SHA under VM artifacts only (pycdlib venv, old ISOs preserved), PowerShell Add-Type -Path 'D:\B.cs';[B]::Run() short UTF16LE -enc, correct-Path, .NET Framework types, no PS1 bypass, QMP paired down/up, Enter once. Host cmds <=120s, helper backup <=30s. Gate: STOP AFTER BACKUP, leave Lite RUNNING, parent verifier inspects before force/uninstall. No product fix/build/commit/CI.

## W4 bounded unit 2026-10-11 (actual guest-only CONSISTENT backup READY; STOP AFTER BACKUP; NO force/uninstall/MSI; W4 open)
Second VM d9e70790-1faa-488b-a23a-a622d99cb202 running (query-status running, query-uuid match, query-block CDROM new ISO ro true). QEMU 1656972/swtpm 1656977; first guest 1648596/1648601 running untouched; host-free 359G. Benign DPMS black woke via rel-x; no lock/creds. Stuck old cmd avoided via new window (Win, cmd, Enter); orphan consoles idle, no reboot.
Media: old sqlite2 ISO 8d2c412ca452c638d59bcd683e3adc52774344a02575f37c8b3b2df89a8db1c3 + installer dbf2470d + native 350e9f64 preserved. NEW VM/w4-backup-src/B.cs 45493 SHA 003046468e3ff8350026c8cf251ca8a2fb4eca22d6edf69ac818db223feeddcf + SHA256SUMS.txt 412 + ISO w4-backup-20261011.iso 110592 SHA 9a731835debd62e6702330a24741856f271fbbe92d6fe2ceab0b39850f923e28 vol ENTROPIA_BKP (pycdlib venv, no global install). QMP eject + blockdev-change-medium to backup ISO; guest Get-Volume D ENTROPIA_BKP CD-ROM 108KB; Get-ChildItem D: B.cs 45493 + SHA256SUMS.txt 412; Get-FileHash D:\B.cs 00304646... match host (case-insensitive). Entry powershell -enc QQBkAG... (Add-Type -Path 'D:\B.cs';[B]::Run(); ) typed 120 chars 0.4s/char verified-before-Enter, Enter once; compiler/load error NONE (Add-Type compiled, Run returned, prompt returned).
Guest backup root C:\Users\EntropiaTest\AppData\Local\Temp\EMSI-Verified-20261011-051221 dest entropia.sqlite. winsqlite_exists True 3.51.1 path_already_has_system32. src 1339392 WAL+SHM True. src_open_v2_readonly ok readonly_main 1 busy_timeout 0. backup_root_created ok dest_open_v2_rwcreate ok dest_busy 0. backup_init ok step DONE calls 3 remaining 0 pagecount 327 busy_retries 0. backup_finish 0 dest_close 0 src_close 0. dest_size_after_backup 1339392. DEST verify readonly 1 quick_check 1 ok finalize 0 FK 0 finalize 0 collections 4 items 14 target 1 (e929e121 in 62af3181 title dummy-pdf_2 metadata 1040 prov key True hint True json_valid 1 identifier dummy-pdf_2 extract finalize 0). Assets 2: parent daedde35 size 7478 + child 3c0c4e00 page1 size 0 finalize 0; WARN child DB 0 vs disk 1373 disk_authoritative no_fix preserved. verify_close 0. dest_snapshot SHA 039965617fef73adaeb8974050e5b7dbba64ceee571690279f4c20bfdbbba6b6 size 1339392 AFTER close/validation. Parent on-disk 7478 29550d778741b8ba2ed38d4e21b7bbc97d84925eec24422a55f8a4d5b1eb9a43 valid; child 1373 756a1f4cd5dcf449a897b40688f46edb48fd14998f6ba9f73f8871305ed7a87c valid. Copy assets 15 research 1 staging 2 total 17 mismatches 0 skipped_db 1 before/after stable; backup IA hashes match. Report backup-report.txt+manifest.json written public only. BACKUP_READY gate_ready coll 4 items 14 copy 17. No literal READY-word before cleanup; helper never forced exit.
PID 6012 entropia-pro-desktop ALIVE 444 handles 127.98s CPU (up from 123.81) via fresh gps-Id-6012; Lite window behind (Coleccio), no force/kill/close/uninstall/MSI/reboot. Old TEMP/EMSI20261011 preserved UNVERIFIED not used. Screens second VM only: w4-backup-wake/volcheck/voltyped/volresult/dirtyped/dirresult/hashtyped/hashresult/runtyped/runresult/head/wheel/pid + newcmd/clear/enter/focus. Probe sources: VM/w4-backup-src/B.cs + SHA256SUMS.txt, ISO above. STOP gate; parent independent verification REQUIRED before force/uninstall. No commits.

## W4 bounded unit 2026-10-11 (readonly 17-file verifier media READY; NOT executed; gate pending; W4 open)

Second VM d9e70790-1faa-488b-a23a-a622d99cb202 running (query-uuid/status match before + after swap). Lite left RUNNING; no kill/close/guest typing by worker; V.Run NOT executed. First guest untouched; no source fix/commits.
Audited B.cs manifest shape (VM/w4-backup-src/B.cs 45493 SHA 00304646...): top keys backup_root/dest_path/dest_sha/dest_size/collections/items/quick_rows/quick_first/fk_rows/backup_finish_rc/src_close_rc/dest_close_rc/verify_close_rc/copy_count/mismatches/files[]; each file {rel,len,sha} with rel roots assets/research/staging. No key guessing.
NEW VM/w4-verify-src/V.cs 93 lines SHA df225dae3d14b390276433a7d178f1512ab2c1332cb3b66c22bf39a70942ee09 + SHA256SUMS.txt 296 bytes; NEW ISO w4-verify-20261011.iso 71680 bytes SHA 32b619e9810089c51c72e9e9da493439bb438d9f480dee3143b4347e1d3f9cdc vol ENTROPIA_VFY (existing pycdlib 1.22.0 venv, no global install). Joliet D:\V.cs + SHA256SUMS.txt verified on host.
V.Run reads ONLY TEMP/EMSI-Verified-20261011-051221 manifest.json + 17 copied files + entropia.sqlite: JavaScriptSerializer via reflection Assembly.Load System.Web.Extensions 4.0 PublicKeyToken31bf3856ad364e35 (no long ReferencedAssemblies CLI, no regex); rooted/escape/reparse/symlink reject; full path must stay under backup root; no APPDATA source reads, no writes/copies/sqlite/process/env changes. Pins: DB 1339392 / 039965617fef73adaeb8974050e5b7dbba64ceee571690279f4c20bfdbbba6b6 + manifest dest_sha/dest_size match; exact files count 17; per-file MATCH rel + count/mismatches + db_sha/size; single PASS line only when 17/17 + DB ok + disposed.
QMP eject + blockdev-change-medium to verify ISO (UUID targeted before swap); query-block: cdrom0=new ISO ro true tray closed, sysdisk unchanged. Prior ISOs preserved: backup 9a731835..., installer dbf2470d..., native 350e9f64..., sqlite2 8d2c412c..., sqlite-v1 b78b47f7... (all present, none deleted).
Entry for independent verifier ONLY: Add-Type -Path 'D:\V.cs';[V]::Run() (35 chars; UTF16LE b64 96 chars, powershell -enc ~112). Gate pending until independent fresh all-17-file + DB check. No commits.

## W4 bounded unit 2026-10-11 (V.cs RED InvalidCastException; V2 fix READY NOT executed; gate PENDING; W4 open)
RED: V.cs:54 `(ArrayList)d["files"]` wrong; JavaScriptSerializer returns object[] -> FAIL InvalidCastException. Source V.cs 93 lines immutable preserved. NEW VM/w4-verify2-src/V2.cs 95 lines SHA 721c50e75724f34428691242fa0cafded2e179bde877ca8fe589334be19c5c7b, class V2, SINGLE fix `fo as ICollection` + null guard (object[]+ArrayList), keep Count 17/foreach Dict/path/hash/DB/disposal/PASS-only-all; FAIL now type+message. No new libs, same reflection. NEW ISO w4-verify2-20261011.iso 69632 SHA a125f0f97624bc74d801789cd2e92a781ef5d76eb6b87250ed08ea51af1f89d6 vol ENTROPIA_VF2, Joliet V2.cs+SHA verified. QMP UUID d9e70790 running verified before/after; eject+change-medium to V2 ISO ro true tray closed, sysdisk unchanged, all old ISOs preserved. Entry verifier ONLY: Add-Type -Path 'D:\V2.cs';[V2]::Run() (37 chars, b64 100, -enc 116). V2 NOT executed; no force/uninstall/MSI/B.Run; host structural only (no Win .NET). No product/backup/data/metadata change. Gate PENDING for independent guest GREEN.

## W4 bounded unit 2026-10-11 (actual force-target + NSIS uninstall PASS + MSI FAILED 1603-type; data PRESERVED; W4 open)
Gate: parent conditional force satisfied by independent V2 GREEN (shots gate-v2-result.png count=17/17 mismatches=0 db_sha 039965617fef73adaeb8974050e5b7dbba64ceee571690279f4c20bfdbbba6b6 size=1339392 PASS readonly 17 files + db match; V2.cs 95 lines SHA 721c50e75724f34428691242fa0cafded2e179bde877ca8fe589334be19c5c7b; ISO w4-verify2-20261011.iso 69632 SHA a125f0f97624bc74d801789cd2e92a781ef5d76eb6b87250ed08ea51af1f89d6). Backup TEMP/EMSI-Verified-20261011-051221 + old EMSI20261011 kept, no deletions/restores/overwrites.
Second VM d9e70790-1faa-488b-a23a-a622d99cb202 running (query-uuid/status/block throughout); QEMU 1656972/swtpm 1656977 + first guest 1648596 running NOT killed; host-free 359G. DPMS wakes via rel-x only; no lock/creds/shares/clipboard/ports/hostdbcopy.
Revalidate: gps entropia-pro-desktop PID 6012 unique 444 handles 133.88s CPU; Path C:\Users\EntropiaTest\AppData\Local\EntropIA Lite\entropia-pro-desktop.exe (1.0.20 registry pin). Authorized Stop-Process -Id 6012 -Force ONLY (typed 96 -enc verified-before-Enter, once); gps -Id 6012 afterwards NoProcessFoundForGivenId -> PID absent observed. No generic kill/reboot.
Source after force (CDROM sqlite2 ISO 8d2c412c ro, S.cs R()): IntPtr 8, winsqlite 3.51.1, db 1339392 readonly_main=1, quick_check 1 ok, FK 0, target e929e121 in 62af3181 dummy-pdf_2 metadata 1040 prov key+hint True, assets 2 parent 7478 29550d77 valid + child 1373 756a1f4c valid (child DB size 0 WARN preserved), PASS. Closed manifest: lock 0, sqlite 1339392 5:37 AM, shm 32768 5:57 AM, wal 4120032 5:53 AM + assets/models/research/staging.
NSIS: registry UninstallString "C:\...\EntropIA Lite\uninstall.exe" verified; Start-Process full UI only, Delete-application-data UNCHECKED. Mouse clicks did NOT deliver (2x no-op, consistent with prior click-delivery history); keyboard Enter activated. Result Uninstallation Complete successfully; registry entry gone (PathNotFound) + install dir gone (PathNotFound). Manifest after uninstall IDENTICAL (1339392/32768/4120032/0 + same mtimes). No manual delete/registry fallback; no license/reboot prompt. hooks.nsh PREUNINSTALL INSTDIR-only consistent with observed binaries-only removal.
MSI: CDROM installer ISO dbf2470d (D ENTROPIA_LITE 49.47 MB); MSI EntropIA Lite_1.0.20_x64_en-US.msi SHA 8948e11b450d16ae174429ff3d5dd0287df0414cc149a9f3abe30c886edcd769 verified before exec. ProductCode/UpgradeCode/ALLUSERS NOT read via COM (not straightforward in SHORT budget; no guess). msiexec /i full UI + /L*v TEMP\MSI-Install-20261011.log (no /qn): Welcome->Destination default Local\EntropIA Lite->Ready->Install->UAC unknown-publisher CD/DVD Yes (guest install consent)->error 1722-type "problem with this Windows Installer package... did not finish as expected"->wizard "ended prematurely... system has not been modified"->Finish. MSI NOT installed (no success 0/3010). Log 328172 bytes 6:22 AM preserved. Manifest after failed MSI IDENTICAL (1339392/32768/4120032). No relaunch/retry/restore. Screens shots/w4-force-*, w4-nsis-*, w4-msi-* 20261011. No commits. Next: parent decides MSI 1722 triage (log) vs close W4 as NSIS-accepted/MSI-blocked; Pro/Rust-native/child-DB-size remain.

## W4 bounded unit 2026-10-11 (readonly MSI-log reader READY NOT executed; MSI FAILED stands; W4 open)
Second VM d9e70790-1faa-488b-a23a-a622d99cb202 running (UUID/status match before+after swap); CDROM now readonly w4-msilog-20261011.iso 69632 bytes SHA 9e04a2bb641b8164447d0a121a04365418ded371494fdd9faa94a577826b9ea9 vol ENTROPIA_MSI; sysdisk unchanged; all old ISOs/backups/screens preserved.
NEW VM/w4-msilog-src/L.cs 86 lines SHA f72b1f338ccbc05a6aeafc8b48a2f99e1f225d926fe6170faa1fb7f7a16fa006 class L entry Run(); reads ONLY TEMP\MSI-Install-20261011.log (ReadAllLines BOM autodetect) + HKCU/HKLM Uninstall EntropIA Lite limited DisplayName/Version/InstallLocation/WindowsInstaller/ProductCode(subkey) + LocalAppData bin exists + backup TEMP\EMSI-Verified-20261011-051221 exist/size/sha + APPDATA com.entropia.shared sqlite/wal/shm exist/len/sha; no writes/copy/SQL/Process/repair/kill; filtered FIRST fatal Error1722/Return value 3/CustomAction-actual + +-4 context dedup <=60 shown (total <=80); prints log size+SHA; cause from log lines only, no WebView inference; no broad dump/env/creds/keyring; no live-vs-snapshot same-hash claim.
MSI actual: 1722-type FAILED, wizard ended prematurely, system not modified; NO success 0/3010 claimed; guest log 328172 bytes preserved. Backup EMSI-Verified 039965617fef73adaeb8974050e5b7dbba64ceee571690279f4c20bfdbbba6b6/1339392 intact, no restore. Post-fail manifest IDENTICAL by SIZE only (1339392/32768/4120032), not bit-for-bit.
Screen shots/w4-msi-log-screen-20261011.png: console APPDATA listing + flooded WLAJ line, NO StickyKeys dialog; worker sent no guest typing (QMP status/uuid/block + screendump only, python -B, pycache untouched); no input/accessibility policy change.
Entry verifier ONLY: Add-Type -Path 'D:\L.cs';[L]::Run(). L.Run NOT executed by worker; no retry/reinstall/fix/restore/force/kill. Gate PENDING. No commits.

## W4 bounded unit 2026-10-11 (actual L-reader diagnosis; MSI FAILED WebView2 bootstrapper; W4 open)
Independent L.cs f72b1f338ccbc05a6aeafc8b48a2f99e1f225d926fe6170faa1fb7f7a16fa006 readonly ISO w4-msilog-20261011.iso 69632 SHA 9e04a2bb641b8164447d0a121a04365418ded371494fdd9faa94a577826b9ea9 audited/executed once via reliable 0.03s event spacing. Coherent online backup EMSI-Verified-20261011-051221 DB SHA 039965617fef73adaeb8974050e5b7dbba64ceee571690279f4c20bfdbbba6b6 quick_ok FK0, 4 collections 14 items, 17 assets fresh hash match. Human conditional force exercised ONLY after gate verified. Lite 6012 path revalidated, killed PID absent; NSIS removed data-checkbox unchecked, entry+binary gone. MSI FULL UI failed rollback/premature finish; NOT installed. Source lengths checked before/after but NO hash baseline, so qualify preservation: no bit-identical claim; originals present + verified backup hash fresh unchanged. Backup must not be mutated/restored without next decision. Root MSI log %TEMP%\MSI-Install-20261011.log 328172B 1704 lines SHA 798e9abcb9f0e6e4138360bd3935a6e5dc8fd565594da3382fd3dc926ed8eb77: FIRST 1047 CustomAction ForceInvokeWebView2Bootstrapper returned -2147219416=0x80040828; Error1722 lines 1048/1051 Path C:\Windows\Installer\MSI8DB5.tmp args /silent/install deferredActionType 1026 BinaryData, UAC accepted, InstallFinalize Return 3 line 1056. Repo windows/fragments/force-webview2-bootstrapper.wxs 45-55 Return check NOT REMOVE before InstallFinalize; tauri.windows.cfg includes fragment. VM prior WebView2 155.0.4283.45 verified; EdgeUpdater reason NOT read. External winget issue 192036 maps 0x80040828 to Runtime already installed; Learn recommends detect pv>0.0.0.0 HKLM WOW/HKCU Client GUID. Label correlation not conclusive. No fix (never ignore-all-errors) implemented; parent ASKs next specific permission for fix/tests/commit/push/Lite build (no tags/releases). Current no matching Lite uninstall entry/local exe absent (no new-version claim). Source current DB SHA 530cf7226f1d0b13dc82bcfe438238795b631bc13ca2bbc9ec84a33f699f3df0; WAL SHA 315c548c06a2257a0df63da0efcd6af35bc376fea94f90aa493530ac0d33feba; SHM 18509a9d8f6530e8a3d62f310a69f8b18b4b13bf063e12eca1c62e4de8aba4c7. DB vs merged-snapshot hash difference NOT corruption proof. Post-fail logical rows not recently queried; no preservation-completion invent. ASSESS unassessable untracked doc; independent functional verifier completed, no native approval claim. W4 OPEN (Pro/Rust-native/DB child-size). No commits.

## W4 current 2026-10-11 — corrected MSI functional PASS (historical pre-fix FAILED preserved above; W4 still OPEN)

This section is the current result. All older MSI FAILED notes above are now historical pre-fix failures and are preserved as history. The Windows task as a whole is NOT marked complete. Native review cleanup stays deferred with read-only views.

The corrected MSI (SHA256 335c8dc7d6f510e69e9562f8e653500d9f16d055a77d837e1e116abaefdbae21, 30707712 bytes, built from source 3b) was installed once through the full user interface. The human authorized exactly one normal install, and the UAC authorization was exercised: the user accepted one normal unknown-publisher prompt from CD/DVD source. No other licence, privacy, reboot, or security prompt appeared. The package is actually unsigned and does not inherit a Microsoft signature.

The install completed with the wizard reporting completion at the default current-user path C:\Users\EntropiaTest\AppData\Local\EntropIA Lite\. The machine shows exactly one 64-bit Windows Installer registration under HKLM, GUID {1AD2370F-66DB-429F-AE0C-A8EE4DC1D4DE}, version 1.0.20, publisher entropia; read-only registry UninstallString `MsiExec.exe /X{1AD2370F-66DB-429F-AE0C-A8EE4DC1D4DE}` was observed, NOT EXECUTED. No uninstallation, removal, or repair was performed; the app remained OPEN as PID 10792. It is not an NSIS entry and not an HKCU entry; it is one HKLM 64-bit Windows Installer registration, and every other registry scan found zero entries. The installed program is 50671616 bytes, 64-bit, version 1.0.20. The Finish-page launch box was left unchecked while preservation was checked, then the app was started once: PID 10792 showed the real EntropIA Lite window from the same path, with child WebView PID 9884 under parent 10792, 5 related processes, and file version 155.0.4283.45. The app was left open on the selected PDF with no forced closure.

The fresh log %TEMP%\msifixed-20261011.log reports the forced WebView2 bootstrapper action return code 1, InstallFinalize return code 1, Logger MainEngineThread server return code 0 and client return code 0 (successful), installation completion status 0, and no Return value 3 appeared. These are return codes, not counts. No raw bootstrapper child exit-code 0 proof was established. The log alone does not prove the numeric helper mapping, so no such claim is made. The older 1722 log is archived separately.

Integrity was checked before install, after install before launch, and after startup while the app was alive. Every run reported PREFLIGHT_PASS plus True, read-only main database 1, quick_check exactly one row ok, zero foreign-key violations, 4 collections and 14 items, target e929e121 present once, 17 live files matched 17 of 17 with zero bad, zero finalizers, zero final close, and zero static-close leaks. The fresh post-install V2 check matched 17 of 17 with zero mismatches against the 1339392-byte backup database SHA 039965617fef73adaeb8974050e5b7dbba64ceee571690279f4c20bfdbbba6b6. The old coherent backup at %TEMP%\EMSI-Verified-20261011-051221 was left immutable with no restore. V2 was not rerun after startup. WAL and SHM were both present, read through a read-only SQLite link, with no physical bit-identity claim. The child asset still reads DB size 0 against 1373 bytes on disk; that follow-up is unchanged.

The library still shows 4 collections and 14 documents (Actas 2, Casas 6, La Capital 5, IA Windows Test 1). The existing IA item e929e121 with dummy-pdf_2 displayed the real text Dummy PDF download. That visible display uses PdfJS and does not prove native Rust Pdfium. The old thumbnail cache stays uncorrelated and NOT VERIFIED, and Pro with local ML stays NOT VERIFIED. Screenshots are protected under R=/home/agusnieto77/.local/share/entropia-windows-test/vm-20261010-211936-unique/shots/ with the exact .png suffix, and the V4 diagnostic ISO (30801920 bytes, SHA 116371c712ddae82274ae24df217ea7504667d9388f8d0c60ce94df988b57822, from source P27e2d3eb4bdff8304c6bca4ddfa1d1e9e307aca50ce056cda85e3669009167e3) is immutable read-only. Rejected audit probes were preserved and never run in the guest. Historical: Evidence C5 0652684a was source documentation; current evidence commit is `e4dc60a1981f738c6d6ca025917752ed11ce7f7e` on unchanged source 3b (doc commits local UNPUSHED). Guest app still OPEN as PID 10792, no new ops. Still OPEN: Pro with Rust Pdfium integration, additional IA cases, and the child-asset DB size bug (W4/task 27 remain OPEN). No commits by this worker.

## W4 bounded unit 2026-10-11 (current native-thumbnail plan; MSI closed; NO guest change yet)

Baseline (actual, app still open): VM d9e70790-1faa-488b-a23a-a622d99cb202, Lite source 3b, app PID 10792. Loaded-Pdfium count 0 is a snapshot only, not a diagnosis. Cache holds only daedde35-dabb-459a-9a46-9e003bde473f.png, 7425 B, 400x566, SHA E3990E3B3646856B7FC877390F74D04113C3313D0B21EA70668FAC57F2F6AE46, UTC 2026-10-10T21:03:11.8897890Z. Installed DLL version 144.0.7543.0, SHA 6b963c2be9cacbaa0c0c7f4bf6d20d2fd16729ebdaa9989978b0f7b119c1c1cb. Read-only P baseline: 17 live files, 0 bad, readonly 1, quick_check ok, FK 0, 4 collections / 14 items, target item 1, final-close 0, True. Host free 382405390336 bytes protected. First VM, secrets, and archives untouched.

Next bounded unit (needs parent authority after user resolve-pending): create ONE new guest-only TEMP public PDF fixture with a different SHA; do not change the old original or cache. Use normal UI: new ASCII collection plus local import to trigger a CARD thumbnail; no direct FPDF invoke/probe. Expect 5 collections / 15 docs if 1 PDF with 1 page; keep existing 4/14. Old snapshot guard P4 hardcodes 4/14, so do NOT rerun old P after the addition and expect false; use a new read-only asset mapping/integrity proof scoped to post-test counts, or an existing source API. Correlate the NEW asset ID from read-only SQLite/UI metadata with cache-absent BEFORE and rendered 400-wide PNG AFTER, plus DLL loaded under same app PID 10792 with installed path and hash. No source fix, CI, cache deletion, forced close, restore, backup overwrite, global settings, ports, or keys. If dedup reuses an asset, do NOT claim PASS; record uncertainty and STOP without deleting old data. Fixture creation/import NOT executed yet; trial types stay 4/14. PDF viewer and direct-DLL render do not prove Rust. No meaningful TDD RED is available for passive probes; functional acceptance checks are required.

W4 stays OPEN: Pro artifact permission is a new record 615; separate workflow still has unsafe old release params and needs a bounded tested CI edit before build. No Pro dispatch yet. Child DB size 0 needs diagnosis before fix. UI/Linux/non-closing/media-version/cache-cleanup stay separate.

## W4 bounded unit 2026-10-11 (actual native-thumbnail trial + bounded security BLOCK; native integration NOT PASS; W4 OPEN)

Fixture (new, outside source): `.git/entropia-native-fixture-20261011/P.pdf` 608 B SHA256 7f9ffea8451c82e61833b4244fc901e7e0582bf2a6cccd74b4454477630017b8, one-page text `Native Rust Pdfium Proof20261011`, valid PDF per independent pdfinfo/pdftotext. ISO 65536 B SHA256 b7b6a2c184ee1b2a770c535efee96eee5ffd90286ff38d06fc88c3f90c5f731c with exactly 2 files P.pdf/SHA256SUMS.txt. Verifier initial NOGO (mode 0664) repaired: parent-owned new ISO chmod 444, freshly PASS regular-file mode 444/digest before guest mount. Current readonly DVD is this ISO; previous v4 host ISO preserved. No source/guest/commit/push/CI/review/root-cleanup/credentials.

BEFORE trial P.Run PASS: 4 collections / 14 docs / 17 source files, readonly 1, quick_check ok, FK 0, final-close 0, True; app 10792 alive; cache ONLY daedde35-dabb-459a-9a46-9e003bde473f.png 7425 B SHA E3990E3B3646856B7FC877390F74D04113C3313D0B21EA70668FAC57F2F6AE46 400x566, loaded-pdfium 0, time 2026-10-11T15:53:14.4450222-03:00.

Trial: created exact new collection `Native Rust Pdfium Test20261011` + D:/P.pdf import via normal UI. Picker first rejected forward-slash (NO import), then normal backslash picked ONE file. Visible PDF one page + card thumbnail text rotated (unresolved; do NOT call renderer PASS). UI shows 5 collections, old counts 2/6/5/1 retained, total 15 arithmetic, NOT fresh SQL. Do NOT record source-17 post hash PASS nor backup post PASS. App/new data remain; no cleanup/kill/relaunch.

BLOCKED: verifier used UNNECESSARY `powershell -enc` readonly module query. Reported Windows Security `Threats found`; subsequent captured `powershell.exe AccessDenied / ResourceUnavailable / NativeCommandFailed`. Parent independent PNG read + scout confirmed ERROR ONLY, NOT threat-toast in shots; detection/cause/quarantine UNKNOWN. Encoding reduces auditability; do NOT reuse. Stopped guest inputs: no retry/allow/exclusion/toggle/restore/dismiss/security-override. Human approval required for next readonly ProtectionHistory inspection before more guest diagnostics.

NOT VERIFIED (so native app integration NOT PASS): same app PID AFTER, DLL, cache NEW GUID PNG mtime/hash, correlated asset ID, post SQL integrity, original 17 file hashes, backup revalidation. Old P.Run hardcodes 4/14 so do NOT run it after new 5/15 test data. MSI unit DONE unchanged. Pro permission 615 = workflow commit/push/build artifacts only, NO releases/tags/PR; NO CI code change or build yet.

Evidence: R shots nativeunit-{preflight-result,cachebaseline-result,fixturehash-result,created,import-start,queue11s,module-command,security-stop}.png. Incident memory 616 = source proof. This bounded incident is NOT a product-bug cause. Past baseline/MSI receipts above preserved unchanged. No future guest action.

## W4 reconciliation 2026-10-11 (CURRENT: native Pdfium integration subcheck PASS; W4 still OPEN)
Passive mirror of `odd/tasks/windows-native-pdfium.md` CURRENT R4 PASS; historical V3 FAIL + MSI FAILED history preserved above unchanged. R4 unit 630 ONE Run PASS `R4_CORRELATION_PASS` + True (R4.cs 68685 `58e158bc...`, ISO 122880 `a7dc4b9a...`, both 444 DVD readonly). Target `83342a03...` -> item `40b166ce...` -> col `990dfc88...` Native Rust Pdfium Test 20261011; source 608 B `7f9ffea8...`; NEW cache PNG 11501 B `B9C87E23...` 300x400 UTC 2026-10-11T18:58:57.1294797Z (pre ONLY old -> post NEW); old `daedde35...` retained, 2/2 pinned. LIVE 5/15 (1,1,2,5,6), backup PRE=POST 1339392 `03996561...`, 17/17 bad 0, readonly 1 quick ok FK 0 closes 0. DB-15 + 2 staging job.json extras = 17 (paths in native file). ES incident: ONE unsubmitted line Escape-cleared, plain commands, no encoding, NEW alert not observed. Console clean prompt; app 10792 last alive prior check. Native Pdfium app-integration subcheck now PASS (NOT pdf.js/direct-DLL). Whole W4 remains OPEN: Pro, additional IA cases, child DB size 0. MSI completed unchanged. No all-Windows-DONE claim. No commit (Native Doc Git grant required; 615 Pro grant different, NOT used). Evidence: native-correlation-v4 shots under vm-20261010-211936-unique/shots/.
