//! Readable text out of a stored HTML snapshot.
//!
//! Zotero keeps a web page as one file. HTML collapses whitespace, so source
//! line breaks mean nothing; structure lives in block elements. The result is
//! one paragraph per block, separated by a blank line, which is exactly what
//! the paragraph chunker (`chunks.rs`) packs. `<br>` is a soft break kept as a
//! single newline inside its paragraph. Page furniture (scripts, navigation,
//! footers, hidden content) is dropped, so a boilerplate-only page yields an
//! empty string rather than noise.

/// Upper bound on the text one document may contribute: the same bound a
/// PDF page gets, so a pathological page cannot outgrow a page row.
const MAX_TEXT_BYTES: usize =
    crate::bibliography::processing::BIBLIOGRAPHY_PAGE_CONTENT_LIMIT_BYTES;

/// Elements whose content is page furniture or not content at all.
const DROPPED: &[&str] = &[
    "head", "script", "style", "noscript", "nav", "header", "footer", "aside", "form", "template",
    "svg", "iframe",
];

/// Elements that start and end a paragraph. Inline markup never splits.
const BLOCKS: &[&str] = &[
    "p",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "li",
    "blockquote",
    "td",
    "th",
    "pre",
    "figcaption",
    "dd",
    "dt",
    "div",
    "section",
    "article",
    "main",
    "ul",
    "ol",
    "dl",
    "table",
    "tr",
    "thead",
    "tbody",
    "tfoot",
    "caption",
    "figure",
    "details",
    "summary",
    "address",
    "hr",
    "body",
];

/// Marks a `<br>` inside the paragraph being collected; control characters
/// cannot appear in collapsed text, so it never collides with content.
const SOFT_BREAK: char = '\u{1}';

fn is_hidden(node: &dom_query::NodeRef<'_>) -> bool {
    if node.has_attr("hidden") {
        return true;
    }
    if node
        .attr("aria-hidden")
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("true"))
    {
        return true;
    }
    node.attr("style").is_some_and(|style| {
        let style: String = style
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .to_ascii_lowercase();
        style.contains("display:none") || style.contains("visibility:hidden")
    })
}

/// Collapses the collected run into one paragraph: whitespace runs become a
/// single space, soft breaks become single newlines, empty lines vanish.
fn push_paragraph(run: &mut String, out: &mut Vec<String>, size: &mut usize) {
    let lines: Vec<String> = run
        .split(SOFT_BREAK)
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect();
    run.clear();
    if lines.is_empty() {
        return;
    }
    let paragraph = lines.join("\n");
    *size += paragraph.len() + 2;
    out.push(paragraph);
}

/// The text of one HTML document as blank-line separated paragraphs.
pub fn html_to_paragraphs(html: &str) -> String {
    use dom_query::Document;
    let document = Document::from(html);
    let mut paragraphs: Vec<String> = Vec::new();
    let mut size = 0usize;
    let mut run = String::new();
    // Iterative on purpose: a hostile or machine-generated page can nest
    // thousands of levels deep. `true` marks the exit of a block.
    let mut stack = vec![(document.root(), false)];
    while let Some((node, exiting)) = stack.pop() {
        if size > MAX_TEXT_BYTES {
            break;
        }
        if exiting {
            push_paragraph(&mut run, &mut paragraphs, &mut size);
            continue;
        }
        if node.is_text() {
            run.push_str(&node.text());
            continue;
        }
        if !node.is_element() {
            // Document root or fragment: just descend.
            for child in node.children().into_iter().rev() {
                stack.push((child, false));
            }
            continue;
        }
        let Some(name) = node.node_name() else {
            continue;
        };
        let name = name.to_ascii_lowercase();
        if DROPPED.contains(&name.as_str()) || is_hidden(&node) {
            continue;
        }
        if name == "br" {
            run.push(SOFT_BREAK);
            continue;
        }
        if BLOCKS.contains(&name.as_str()) {
            push_paragraph(&mut run, &mut paragraphs, &mut size);
            stack.push((node, true));
        }
        for child in node.children().into_iter().rev() {
            stack.push((child, false));
        }
    }
    push_paragraph(&mut run, &mut paragraphs, &mut size);
    paragraphs.join("\n\n")
}

/// The charset a `<meta>` in the first bytes declares, if the label is one
/// the Encoding Standard knows. UTF-16 declarations are ignored: the bytes
/// are ASCII-compatible here, so the declaration cannot be true.
fn declared_charset(bytes: &[u8]) -> Option<&'static encoding_rs::Encoding> {
    let head: String = bytes
        .iter()
        .take(4096)
        .map(|&byte| char::from(byte).to_ascii_lowercase())
        .collect();
    let at = head.find("charset")? + "charset".len();
    let rest = head[at..].trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let rest = rest.trim_start_matches(['"', '\'']);
    let label: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'))
        .collect();
    let encoding = encoding_rs::Encoding::for_label(label.as_bytes())?;
    if encoding == encoding_rs::UTF_16LE || encoding == encoding_rs::UTF_16BE {
        return None;
    }
    Some(encoding)
}

/// Decodes a stored HTML file to text: BOM first, then a `<meta charset>`
/// declaration in the first bytes, then UTF-8, then Windows-1252 as the
/// last resort for undeclared legacy bytes. Never fails.
pub fn decode_html_bytes(bytes: &[u8]) -> String {
    if let Some((encoding, bom_len)) = encoding_rs::Encoding::for_bom(bytes) {
        return encoding
            .decode_without_bom_handling(&bytes[bom_len..])
            .0
            .into_owned();
    }
    if let Some(encoding) = declared_charset(bytes) {
        return encoding.decode_without_bom_handling(bytes).0.into_owned();
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        Err(_) => encoding_rs::WINDOWS_1252
            .decode_without_bom_handling(bytes)
            .0
            .into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_blocks_and_drops_page_furniture() {
        let html = r#"<!doctype html><html><head><title>T</title>
            <style>p { color: red }</style><script>var x = "no";</script></head>
            <body>
            <nav><a href="/">Home</a> <a href="/x">About</a></nav>
            <header>Site header</header>
            <h1>Un   titular
                largo</h1>
            <p>Primer párrafo con
               saltos de línea   de fuente.</p>
            <ul><li>uno</li><li>dos <b>negrita</b> tres</li></ul>
            <blockquote>Cita textual.</blockquote>
            <table><tr><th>Col</th><td>Celda</td></tr></table>
            <p>Línea A<br>Línea B</p>
            <aside>Publicidad</aside>
            <form><p>Suscribite</p></form>
            <noscript>Activá JavaScript</noscript>
            <template><p>plantilla</p></template>
            <footer>Pie</footer>
            </body></html>"#;
        assert_eq!(
            html_to_paragraphs(html),
            "Un titular largo\n\n\
             Primer párrafo con saltos de línea de fuente.\n\n\
             uno\n\n\
             dos negrita tres\n\n\
             Cita textual.\n\n\
             Col\n\n\
             Celda\n\n\
             Línea A\nLínea B"
        );
    }

    #[test]
    fn bare_text_and_divs_still_form_paragraphs() {
        let html = "<body>Suelto al inicio<div>Dentro de div<div>anidado</div></div>cola</body>";
        assert_eq!(
            html_to_paragraphs(html),
            "Suelto al inicio\n\nDentro de div\n\nanidado\n\ncola"
        );
    }

    #[test]
    fn inline_markup_does_not_split_and_entities_decode() {
        let html =
            "<p>Tom &amp; Jerry &mdash; <em>caf&eacute;</em>&nbsp;&#8211; <a href=x>enlace</a>.</p>";
        assert_eq!(
            html_to_paragraphs(html),
            "Tom & Jerry \u{2014} café \u{2013} enlace."
        );
    }

    #[test]
    fn hidden_elements_are_dropped() {
        let html = r#"<p>visible</p><p hidden>oculto</p>
            <div aria-hidden="true"><p>aria</p></div>
            <p style="display: none">css</p>
            <p style="color:red;visibility:hidden">css2</p><p>fin</p>"#;
        assert_eq!(html_to_paragraphs(html), "visible\n\nfin");
    }

    #[test]
    fn boilerplate_only_and_empty_pages_yield_nothing() {
        assert_eq!(html_to_paragraphs(""), "");
        assert_eq!(html_to_paragraphs("   \n  "), "");
        assert_eq!(
            html_to_paragraphs(
                "<html><body><nav>x</nav><script>y</script><footer>z</footer></body></html>"
            ),
            ""
        );
        assert_eq!(html_to_paragraphs("<p> <br> &nbsp; </p>"), "");
    }

    #[test]
    fn broken_markup_does_not_panic_and_deep_nesting_is_bounded() {
        assert_eq!(html_to_paragraphs("<p>uno<p>dos</b></i><div"), "uno\n\ndos");
        let deep = format!("{}texto{}", "<div>".repeat(3_000), "</div>".repeat(3_000));
        let _ = html_to_paragraphs(&deep);
    }

    #[test]
    fn decoding_honours_bom_meta_charset_utf8_and_falls_back_to_latin1() {
        let mut bom = vec![0xEF, 0xBB, 0xBF];
        bom.extend_from_slice("<p>ñandú</p>".as_bytes());
        assert_eq!(decode_html_bytes(&bom), "<p>ñandú</p>");

        let utf16: Vec<u8> = [0xFF, 0xFE]
            .into_iter()
            .chain("<p>ñ</p>".encode_utf16().flat_map(u16::to_le_bytes))
            .collect();
        assert_eq!(decode_html_bytes(&utf16), "<p>ñ</p>");

        let mut declared = b"<meta charset=\"iso-8859-1\"><p>ca".to_vec();
        declared.push(0xF1);
        declared.extend_from_slice(b"on</p>");
        assert_eq!(
            decode_html_bytes(&declared),
            "<meta charset=\"iso-8859-1\"><p>cañon</p>"
        );

        let http_equiv = b"<meta http-equiv=\"Content-Type\" content=\"text/html; charset=windows-1252\"><p>\x93hola\x94</p>";
        assert!(decode_html_bytes(http_equiv).contains("\u{201C}hola\u{201D}"));

        assert_eq!(decode_html_bytes("<p>año</p>".as_bytes()), "<p>año</p>");

        let undeclared_latin1 = b"<p>a\xF1o</p>";
        assert_eq!(decode_html_bytes(undeclared_latin1), "<p>año</p>");
    }
}
