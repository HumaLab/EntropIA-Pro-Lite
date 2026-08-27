import { describe, expect, it } from 'vitest'

import viteConfig from '../vite.config'

describe('desktop Vite config', () => {
  it('does not prebundle the removed html2pdf rasterizer', () => {
    expect(viteConfig.optimizeDeps?.include).not.toContain('html2pdf.js')
  })
})
