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
   and stamps the 4-segment Store version (default `1.8.5.0`, i.e. the
   `apps/desktop/package.json` version plus `.0`, passed by the Release workflow).
3. Regenerates every existing `Assets/*.png` from the canonical
   `apps/desktop/src-tauri/icons/icon.png`, preserving each package asset's
   required pixel dimensions.
4. **Swaps in the freshly built lean `entropia-lite-desktop.exe`** over the one in
   the captured payload, and adds `resources/lib/pdfium.dll` (`-PdfiumPath`,
   `scripts/store-msix-payload.ps1`). The capture never had it, so up to 1.0.15
   the Store build could not open, split or crop a PDF; the app looks for it
   beside the exe, where the NSIS/MSI installers also put it.

   The assistant connector (`-ConnectorPath`, built by
   `apps/desktop/scripts/build-agent-connector.mjs`) is added the same way, beside
   the app exe, and the manifest declares a `uap3:AppExecutionAlias`
   `entropia-agent-connector.exe` for it (D15). The alias is required: the
   packaged exe sits under the versioned `WindowsApps` directory, which Windows
   refuses to launch from an ordinary command line. The repack asserts both the
   alias and the packaged bytes.
5. Strips `AppxBlockMap.xml` / `AppxSignature.p7x` / `[Content_Types].xml`
   (regenerated on pack) — the MSIX ships **unsigned**; the Microsoft Store
   applies its own signature.
6. **Removes every shortcut inherited from the MSI capture** (see below).
7. Asserts the application registration (identity, `Application Id`,
   `Executable`, `EntryPoint`) before packing.
8. Repacks with `makeappx`, reports identity, compares every packaged icon
   byte-for-byte with the generated payload, re-verifies that no shortcut
   survived into the packed archive, checks that the packed Pdfium is
   byte-identical to the one given, and checks that the packed connector is
   byte-identical to the one given.

## Shortcuts: why the Store package has none

The base MSIX was captured from the Win32 MSI, so it inherited two MSI-era
shortcut mechanisms — both of which are wrong for a Store package:

| Inherited from the capture                                                                                                   | Why it breaks                                                                                                                                                                                                                                                                                                                       |
| ---------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `VFS\Common Desktop\EntropIA Lite.lnk`, `VFS\Common Programs\EntropIA Lite\EntropIA Lite.lnk`, `Uninstall EntropIA Lite.lnk` | MSIX deploys these `.lnk` files verbatim. Their target resolves to `C:\Program Files\WindowsApps\CONICET.EntropIALite_<version>_x64__b16na7gwepwme\entropia-lite-desktop.exe`, and Windows refuses to launch an ordinary shortcut from `WindowsApps` — the user gets _"Windows cannot access the specified device, path, or file"_. |
| Two `desktop7:Extension Category="windows.shortcut"` declarations                                                            | Their `Icon` pointed at `[{Package}]\entropia-lite-desktop.exe`, a path that carries the package version and therefore breaks on every Store update.                                                                                                                                                                                |

A Store package needs neither. The `<Application>` node alone registers the app
with Start, taskbar pinning and Windows Search through the package identity,
which is exactly why those three launch paths kept working while the desktop
shortcut did not.

`scripts/store-msix-shortcuts.ps1` strips both mechanisms and guards them:
`Assert-StoreMsixAppRegistration` runs before `makeappx pack` and
`Assert-StoreMsixShortcutHygieneInArchive` runs after it, so a re-captured base
that reintroduces a shortcut fails the release instead of shipping.

**ACLs on `C:\Program Files\WindowsApps` are never touched** — the fix is to
stop producing an invalid shortcut, not to widen permissions on a
system-protected directory.

### If a desktop shortcut is ever wanted again

Never point it at the physical exe. Target the registered application identity:

```text
PackageFamilyName  CONICET.EntropIALite_b16na7gwepwme
Application Id     EntropIALite
AUMID              CONICET.EntropIALite_b16na7gwepwme!EntropIALite
Shortcut target    explorer.exe shell:AppsFolder\CONICET.EntropIALite_b16na7gwepwme!EntropIALite
```

The package family name is derived from the Store identity name and publisher
only — it carries **no version segment** — so an AUMID shortcut keeps working
across Store updates. The repack prints both values on every run
(`PackageFamilyName` / `Aumid`).

The Win32 **MSI and NSIS installers are unaffected** and keep their classic
desktop shortcut: under `C:\Program Files` a direct `.exe` target is valid.

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

## EntropIA Pro: generated MSIX (no base)

Pro has no captured base: `scripts/build-pro-store-msix.ps1` (entry point,
helpers in `scripts/store-msix-pro.ps1`) builds the Pro Store MSIX from scratch,
locally only — no CI leg builds it and the Release workflow is unchanged.

The script takes the Pro MSI layout instead of a capture: it runs `msiexec /a`
(administrative extraction — files only, no custom actions), copies the layout
of the directory holding `entropia-pro-desktop.exe`, and drops every `.lnk`.
It renders `Assets/` (`StoreLogo` 50, `Square44x44`, `Square71x71`,
`Square150x150`, `Square310x310`, `Wide310x150`) from `../icons/icon.png`, and
writes the AppxManifest from scratch: x64, `runFullTrust` only, no
PackageDependency, `TargetDeviceFamily Windows.Desktop 10.0.17763.0`, `en-us`
resource, Application Id `EntropIAPro`.

The manifest declares the execution alias `entropia-pro-agent-connector.exe` —
its own name so it does not collide with Lite's
`entropia-agent-connector.exe`. `automation/connector.rs` (`CONNECTOR_ALIAS`)
is variant-specific and names that same alias when packaged. The script asserts
the registration and the alias before `makeappx pack`, then asserts icons,
shortcut hygiene, and Pdfium/connector bytes afterwards. It outputs
`EntropIAPro-Store-<StoreVersion>.msix` and prints the PackageFamilyName,
AUMID, and SHA256.

The Store identity has no default: `-IdentityName`, `-IdentityPublisher`
(`CN=...`), and `-PublisherDisplayName` are mandatory, taken from Partner
Center > Product management > Product identity (release 1.8.0 is the first Pro
submission). The Partner Center values are:

```text
Name                 = CONICET.EntropIAPro
Publisher            = CN=89DF40E5-581A-4120-9A24-F701205485D6   (same publisher account as Lite)
PublisherDisplayName = HLab
PackageFamilyName    = CONICET.EntropIAPro_b16na7gwepwme
AUMID                = CONICET.EntropIAPro_b16na7gwepwme!EntropIAPro
```

They stay parameters, not literals in the script, but `store-msix-pro.Tests.ps1`
pins the package family name they produce. `-StoreVersion` defaults to the app version plus `.0` and is kept in sync by
`pnpm version:bump` (`apps/desktop/scripts/version.mjs`).

The prerequisite MSI comes from a Pro release build, run from `apps/desktop`
after building the connector:

```powershell
node apps/desktop/scripts/build-agent-connector.mjs --target x86_64-pc-windows-msvc --variant pro
pnpm exec tauri build --features local-ml --bundles msi --config src-tauri/tauri.connector.pro.conf.json
```

with the three `ENTROPIA_RUNTIME_BOOTSTRAP_*` variables set as in
`.github/workflows/release.yml` (MNN compiles from source on the first build,
~30 min). Then:

```powershell
./apps/desktop/src-tauri/scripts/build-pro-store-msix.ps1 -MsiPath "apps/desktop/src-tauri/target/release/bundle/msi/EntropIA Pro_1.8.0_x64_en-US.msi" -IdentityName CONICET.EntropIAPro -IdentityPublisher "CN=89DF40E5-581A-4120-9A24-F701205485D6" -PublisherDisplayName HLab -OutDir .tmp/pro-msix
```

WebView2: the MSIX cannot run the installer's bootstrapper, so it relies on
the system Evergreen WebView2 runtime (preinstalled on Windows 11 and current
Windows 10).

Tests live in `scripts/tests/store-msix-pro.Tests.ps1` (Pester, in the CI
Pester suite list; the real-pack test skips without the Windows SDK). The
in-app Store update notice (`src/store_updates.rs`) stays Lite-only for now.
