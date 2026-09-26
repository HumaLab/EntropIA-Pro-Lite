"""Build the offline HTML editions from the canonical Markdown files.

Install the pinned renderer with ``python -m pip install -r requirements.txt``,
then run ``python build_html.py`` from any working directory to write the
HTML next to the Markdown sources, or ``python build_html.py --out _site``
to build the full published site layout (index page, ``.nojekyll``, the two
manual pages and their images) into ``_site/``.
"""

from __future__ import annotations

import argparse
import html
import re
import shutil
import unicodedata
import xml.etree.ElementTree as ET
from pathlib import Path
from typing import List, Tuple

from markdown_it import MarkdownIt
from PIL import Image

ROOT = Path(__file__).resolve().parent
MANUAL_SOURCE = ROOT / "manual-usuario.md"
INVENTORY_SOURCE = ROOT / "inventario-funciones-manual.md"
MANUAL_OUTPUT = ROOT / "manual-usuario.html"
INVENTORY_OUTPUT = ROOT / "inventario-funciones-manual.html"
INDEX_SOURCE = ROOT.parent / "index.html"
IMAGES_SOURCE = ROOT / "images"
EXPECTED_CHAPTERS = 21

CHAPTER_PATTERN = re.compile(
    r'(?m)^<a id="(?P<anchor>capitulo-\d+[^\"]*)"></a>\r?\n'
    r'## Capítulo (?P<number>\d+)\.\s*(?P<title>[^\r\n]+)$'
)
IMAGE_PATTERN = re.compile(r"!\[[^\]]*\]\(([^)]+)\)")

STYLES = r"""
/* ================================================================
   Theme tokens — dark (default) / light, matching hlab.com.ar.
   Applied via data-theme="dark|light" on <html>.
   ================================================================ */
:root,
[data-theme="dark"] {
  color-scheme: dark;
  --bg:            #080808;
  --bg-alt:        #0f0f0f;
  --surface:       #101010;
  --surface-soft:  rgba(255,255,255,.04);
  --ink:           #f0f0ec;
  --muted:         rgba(255,255,255,.6);
  --subtle:        rgba(255,255,255,.35);
  --line:          rgba(255,255,255,.1);
  --line-soft:     rgba(255,255,255,.06);
  /* Lightened from the site's #5a67d8 brand hue: on #080808 the base hue
     clears only ~4.2:1, so body-text links use this ~6:1 tint instead;
     the true brand hue stays in --net-pattern for decorative use only. */
  --accent:        #7c86e4;
  --accent-hover:  #98a2ee;
  --accent-soft:   rgba(90,103,216,.16);
  --focus:         #7c86e4;
  --code:          rgba(255,255,255,.05);
  --btn-bg:        #fff;
  --btn-text:      #080808;
  --btn-line-text: rgba(255,255,255,.75);
  --btn-line-line: rgba(255,255,255,.18);
  --shadow:        0 18px 40px rgb(0 0 0 / 35%);
  --net-opacity:   .16;
}
[data-theme="light"] {
  color-scheme: light;
  --bg:            #ffffff;
  --bg-alt:        #fafafa;
  --surface:       #ffffff;
  --surface-soft:  #f3f3f6;
  --ink:           #1a1a1a;
  --muted:         #555;
  --subtle:        #888;
  --line:          #e4e4e8;
  --line-soft:     #eee;
  --accent:        #5a67d8;
  --accent-hover:  #4c5ac0;
  --accent-soft:   rgba(90,103,216,.08);
  --focus:         #4c5ac0;
  --code:          #f3f3f6;
  --btn-bg:        #1a1a1a;
  --btn-text:      #fff;
  --btn-line-text: #444;
  --btn-line-line: #ddd;
  --shadow:        0 12px 30px rgb(0 0 0 / 8%);
  --net-opacity:   .1;
}
@media (prefers-color-scheme: light) {
  :root:not([data-theme]) {
    color-scheme: light;
    --bg:            #ffffff;
    --bg-alt:        #fafafa;
    --surface:       #ffffff;
    --surface-soft:  #f3f3f6;
    --ink:           #1a1a1a;
    --muted:         #555;
    --subtle:        #888;
    --line:          #e4e4e8;
    --line-soft:     #eee;
    --accent:        #5a67d8;
    --accent-hover:  #4c5ac0;
    --accent-soft:   rgba(90,103,216,.08);
    --focus:         #4c5ac0;
    --code:          #f3f3f6;
    --btn-bg:        #1a1a1a;
    --btn-text:      #fff;
    --btn-line-text: #444;
    --btn-line-line: #ddd;
    --shadow:        0 12px 30px rgb(0 0 0 / 8%);
    --net-opacity:   .1;
  }
}
:root {
  --sans: "Space Grotesk", "Segoe UI", system-ui, -apple-system, "Segoe UI Variable Text", sans-serif;
  --serif: "Fraunces", Georgia, "Iowan Old Style", Cambria, serif;
  --mono: "JetBrains Mono", "Cascadia Mono", Consolas, "Courier New", monospace;
  /* Static constellation tile — no JS animation, low opacity via --net-opacity. */
  --net-pattern: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='400' height='400' viewBox='0 0 400 400'%3E%3Cg fill='none' stroke='%235a67d8' stroke-width='.75' stroke-opacity='.5'%3E%3Cline x1='40' y1='60' x2='140' y2='130'/%3E%3Cline x1='140' y1='130' x2='90' y2='230'/%3E%3Cline x1='140' y1='130' x2='260' y2='90'/%3E%3Cline x1='260' y1='90' x2='340' y2='170'/%3E%3Cline x1='90' y1='230' x2='180' y2='300'/%3E%3Cline x1='180' y1='300' x2='300' y2='330'/%3E%3Cline x1='260' y1='90' x2='220' y2='220'/%3E%3Cline x1='220' y1='220' x2='180' y2='300'/%3E%3Cline x1='340' y1='170' x2='300' y2='330'/%3E%3Cline x1='40' y1='60' x2='10' y2='190'/%3E%3Cline x1='10' y1='190' x2='90' y2='230'/%3E%3Cline x1='220' y1='220' x2='340' y2='170'/%3E%3C/g%3E%3Cg fill='%235a67d8' fill-opacity='.85'%3E%3Ccircle cx='40' cy='60' r='2.2'/%3E%3Ccircle cx='140' cy='130' r='2.6'/%3E%3Ccircle cx='260' cy='90' r='2.2'/%3E%3Ccircle cx='340' cy='170' r='2.4'/%3E%3Ccircle cx='90' cy='230' r='2.2'/%3E%3Ccircle cx='220' cy='220' r='2.6'/%3E%3Ccircle cx='180' cy='300' r='2.2'/%3E%3Ccircle cx='300' cy='330' r='2.4'/%3E%3Ccircle cx='10' cy='190' r='2'/%3E%3C/g%3E%3C/svg%3E");
}
* { box-sizing: border-box; }
html { scroll-behavior: smooth; }
body {
  position: relative;
  margin: 0;
  background: var(--bg);
  color: var(--ink);
  font: 1.05rem/1.75 var(--serif);
  text-rendering: optimizeLegibility;
  transition: background-color .2s ease, color .2s ease;
}
body::before {
  content: "";
  position: fixed;
  inset: 0;
  z-index: 0;
  background: var(--net-pattern) center / 46rem repeat;
  opacity: var(--net-opacity);
  pointer-events: none;
}
.topbar, .page-layout { position: relative; z-index: 1; }
a { color: var(--accent); text-decoration-thickness: .08em; text-underline-offset: .18em; }
a:hover { color: var(--accent-hover); }
a:focus-visible, summary:focus-visible, input:focus-visible, button:focus-visible {
  outline: 2px solid var(--focus);
  outline-offset: 3px;
  border-radius: 2px;
}
.skip-link {
  position: absolute;
  z-index: 5;
  top: .6rem;
  left: .6rem;
  transform: translateY(-160%);
  padding: .65rem .9rem;
  border-radius: 4px;
  background: var(--accent);
  color: #fff;
  font: 650 .95rem/1.2 var(--sans);
}
.skip-link:focus { transform: none; }
.topbar {
  position: sticky;
  z-index: 3;
  top: 0;
  border-bottom: 1px solid var(--line-soft);
  background: color-mix(in srgb, var(--bg) 92%, transparent);
  backdrop-filter: blur(10px);
}
.progress {
  height: 2px;
  transform: scaleX(0);
  transform-origin: left;
  background: var(--accent);
}
.topbar-inner {
  max-width: 1440px;
  min-height: 4.1rem;
  margin: 0 auto;
  padding: .7rem 1.4rem;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 1rem;
}
.brand {
  display: flex;
  align-items: center;
  gap: .5rem;
  color: var(--ink);
  font: 700 1.05rem/1.1 var(--sans);
  letter-spacing: -.02em;
  text-decoration: none;
}
.brand:hover { color: var(--ink); }
.brand-mark { display: block; width: 1.15rem; height: 1.15rem; color: var(--accent); flex-shrink: 0; }
.brand span { display: none; }
.topbar-actions { display: flex; align-items: center; gap: .6rem; }
.document-nav { display: flex; flex-wrap: wrap; gap: .1rem; font: 600 .85rem/1.2 var(--sans); }
.document-nav a {
  position: relative;
  padding: .5rem .75rem;
  color: var(--muted);
  text-decoration: none;
  letter-spacing: .01em;
}
.document-nav a::after {
  content: "";
  position: absolute;
  left: .75rem; right: .75rem; bottom: 0;
  height: 1px;
  background: var(--accent);
  transform: scaleX(0);
  transform-origin: left center;
  transition: transform .2s ease;
}
.document-nav a:hover { color: var(--ink); }
.document-nav a:hover::after { transform: scaleX(1); }
.document-nav a[aria-current="page"] { color: var(--ink); }
.document-nav a[aria-current="page"]::after { transform: scaleX(1); }
.theme-toggle {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 2rem;
  height: 2rem;
  padding: 0;
  border: 1px solid var(--line);
  border-radius: 4px;
  background: transparent;
  color: var(--muted);
  cursor: pointer;
  transition: border-color .2s, color .2s;
}
.theme-toggle:hover { border-color: var(--accent); color: var(--accent); }
.theme-toggle svg { width: 15px; height: 15px; }
/* Icon shows the theme that is currently active. */
.theme-toggle .icon-sun { display: none; }
.theme-toggle .icon-moon { display: block; }
[data-theme="light"] .theme-toggle .icon-sun { display: block; }
[data-theme="light"] .theme-toggle .icon-moon { display: none; }
@media (prefers-color-scheme: light) {
  :root:not([data-theme]) .theme-toggle .icon-sun { display: block; }
  :root:not([data-theme]) .theme-toggle .icon-moon { display: none; }
}
.page-layout {
  display: grid;
  grid-template-columns: minmax(15rem, 18.5rem) minmax(0, 52rem);
  align-items: start;
  justify-content: center;
  gap: 1.6rem;
  max-width: 1440px;
  margin: 0 auto;
  padding: 1.4rem;
}
.sidebar { position: sticky; top: 5.4rem; display: flex; max-height: calc(100vh - 6.4rem); overflow: hidden; font-family: var(--sans); }
.contents { display: flex; flex-direction: column; min-height: 0; padding: 1rem; border: 1px solid var(--line-soft); border-radius: 8px; background: var(--surface); }
.contents summary {
  cursor: pointer;
  font-size: .68rem;
  font-weight: 600;
  letter-spacing: .16em;
  text-transform: uppercase;
  color: var(--subtle);
  font-family: var(--mono);
}
.filter-control { display: none; margin-top: .9rem; }
.js .filter-control { display: block; }
.filter-control label { display: block; margin-bottom: .35rem; font-size: .78rem; font-weight: 600; color: var(--muted); }
.filter-control input {
  width: 100%;
  min-height: 2.6rem;
  padding: .55rem .7rem;
  border: 1px solid var(--line);
  border-radius: 4px;
  background: var(--bg-alt);
  color: var(--ink);
  font: 400 .95rem/1.3 var(--sans);
}
.filter-control input:focus-visible { outline: 2px solid var(--focus); outline-offset: 1px; }
.nav-list { flex: 1 1 auto; min-height: 0; overflow: auto; margin: .8rem 0 0; padding: 0; list-style: none; }
.nav-list li[hidden] { display: none; }
.nav-list a {
  display: block;
  padding: .42rem .55rem .42rem .7rem;
  border-left: 2px solid transparent;
  border-radius: 0 4px 4px 0;
  color: var(--muted);
  font-size: .88rem;
  line-height: 1.4;
  text-decoration: none;
}
.nav-list a:hover { background: var(--surface-soft); color: var(--ink); }
.nav-list a[aria-current="true"] { border-left-color: var(--accent); background: var(--accent-soft); color: var(--ink); font-weight: 600; }
.filter-empty { margin: .8rem 0 0; color: var(--muted); font-size: .88rem; }
.other-document { display: block; margin-top: .9rem; padding-top: .8rem; border-top: 1px solid var(--line-soft); font-size: .82rem; font-weight: 600; font-family: var(--sans); color: var(--accent); }
.doc-content {
  min-width: 0;
  padding: clamp(1.4rem, 4vw, 3rem);
  border: 1px solid var(--line-soft);
  border-radius: 8px;
  background: var(--surface);
  box-shadow: var(--shadow);
  overflow-wrap: break-word;
  max-width: 74ch;
}
.doc-content h1, .doc-content h2, .doc-content h3, .doc-content h4 {
  font-family: var(--sans);
  color: var(--ink);
  line-height: 1.2;
  scroll-margin-top: 6rem;
  font-weight: 700;
  letter-spacing: -.02em;
}
.doc-content > h1 { margin: 0 0 .2em; font-size: clamp(2rem, 5vw, 3rem); }
.doc-content h2 { margin-top: 2.6rem; padding-top: .9rem; border-top: 1px solid var(--line-soft); font-size: clamp(1.4rem, 3vw, 1.85rem); }
.doc-content h2:first-of-type { margin-top: .2rem; border-top: 0; padding-top: 0; }
.doc-content h2:first-of-type::before { display: none; }
.doc-content h2::before { content: ""; display: block; width: 2.2rem; height: 2px; margin-bottom: .6rem; background: var(--accent); }
.doc-content h3 { margin-top: 1.7rem; font-size: 1.22rem; }
.doc-content h4 { margin-top: 1.3rem; font-size: 1.04rem; }
.doc-content p { margin: .85rem 0; max-width: 70ch; }
.doc-content li + li { margin-top: .28rem; }
.doc-content ul, .doc-content ol { padding-left: 1.35rem; max-width: 70ch; }
.doc-content em, .doc-content i { font-style: italic; color: var(--accent); }
.doc-content blockquote, .callout {
  margin: 1.15rem 0;
  padding: .85rem 1rem;
  border-left: 3px solid var(--accent);
  border-radius: 0 6px 6px 0;
  background: var(--surface-soft);
  font-family: var(--sans);
  font-size: .96rem;
}
.callout-warning { border-left-color: #e0a634; background: color-mix(in srgb, #e0a634 12%, var(--surface-soft)); }
.callout-important { border-left-color: var(--accent); background: var(--accent-soft); }
.callout-tip { border-left-color: #4ade80; background: color-mix(in srgb, #4ade80 10%, var(--surface-soft)); }
.callout-guide { border-left-color: var(--subtle); background: var(--surface-soft); }
.doc-content img { display: block; max-width: 100%; height: auto; margin: 1.3rem auto; border: 1px solid var(--line); border-radius: 6px; background: #fff; }
.manual-figure { margin: 1.6rem 0; }
.manual-figure > a { display: block; border-radius: 6px; cursor: zoom-in; }
.manual-figure img { margin: 0 auto; }
.manual-figure figcaption { margin-top: .65rem; color: var(--muted); font: .85rem/1.6 var(--sans); }
.figure-hint { display: block; margin-top: .2rem; color: var(--accent); font-size: .78rem; }
.doc-content hr { margin: 2rem 0; border: 0; border-top: 1px solid var(--line-soft); }
.doc-content table { display: block; max-width: 100%; overflow-x: auto; border-collapse: collapse; font: .92rem/1.5 var(--sans); }
.doc-content th, .doc-content td { padding: .6rem .7rem; border: 1px solid var(--line-soft); text-align: left; vertical-align: top; }
.doc-content th { background: var(--surface-soft); color: var(--ink); font-weight: 600; }
.doc-content tr:nth-child(even) td { background: var(--surface-soft); }
.doc-content pre, .doc-content code { font-family: var(--mono); }
.doc-content pre { max-width: 100%; overflow: auto; padding: .9rem 1rem; border-radius: 6px; background: var(--code); border: 1px solid var(--line-soft); }
.doc-content code { padding: .08em .28em; border-radius: 3px; background: var(--code); font-size: .87em; }
.doc-content pre code { padding: 0; background: transparent; border: 0; }
.site-footer {
  max-width: 1440px;
  margin: 0 auto;
  padding: 1.4rem;
  text-align: center;
  font: .8rem/1.5 var(--mono);
  letter-spacing: .04em;
  color: var(--subtle);
}
.site-footer a { color: var(--subtle); }
.site-footer a:hover { color: var(--accent); }
.print-index { display: none; }
@media (max-width: 900px) {
  .topbar { position: static; }
  .topbar-inner { align-items: flex-start; flex-direction: column; }
  .page-layout { grid-template-columns: minmax(0, 1fr); padding: 1rem; }
  .sidebar { position: static; }
  .sidebar, .contents { display: block; max-height: none; overflow: visible; }
  .nav-list { max-height: 15rem; }
}
@media (max-width: 540px) {
  .page-layout, .doc-content, .contents { padding-inline: .9rem; }
  .doc-content { border-radius: 6px; }
  .topbar-inner { padding-inline: .9rem; }
}
@media (prefers-reduced-motion: reduce) {
  html { scroll-behavior: auto; }
  .progress { display: none; }
}
@media print {
  body { background: #fff !important; color: #000; font-size: 11pt; }
  body::before { display: none; }
  .skip-link, .topbar, .sidebar, .progress, .site-footer { display: none !important; }
  .page-layout { display: block; max-width: none; margin: 0; padding: 0; }
  .doc-content { padding: 0; border: 0; border-radius: 0; box-shadow: none; max-width: none; }
  .print-index { display: block; margin: 0 0 1.5rem; break-after: page; }
  .print-index ol { columns: 2; padding-left: 1.2rem; }
  .doc-content h1, .doc-content h2, .doc-content h3 { break-after: avoid; }
  .doc-content img, .doc-content blockquote, .doc-content tr { break-inside: avoid; }
  .manual-figure { break-inside: avoid; }
  .manual-figure img { max-height: 15cm; width: auto; }
  .figure-hint { display: none; }
  .doc-content a { color: inherit; }
}
@supports not (color: color-mix(in srgb, red, blue)) {
  .topbar { background: var(--bg); }
  .callout-warning { background: var(--surface-soft); }
  .callout-tip { background: var(--surface-soft); }
}
"""

THEME_INIT_SCRIPT = r"""
(() => {
  try {
    const stored = window.localStorage.getItem("entropia-manual-theme");
    if (stored === "light" || stored === "dark") {
      document.documentElement.setAttribute("data-theme", stored);
    }
  } catch (err) {
    /* Private mode or blocked storage: fall back to prefers-color-scheme. */
  }
})();
"""

PAGE_SCRIPT = r"""
(() => {
  document.documentElement.classList.add("js");

  const toggle = document.querySelector("[data-theme-toggle]");
  const STORAGE_KEY = "entropia-manual-theme";
  const prefersLight = window.matchMedia && window.matchMedia("(prefers-color-scheme: light)").matches;
  const currentTheme = () =>
    document.documentElement.getAttribute("data-theme") || (prefersLight ? "light" : "dark");
  if (toggle) {
    toggle.addEventListener("click", () => {
      const next = currentTheme() === "light" ? "dark" : "light";
      document.documentElement.setAttribute("data-theme", next);
      try {
        window.localStorage.setItem(STORAGE_KEY, next);
      } catch (err) {
        /* Private mode or blocked storage: theme just won't persist. */
      }
    });
  }

  const bar = document.querySelector("[data-progress]");
  const updateProgress = () => {
    if (!bar) return;
    const scrollable = document.documentElement.scrollHeight - window.innerHeight;
    const ratio = scrollable > 0 ? window.scrollY / scrollable : 0;
    bar.style.transform = `scaleX(${Math.min(1, Math.max(0, ratio))})`;
  };
  updateProgress();
  window.addEventListener("scroll", updateProgress, { passive: true });

  const input = document.querySelector("[data-nav-filter]");
  const items = Array.from(document.querySelectorAll("[data-nav-item]"));
  const empty = document.querySelector("[data-no-results]");
  const normalize = (text) => text.normalize("NFD").replace(/[\u0300-\u036f]/g, "").toLocaleLowerCase("es");
  if (input && items.length) {
    const filterItems = () => {
      const query = normalize(input.value.trim());
      let visible = 0;
      for (const item of items) {
        const matches = normalize(item.textContent || "").includes(query);
        item.hidden = !matches;
        if (matches) visible += 1;
      }
      if (empty) empty.hidden = visible !== 0;
    };
    input.addEventListener("input", filterItems);
  }

  const links = Array.from(document.querySelectorAll("#section-nav a"));
  const targets = links.map((link) => document.querySelector(link.getAttribute("href"))).filter(Boolean);
  if (!targets.length || !("IntersectionObserver" in window)) return;
  const visible = new Map();
  const mark = () => {
    let active = null;
    for (let index = targets.length - 1; index >= 0; index -= 1) {
      if (visible.get(targets[index])) {
        active = targets[index];
        break;
      }
    }
    if (!active) {
      const line = window.scrollY + 96;
      for (const target of targets) if (target.offsetTop <= line) active = target;
    }
    for (const link of links) {
      const on = active && link.getAttribute("href") === `#${active.id}`;
      if (on) link.setAttribute("aria-current", "true");
      else link.removeAttribute("aria-current");
    }
  };
  const observer = new IntersectionObserver((entries) => {
    for (const entry of entries) visible.set(entry.target, entry.isIntersecting);
    mark();
  }, { rootMargin: "-15% 0px -55% 0px", threshold: [0, 1] });
  for (const target of targets) observer.observe(target);
  mark();
})();
"""

PAGE_TEMPLATE = """<!doctype html>
<html lang="es">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <meta name="color-scheme" content="dark light">
  <title>@@TITLE@@</title>
  <link rel="icon" type="image/x-icon" href="images/entropia.ico">
  <script>
@@THEME_INIT_SCRIPT@@
  </script>
  <link rel="preconnect" href="https://fonts.googleapis.com">
  <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
  <link href="https://fonts.googleapis.com/css2?family=Space+Grotesk:wght@400;600;700&family=Fraunces:ital,wght@0,400;0,500;1,500&family=JetBrains+Mono:wght@400;600&display=swap" rel="stylesheet">
  <style>
@@STYLES@@
  </style>
</head>
<body>
  <a class="skip-link" href="#contenido">Saltar al contenido principal</a>
  <header class="topbar">
    <div class="progress" data-progress aria-hidden="true"></div>
    <div class="topbar-inner">
      <a class="brand" href="manual-usuario.html">
        <svg class="brand-mark" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" aria-hidden="true">
          <circle cx="5" cy="5" r="1.6" fill="currentColor" stroke="none"/>
          <circle cx="19" cy="6" r="1.6" fill="currentColor" stroke="none"/>
          <circle cx="12" cy="12" r="1.6" fill="currentColor" stroke="none"/>
          <circle cx="6" cy="19" r="1.6" fill="currentColor" stroke="none"/>
          <circle cx="18" cy="18" r="1.6" fill="currentColor" stroke="none"/>
          <path d="M5 5 12 12M19 6 12 12M12 12 6 19M12 12 18 18"/>
        </svg>
        EntropIA Lite
      </a>
@@DOCUMENT_NAV@@
      <div class="topbar-actions">
        <button type="button" class="theme-toggle" data-theme-toggle aria-label="Cambiar entre tema claro y oscuro">
          <svg class="icon-sun" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true"><circle cx="12" cy="12" r="4.2"/><path d="M12 2.5v2.4M12 19.1v2.4M4.4 4.4l1.7 1.7M17.9 17.9l1.7 1.7M2.5 12h2.4M19.1 12h2.4M4.4 19.6l1.7-1.7M17.9 6.1l1.7-1.7"/></svg>
          <svg class="icon-moon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true"><path d="M20 14.2A8.2 8.2 0 1 1 9.8 4a6.6 6.6 0 0 0 10.2 10.2z"/></svg>
        </button>
      </div>
    </div>
  </header>
  <div class="page-layout">
    <aside class="sidebar" aria-label="@@ASIDE_LABEL@@">
      <details class="contents" open>
        <summary>@@SUMMARY@@</summary>
        <div class="filter-control">
          <label for="nav-filter">@@FILTER_LABEL@@</label>
          <input id="nav-filter" type="search" data-nav-filter aria-controls="section-nav" autocomplete="off" placeholder="Escribí un título o tema">
        </div>
@@SECTION_NAV@@
        <p class="filter-empty" data-no-results role="status" hidden>No hay coincidencias en este índice.</p>
        <a class="other-document" href="@@OTHER_HREF@@">@@OTHER_LABEL@@</a>
      </details>
    </aside>
    <main id="contenido" class="doc-content" tabindex="-1">
@@PRINT_INDEX@@
@@DOCUMENT_BODY@@
    </main>
  </div>
  <footer class="site-footer">
    <p>Desarrollado por <a href="https://hlab.com.ar/" rel="noopener">HLab</a></p>
  </footer>
  <script>
@@PAGE_SCRIPT@@
  </script>
</body>
</html>
"""


def slugify(value: str) -> str:
    normalized = unicodedata.normalize("NFKD", value)
    plain = "".join(char for char in normalized if not unicodedata.combining(char))
    return re.sub(r"[^a-z0-9]+", "-", plain.lower()).strip("-")


def check_local_images(markdown: str) -> None:
    for match in IMAGE_PATTERN.finditer(markdown):
        target = match.group(1).split()[0].strip("<>")
        if target.startswith(("http://", "https://", "data:")):
            continue
        local_path = target.split("#", 1)[0]
        if local_path and not (ROOT / local_path).is_file():
            raise ValueError(f"No se encontró la imagen local: {target}")


def chapter_items(markdown: str) -> List[Tuple[str, str]]:
    matches = list(CHAPTER_PATTERN.finditer(markdown))
    numbers = [int(match.group("number")) for match in matches]
    expected = list(range(1, EXPECTED_CHAPTERS + 1))
    if numbers != expected:
        raise ValueError(f"Se esperaban los capítulos {expected}; se encontraron {numbers}.")

    items = [
        (match.group("anchor"), f"{match.group('number')}. {match.group('title')}")
        for match in matches
    ]
    if len({anchor for anchor, _ in items}) != len(items):
        raise ValueError("El manual contiene anclas de capítulo repetidas.")
    return items


def inventory_body_and_items(parser: MarkdownIt, markdown: str) -> Tuple[str, List[Tuple[str, str]]]:
    tokens = parser.parse(markdown)
    items: List[Tuple[str, str]] = []
    for index, token in enumerate(tokens[:-1]):
        if token.type != "heading_open" or token.tag != "h2":
            continue
        title = tokens[index + 1].content
        anchor = f"seccion-{slugify(title)}"
        token.attrSet("id", anchor)
        items.append((anchor, title))

    if len({anchor for anchor, _ in items}) != len(items):
        raise ValueError("El inventario contiene títulos de sección repetidos.")
    return parser.renderer.render(tokens, parser.options, {}), items


def nav_markup(items: List[Tuple[str, str]], label: str) -> str:
    links = "\n".join(
        f'          <li data-nav-item><a href="#{html.escape(anchor, quote=True)}">{html.escape(title)}</a></li>'
        for anchor, title in items
    )
    return (
        f'        <nav id="section-nav" aria-label="{html.escape(label, quote=True)}">\n'
        f'          <ul class="nav-list">\n{links}\n          </ul>\n'
        f'        </nav>'
    )


def document_switcher(current: str) -> str:
    pages = (
        ("manual", "manual-usuario.html", "Manual"),
        ("inventory", "inventario-funciones-manual.html", "Inventario de funciones"),
    )
    links = []
    for key, href, label in pages:
        current_attr = ' aria-current="page"' if key == current else ""
        links.append(f'        <a href="{href}"{current_attr}>{label}</a>')
    return '      <nav class="document-nav" aria-label="Documentos">\n' + "\n".join(links) + "\n      </nav>"


def rewrite_document_links(rendered: str) -> str:
    pairs = (
        ("manual-usuario.md", "manual-usuario.html"),
        ("inventario-funciones-manual.md", "inventario-funciones-manual.html"),
    )
    for source, target in pairs:
        pattern = re.compile(rf'href="{re.escape(source)}(?P<fragment>#[^\"]*)?"')
        rendered = pattern.sub(
            lambda match: f'href="{target}{match.group("fragment") or ""}"',
            rendered,
        )
    return rendered


def attach_explicit_ids(rendered: str) -> str:
    rendered = re.sub(
        r'<p><a id="([^"]+)"></a></p>\s*<(h[1-6])>',
        r'<\2 id="\1">',
        rendered,
    )
    return re.sub(
        r'<a id="([^"]+)"></a>\s*<(h[1-6])>',
        r'<\2 id="\1">',
        rendered,
    )


def classify_callouts(rendered: str) -> str:
    def replace(match: re.Match[str]) -> str:
        inner = match.group(1)
        opening = re.sub(r"<[^>]+>", "", inner)[:160]
        if "Atención" in opening:
            kind = "warning"
        elif "Importante" in opening:
            kind = "important"
        elif "Consejo" in opening:
            kind = "tip"
        elif "Cómo leer" in opening:
            kind = "guide"
        else:
            kind = "note"
        return f'<blockquote class="callout callout-{kind}">{inner}</blockquote>'

    return re.sub(r"<blockquote>(.*?)</blockquote>", replace, rendered, flags=re.S)


def optimize_images(rendered: str) -> str:
    seen = 0

    def replace(match: re.Match[str]) -> str:
        nonlocal seen
        seen += 1
        priority = 'fetchpriority="high"' if seen == 1 else 'loading="lazy"'
        attributes = match.group(1)
        source = re.search(r'\bsrc="([^"]+)"', attributes)
        dimensions = ""
        if source is not None:
            path = ROOT / html.unescape(source.group(1))
            if path.is_file():
                if path.suffix.lower() == ".svg":
                    svg = ET.parse(path).getroot()
                    width, height = svg.attrib["width"], svg.attrib["height"]
                else:
                    with Image.open(path) as image:
                        width, height = image.size
                dimensions = f' width="{width}" height="{height}"'
        return f'<img {priority} decoding="async"{dimensions} {attributes}>'

    rendered = re.sub(r"<img ([^>]+)>", replace, rendered)

    def figure(match: re.Match[str]) -> str:
        image = match.group(1)
        source = re.search(r'\bsrc="([^"]+)"', image)
        caption = re.search(r'\balt="([^"]*)"', image)
        if source is None or caption is None:
            return match.group(0)
        return (
            f'<figure class="manual-figure"><a href="{source.group(1)}" '
            f'target="_blank" rel="noopener" '
            f'aria-label="{caption.group(1)} — Abrir imagen a tamaño completo (nueva pestaña)">'
            f'{image}</a><figcaption>{caption.group(1)}'
            '<span class="figure-hint">Pulsá la imagen para verla a tamaño completo '
            'en otra pestaña.</span></figcaption></figure>'
        )

    return re.sub(r"<p>(<img [^>]+>)</p>", figure, rendered)


def prepare_body(rendered: str) -> str:
    return optimize_images(classify_callouts(attach_explicit_ids(rendered)))


def print_index(items: List[Tuple[str, str]]) -> str:
    links = "".join(
        f'<li><a href="#{html.escape(anchor, quote=True)}">{html.escape(title)}</a></li>'
        for anchor, title in items
    )
    return f'<nav class="print-index" aria-label="Índice para impresión"><ol>{links}</ol></nav>'


def page_markup(
    title: str,
    current: str,
    aside_label: str,
    summary: str,
    filter_label: str,
    section_label: str,
    items: List[Tuple[str, str]],
    other_href: str,
    other_label: str,
    body: str,
) -> str:
    replacements = {
        "@@TITLE@@": html.escape(title),
        "@@STYLES@@": STYLES,
        "@@THEME_INIT_SCRIPT@@": THEME_INIT_SCRIPT,
        "@@DOCUMENT_NAV@@": document_switcher(current),
        "@@ASIDE_LABEL@@": html.escape(aside_label),
        "@@SUMMARY@@": html.escape(summary),
        "@@FILTER_LABEL@@": html.escape(filter_label),
        "@@SECTION_NAV@@": nav_markup(items, section_label),
        "@@OTHER_HREF@@": html.escape(other_href, quote=True),
        "@@OTHER_LABEL@@": html.escape(other_label),
        "@@PRINT_INDEX@@": print_index(items),
        "@@DOCUMENT_BODY@@": body,
        "@@PAGE_SCRIPT@@": PAGE_SCRIPT,
    }
    page = PAGE_TEMPLATE
    for placeholder, value in replacements.items():
        page = page.replace(placeholder, value)
    if "@@" in page:
        raise ValueError("Quedó un marcador de plantilla sin reemplazar.")
    return page


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--out",
        type=Path,
        default=None,
        help=(
            "Site output directory (e.g. _site). When given, the full "
            "published layout is written there: index.html, .nojekyll, "
            "manual-usuario/{manual-usuario.html,inventario-funciones-manual.html,images/}. "
            "When omitted, the two HTML files are written next to their "
            "Markdown sources, matching the previous behaviour."
        ),
    )
    return parser.parse_args()


def build_site(out_dir: Path) -> None:
    """Assemble the non-generated parts of the published site layout."""
    manual_out_dir = out_dir / "manual-usuario"
    manual_out_dir.mkdir(parents=True, exist_ok=True)

    images_dst = manual_out_dir / "images"
    if images_dst.exists():
        shutil.rmtree(images_dst)
    shutil.copytree(IMAGES_SOURCE, images_dst)

    shutil.copy2(INDEX_SOURCE, out_dir / "index.html")
    (out_dir / ".nojekyll").touch()


def main() -> None:
    args = parse_args()
    if args.out is not None:
        out_dir = args.out.resolve()
        manual_output = out_dir / "manual-usuario" / "manual-usuario.html"
        inventory_output = out_dir / "manual-usuario" / "inventario-funciones-manual.html"
    else:
        out_dir = None
        manual_output = MANUAL_OUTPUT
        inventory_output = INVENTORY_OUTPUT

    manual = MANUAL_SOURCE.read_text(encoding="utf-8")
    inventory = INVENTORY_SOURCE.read_text(encoding="utf-8")
    check_local_images(manual)
    check_local_images(inventory)

    parser = MarkdownIt("default", {"html": True})
    chapters = chapter_items(manual)
    # The sidebar replaces the repeated inline chapter list in the HTML edition.
    manual_html_source = re.sub(
        r"(?ms)^## Índice\s*\n.*?^---\s*$",
        "",
        manual,
        count=1,
    )
    manual_body = prepare_body(rewrite_document_links(parser.render(manual_html_source)))

    inventory_body, sections = inventory_body_and_items(parser, inventory)
    inventory_body = prepare_body(rewrite_document_links(inventory_body))

    manual_page = page_markup(
        title="Manual de usuario · EntropIA Lite",
        current="manual",
        aside_label="Índice del manual",
        summary="Índice de capítulos",
        filter_label="Filtrar capítulos",
        section_label="Capítulos del manual",
        items=chapters,
        other_href="inventario-funciones-manual.html",
        other_label="Ver inventario de funciones",
        body=manual_body,
    )
    inventory_page = page_markup(
        title="Inventario de funciones · EntropIA Lite",
        current="inventory",
        aside_label="Índice del inventario",
        summary="Índice de secciones",
        filter_label="Filtrar secciones",
        section_label="Secciones del inventario",
        items=sections,
        other_href="manual-usuario.html",
        other_label="Volver al manual de usuario",
        body=inventory_body,
    )

    if out_dir is not None:
        build_site(out_dir)

    manual_output.parent.mkdir(parents=True, exist_ok=True)
    manual_output.write_text(manual_page, encoding="utf-8")
    inventory_output.write_text(inventory_page, encoding="utf-8")
    print(f"Generado: {manual_output.name} ({len(chapters)} capítulos)")
    print(f"Generado: {inventory_output.name} ({len(sections)} secciones)")


if __name__ == "__main__":
    main()
