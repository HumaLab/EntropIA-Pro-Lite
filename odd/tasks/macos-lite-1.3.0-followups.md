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
- [x] **«no collection…» when dragging a file in.** Dragged a screenshot into
      collection «Prueba» and the app said that collection didn't exist,
      although it was visible in the list. Right after, a PDF imported fine
      into the same collection, so it's intermittent. Message comes from
      `apps/desktop/src-tauri/src/corpus/ingest.rs:486`. Cause not
      investigated.
      - *Cause:* deleting documents also deleted the collection they left empty
        (`corpus/delete.rs`, `delete_items_rows`), and the renderer never learned
        it: the sidebar and the open collection kept the deleted row, so the next
        drag asked the backend to import into a collection that no longer
        existed. Not macOS-specific: the delete path is the same on every
        platform.
      - *Fix:* deleting documents never deletes their collection — the owner
        deletes collections explicitly — so an emptied collection keeps its row,
        its folder and produces no sync tombstone. If an import still finds the
        collection gone (deleted on this or a synced device in 1.3.0), the view
        says so in plain words and refreshes the sidebar instead of showing the
        raw backend error. Branch `fix/keep-emptied-collections`.
- [ ] **Permissions.** Before it worked, the app threw permission errors; they
      went away only after granting **Full Disk Access**. That shouldn't be
      needed: find which permission it really requires and document it in the
      manual.
- [ ] **Keychain password prompt** («EntropIA Lite quiere acceder a la llave
      com.entropia.desktop credentials»). Expected for an unsigned app: answer
      with the Mac's password and **Permitir siempre**. Explain it in the
      manual. Goes away for real once the installers are signed.
