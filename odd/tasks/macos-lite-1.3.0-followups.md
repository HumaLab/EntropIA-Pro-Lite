# macOS: Lite 1.3.0 install follow-ups

**Status:** pending. Reported 2026-11-05 by a user who installed Lite 1.3.0
from the GitHub `.dmg` on an Intel Mac with **macOS Ventura 13.2 and Safari
16.3**.

- [ ] **PDFs don't open: «Unexpected token '{'».** Almost certain cause: on
      macOS the webview is the system Safari (WKWebView), and pdf.js 4.10 uses
      syntax that Safari only supports from 16.4 (March 2023). The frontend is
      built for Windows' engine only (`apps/desktop/vite.config.ts:116`,
      `target: 'chrome105'`), and the pdf.js worker is loaded as shipped
      (`apps/desktop/src/lib/ocr-rich-text.ts:468`).
      - *Workaround for that user:* update macOS to 13.3+ (ships Safari 16.4).
      - *Real fix, to decide:* transpile the macOS build for older Safari, or
        declare a minimum macOS version and warn at startup.
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
