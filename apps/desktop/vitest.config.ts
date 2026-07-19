import { defineConfig } from 'vitest/config'
import { svelte } from '@sveltejs/vite-plugin-svelte'
import { resolve } from 'path'
import liteTauriConfig from './src-tauri/tauri.lite.conf.json' with { type: 'json' }
import productionTauriConfig from './src-tauri/tauri.conf.json' with { type: 'json' }

// Mirror the build-time VITE_LOCAL_ML define from vite.config.ts so the
// capability/brand/default flags resolve identically under test. Defaults to the
// full (Pro) variant ('1') — the same default the production build uses — so the
// default test run exercises the full UI surface. Set VITE_LOCAL_ML=0 to run the
// lean variant.
const localMl = process.env.VITE_LOCAL_ML ?? '1'
const productName =
  localMl === '1' ? productionTauriConfig.productName : liteTauriConfig.productName

export default defineConfig({
  plugins: [svelte({ hot: !process.env.VITEST })],
  define: {
    'import.meta.env.VITE_LOCAL_ML': JSON.stringify(localMl),
    'import.meta.env.VITE_PRODUCT_NAME': JSON.stringify(productName),
  },
  resolve: {
    alias: { $lib: resolve(__dirname, './src/lib') },
    conditions: ['browser'],
  },
  test: {
    name: 'desktop',
    globals: true,
    environment: 'happy-dom',
    include: ['src/**/*.test.ts'],
    setupFiles: ['./src/test-setup.ts'],
    coverage: {
      provider: 'v8',
      include: ['src/**/*.{ts,svelte}'],
      exclude: ['src/**/*.test.ts', 'src/test-setup.ts'],
    },
  },
})
