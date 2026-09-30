/**
 * Build-time capability flags for the unified Lite/Pro build.
 *
 * `LOCAL_ML` is `true` in the full (Pro) build and `false` in the API-only
 * (Lite) variant. It is driven by `VITE_LOCAL_ML`, which CI sets from the same
 * matrix dimension that selects the Cargo `local-ml` feature — the Rust backend
 * and the frontend must never disagree about which variant this is.
 *
 * Use it to gate Svelte templates (`{#if LOCAL_ML}`). NOTE: it does NOT remove
 * static top-level imports — Pro-only modules must be loaded via dynamic
 * `import()` behind this flag, and shared modules consumed by both variants must
 * export no-op stubs under the lite variant. See the strangler plan (P6).
 */
export const LOCAL_ML: boolean = import.meta.env.VITE_LOCAL_ML === '1'

/**
 * The experimental Navegador section. Off unless `VITE_NAVEGADOR=1`, and it
 * only does anything in a build that also has the Cargo feature `navegador`
 * (`pnpm exec tauri dev --features navegador`): the backend commands answer
 * "not available" otherwise. Unlike `LOCAL_ML` this is not a build variant, so
 * nothing about it is inferred from the other flag.
 */
export const NAVEGADOR: boolean = import.meta.env.VITE_NAVEGADOR === '1'
