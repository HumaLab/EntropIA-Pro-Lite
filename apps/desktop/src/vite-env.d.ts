/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** Application version injected from the desktop package manifest at build time. */
  readonly VITE_APP_VERSION: string

  /**
   * Build-variant switch: '1' in the full (Pro) build, '0' in the API-only
   * (Lite) variant. Set by CI from the same matrix dimension that selects the
   * Cargo `local-ml` feature. Read it through `$lib/capabilities`, not directly.
   */
  readonly VITE_LOCAL_ML: string
}

interface ImportMeta {
  readonly env: ImportMetaEnv
}
