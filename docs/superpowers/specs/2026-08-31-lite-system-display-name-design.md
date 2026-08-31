# EntropIA Lite System Display Name Design

## Problem

The Lite package already declares `productName: "EntropIA Lite"`, so installer and bundle metadata use the correct edition name. Its Tauri overlay does not override the native main-window definition, however, so the Lite build inherits `app.windows[0].title: "EntropIA Pro"` from the base configuration. Windows displays that inherited title in the taskbar thumbnail preview and other native window surfaces.

## Scope

Update the Lite Tauri configuration and add focused configuration coverage. Preserve the Pro name for Pro builds, the existing Lite identifier, frontend product-name selection, window geometry, decorations, visibility, and all unrelated bundle metadata.

## Configuration contract

`apps/desktop/src-tauri/tauri.lite.conf.json` must define both OS-visible Lite names:

- `productName` remains `EntropIA Lite` for bundle, installer, and installed-application metadata.
- `app.windows[0].title` becomes `EntropIA Lite` for the native window title and Windows taskbar thumbnail caption.

Tauri applies variant files with JSON Merge Patch semantics. Because arrays are replaced rather than merged element-by-element, the Lite overlay must include the complete main-window object. It will copy the base window settings unchanged and replace only the title. The base Pro configuration remains untouched.

No runtime `setTitle` call will be added. A configuration-time title avoids a startup interval with the wrong caption and keeps native metadata in its existing source of truth.

## Verification

Add a focused Vitest configuration test that fails before the change and verifies:

1. The Lite `productName` is `EntropIA Lite`.
2. The Lite native main-window title is `EntropIA Lite`.
3. The Lite window override preserves the base main-window settings other than the edition-specific title.
4. The base Pro product and window names remain `EntropIA Pro`.

Run the focused test, Lite frontend typecheck, and a Lite Tauri configuration smoke check. If the local desktop runtime is available without triggering a full bundle build, launch the Lite configuration and confirm the actual native window/taskbar preview caption.