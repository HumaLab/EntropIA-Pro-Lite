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
  --canvas: #f3f2eb;
  --surface: #fffefa;
  --surface-soft: #e8eee8;
  --ink: #1d2b28;
  --muted: #4d5d59;
  --line: #c7d1ca;
  --brand: #20574b;
  --brand-dark: #164237;
  --focus: #974900;
  --code: #eeece3;
}
* { box-sizing: border-box; }
html { scroll-behavior: smooth; }
body {
  margin: 0;
  background: var(--canvas);
  color: var(--ink);
  font: 1rem/1.7 Georgia, "Times New Roman", serif;
}
a { color: var(--brand-dark); text-decoration-thickness: .08em; text-underline-offset: .16em; }
a:hover { color: #102f27; }
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
  border-radius: .45rem;
  background: var(--brand-dark);
  color: white;
  font: 600 .95rem/1.2 "Segoe UI", Arial, sans-serif;
}
.skip-link:focus { transform: translateY(0); }
.topbar {
  position: sticky;
  z-index: 3;
  top: 0;
  border-bottom: 1px solid var(--line);
  background: var(--surface);
}
.topbar-inner {
  max-width: 1440px;
  min-height: 4.2rem;
  margin: 0 auto;
  padding: .7rem 1.4rem;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 1rem;
}
.brand {
  color: var(--ink);
  font: 700 1.12rem/1.2 "Segoe UI", Arial, sans-serif;
  letter-spacing: -.02em;
  text-decoration: none;
  white-space: nowrap;
}
.document-nav {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: .3rem;
  font: 600 .92rem/1.3 "Segoe UI", Arial, sans-serif;
}
.document-nav a {
  padding: .58rem .78rem;
  border-radius: 999px;
  color: var(--brand-dark);
  text-decoration: none;
}
.document-nav a:hover { background: var(--surface-soft); }
.document-nav a[aria-current="page"] { background: var(--brand-dark); color: #fff; }
.page-layout {
  display: grid;
  grid-template-columns: minmax(15rem, 18.5rem) minmax(0, 54rem);
  align-items: start;
  justify-content: center;
  gap: 2rem;
  max-width: 1440px;
  margin: 0 auto;
  padding: 1.5rem;
}
.sidebar {
  position: sticky;
  top: 5.5rem;
  max-height: calc(100vh - 6.5rem);
  overflow: auto;
  font-family: "Segoe UI", Arial, sans-serif;
}
.contents {
  padding: 1rem;
  border: 1px solid var(--line);
  border-radius: 14px;
  background: var(--surface);
  box-shadow: 0 5px 18px rgb(29 43 40 / 5%);
}
.contents summary {
  cursor: pointer;
  color: var(--ink);
  font-size: 1rem;
  font-weight: 700;
}
.filter-control { display: none; margin-top: 1rem; }
.js .filter-control { display: block; }
.filter-control label {
  display: block;
  margin-bottom: .35rem;
  color: var(--ink);
  font-size: .9rem;
  font-weight: 650;
}
.filter-control input {
  width: 100%;
  min-height: 2.75rem;
  padding: .6rem .7rem;
  border: 1px solid #879790;
  border-radius: 7px;
  background: #fff;
  color: var(--ink);
  font: 400 .95rem/1.3 "Segoe UI", Arial, sans-serif;
}
.nav-list {
  max-height: calc(100vh - 18rem);
  overflow: auto;
  margin: 1rem 0 0;
  padding: 0;
  list-style: none;
}
.nav-list li + li { margin-top: .14rem; }
.nav-list li[hidden] { display: none; }
.nav-list a {
  display: block;
  padding: .4rem .5rem;
  border-radius: 6px;
  color: var(--brand-dark);
  font-size: .9rem;
  line-height: 1.38;
  text-decoration: none;
}
.nav-list a:hover { background: var(--surface-soft); }
.filter-empty {
  margin: .8rem 0 0;
  color: var(--muted);
  font-size: .9rem;
}
.other-document {
  display: block;
  margin-top: 1rem;
  padding-top: .85rem;
  border-top: 1px solid var(--line);
  font-size: .9rem;
  font-weight: 650;
}
.doc-content {
  min-width: 0;
  padding: clamp(1.35rem, 4vw, 3.2rem);
  border: 1px solid var(--line);
  border-radius: 16px;
  background: var(--surface);
  box-shadow: 0 8px 28px rgb(29 43 40 / 6%);
  overflow-wrap: anywhere;
}
.doc-content h1, .doc-content h2, .doc-content h3, .doc-content h4 {
  color: #1a302b;
  line-height: 1.2;
  scroll-margin-top: 6rem;
}
.doc-content > h1 { margin: 0 0 .25em; font-size: clamp(2rem, 4vw, 3rem); letter-spacing: -.035em; }
.doc-content h2 { margin-top: 2.35rem; font-size: clamp(1.45rem, 3vw, 2rem); }
.doc-content h3 { margin-top: 1.8rem; font-size: 1.28rem; }
.doc-content h4 { margin-top: 1.35rem; font-size: 1.08rem; }
.doc-content p { margin: .9rem 0; }
.doc-content li + li { margin-top: .35rem; }
.doc-content ul, .doc-content ol { padding-left: 1.45rem; }
.doc-content blockquote {
  margin: 1.2rem 0;
  padding: .2rem 1rem;
  border-left: 4px solid var(--brand);
  background: var(--surface-soft);
  color: #263a35;
}
.doc-content img {
  display: block;
  max-width: 100%;
  height: auto;
  margin: 1.35rem auto;
  border: 1px solid var(--line);
  border-radius: 10px;
  background: #fff;
}
.doc-content hr { margin: 2.2rem 0; border: 0; border-top: 1px solid var(--line); }
.doc-content table {
  display: block;
  max-width: 100%;
  overflow-x: auto;
  border-collapse: collapse;
  font-size: .94rem;
  line-height: 1.55;
}
.doc-content th, .doc-content td {
  padding: .65rem .75rem;
  border: 1px solid var(--line);
  text-align: left;
  vertical-align: top;
}
.doc-content th { background: var(--surface-soft); font-family: "Segoe UI", Arial, sans-serif; }
.doc-content pre {
  max-width: 100%;
  overflow: auto;
  padding: .9rem 1rem;
  border: 1px solid var(--line);
  border-radius: 8px;
  background: var(--code);
  line-height: 1.5;
}
.doc-content code { padding: .08em .25em; border-radius: 4px; background: var(--code); font: .92em/1.5 Consolas, "Courier New", monospace; }
.doc-content pre code { padding: 0; background: transparent; }
.doc-content strong { color: #172e29; }
@media (max-width: 900px) {
  .topbar { position: static; }
  .topbar-inner { align-items: flex-start; flex-direction: column; padding: .8rem 1rem; }
  .page-layout { grid-template-columns: minmax(0, 1fr); gap: 1rem; max-width: 58rem; padding: 1rem; }
  .sidebar { position: static; max-height: none; }
  .nav-list { max-height: 15rem; }
}
@media (max-width: 540px) {
  .page-layout { padding: .65rem; }
  .doc-content { padding: 1.1rem; border-radius: 11px; }
  .document-nav { gap: .1rem; }
  .document-nav a { padding: .5rem .6rem; font-size: .88rem; }
  .contents { padding: .85rem; }
  .doc-content table { font-size: .88rem; }
}
@media (prefers-reduced-motion: reduce) {
  html { scroll-behavior: auto; }
  *, *::before, *::after { animation-duration: .01ms !important; animation-iteration-count: 1 !important; transition-duration: .01ms !important; }
}
@media print {
  body { background: #fff; color: #000; font-size: 11pt; }
  .skip-link, .topbar, .sidebar { display: none !important; }
  .page-layout { display: block; max-width: none; margin: 0; padding: 0; }
  .doc-content { padding: 0; border: 0; border-radius: 0; box-shadow: none; }
  .doc-content h1, .doc-content h2, .doc-content h3 { break-after: avoid; }
  .doc-content img, .doc-content blockquote, .doc-content tr { break-inside: avoid; }
  .doc-content a { color: inherit; }
}
"""

FILTER_SCRIPT = r"""
(() => {
  document.documentElement.classList.add("js");
  const input = document.querySelector("[data-nav-filter]");
  const items = Array.from(document.querySelectorAll("[data-nav-item]"));
  const empty = document.querySelector("[data-no-results]");
  if (!input || !items.length) return;

  const normalize = (text) => text.normalize("NFD").replace(/[\u0300-\u036f]/g, "").toLocaleLowerCase("es");
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
  filterItems();
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
@@DOCUMENT_BODY@@
    </main>
  </div>
  <script>
@@FILTER_SCRIPT@@
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
        "@@DOCUMENT_BODY@@": body,
        "@@FILTER_SCRIPT@@": FILTER_SCRIPT,
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
    manual_body = rewrite_document_links(parser.render(manual_html_source))

    inventory_body, sections = inventory_body_and_items(parser, inventory)
    inventory_body = rewrite_document_links(inventory_body)

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
