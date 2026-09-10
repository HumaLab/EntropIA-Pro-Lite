# Encrypted PDF Import Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Import the encrypted PDFs that archival digitisation produces, and — where the file genuinely cannot be opened — say so in words that name the cause and the way out.

**Architecture:** `lopdf` already exposes `is_encrypted()` and `decrypt(password)`. Most archival scans carry only an *owner* password: they open with an empty *user* password, and the restriction is on printing or copying, not on reading. So the first move is not a better message — it is to actually open them. The message is the fallback for the minority that carry a real user password.

**Tech Stack:** Rust, `lopdf` 0.34, Tauri v2.

**Reported by:** importing 18 PDFs from `DIPBA / legajos mar del plata`, every one failing with `Cannot split a PDF without pages`. The files are valid (`%PDF-1.6`, linearised, `/Type/Page` present, 5 pages declared) and carry `/Encrypt`. `lopdf` parses the structure, cannot decrypt the object streams, reports zero pages, and the guard fires with a message about the wrong thing.

## Global Constraints

- This is a pre-existing limitation, unrelated to the shared-data-directory work. Keep the two apart in commits.
- Rust artifacts — identifiers, comments, doc comments, test names — in English, matching the surrounding code.
- Never prompt for a password. Reading a file the user chose to import is not the place for a credential dialog, and an owner password is not a secret the user is expected to hold.
- No new dependency. `lopdf` 0.34 already carries what this needs.
- Run Rust tests from `apps/desktop/src-tauri`.

---

### Task 1: Open what can be opened

`load_lopdf_document` (`ocr/pdf.rs:362`) is the single door: page counting (`:358`), splitting (`:430`) and rotating (`:558`) all pass through it. It already repairs one class of malformed file — an invalid `Prev` pointer — so the shape for "try harder before giving up" exists and should be followed rather than invented.

An empty user password is the overwhelmingly common case in digitised archives: the producer sets an owner password to restrict printing, and every reader opens the file without asking anything.

**Files:**
- Modify: `apps/desktop/src-tauri/src/ocr/pdf.rs:362-380` (`load_lopdf_document`)
- Test: `apps/desktop/src-tauri/src/ocr/pdf.rs` — existing `mod tests`

**Interfaces:**
- Consumes: `lopdf::Document::is_encrypted()`, `lopdf::Document::decrypt(password)`
- Produces: `load_lopdf_document` returns a decrypted document where an empty user password suffices; otherwise a typed encryption error (Task 2)

- [ ] **Step 1: Write the failing test**

A fixture is needed. Build it from a file the repository already has rather than committing a new binary: take an existing test PDF, encrypt it with an owner password and an empty user password, and keep the generator beside the test so the fixture is reproducible. If no such tooling is available in-tree, add the smallest possible hand-written encrypted PDF as a fixture under `tests/fixtures/` and document where it came from.

```rust
    #[test]
    fn load_lopdf_document_opens_a_pdf_with_an_owner_password_only() {
        // Archival scans routinely carry an owner password restricting printing
        // while opening freely with an empty user password. Refusing those is
        // refusing most digitised archives.
        let bytes = owner_password_only_pdf_bytes();

        let document = load_lopdf_document(&bytes, "splitting").expect("opens");

        assert!(
            !document.get_pages().is_empty(),
            "an owner-password PDF must yield its pages"
        );
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib load_lopdf_document_opens`
Expected: FAIL — the document loads but reports zero pages, which is exactly the reported defect.

- [ ] **Step 3: Implement**

In `load_lopdf_document`, after a successful `load_mem`, attempt the empty user password when the document reports itself encrypted:

```rust
/// Opens a PDF, decrypting it when an empty user password suffices.
///
/// Digitised archives commonly carry an OWNER password — a restriction on
/// printing or copying — while opening freely for reading. `lopdf` parses such
/// a file successfully but cannot read its object streams, so `get_pages()`
/// comes back empty and every downstream operation reports a document without
/// pages. Trying the empty password here turns that into an ordinary import.
///
/// A file that needs a real USER password is a different matter: it is returned
/// as [`PdfLoadError::Encrypted`] so the caller can say so plainly.
fn try_decrypt_in_place(document: &mut lopdf::Document) -> Result<(), PdfLoadError> {
    if !document.is_encrypted() {
        return Ok(());
    }
    document
        .decrypt("")
        .map_err(|_| PdfLoadError::Encrypted)
}
```

Call it on every successful load path, including the repaired-`Prev` retry.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib pdf::`
Expected: PASS, with every existing PDF test untouched.

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/src-tauri/src/ocr/pdf.rs
git commit -m "fix(pdf): abrir los PDF con contraseña de propietario

Un escaneo de archivo suele traer contraseña de propietario —una restricción
para imprimir o copiar— y abrirse sin problema para leer. lopdf parseaba el
archivo pero no podía leer sus flujos de objetos, get_pages() volvía vacío, y
cada operación reportaba un documento sin páginas.

Se intenta la contraseña vacía al cargar. Los 18 legajos de la DIPBA que
fallaban al importar entran por este camino."
```

---

### Task 2: Name the cause when the file really is locked

A file with a genuine user password cannot be opened, and that is legitimate. What is not legitimate is telling the user its pages are missing.

**Files:**
- Modify: `apps/desktop/src-tauri/src/ocr/pdf.rs` — introduce `PdfLoadError`, thread it through `load_lopdf_document` and its three callers (`:358`, `:430`, `:558`)
- Test: `apps/desktop/src-tauri/src/ocr/pdf.rs`

**Interfaces:**
- Produces: `PdfLoadError { Encrypted, Malformed(String) }`, with `Display` carrying the user-facing sentence

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn a_user_password_pdf_reports_encryption_not_missing_pages() {
        // The reported defect: "Cannot split a PDF without pages" sent the user
        // looking at the wrong thing. The file was neither empty nor broken.
        let bytes = user_password_pdf_bytes();

        let error = split_pdf_to_single_page_bytes(&bytes).expect_err("cannot split");

        assert!(
            error.contains("protegido") || error.contains("contraseña"),
            "the message must name encryption, not absent pages: {error}"
        );
        assert!(
            !error.contains("without pages"),
            "the misleading message must be gone: {error}"
        );
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib a_user_password_pdf_reports`
Expected: FAIL — the current message still talks about missing pages.

- [ ] **Step 3: Implement**

The message reaches the import summary, so it is user-facing copy and follows the app's language:

> `El PDF está protegido con contraseña y no se puede leer. Quitale la protección y volvé a importarlo.`

Keep `Cannot split a PDF without pages` for a document that genuinely has none — that guard is still correct for its own case.

- [ ] **Step 4: Run the tests**

Run: `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib pdf::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/src-tauri/src/ocr/pdf.rs
git commit -m "fix(pdf): decir que el PDF está protegido, no que no tiene páginas

«Cannot split a PDF without pages» mandaba a mirar el lugar equivocado: el
archivo no estaba vacío ni roto, estaba cifrado. Con 18 archivos fallando a la
vez, ese desconcierto escala mal.

El guard de documento sin páginas se conserva para el caso que realmente
describe."
```

---

### Task 3: Report the failure per file, not as a wall

**Files:**
- Modify: `apps/desktop/src/views/CollectionView.svelte` — the import summary
- Test: `apps/desktop/src/views/CollectionView.test.ts`

The reported run produced eighteen lines, each repeating the full path and the same sentence, with `Importados 0 · Omitidos 0 · Errores 18`. The information a person needs — *these files share one cause, and here is what to do* — is buried in the repetition.

- [ ] **Step 1: Write the failing test**

Assert that several failures sharing one cause are grouped under a single explanation with the affected file names listed beneath, rather than one full sentence per file.

- [ ] **Step 2: Run it, implement the grouping, run it again**

Run: `npm test -- CollectionView` from `apps/desktop`

- [ ] **Step 3: Commit**

```bash
git add apps/desktop/src/views/CollectionView.svelte apps/desktop/src/views/CollectionView.test.ts
git commit -m "fix(import): agrupar los errores de importación por causa

Dieciocho líneas repitiendo la misma frase y la ruta completa esconden justo lo
que hace falta saber: que comparten una causa y qué hacer al respecto."
```

---

## Definition of done

- [ ] `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` passes.
- [ ] `npm test` passes from `apps/desktop`.
- [ ] `cargo clippy --all-targets` reports no new findings.
- [ ] The eighteen DIPBA files import, verified against the real directory: `D:\ARCHIVOS\Archivo disco archivos\Agustín\FOTOSs\DIPBA\legajos mar del plata`.
- [ ] A PDF with a genuine user password reports encryption, and its pages are not silently lost.

## What this plan does not do

- **Prompt for a password.** Importing a file is not the place for a credential dialog, and an owner password is not something the user is expected to hold.
- **Strip protection from the stored copy.** The asset keeps the bytes the user gave it. Decryption happens in memory, for reading.
- **Handle every encryption scheme.** `lopdf` covers the common RC4 and AES cases. A file it cannot open lands in Task 2's message, which is the honest outcome.

## Verification that requires the user

The decisive check runs against the real archive, not a fixture: import that DIPBA directory and confirm the eighteen files land with their pages. Those files are not in the repository and cannot be committed — they are archival material — so this step is handed over rather than automated.
