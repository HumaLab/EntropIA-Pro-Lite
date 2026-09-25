"""Build the offline HTML editions from the canonical Markdown files.

Install the pinned renderer with ``python -m pip install -r requirements.txt``,
then run ``python build_html.py`` from any working directory.
"""

from __future__ import annotations

import html
import re
import unicodedata
from pathlib import Path
from typing import List, Tuple

from markdown_it import MarkdownIt

ROOT = Path(__file__).resolve().parent
MANUAL_SOURCE = ROOT / "manual-usuario.md"
INVENTORY_SOURCE = ROOT / "inventario-funciones-manual.md"
MANUAL_OUTPUT = ROOT / "manual-usuario.html"
INVENTORY_OUTPUT = ROOT / "inventario-funciones-manual.html"
EXPECTED_CHAPTERS = 21

CHAPTER_PATTERN = re.compile(
    r'(?m)^<a id="(?P<anchor>capitulo-\d+[^\"]*)"></a>\r?\n'
    r'## Capítulo (?P<number>\d+)\.\s*(?P<title>[^\r\n]+)$'
)
IMAGE_PATTERN = re.compile(r"!\[[^\]]*\]\(([^)]+)\)")

STYLES = r"""
:root {
  color-scheme: light;
  --paper: #f4efe4;
  --surface: #fffdf8;
  --surface-soft: #e8f0ea;
  --ink: #1c2421;
  --muted: #4e5c56;
  --line: #c8bba6;
  --brand: #1f4d43;
  --brand-dark: #14362f;
  --copper: #8a3e12;
  --focus: #8a3e12;
  --code: #efe8da;
  --serif: "Palatino Linotype", Palatino, "Iowan Old Style", Cambria, serif;
  --sans: "Segoe UI Variable Text", "Segoe UI", Candara, sans-serif;
  --mono: "Cascadia Mono", Consolas, "Courier New", monospace;
}
* { box-sizing: border-box; }
html { scroll-behavior: smooth; }
body {
  margin: 0;
  background:
    radial-gradient(circle at 12% -8%, rgb(196 106 50 / 12%), transparent 28rem),
    linear-gradient(180deg, #f7f3ea 0, var(--paper) 18rem);
  color: var(--ink);
  font: 1.05rem/1.7 var(--serif);
  text-rendering: optimizeLegibility;
}
a { color: var(--brand-dark); text-decoration-thickness: .08em; text-underline-offset: .18em; }
a:hover { color: #0d2823; }
a:focus-visible, summary:focus-visible, input:focus-visible {
  outline: 3px solid var(--focus);
  outline-offset: 3px;
}
.skip-link {
  position: absolute;
  z-index: 5;
  top: .6rem;
  left: .6rem;
  transform: translateY(-160%);
  padding: .65rem .9rem;
  border-radius: .35rem;
  background: var(--brand-dark);
  color: #fff;
  font: 650 .95rem/1.2 var(--sans);
}
.skip-link:focus { transform: none; }
.topbar {
  position: sticky;
  z-index: 3;
  top: 0;
  border-bottom: 1px solid var(--line);
  background: var(--surface);
}
.progress {
  height: 3px;
  transform: scaleX(0);
  transform-origin: left;
  background: var(--copper);
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
  color: var(--ink);
  font: 700 1.15rem/1.1 var(--serif);
  letter-spacing: -.03em;
  text-decoration: none;
}
.brand span { display: none; }
.document-nav { display: flex; flex-wrap: wrap; gap: .35rem; font: 650 .92rem/1.2 var(--sans); }
.document-nav a { padding: .55rem .8rem; border: 1px solid transparent; border-radius: 999px; color: var(--brand-dark); text-decoration: none; }
.document-nav a:hover { border-color: var(--line); background: var(--surface); }
.document-nav a[aria-current="page"] { border-color: var(--brand-dark); background: var(--brand-dark); color: #fff; }
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
.contents { display: flex; flex-direction: column; min-height: 0; padding: 1rem; border: 1px solid var(--line); border-radius: 8px 18px 8px 18px; background: var(--surface); }
.contents summary { cursor: pointer; font-size: .98rem; font-weight: 750; }
.filter-control { display: none; margin-top: .9rem; }
.js .filter-control { display: block; }
.filter-control label { display: block; margin-bottom: .35rem; font-size: .86rem; font-weight: 700; }
.filter-control input {
  width: 100%;
  min-height: 2.7rem;
  padding: .55rem .7rem;
  border: 1px solid #879790;
  border-radius: 7px;
  background: #fff;
  color: var(--ink);
  font: 400 .95rem/1.3 var(--sans);
}
.nav-list { flex: 1 1 auto; min-height: 0; overflow: auto; margin: .8rem 0 0; padding: 0; list-style: none; }
.nav-list li[hidden] { display: none; }
.nav-list a {
  display: block;
  padding: .42rem .55rem .42rem .7rem;
  border-left: 3px solid transparent;
  border-radius: 0 6px 6px 0;
  color: var(--brand-dark);
  font-size: .9rem;
  line-height: 1.35;
  text-decoration: none;
}
.nav-list a:hover { background: var(--surface-soft); }
.nav-list a[aria-current="true"] { border-left-color: var(--copper); background: #f3e2d4; color: #5b2b12; font-weight: 750; }
.filter-empty { margin: .8rem 0 0; color: var(--muted); font-size: .9rem; }
.other-document { display: block; margin-top: .9rem; padding-top: .8rem; border-top: 1px solid var(--line); font-size: .9rem; font-weight: 750; }
.doc-content {
  min-width: 0;
  padding: clamp(1.4rem, 4vw, 3rem);
  border: 1px solid var(--line);
  border-radius: 8px 22px 8px 22px;
  background: var(--surface);
  box-shadow: 0 18px 40px rgb(28 36 33 / 5%);
  overflow-wrap: break-word;
}
.doc-content h1, .doc-content h2, .doc-content h3, .doc-content h4 { color: #172e29; line-height: 1.18; scroll-margin-top: 6rem; font-weight: 700; letter-spacing: -.03em; }
.doc-content > h1 { margin: 0 0 .2em; font-size: clamp(2.2rem, 5vw, 3.4rem); }
.doc-content h2 { margin-top: 2.6rem; padding-top: .35rem; border-top: 1px solid var(--line); font-size: clamp(1.5rem, 3vw, 2.05rem); }
.doc-content h2:first-of-type { margin-top: .2rem; border-top: 0; }
.doc-content h2:first-of-type::before { display: none; }
.doc-content h2::before { content: ""; display: block; width: 2.4rem; height: 3px; margin-bottom: .55rem; background: var(--copper); }
.doc-content h3 { margin-top: 1.7rem; font-size: 1.28rem; }
.doc-content h4 { margin-top: 1.3rem; font-size: 1.08rem; }
.doc-content p { margin: .85rem 0; }
.doc-content li + li { margin-top: .28rem; }
.doc-content ul, .doc-content ol { padding-left: 1.35rem; }
.doc-content blockquote, .callout { margin: 1.15rem 0; padding: .85rem 1rem; border-left: 4px solid var(--brand); border-radius: 0 8px 8px 0; background: var(--surface-soft); }
.callout-warning { border-left-color: var(--copper); background: #f8efe6; }
.callout-important { border-left-color: var(--brand-dark); background: #e7efe9; }
.callout-tip { border-left-color: #2f6f4e; background: #f3f7f1; }
.callout-guide { border-left-color: #6d5a3a; background: #f7f1e4; }
.doc-content img { display: block; max-width: 100%; height: auto; margin: 1.3rem auto; border: 1px solid var(--line); border-radius: 8px; background: #fff; }
.doc-content hr { margin: 2rem 0; border: 0; border-top: 1px solid var(--line); }
.doc-content table { display: block; max-width: 100%; overflow-x: auto; border-collapse: collapse; font: .95rem/1.5 var(--sans); }
.doc-content th, .doc-content td { padding: .62rem .72rem; border: 1px solid var(--line); text-align: left; vertical-align: top; }
.doc-content th { background: #f3e2d4; color: #5b2b12; }
.doc-content tr:nth-child(even) td { background: #fbf8f2; }
.doc-content pre, .doc-content code { font-family: var(--mono); }
.doc-content pre { max-width: 100%; overflow: auto; padding: .9rem 1rem; border-radius: 8px; background: var(--code); }
.doc-content code { padding: .08em .28em; border-radius: 4px; background: var(--code); font-size: .9em; }
.doc-content pre code { padding: 0; background: transparent; }
.print-index { display: none; }
@media (max-width: 900px) {
  .topbar { position: static; }
  .topbar-inner { align-items: flex-start; flex-direction: column; }
  .page-layout { grid-template-columns: minmax(0, 1fr); padding: 1rem; }
  .sidebar, .contents { display: block; max-height: none; overflow: visible; }
  .nav-list { max-height: 15rem; }
}
@media (max-width: 540px) {
  .page-layout, .doc-content, .contents { padding-inline: .8rem; }
  .doc-content { border-radius: 8px; }
}
@media (prefers-reduced-motion: reduce) {
  html { scroll-behavior: auto; }
  .progress { display: none; }
}
@media print {
  body { background: #fff; color: #000; font-size: 11pt; }
  .skip-link, .topbar, .sidebar, .progress { display: none !important; }
  .page-layout { display: block; max-width: none; margin: 0; padding: 0; }
  .doc-content { padding: 0; border: 0; border-radius: 0; box-shadow: none; }
  .print-index { display: block; margin: 0 0 1.5rem; break-after: page; }
  .print-index ol { columns: 2; padding-left: 1.2rem; }
  .doc-content h1, .doc-content h2, .doc-content h3 { break-after: avoid; }
  .doc-content img, .doc-content blockquote, .doc-content tr { break-inside: avoid; }
  .doc-content a { color: inherit; }
}
"""

PAGE_SCRIPT = r"""
(() => {
  document.documentElement.classList.add("js");
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
  <meta name="color-scheme" content="light">
  <title>@@TITLE@@</title>
  <style>
@@STYLES@@
  </style>
</head>
<body>
  <a class="skip-link" href="#contenido">Saltar al contenido principal</a>
  <header class="topbar">
    <div class="progress" data-progress aria-hidden="true"></div>
    <div class="topbar-inner">
      <a class="brand" href="manual-usuario.html">EntropIA Lite</a>
@@DOCUMENT_NAV@@
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
        return f'<img {priority} decoding="async" {match.group(1)}>'

    return re.sub(r"<img ([^>]+)>", replace, rendered)


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


def main() -> None:
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

    MANUAL_OUTPUT.write_text(manual_page, encoding="utf-8")
    INVENTORY_OUTPUT.write_text(inventory_page, encoding="utf-8")
    print(f"Generado: {MANUAL_OUTPUT.name} ({len(chapters)} capítulos)")
    print(f"Generado: {INVENTORY_OUTPUT.name} ({len(sections)} secciones)")


if __name__ == "__main__":
    main()
