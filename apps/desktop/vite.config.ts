import { defineConfig } from 'vite'
import { svelte } from '@sveltejs/vite-plugin-svelte'
import { resolve } from 'path'
import desktopPackage from './package.json' with { type: 'json' }
import developmentTauriConfig from './src-tauri/tauri.dev.conf.json' with { type: 'json' }
import liteTauriConfig from './src-tauri/tauri.lite.conf.json' with { type: 'json' }
import productionTauriConfig from './src-tauri/tauri.conf.json' with { type: 'json' }

// Tauri 2 injects TAURI_ENV_DEBUG as the string 'true'/'false'; an explicit
// comparison is required because 'false' is truthy.
const isTauriDebug = process.env.TAURI_ENV_DEBUG === 'true'

// Build-variant switch for the strangler unification (Lite/Pro from one tree).
// '1' = full build (local OCR/LLM/embeddings UI); '0' = API-only "lite" variant.
// CI must drive this from the SAME matrix dimension that selects the Cargo
// `local-ml` feature so the Rust backend and the frontend never disagree about
// which variant is being built. Defaults to '1' (Pro) for local dev.
const localMl = process.env.VITE_LOCAL_ML ?? '1'
const developmentProductName = developmentTauriConfig.app.windows[0]?.title
if (!developmentProductName) {
  throw new Error('tauri.dev.conf.json is missing the main window title')
}
const productName = isTauriDebug
  ? developmentProductName
  : localMl === '1'
    ? productionTauriConfig.productName
    : liteTauriConfig.productName

export default defineConfig({
  plugins: [
    svelte(),
    {
      name: 'entropia-product-title',
      transformIndexHtml(html) {
        return html.replace('%ENTROPIA_PRODUCT_TITLE%', productName)
      },
    },
  ],
  define: {
    'import.meta.env.VITE_LOCAL_ML': JSON.stringify(localMl),
    'import.meta.env.VITE_APP_VERSION': JSON.stringify(desktopPackage.version),
    'import.meta.env.VITE_PRODUCT_NAME': JSON.stringify(productName),
  },
  optimizeDeps: {
    // Restrict dep-scan to the real frontend entry.
    // Without this, Vite may crawl every HTML file under apps/desktop,
    // including Rustdoc output under src-tauri/target/doc, which on Windows
    // can trigger EMFILE loops during dependency re-optimization.
    entries: ['index.html'],

    // @entropia/ui and @entropia/store are linked workspace packages that export
    // source files. Letting Vite discover their transitive bare imports during the
    // first browser crawl can rewrite the optimized dependency graph mid-startup.
    // Tauri's WebView is particularly sensitive to that cache churn and can end up
    // requesting stale chunk-*.js files from a previous optimization pass.
    //
    // Pin the full runtime dep set up front so the prebundle result is deterministic
    // across Linux and Windows cold starts.
    include: [
      '@tauri-apps/api/core',
      '@tauri-apps/api/event',
      '@tauri-apps/api/path',
      '@tauri-apps/api/webview',
      '@tauri-apps/plugin-dialog',
      '@tauri-apps/plugin-fs',
      '@tiptap/core',
      '@tiptap/extension-document',
      '@tiptap/extension-gapcursor',
      '@tiptap/extension-link',
      '@tiptap/extension-placeholder',
      '@tiptap/extension-table',
      '@tiptap/extension-table-cell',
      '@tiptap/extension-table-header',
      '@tiptap/extension-table-row',
      '@tiptap/extension-underline',
      '@tiptap/pm/model',
      '@tiptap/pm/state',
      '@tiptap/pm/tables',
      '@tiptap/pm/view',
      '@tiptap/starter-kit',
      // Pulls its own ProseMirror unless it is bundled with the rest: two
      // copies mean its nodes register against a different schema, and the
      // footnotes block silently never materialises.
      'tiptap-footnotes',
      'drizzle-orm',
      'drizzle-orm/sqlite-core',
      'drizzle-orm/sqlite-proxy',
      'leaflet',
      'pdfjs-dist',
      'svelte',
      'svelte/store',
    ],
    noDiscovery: true,
    holdUntilCrawlEnd: true,
  },
  resolve: {
    alias: {
      $lib: resolve(__dirname, './src/lib'),
    },
    // ProseMirror keys its plugins by identity, so two copies of these packages
    // make the editor refuse to build with "Adding different instances of a
    // keyed plugin". That happens easily here: @entropia/ui exports source, so
    // its imports of @tiptap/pm/* resolve separately from the prebundled copies
    // the Tiptap extensions pull in. One copy each, always.
    dedupe: [
      'prosemirror-commands',
      'prosemirror-history',
      'prosemirror-keymap',
      'prosemirror-model',
      'prosemirror-schema-list',
      'prosemirror-state',
      'prosemirror-tables',
      'prosemirror-transform',
      'prosemirror-view',
      '@tiptap/core',
      '@tiptap/pm',
    ],
  },
  // Tauri expects a fixed port in dev
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      // Watch packages/ui for cross-package HMR
      ignored: ['**/src-tauri/**'],
    },
  },
  build: {
    target: 'chrome105',
    minify: isTauriDebug ? false : 'esbuild',
    sourcemap: isTauriDebug,
    // Vite's 500 kB warning assumes a network download. Tauri loads from disk,
    // and the heaviest chunks (pdfmake and its fonts, html-docx) are fetched
    // only when the user exports. The limit sits just above today's largest
    // legitimate chunk, so real growth of the startup chunk still warns.
    chunkSizeWarningLimit: 1024,
    rollupOptions: {
      // splash.html is a second, dependency-free entry: the Rust `setup()` hook
      // opens it as a transparent window before the main webview boots, so it
      // must exist in frontendDist alongside index.html. It pulls its only asset
      // from public/ (splash-mark.png), which keeps it out of the module graph
      // and preserves the single-entry assumption in optimizeDeps.entries above.
      input: {
        main: resolve(__dirname, 'index.html'),
        splash: resolve(__dirname, 'splash.html'),
      },
    },
  },
})
