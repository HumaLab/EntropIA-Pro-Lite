/*
 * Capture script for the Navegador. The backend substitutes the kind in place of
 * the kind placeholder below and evaluates the result in the page through
 * the platform's script evaluation (Webview::eval_with_callback), which needs no
 * IPC. It runs in the page's own world, so:
 *  - it wraps everything in try/catch and always returns a plain,
 *    JSON-serialisable object, never throws;
 *  - what it returns is untrusted on the Rust side, which validates it again
 *    (see capture.rs); the limits below must match that file, a test compares them.
 *
 * kind "page":      readable text plus the HTML snapshot.
 * kind "selection": the exact quote plus text around it (W3C TextQuoteSelector
 *                   style: prefix and suffix, whitespace collapsed).
 */
;(function () {
  'use strict'

  var TEXT_MAX = 2 * 1024 * 1024
  var HTML_MAX = 10 * 1024 * 1024
  var CONTEXT = 200
  /** A main region shorter than this is a stub, not the page's content. */
  var MAIN_MIN_CHARS = 200
  var kind = '__KIND__'

  /** Replace unpaired surrogates: they cannot be encoded as UTF-8 or JSON. */
  function wellFormed(value) {
    var s = String(value)
    if (typeof s.toWellFormed === 'function') return s.toWellFormed()
    var out = ''
    for (var i = 0; i < s.length; i++) {
      var c = s.charCodeAt(i)
      if (c >= 0xd800 && c <= 0xdbff) {
        var next = s.charCodeAt(i + 1)
        if (next >= 0xdc00 && next <= 0xdfff) {
          out += s.charAt(i) + s.charAt(i + 1)
          i++
        } else {
          out += '�'
        }
      } else if (c >= 0xdc00 && c <= 0xdfff) {
        out += '�'
      } else {
        out += s.charAt(i)
      }
    }
    return out
  }

  /** Cut to at most `max` bytes of UTF-8 without splitting a character. */
  function cap(value, max) {
    var s = wellFormed(value)
    // A UTF-16 unit is at most 3 bytes of UTF-8: short strings need no encoding.
    if (s.length * 3 <= max) return { value: s, truncated: false }
    var bytes = new TextEncoder().encode(s)
    if (bytes.length <= max) return { value: s, truncated: false }
    var cut = new TextDecoder('utf-8').decode(bytes.subarray(0, max))
    return { value: cut.replace(/�+$/, ''), truncated: true }
  }

  function line(value) {
    return typeof value === 'string' && value ? wellFormed(value) : null
  }

  function canonicalUrl() {
    var links = document.querySelectorAll('link[rel]')
    for (var i = 0; i < links.length; i++) {
      var rels = String(links[i].getAttribute('rel')).toLowerCase().split(/\s+/)
      // The href property is already resolved against the document's base URL.
      if (rels.indexOf('canonical') !== -1) return line(links[i].href)
    }
    return null
  }

  function siteName() {
    var meta = document.querySelector('meta[property="og:site_name"]')
    return meta ? line(meta.getAttribute('content')) : null
  }

  function readableText() {
    var body = document.body
    var main =
      document.querySelector('main') ||
      document.querySelector('[role="main"]') ||
      document.querySelector('article')
    if (main) {
      var text = main.innerText
      if (typeof text === 'string' && text.trim().length >= MAIN_MIN_CHARS) return text
    }
    if (body && typeof body.innerText === 'string') return body.innerText
    return document.documentElement.textContent || ''
  }

  function doctype() {
    try {
      if (document.doctype) return new XMLSerializer().serializeToString(document.doctype)
    } catch (e) {
      // fall through to the default
    }
    return '<!DOCTYPE html>'
  }

  function collapse(value) {
    return value.replace(/\s+/g, ' ')
  }

  function around(range) {
    var root = document.body || document.documentElement
    var before = document.createRange()
    before.selectNodeContents(root)
    before.setEnd(range.startContainer, range.startOffset)
    var after = document.createRange()
    after.selectNodeContents(root)
    after.setStart(range.endContainer, range.endOffset)
    var prefix = collapse(before.toString())
    var suffix = collapse(after.toString())
    return {
      prefix: wellFormed(prefix.slice(Math.max(0, prefix.length - CONTEXT))),
      suffix: wellFormed(suffix.slice(0, CONTEXT)),
    }
  }

  function base(extra) {
    var result = {
      ok: true,
      kind: kind,
      url: String(location.href),
      title: line(document.title),
      canonicalUrl: canonicalUrl(),
      siteName: siteName(),
      lang: line(document.documentElement.getAttribute('lang')),
      quote: null,
      quotePrefix: null,
      quoteSuffix: null,
      html: null,
    }
    for (var key in extra) result[key] = extra[key]
    return result
  }

  function capturePage() {
    var text = cap(readableText(), TEXT_MAX)
    var html = cap(doctype() + document.documentElement.outerHTML, HTML_MAX)
    return base({ text: text.value, html: html.value, truncated: text.truncated || html.truncated })
  }

  function captureSelection() {
    var selection = window.getSelection()
    if (!selection || selection.rangeCount === 0 || selection.isCollapsed) {
      return { ok: false, error: 'no_selection' }
    }
    var raw = selection.toString()
    if (!raw.trim()) return { ok: false, error: 'no_selection' }
    var quote = cap(raw, TEXT_MAX)
    var context = around(selection.getRangeAt(0))
    return base({
      text: quote.value,
      quote: quote.value,
      quotePrefix: context.prefix,
      quoteSuffix: context.suffix,
      truncated: quote.truncated,
    })
  }

  try {
    if (kind !== 'page' && kind !== 'selection') throw new Error('unknown kind')
    if (document.contentType === 'application/pdf') return { ok: false, error: 'pdf_document' }
    return kind === 'page' ? capturePage() : captureSelection()
  } catch (e) {
    var message
    try {
      message = String(e && e.message ? e.message : e)
    } catch (again) {
      message = 'unprintable error'
    }
    return { ok: false, error: 'script_error: ' + message.slice(0, 200) }
  }
})()
