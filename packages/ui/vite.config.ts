import { defineConfig } from 'vite'
import { svelte } from '@sveltejs/vite-plugin-svelte'
import { resolve } from 'node:path'

export default defineConfig({
  plugins: [svelte()],
  optimizeDeps: {
    include: ['pdfjs-dist'],
  },
  build: {
    lib: {
      entry: resolve(__dirname, 'src/index.ts'),
      formats: ['es'],
      fileName: 'index',
    },
    outDir: 'dist',
    // This library bundle is a build check only: the app consumes src through
    // the package "exports", so its size never reaches a user. The limit sits
    // just above today's bundle so further bloat still warns.
    chunkSizeWarningLimit: 2600,
    rollupOptions: {
      external: ['svelte', 'svelte/internal'],
    },
  },
})
