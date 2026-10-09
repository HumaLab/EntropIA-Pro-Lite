import { describe, expect, it } from 'vitest'
import { bodyOf } from './writing-publish'

describe('bodyOf', () => {
  it('keeps only what the page puts inside <body>', () => {
    const page =
      '<!doctype html>\n<html><head><title>x</title></head>\n<body>\n<p>Hola</p>\n<section class="footnotes"></section>\n</body>\n</html>\n'
    expect(bodyOf(page)).toBe('<p>Hola</p>\n<section class="footnotes"></section>')
  })
})
