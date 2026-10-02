/**
 * The capture script the backend runs inside the page (src-tauri/src/navegador/
 * capture.js), exercised against a DOM. It runs in the page's own world, so
 * the one thing that must always hold is that it returns a plain object and
 * never throws.
 */
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import script from '../../src-tauri/src/navegador/capture.js?raw'
import captureRs from '../../src-tauri/src/navegador/capture.rs?raw'

type Result = {
  ok: boolean
  error?: string
  kind?: string
  url?: string
  title?: string
  canonicalUrl?: string | null
  siteName?: string | null
  lang?: string | null
  text?: string
  quote?: string | null
  quotePrefix?: string | null
  quoteSuffix?: string | null
  html?: string | null
  truncated?: boolean
}

/** Run the script the way the backend does: the kind is substituted in. */
function run(kind: string): Result {
  const source = script.replace("'__KIND__'", JSON.stringify(kind))
  return (0, eval)(source) as Result
}

function load(html: string, url = 'https://example.com/article') {
  ;(window as unknown as { happyDOM: { setURL(url: string): void } }).happyDOM.setURL(url)
  document.documentElement.innerHTML = html
}

function select(node: Node, start: number, end: number) {
  const range = document.createRange()
  range.setStart(node, start)
  range.setEnd(node, end)
  const selection = window.getSelection()!
  selection.removeAllRanges()
  selection.addRange(range)
}

const LONG = 'Lorem ipsum dolor sit amet, consectetur adipiscing elit. '.repeat(8)

beforeEach(() => {
  document.title = ''
  document.documentElement.removeAttribute('lang')
  load('<head></head><body></body>')
})

afterEach(() => {
  window.getSelection()?.removeAllRanges()
})

describe('capture script: page', () => {
  it('reports the page the way the backend expects', () => {
    load(
      `<head><title>An article</title>
       <link rel="canonical" href="/canonical/path">
       <meta property="og:site_name" content="Example News"></head>
       <body><p>Hello world</p></body>`
    )
    document.documentElement.setAttribute('lang', 'es-AR')
    const result = run('page')
    expect(result.ok).toBe(true)
    expect(result.kind).toBe('page')
    expect(result.url).toBe('https://example.com/article')
    expect(result.title).toBe('An article')
    expect(result.canonicalUrl).toBe('https://example.com/canonical/path')
    expect(result.siteName).toBe('Example News')
    expect(result.lang).toBe('es-AR')
    expect(result.truncated).toBe(false)
    expect(result.text).toContain('Hello world')
    expect(result.html!.startsWith('<!DOCTYPE html>')).toBe(true)
    expect(result.html).toContain('<p>Hello world</p>')
  })

  it('leaves absent metadata as null', () => {
    load('<head></head><body><p>x</p></body>')
    const result = run('page')
    expect(result.canonicalUrl ?? null).toBeNull()
    expect(result.siteName ?? null).toBeNull()
    expect(result.lang ?? null).toBeNull()
  })

  it('reads the main content when it is substantial', () => {
    load(
      `<body><nav>MENU LINKS</nav><main><p>${LONG}</p></main><footer>FOOTER JUNK</footer></body>`
    )
    const { text } = run('page')
    expect(text).toContain('Lorem ipsum')
    expect(text).not.toContain('MENU LINKS')
    expect(text).not.toContain('FOOTER JUNK')
  })

  it('falls back to the whole body when the main content is tiny', () => {
    load('<body><nav>MENU LINKS</nav><main>short</main><p>Body text</p></body>')
    const { text } = run('page')
    expect(text).toContain('MENU LINKS')
    expect(text).toContain('Body text')
  })

  it('understands article and role=main as main content', () => {
    load(`<body><nav>MENU</nav><article>${LONG}</article></body>`)
    expect(run('page').text).not.toContain('MENU')
    load(`<body><nav>MENU</nav><div role="main">${LONG}</div></body>`)
    expect(run('page').text).not.toContain('MENU')
  })

  it('keeps the scripts in the snapshot: stripping happens when a copy is displayed', () => {
    load('<body><script>window.tracker = 1</script><p>x</p></body>')
    expect(run('page').html).toContain('window.tracker = 1')
  })

  it('caps the text and the html and says so', () => {
    load(`<body><p>${'a'.repeat(3 * 1024 * 1024)}</p></body>`)
    const result = run('page')
    expect(result.truncated).toBe(true)
    expect(new TextEncoder().encode(result.text!).length).toBeLessThanOrEqual(2 * 1024 * 1024)
    expect(new TextEncoder().encode(result.html!).length).toBeLessThanOrEqual(10 * 1024 * 1024)
  })

  it('caps by bytes, not characters, and never cuts a character in half', () => {
    load(`<body><p>${'é'.repeat(1_500_000)}</p></body>`)
    const { text, truncated } = run('page')
    const bytes = new TextEncoder().encode(text!).length
    expect(truncated).toBe(true)
    expect(bytes).toBeLessThanOrEqual(2 * 1024 * 1024)
    expect(text).not.toContain('�')
  })

  it('leaves no lone surrogate that would break the JSON on the other side', () => {
    load('<body><p>ok</p></body>')
    document.title = 'bad \uD800 title'
    const json = JSON.stringify(run('page'))
    expect(json).not.toMatch(/\\ud800/i)
    expect(run('page').title).toBe('bad � title')
  })

  it('refuses the browser built-in PDF viewer, which has no text to read', () => {
    Object.defineProperty(document, 'contentType', { value: 'application/pdf', configurable: true })
    try {
      expect(run('page')).toEqual({ ok: false, error: 'pdf_document' })
    } finally {
      delete (document as unknown as { contentType?: string }).contentType
    }
  })
})

describe('capture script: selection', () => {
  it('returns the exact quote with the text around it', () => {
    load('<body><p id="p">Alpha beta gamma delta</p></body>')
    const text = document.getElementById('p')!.firstChild!
    select(text, 6, 10) // "beta"
    const result = run('selection')
    expect(result.ok).toBe(true)
    expect(result.kind).toBe('selection')
    expect(result.url).toBe('https://example.com/article')
    expect(result.quote).toBe('beta')
    expect(result.quotePrefix).toBe('Alpha ')
    expect(result.quoteSuffix).toBe(' gamma delta')
  })

  it('takes about 200 characters of context, the nearest ones', () => {
    const before = 'b'.repeat(500)
    const after = 'a'.repeat(500)
    load(`<body><p id="p">${before}TARGET${after}</p></body>`)
    select(document.getElementById('p')!.firstChild!, 500, 506)
    const result = run('selection')
    expect(result.quote).toBe('TARGET')
    expect(result.quotePrefix).toBe('b'.repeat(200))
    expect(result.quoteSuffix).toBe('a'.repeat(200))
  })

  it('takes context across elements and collapses whitespace', () => {
    load('<body><p>First\n   paragraph.</p><p id="p">Second paragraph.</p><p>Third.</p></body>')
    select(document.getElementById('p')!.firstChild!, 0, 6)
    const result = run('selection')
    expect(result.quote).toBe('Second')
    expect(result.quotePrefix).toBe('First paragraph.')
    expect(result.quoteSuffix).toBe(' paragraph.Third.')
  })

  it('answers no_selection when nothing is selected', () => {
    load('<body><p>text</p></body>')
    expect(run('selection')).toEqual({ ok: false, error: 'no_selection' })
  })

  it('answers no_selection for a collapsed caret or blank text', () => {
    load('<body><p id="p">a   b</p></body>')
    const text = document.getElementById('p')!.firstChild!
    select(text, 2, 2)
    expect(run('selection')).toEqual({ ok: false, error: 'no_selection' })
    select(text, 1, 4) // three spaces
    expect(run('selection')).toEqual({ ok: false, error: 'no_selection' })
  })

  it('caps a huge selection by bytes', () => {
    load(`<body><p id="p">${'x'.repeat(3 * 1024 * 1024)}</p></body>`)
    const node = document.getElementById('p')!.firstChild!
    select(node, 0, 3 * 1024 * 1024)
    const result = run('selection')
    expect(result.truncated).toBe(true)
    expect(new TextEncoder().encode(result.quote!).length).toBeLessThanOrEqual(2 * 1024 * 1024)
  })

  it('does not send the html of the whole page for a selection', () => {
    load('<body><p id="p">Alpha beta</p></body>')
    select(document.getElementById('p')!.firstChild!, 0, 5)
    expect(run('selection').html ?? null).toBeNull()
  })
})

describe('capture script: robustness', () => {
  it('never throws and always returns a JSON-serialisable object', () => {
    load('<body><p>x</p></body>')
    for (const kind of ['page', 'selection', 'other', '']) {
      const result = run(kind)
      expect(typeof result).toBe('object')
      expect(JSON.parse(JSON.stringify(result))).toEqual(result)
    }
  })

  it('turns an unknown kind into an error result', () => {
    const result = run('other')
    expect(result.ok).toBe(false)
    expect(result.error).toMatch(/^script_error/)
  })

  it('turns an exception inside the page into an error result', () => {
    Object.defineProperty(document, 'title', {
      configurable: true,
      get() {
        throw new Error('page is hostile')
      },
    })
    try {
      const result = run('page')
      expect(result.ok).toBe(false)
      expect(result.error).toContain('page is hostile')
    } finally {
      delete (document as unknown as { title?: string }).title
    }
  })

  it('never puts a thrown value that is not a string into the result', () => {
    Object.defineProperty(document, 'title', {
      configurable: true,
      get() {
        throw {
          toString: () => {
            throw new Error('nested')
          },
        } // eslint-disable-line no-throw-literal
      },
    })
    try {
      const result = run('page')
      expect(result.ok).toBe(false)
      expect(typeof result.error).toBe('string')
    } finally {
      delete (document as unknown as { title?: string }).title
    }
  })
})

describe('capture script and its Rust validator agree', () => {
  const limit = (source: string, name: string) =>
    new RegExp(`(?:const|var)?\\s*${name}\\s*(?::\\s*usize)?\\s*=\\s*([0-9 *_]+?)\\s*[;,\\n]`).exec(
      source
    )?.[1]

  it('marks the kind placeholder exactly once, which the backend substitutes', () => {
    expect(script.split("'__KIND__'").length - 1).toBe(1)
  })

  it.each([
    ['TEXT_MAX', 'TEXT_MAX_BYTES'],
    ['HTML_MAX', 'HTML_MAX_BYTES'],
    ['CONTEXT', 'CONTEXT_MAX_CHARS'],
  ])('uses the same %s as the Rust side', (jsName, rustName) => {
    const js = limit(script, jsName)
    const rust = limit(captureRs, rustName)
    expect(js, `${jsName} not found in capture.js`).toBeDefined()
    expect(rust, `${rustName} not found in capture.rs`).toBeDefined()
    // CONTEXT in the script is the context it takes; Rust keeps up to twice that.
    if (jsName === 'CONTEXT') {
      expect(Number(rust!.replace(/_/g, '')) / 2).toBe(Number(js!.replace(/_/g, '')))
    } else {
      expect(js!.replace(/[_ ]/g, '')).toBe(rust!.replace(/[_ ]/g, ''))
    }
  })
})
