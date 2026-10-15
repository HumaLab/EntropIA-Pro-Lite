# Windows Pro guest install and Internet Archive collection test (2026-10-14)

User authorized (2026-10-14): push of `feat/internet-archive`, Pro installation in the isolated VM, and an Internet Archive test with a collection of 10 to 100 items. One-time read of the guest login password was authorized to unlock the VM; the secret was typed through QMP without being printed or logged. The recovery file was not read.

## Specs
- S1: Install the Pro build from CI run 38524775653 in the isolated Windows VM and verify startup, WebView2 and preservation of existing data.
- S2: Test Internet Archive import with a collection of 10 to 100 items without overloading the task.

## Tasks
- T1 (S1) Install Pro MSI, launch once, check data. DONE.
- T2 (S2) Browse a small collection, import a few items, verify pages, viewer and deduplication. DONE.
- T3 Record evidence. DONE (this document).

## Log
### Media
Artifact `windows-pro-installers` (id 11876854268, head `18aafaae`) downloaded on the host:
- MSI `EntropIA Pro_1.0.20_x64_en-US.msi` 82567168 B SHA256 `08089fa85eddd8939b4d34fadd6a6da2a40f67f68adbc947612c0898b92037b2`
- NSIS `EntropIA Pro_1.0.20_x64-setup.exe` 61806220 B SHA256 `736d0b7e6234386803e0044e1f73f13672c28584d20738c994d6a4aaad298f23`

Sizes match the CI guard log. Read-only ISO `pro-installers-20261014.iso` (SHA256 `75835ca3c08afd7917b0b8699cc59739cdb362f109b58a945f4c50faebc178ca`, label `ENTROPIA_PRO`) mounted on `cdrom0`. In the guest, `Get-FileHash D:\pro.msi` returned the same MSI hash.

### Pre-install baseline (guest)
- `entropia.sqlite` 1351680 B SHA256 `E26D92045BAE4EB2CFE0770EE1F1027A787F981973890BC192F411EE6051FEE6`
- `entropia.sqlite-wal` 4120032 B SHA256 `47577F6735B9B29606A3AD0B2C81917908FC2FCCA67FF5DB0D34D7A637D6D7A8`
- `assets`: 17 files, 5322614 B. No EntropIA process running before install (Lite PID 10792 had ended).

### T1: Pro MSI install (PASS)
- `msiexec /i D:\pro.msi /L*v %TEMP%\pro-msi-20261014.log`. Default per-machine path `C:\Program Files\EntropIA Pro\`.
- One expected UAC prompt: `D:\pro.msi`, publisher Unknown, origin CD/DVD drive. Answered Yes once.
- Log: `ForceInvokeWebView2Bootstrapper` ran with return value 1, `InstallFinalize` return value 1, `Installation completed successfully`, product version 1.0.20.
- "Launch EntropIA Pro" was unchecked before Finish. The app was then started once with `Start-Process`.
- `entropia-pro-desktop.exe` is 60894208 B and sits next to `resources`, `scripts` and `target`.
- Pro started and WebView2 rendered. The library showed 5 collections and 15 documents, all previously existing data.

Finding (not a regression of this work): Pro shows a banner "Runtime de release pendiente para windows-x86_64 (modo desarrollo activo)". OCR, transcription and NLP are blocked, and there is no "Reparar runtime" button in the `fixture` state. Expected for a lean installer whose real runtime downloads on first launch. Whether the bootstrap download should have started automatically was not investigated.

### T2: Internet Archive collection test (PASS)
Collection `habla-san-diego-periodico-sevilla-espana`:
- 97 texts, 0 access-restricted, license CC BY-ND 4.0.
- PDFs of 1 to 10 MB; most items carry an original PDF plus an IA-derived one.

Rejected candidates:
- `ministerio-de-cultura-y-educacion-argentina`: 53 items, but each 50 to 424 MB.
- `pub_*` journals and `fav-*` personal lists.

Steps and results:
- Collection tab: "Página 1 de 10", 10 items per page. Navigation Siguiente/Anterior worked and the selection was kept across pages.
- Selected 4 PDFs from 3 items:
  - `diciembre-1991-n-15`: original 2.6 MB plus derived 1.2 MB, so two files from one item.
  - `abril-1999-n-90`: 1.1 MB.
  - `abril-2000-n-98`: 2.7 MB.
- Preparation: 4 files, 7.5 MB known size, no duplicates, estimated synced volume 15 MB.
- Import into the new collection "Habla San Diego IA Test" finished with 4 imported, 0 already present, 0 errors.
- The collection shows 4 documents and 56 pages with rendered thumbnails. The `n-98` viewer showed page 1/16. The corpus went from 5/15 to 6/19.
- Host cross-check: the same 4 PDFs downloaded from archive.org have 16 + 16 + 12 + 12 = 56 pages (`pdfinfo`). Sizes match: 2680380, 1206276, 1140668 and 2837092 B.
- Link tab with the full URL `https://archive.org/details/habla-san-diego-periodico-abril-1999-n-90-sevilla-espana` resolved the item.
- Re-importing it into the same collection gave a preparation of 0 files, marked "Ya importado en esta colección", 0 B sync. The final result was 0 imported, 1 already present, 0 errors.

UX observations:
- The destination chosen in "Importar fuentes" is not carried into the Internet Archive dialog, which asks for the destination again.
- After returning to Inicio the layout shifted down by one "Volver" row.

### Not verified
- SQLite integrity after these imports (no new probe was run) and per-page `size` of the new page assets. This build predates fix `7f908c60`, so new split pages are expected to keep `size` NULL.
- Pro ML runtime bootstrap, OCR and transcription.
