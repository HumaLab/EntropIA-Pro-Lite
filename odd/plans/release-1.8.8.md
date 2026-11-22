# Release 1.8.8

## Part 1 — Release notes (draft for the GitHub release body)

## Español

Esta versión corrige un problema de **EntropIA Pro 1.8.7** y actualiza una biblioteca con un aviso de seguridad. El problema: Parakeet, el motor de transcripción local, pasaba al inglés partes de un audio en español. En 1.8.7 la app lo detectaba y volvía a transcribir todo el audio con Whisper, que es más lento, comete más errores en las entrevistas y pierde los hablantes.

**No agrega migraciones.** Un equipo con 1.8.7 sigue sincronizando con uno que tenga 1.8.8.

### Pro

- **Parakeet y el audio en español.** La app revisa cada tramo de 30 segundos. Si un tramo pasó al inglés, lo vuelve a transcribir en trozos cortos, cortados en las pausas, que quedan en español. Los demás tramos no cambian, así que las entrevistas se transcriben igual que antes y los hablantes se conservan. En las pruebas con narración de radio, el error por palabra bajó del 54 % al 5 %. Si a pesar de eso el resultado sigue en inglés, la app todavía recurre a Whisper, como en 1.8.7.

### Pro y Lite

- **Seguridad.** Actualiza la biblioteca `url` a 2.5.9, que corrige un desborde de pila con ciertas direcciones `blob:` anidadas (RUSTSEC-2026-0592).

## English

This release fixes a problem in **EntropIA Pro 1.8.7** and updates a library with a security advisory. The problem: Parakeet, the local transcription engine, turned parts of Spanish audio into English. In 1.8.7 the app detected it and transcribed the whole audio again with Whisper, which is slower, makes more mistakes on interviews, and loses the speakers.

**It adds no migration.** A device on 1.8.7 still syncs with one on 1.8.8.

### Pro

- **Parakeet and Spanish audio.** The app checks each 30-second stretch. When a stretch turned into English, it transcribes it again in short pieces cut at pauses, which stay Spanish. The other stretches do not change, so interviews are transcribed as before and the speakers are kept. In tests on radio narration, the word error rate went from 54% to 5%. If the result is still English despite that, the app still falls back to Whisper, as in 1.8.7.

### Pro and Lite

- **Security.** Updates the `url` library to 2.5.9, which fixes a stack overflow on certain nested `blob:` addresses (RUSTSEC-2026-0592).

---

## Part 2 — Release checklist

- [x] Version bump — `pnpm version:bump 1.8.8`; `pnpm version:check` passes.
- [x] Runtime: 1.8.8 changes nothing in the Python runtime (`git diff v1.8.7 main` touches only `transcription/parakeet.rs` and a plan doc), so `2026.11.10` is reused. Bootstrap published for **app_version 1.8.8** on both platforms from Build Runtime Pack runs 44877086448 (Windows) and 44877088996 (Linux).
- [x] `cargo audit` on main failed on RUSTSEC-2026-0592 (url 2.5.8, published 2026-11-20, after the Parakeet PR ran): `url` 2.5.9 and `form_urlencoded` 1.2.3 in `Cargo.lock`, entropia-agent git pin kept.
- [ ] PR to main, CI green, merge.
- [ ] Push tag `v1.8.8` on main (Pro installers + Lite MSIX + draft release).
- [ ] Release draft: title «EntropIA Pro & Lite v1.8.8», paste Part 1 as the body, publish.
- [ ] Check in the real app (Pro build): a radio narration (MICRO 1/2) stays Spanish with speakers, and a SOIP interview transcribes as in 1.8.7. Not checked before tagging, by the maintainer's decision.
- [ ] Build the Pro Store MSIX 1.8.8.0 (`build-pro-store-msix.ps1`) and upload Pro and Lite MSIX 1.8.8.0 to Partner Center; the Store number does not move with the app version on its own.
