# Vendored base MSIX — `EntropIALite-base.msix`

## What this is

`EntropIALite-base.msix` is the **captured base MSIX** for the EntropIA Lite
Microsoft Store package. It was produced ONCE by the MSIX Packaging Tool inside a
full Hyper-V Windows 11 VM (the capture driver `Msix.PackagingTool.Driver` does
not install in Windows Sandbox, so a real VM is required). It carries the captured
VFS / Assets / Registry / `Resources.pri` shape plus a placeholder identity:

```text
Name                 = EntropIA.Lite
Publisher            = CN=EntropIA Lite
Version              = 0.1.0.0
PublisherDisplayName = EntropIA Lite
```

These placeholders are **rewritten** to the real Partner Center identity by the
repack (see below). Size on disk: `8,237,198` bytes.

## How it is used in CI

`apps/desktop/src-tauri/scripts/repack-store-msix.ps1` consumes this fixture on
the `lite` leg of `.github/workflows/release.yml`. The repack:

1. Unpacks this base with `makeappx`.
2. Rewrites the AppxManifest identity to the exact Store values
   (`CONICET.EntropIALite` / `CN=89DF40E5-581A-4120-9A24-F701205485D6` / `HLab`)
   and stamps the 4-segment Store version (default `1.0.11.0`).
3. Regenerates every existing `Assets/*.png` from the canonical
   `apps/desktop/src-tauri/icons/icon.png`, preserving each package asset's
   required pixel dimensions.
4. **Swaps in the freshly built lean `entropia-lite-desktop.exe`** over the one in
   the captured payload.
5. Strips `AppxBlockMap.xml` / `AppxSignature.p7x` / `[Content_Types].xml`
   (regenerated on pack) — the MSIX ships **unsigned**; the Microsoft Store
   applies its own signature.
6. Repacks with `makeappx`, reports identity, and compares every packaged icon
   byte-for-byte with the generated payload.

Routine releases therefore replace the EXE, version, and complete visual asset
set. A logo change in `icons/icon.png` automatically reaches the Store package;
stale artwork captured inside this fixture cannot survive a successful repack.

## When to re-capture (manual, needs the Hyper-V VM)

Re-capture this fixture only when the package's captured **shape** changes, for
example:

- visual asset filenames or AppxManifest visual declarations change,
- the declared **capabilities** change,
- the **VC-runtime DLL set** or other VFS payload changes,
- the AppxManifest dependencies (for example `Microsoft.WindowsAppRuntime`) change.

Changing only the artwork does **not** require re-capture while the existing
asset filenames remain valid; the repack regenerates those PNGs from the
canonical icon.

Shape changes are **not** CI-automatable: stock GitHub-hosted Windows runners
have no nested virtualization / Hyper-V. Run the capture locally with the
`run-hyperv-msix-*.ps1` orchestration in the EntropIA-Lite repo
(`.tmp/msix-vm/`), then hand the new base back into this path.
