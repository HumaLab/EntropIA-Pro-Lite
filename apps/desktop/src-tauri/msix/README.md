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
   and stamps the 4-segment Store version (default `1.0.9.0`).
3. **Swaps in the freshly built lean `entropia-lite-desktop.exe`** over the one in
   the captured payload.
4. Strips `AppxBlockMap.xml` / `AppxSignature.p7x` / `[Content_Types].xml` (regenerated
   on pack) — the MSIX ships **unsigned**; the Microsoft Store applies its own signature.
5. Repacks with `makeappx` and re-reads the manifest to verify identity.

So **routine releases only swap the exe + bump the version** — they reuse this
captured base verbatim. No VM is needed per release.

## When to re-capture (manual, needs the Hyper-V VM)

Re-capture this fixture ONLY if the package's captured shape changes, e.g.:

- the app's window assets / icons in the package change,
- the declared **capabilities** change,
- the **VC-runtime DLL set** or other VFS payload changes,
- the AppxManifest dependencies (e.g. `Microsoft.WindowsAppRuntime`) change.

Re-capture is **not** CI-automatable: stock GitHub-hosted Windows runners have no
nested virtualization / Hyper-V. Run the capture locally with the
`run-hyperv-msix-*.ps1` orchestration in the EntropIA-Lite repo
(`.tmp/msix-vm/`), then hand the new base back into this path. A plain version
bump or exe change does **not** require re-capture — the repack handles those.
