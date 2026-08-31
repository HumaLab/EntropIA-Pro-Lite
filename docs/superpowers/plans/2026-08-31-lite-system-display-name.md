# EntropIA Lite System Display Name Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every Lite build expose `EntropIA Lite` as both its packaged application name and native Windows window/taskbar-preview title.

**Architecture:** Keep edition branding in Tauri configuration. Extend the existing Lite overlay with a complete main-window replacement whose behavior matches the Pro base window except for its title; add a focused Vitest contract test over the real JSON configurations.

**Tech Stack:** Tauri 2 JSON configuration, TypeScript, Vitest, pnpm 9, Svelte/Vite desktop workspace.

## Global Constraints

- Preserve `EntropIA Pro` in the base Pro configuration.
- Preserve the Lite identifier `com.entropia.lite` and version `1.0.9`.
- Preserve the base window width, height, minimum dimensions, decorations, resizability, centering, and initial visibility.
- Keep the existing frontend title selection in `vite.config.ts` and `index.html` unchanged.
- Do not add a runtime `setTitle` call or trigger a full Tauri bundle build.
- Build and typecheck the Lite frontend with `VITE_LOCAL_ML=0`; do not enable Rust feature `local-ml`.

---

### Task 1: Correct and verify Lite system display metadata

**Files:**
- Create: `apps/desktop/src/lib/system-display-name.test.ts`
- Modify: `apps/desktop/src-tauri/tauri.lite.conf.json:1-5`

**Interfaces:**
- Consumes: Tauri's JSON Merge Patch overlay behavior for `tauri build/dev --config src-tauri/tauri.lite.conf.json`.
- Produces: `tauri.lite.conf.json` with `productName: "EntropIA Lite"` and `app.windows[0].title: "EntropIA Lite"`, while all non-title window properties remain equal to the base Pro window.

- [ ] **Step 1: Write the failing configuration contract test**

Create `apps/desktop/src/lib/system-display-name.test.ts`:

```ts
import { describe, expect, it } from 'vitest'
import liteTauriConfigJson from '../../src-tauri/tauri.lite.conf.json' with { type: 'json' }
import proTauriConfigJson from '../../src-tauri/tauri.conf.json' with { type: 'json' }

type WindowConfig = Record<string, unknown> & { title?: string }

interface SystemDisplayConfig {
  productName: string
  app?: {
    windows?: WindowConfig[]
  }
}

const liteTauriConfig = liteTauriConfigJson as SystemDisplayConfig
const proTauriConfig = proTauriConfigJson as SystemDisplayConfig

function windowSettingsWithoutTitle(window: WindowConfig | undefined): Record<string, unknown> {
  expect(window).toBeDefined()
  const { title: _title, ...settings } = window ?? {}
  return settings
}

describe('Tauri system display metadata', () => {
  it('uses edition-specific names without changing main-window behavior', () => {
    const liteWindow = liteTauriConfig.app?.windows?.[0]
    const proWindow = proTauriConfig.app?.windows?.[0]

    expect(liteTauriConfig.productName).toBe('EntropIA Lite')
    expect(liteWindow?.title).toBe('EntropIA Lite')
    expect(proTauriConfig.productName).toBe('EntropIA Pro')
    expect(proWindow?.title).toBe('EntropIA Pro')
    expect(windowSettingsWithoutTitle(liteWindow)).toEqual(windowSettingsWithoutTitle(proWindow))
  })
})
```

- [ ] **Step 2: Run the focused test and verify the regression is reproduced**

From the repository root, run:

```bash
pnpm --filter @entropia-pro/desktop test -- src/lib/system-display-name.test.ts
```

Expected: FAIL because `liteTauriConfig.app?.windows?.[0]?.title` is currently `undefined`; the Lite overlay therefore inherits the base Pro title at merge time.

- [ ] **Step 3: Add the complete Lite main-window override**

Replace `apps/desktop/src-tauri/tauri.lite.conf.json` with:

```json
{
  "productName": "EntropIA Lite",
  "identifier": "com.entropia.lite",
  "version": "1.0.9",
  "app": {
    "windows": [
      {
        "title": "EntropIA Lite",
        "width": 1280,
        "height": 800,
        "minWidth": 900,
        "minHeight": 600,
        "decorations": false,
        "resizable": true,
        "center": true,
        "visible": false
      }
    ]
  }
}
```

Do not modify the Pro base config or the Vite frontend product-title logic.

- [ ] **Step 4: Run the focused test and Lite typecheck**

From the repository root, run:

```bash
pnpm --filter @entropia-pro/desktop test -- src/lib/system-display-name.test.ts
VITE_LOCAL_ML=0 pnpm --filter @entropia-pro/desktop typecheck
```

Expected: both commands exit zero. The test proves both edition names and every non-title main-window property; the typecheck reports no errors for the Lite frontend.

- [ ] **Step 5: Smoke-test the actual native Lite title**

From `apps/desktop`, start the lean Tauri runtime without bundling:

```powershell
$env:VITE_LOCAL_ML = '0'
pnpm exec tauri dev --config src-tauri/tauri.lite.conf.json --no-watch
```

After the window appears:

1. Confirm the native window is the Lite UI.
2. Hover its Windows taskbar icon.
3. Confirm the thumbnail caption is exactly `EntropIA Lite`.
4. Confirm no `EntropIA Pro` caption appears for that running Lite process.
5. Stop the development process and clear the session variable with `Remove-Item Env:VITE_LOCAL_ML`.

Expected: the native title and taskbar thumbnail both display `EntropIA Lite`; the app starts without a full bundle build or the `local-ml` feature.

- [ ] **Step 6: Commit the complete fix**

```bash
git add apps/desktop/src-tauri/tauri.lite.conf.json apps/desktop/src/lib/system-display-name.test.ts
git commit -m "fix(desktop): correct Lite system display name"
```

The rollback boundary is exactly these two files: reverting the commit restores the previous Lite overlay and removes its focused regression test without affecting Pro behavior.