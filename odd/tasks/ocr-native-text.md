# OCR over native text, and native text at import

**Status:** in progress (2026-11-12), branch `fix/ocr-include-native`, PR #282. Refs issue #250.
Forcing OCR over a PDF's text layer (batch option and the item's OCRH button), and storing that text layer when the PDF is imported.

## Specs

- S1. Issue #250, verbatim: «Sería muy útil que se pueda ignorar el OCR nativo de un pdf y forzar GLM-OCR (también en lotes).»
- S2. A batch with «Incluir texto nativo» runs the OCR engine (GLM-OCR) on pages whose text came from the PDF's text layer. Without the option those pages keep their native text and are not sent to GLM-OCR. Pages already read by an engine and pages edited by hand keep their protections. (Owner, verified in the app: «el procesameitno por Lote fiunciona eprfecto, cuando no está cliquedo "incluir texto nativo" las imaegnes con texto nativo no son encviadas a GLM-OCR y se extrae su texto nativo».)
- S3. The item's OCRH button always runs the engine, never the text layer. Owner, verbatim: «prefiero que en el #282 OCRH fuerce GLM-OCR siempre».
- S4. When a PDF is imported, every page whose text layer is usable gets that text stored right away as native text, so it shows as native text from the moment of import. Owner, verbatim: «los pdf con texto nativo deben mostrar desd el momento de su importacion el texto como texto nativo» and «que la importación con texto nativo se haga ya, en el mismo PR». Usable = the same `is_quality_text` rule the OCR executor applies. Pages without a usable layer get no text, as today. A failure reading the layer never fails the import.

## Tasks

- [x] T1 (S1, S2, S3) Executor and Pro local pipeline read past the text layer when an active request is manual or has `ocr_include_native` — inline — `ca57006d`.
- [x] T2 (CI) Pro clippy: `process_pdf` has 8 arguments — inline — `155dc629`.
- [x] T3 (S4) Store native text at PDF import — worker — `cb1edee0`.
- [ ] Owner check: import a native-text PDF in the dev profile `ocrnativo`: its pages show native text at once; OCRH on one of them returns GLM-OCR text; the batch keeps S2.

## Log

- L1 (2026-11-12, owner, verbatim): «lo probé en la coleccion 'uruguay2' y efectivamente no fuerza el ocr de GLM-OCR, se queda con el texto nativo. La resolución de esto es prioritaria»
- L2 (2026-11-12, owner, verbatim): «probé manualmente con el botón OCRH y el procesameitno por Lote, el procesameitno por Lote fiunciona eprfecto, cuando no está cliquedo "incluir texto nativo" las imaegnes con texto nativo no son encviadas a GLM-OCR y se extrae su texto nativo. Sin emabrgo el boton manuel del item pro asset cuando el texto no está extraido y el pedf tiene texto nativo, por defecto procesa con glm.-ocr, lo que me hizo pensar esto es que los pdf con texto nativo deben mostrar desd el momento de su importacion el texto como texto nativo ¿me explico?»
- L3 (2026-11-12, owner, verbatim, choosing over "OCRH forces only when native text is already stored; import as a separate task"): «prefiero que en el #282 OCRH fuerce GLM-OCR siempre, y que la importación con texto nativo se haga ya, en el mismo PR»
- L4 (2026-11-12): CI on `ca57006d` failed Rust Pro (Linux) and Windows Rust Feature Contract: clippy `too_many_arguments` (8/7) on `ocr::process_pdf` (`paddle-ocr` only). Lite checks passed.
- L5 (2026-11-12): T3 verified by the parent: `cargo fmt --check` clean, `cargo test --lib corpus::` 51 passed (4 new import tests), clippy lean clean; the worker also ran processing:: 219 and ocr:: 240 passed. Pro (`paddle-ocr`) compile left to CI.
