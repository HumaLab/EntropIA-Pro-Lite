# macOS: Lite 1.3.0 install follow-ups

**Status:** pending. Reported 2026-11-05 by a user who installed Lite 1.3.0
from the GitHub `.dmg` on an Intel Mac with **macOS Ventura 13.2 and Safari
16.3**.

- [x] **PDFs don't open: «Unexpected token '{'».** Cause confirmed: on macOS
      the webview is the system Safari (WKWebView), and pdf.js 4.10.38 uses two
      APIs no Safari reaches before 16.4 and 17.4 respectively — class static
      blocks in `build/pdf.mjs` and both workers (Safari 16.4) and
      `Promise.withResolvers` with no polyfill in the modern entry (Safari
      17.4). The frontend target (`apps/desktop/vite.config.ts`,
      `target: 'chrome105'`) never described the macOS engine.
      - *Fix (branch `fix/macos-pdfjs-legacy`):* both runtime load sites import
        the legacy build (`pdfjs-dist/legacy/build/pdf.mjs` and
        `pdfjs-dist/legacy/build/pdf.worker.min.mjs`), which is transpiled and
        ships the polyfill; `apps/desktop/src/lib/pdfjs-legacy-build.test.ts`
        keeps the modern build out. The macOS installer now declares
        `bundle.macOS.minimumSystemVersion: "13.3"` (the Ventura release that
        ships Safari 16.4), stated in the README and the user manual. Older
        Macs are not supported.
      - *Workaround for that user:* update macOS to 13.3+.
- [ ] **«no collection…» when dragging a file in.** Dragged a screenshot into
      collection «Prueba» and the app said that collection didn't exist,
      although it was visible in the list. Right after, a PDF imported fine
      into the same collection, so it's intermittent. Message comes from
      `apps/desktop/src-tauri/src/corpus/ingest.rs:486`. Cause not
      investigated.
- [ ] **Permissions.** Before it worked, the app threw permission errors; they
      went away only after granting **Full Disk Access**. That shouldn't be
      needed: find which permission it really requires and document it in the
      manual.
- [ ] **Keychain password prompt** («EntropIA Lite quiere acceder a la llave
      com.entropia.desktop credentials»). Expected for an unsigned app: answer
      with the Mac's password and **Permitir siempre**. Explain it in the
      manual. Goes away for real once the installers are signed.
