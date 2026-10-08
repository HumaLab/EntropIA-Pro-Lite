//! OCR markup to readable Markdown text.
//!
//! GLM-OCR answers statistical tables as HTML (`<table class="table
//! table-bordered">…</table>`) and the surrounding content as plain lines.
//! Stored as-is that markup poisons every downstream reader: the garbled-text
//! detector ([`crate::ocr::pdf::is_garbled_text`]) sees `td/tr/th` letters
//! with almost no vowels and grades the page `empty` (so every sync
//! re-demands the extraction and pays for OCR again), the chunker indexes the
//! tag soup, and the Biblioteca text tab shows raw tags.
//!
//! [`ocr_markup_to_text`] converts once, deterministically: HTML tables
//! become GitHub-flavoured Markdown pipe tables (first row with `<th>`, or
//! the first row, is the header; a colspan cell keeps its text in the first
//! cell and pads the span with empty cells), every other tag is stripped
//! keeping its text, and text without markup leaves this module
//! byte-identical — plain text and existing Markdown pass through untouched.
//!
//! Parsing reuses `dom_query`, the parser behind
//! [`crate::bibliography::html_text`], so nested or malformed markup
//! degrades to text instead of panicking and entities decode on the way out.

use dom_query::Document;

/// A tag-like region starts at `<` followed by a name, `/`, `!` or `?`, and
/// closes at a later `>`. "a < b" and "2 < 3 > 1" are not markup: they never
/// enter the parser and leave this module unchanged.
fn has_html_markup(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut from = 0usize;
    while let Some(offset) = text[from..].find('<') {
        let at = from + offset;
        let start = at + 1;
        let tag_like = match bytes.get(start) {
            Some(&byte) => byte.is_ascii_alphabetic() || matches!(byte, b'/' | b'!' | b'?'),
            None => false,
        };
        if tag_like && text[start..].contains('>') {
            return true;
        }
        from = start;
    }
    false
}

/// Whitespace-collapsed cell text with pipe-table syntax escaped.
fn cell_text(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('|', "\\|")
}

/// Block tags flush the collected inline run into its own output block, so
/// their boundaries become the blank lines paragraphs need. `br` is handled
/// apart: a soft break, a single newline inside its block.
const BLOCK_TAGS: &[&str] = &[
    "p",
    "div",
    "section",
    "article",
    "main",
    "header",
    "footer",
    "aside",
    "address",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "ul",
    "ol",
    "li",
    "blockquote",
    "pre",
    "figure",
    "figcaption",
    "caption",
    "dl",
    "dt",
    "dd",
    "details",
    "summary",
    "form",
    "hr",
    "table",
    "thead",
    "tbody",
    "tfoot",
    "tr",
    "td",
    "th",
];

/// `colspan` padding bound, the same bound the renderer's sanitizer copies:
/// a hostile or broken span cannot flood the output with empty cells.
const MAX_COL_SPAN: usize = 100;

/// Appends `text` to the inline run with HTML whitespace semantics: a
/// whitespace run collapses to one space — or one newline when the source
/// had one, so the plain footnote lines after a GLM-OCR table keep their
/// shape instead of melting into one paragraph — and a separator at the
/// run's start or after whitespace is dropped.
fn push_collapsed(run: &mut String, text: &str) {
    let mut in_whitespace = false;
    let mut separator_newline = false;
    for c in text.chars() {
        if c.is_whitespace() {
            if !in_whitespace {
                in_whitespace = true;
                separator_newline = c == '\n';
            } else if c == '\n' {
                separator_newline = true;
            }
            continue;
        }
        if in_whitespace {
            in_whitespace = false;
            if !run.is_empty() && !run.ends_with(char::is_whitespace) {
                run.push(if separator_newline { '\n' } else { ' ' });
            }
        }
        run.push(c);
    }
    // Trailing whitespace lands now: it separates whatever the next node
    // contributes, and that node's own leading whitespace collapses against
    // it.
    if in_whitespace && !run.is_empty() && !run.ends_with(char::is_whitespace) {
        run.push(if separator_newline { '\n' } else { ' ' });
    }
}

/// The collected run becomes one output block (trimmed; interior soft
/// newlines stay), skipped when empty.
fn flush_block(run: &mut String, blocks: &mut Vec<String>) {
    let block = run.trim();
    if !block.is_empty() {
        blocks.push(block.to_string());
    }
    run.clear();
}

/// One table cell: its collapsed text and the columns it spans.
struct MarkupCell {
    text: String,
    span: usize,
    header: bool,
}

/// The cells of one `<tr>`, in order. Cells are `th`/`td` found without
/// descending into them (their text flattens wholesale, nested tables
/// included) and without ever entering a nested table.
fn table_row_cells(row: &dom_query::NodeRef<'_>) -> Vec<MarkupCell> {
    let mut cells = Vec::new();
    let mut stack: Vec<dom_query::NodeRef<'_>> = row.children().into_iter().collect();
    stack.reverse();
    while let Some(node) = stack.pop() {
        if !node.is_element() {
            continue;
        }
        let name = node.node_name().unwrap_or_default().to_ascii_lowercase();
        match name.as_str() {
            "td" | "th" => {
                let span = node
                    .attr("colspan")
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .unwrap_or(1)
                    .clamp(1, MAX_COL_SPAN);
                cells.push(MarkupCell {
                    text: cell_text(&node.text()),
                    span,
                    header: name == "th",
                });
            }
            "table" => {}
            _ => {
                for child in node.children().into_iter().rev() {
                    stack.push(child);
                }
            }
        }
    }
    cells
}

/// Every `<tr>` of one table in document order, without ever crossing into
/// a nested table: its rows stay its own.
fn table_rows(table: &dom_query::NodeRef<'_>) -> Vec<Vec<MarkupCell>> {
    let mut rows = Vec::new();
    let mut stack: Vec<dom_query::NodeRef<'_>> = table.children().into_iter().collect();
    stack.reverse();
    while let Some(node) = stack.pop() {
        if !node.is_element() {
            continue;
        }
        let name = node.node_name().unwrap_or_default().to_ascii_lowercase();
        match name.as_str() {
            "tr" => rows.push(table_row_cells(&node)),
            "table" => {}
            _ => {
                for child in node.children().into_iter().rev() {
                    stack.push(child);
                }
            }
        }
    }
    rows
}

/// One pipe-table line: the cells padded to `columns`, one pipe apart.
fn pipe_row(cells: &[String], columns: usize) -> String {
    let mut line = String::from("|");
    for index in 0..columns {
        line.push(' ');
        line.push_str(cells.get(index).map(String::as_str).unwrap_or(""));
        line.push(' ');
        line.push('|');
    }
    line
}

/// One `<table>` as GitHub-flavoured Markdown: the first row holding a
/// `th` cell (or simply the first row) is the header, a colspan cell keeps
/// its text in its first column and pads the rest empty, and captions read
/// as their own block above the table. Emits nothing for a table with no
/// rows.
fn push_markdown_table(table: &dom_query::NodeRef<'_>, blocks: &mut Vec<String>) {
    for child in table.children() {
        if child.is_element()
            && child
                .node_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("caption"))
        {
            let caption = cell_text(&child.text());
            if !caption.is_empty() {
                blocks.push(caption);
            }
        }
    }
    let rows = table_rows(table);
    let Some(header_index) = rows
        .iter()
        .position(|row| row.iter().any(|cell| cell.header))
        .or_else(|| (!rows.is_empty()).then_some(0))
    else {
        return;
    };
    let columns = rows
        .iter()
        .map(|row| row.iter().map(|cell| cell.span).sum::<usize>())
        .max()
        .unwrap_or(0);
    if columns == 0 {
        return;
    }
    let mut lines = Vec::with_capacity(rows.len() + 1);
    let mut body = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        // A colspan cell keeps its text in the first column of its span and
        // pads the rest with empty cells, so every row has `columns` cells.
        let mut expanded: Vec<String> = Vec::with_capacity(columns);
        for cell in row {
            expanded.push(cell.text.clone());
            for _ in 1..cell.span {
                expanded.push(String::new());
            }
        }
        let line = pipe_row(&expanded, columns);
        if index == header_index {
            lines.push(line);
            lines.push(format!("| {} |", vec!["---"; columns].join(" | ")));
        } else {
            body.push(line);
        }
    }
    lines.extend(body);
    blocks.push(lines.join("\n"));
}

/// The text a reader gets from one OCR answer or stored page: HTML tables as
/// GitHub-flavoured Markdown pipe tables, every other tag stripped to its
/// text (block tags become newlines), plain text and Markdown unchanged.
/// Deterministic and idempotent — the output holds no tags, so converting
/// twice changes nothing.
pub fn ocr_markup_to_text(text: &str) -> String {
    if !has_html_markup(text) {
        return text.to_string();
    }
    let document = Document::from(text);
    let mut blocks: Vec<String> = Vec::new();
    let mut run = String::new();
    // Iterative on purpose: a hostile page can nest thousands deep. `true`
    // marks the exit of a block element.
    let mut stack = vec![(document.root(), false)];
    while let Some((node, exiting)) = stack.pop() {
        if exiting {
            flush_block(&mut run, &mut blocks);
            continue;
        }
        if node.is_text() {
            push_collapsed(&mut run, &node.text());
            continue;
        }
        if !node.is_element() {
            for child in node.children().into_iter().rev() {
                stack.push((child, false));
            }
            continue;
        }
        let name = node.node_name().unwrap_or_default().to_ascii_lowercase();
        match name.as_str() {
            "table" => {
                flush_block(&mut run, &mut blocks);
                push_markdown_table(&node, &mut blocks);
                continue;
            }
            "br" => {
                run.push('\n');
                continue;
            }
            _ if BLOCK_TAGS.contains(&name.as_str()) => {
                flush_block(&mut run, &mut blocks);
                stack.push((node, true));
            }
            _ => {}
        }
        for child in node.children().into_iter().rev() {
            stack.push((child, false));
        }
    }
    flush_block(&mut run, &mut blocks);
    blocks.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::ocr_markup_to_text;

    /// The owner's real shape: a GLM-OCR statistical table as one line of
    /// HTML (thead with `th` cells, a colspan caption row, numeric rows)
    /// followed by plain footnote lines.
    const OWNER_HTML: &str = concat!(
        r#"<table class="table table-bordered"><thead><tr><th>Mio</th><th>Enero</th><th>Febrero</th><th>Marzo</th><th>Abril</th><th>Mayo</th><th>Junio</th></tr></thead><tbody><tr><td colspan="7">- en centavos de dólar norteamericano -</td></tr><tr><td>1926</td><td>20,5</td><td>21,0</td><td>21,5</td><td>22,0</td><td>22,5</td><td>23,0</td></tr><tr><td>1927</td><td>24,5</td><td>25,0</td><td>25,5</td><td>26,0</td><td>26,5</td><td>27,0</td></tr></tbody></table>"#,
        "\nFuente: Boletín Mensual de Estadística, Buenos Aires.",
        "\na) Cifras correspondientes a los primeros quince días del mes."
    );

    /// The Markdown the owner's page must become: one pipe table (header
    /// row and delimiter, the colspan row padded to seven columns) and the
    /// footnote lines kept as their own block.
    const OWNER_MARKDOWN: &str = concat!(
        "| Mio | Enero | Febrero | Marzo | Abril | Mayo | Junio |\n",
        "| --- | --- | --- | --- | --- | --- | --- |\n",
        "| - en centavos de dólar norteamericano - |  |  |  |  |  |  |\n",
        "| 1926 | 20,5 | 21,0 | 21,5 | 22,0 | 22,5 | 23,0 |\n",
        "| 1927 | 24,5 | 25,0 | 25,5 | 26,0 | 26,5 | 27,0 |\n",
        "\n",
        "Fuente: Boletín Mensual de Estadística, Buenos Aires.\n",
        "a) Cifras correspondientes a los primeros quince días del mes."
    );

    /// The same table without its footnote lines: what a long statistical
    /// page mostly is — one line of tags with almost no word separation.
    /// That is the shape the letter statistics read as garbled.
    const OWNER_TABLE_ONLY_HTML: &str = r#"<table class="table table-bordered"><thead><tr><th>Mio</th><th>Enero</th><th>Febrero</th><th>Marzo</th><th>Abril</th><th>Mayo</th><th>Junio</th></tr></thead><tbody><tr><td colspan="7">- en centavos de dólar norteamericano -</td></tr><tr><td>1926</td><td>20,5</td><td>21,0</td><td>21,5</td><td>22,0</td><td>22,5</td><td>23,0</td></tr><tr><td>1927</td><td>24,5</td><td>25,0</td><td>25,5</td><td>26,0</td><td>26,5</td><td>27,0</td></tr></tbody></table>"#;

    #[test]
    fn the_raw_tag_soup_the_owner_stored_reads_as_garbled() {
        // The bug premise: one line of table tags has no word separation,
        // so the detector grades the page garbled/empty and every sync
        // re-demanded the extraction and re-OCRed it.
        assert!(
            crate::ocr::pdf::is_garbled_text(OWNER_TABLE_ONLY_HTML),
            "the raw table page must reproduce the reported garbled verdict"
        );
    }

    #[test]
    fn owner_html_table_becomes_a_markdown_pipe_table_with_its_footnotes() {
        assert_eq!(ocr_markup_to_text(OWNER_HTML), OWNER_MARKDOWN);
    }

    #[test]
    fn non_table_html_is_stripped_to_its_text_with_newlines_at_blocks() {
        assert_eq!(
            ocr_markup_to_text(
                "<p>Un <b>titular</b> largo</p><div>Segundo<br>bloque &amp; tal</div><span>suelt</span>o"
            ),
            "Un titular largo\n\nSegundo\nbloque & tal\n\nsuelto"
        );
    }

    #[test]
    fn plain_text_and_existing_markdown_pass_through_untouched() {
        let plain = "1926 20,5 20,8\n- en centavos de dólar norteamericano -\n\nFuente: boletín.";
        assert_eq!(ocr_markup_to_text(plain), plain);
        let markdown = "| a | b |\n| --- | --- |\n| 1 | 2 |\n\ntexto **con** formato";
        assert_eq!(ocr_markup_to_text(markdown), markdown);
        // A comparison that only looks like a tag is not markup either.
        assert_eq!(ocr_markup_to_text("x < 5 y 3 > 2"), "x < 5 y 3 > 2");
    }

    #[test]
    fn nested_and_malformed_markup_degrades_to_text_without_panicking() {
        // Unclosed cells and rows, an attribute without quotes, a nested
        // table, a stray close: the parser repairs what it can and every
        // cell keeps its text.
        let messy = "<table><tr><td>uno<td>dos<tr><td colspan=2>caf&eacute; &amp; t&eacute;<table><tr><td>anidada</td></tr></table></table></b>cola";
        let out = ocr_markup_to_text(messy);
        assert!(out.contains("| uno | dos |"), "{out}");
        assert!(out.contains("| --- | --- |"), "{out}");
        assert!(out.contains("café & té"), "{out}");
        assert!(out.contains("anidada"), "{out}");
        assert!(out.contains("cola"), "{out}");
        assert!(!out.contains('<'), "{out}");
    }

    #[test]
    fn cell_pipes_are_escaped_and_cells_stay_on_one_line() {
        let out = ocr_markup_to_text(
            "<table><tr><th>a|b</th><th>dos</th></tr><tr><td>uno\ndos</td><td>tres</td></tr></table>",
        );
        assert_eq!(out, "| a\\|b | dos |\n| --- | --- |\n| uno dos | tres |");
    }

    #[test]
    fn conversion_is_idempotent() {
        let once = ocr_markup_to_text(OWNER_HTML);
        assert_eq!(ocr_markup_to_text(&once), once);
    }

    #[test]
    fn the_converted_owner_page_is_not_garbled_and_never_empty() {
        // What a reader gets — the converted Markdown — is real Spanish
        // text and must never grade `empty`: that verdict is what made every
        // sync re-demand the extraction and re-OCR the page.
        let converted = ocr_markup_to_text(OWNER_HTML);
        assert!(
            !crate::ocr::pdf::is_garbled_text(&converted),
            "converted text must not read garbled: {converted}"
        );
        assert!(
            !crate::ocr::pdf::is_garbled_text(&ocr_markup_to_text(OWNER_TABLE_ONLY_HTML)),
            "the converted table-only page must not read garbled"
        );
        assert_ne!(
            crate::bibliography::processing::extraction_quality(OWNER_TABLE_ONLY_HTML),
            "empty",
            "an HTML table page must never grade empty"
        );
        assert_ne!(
            crate::bibliography::processing::extraction_quality(OWNER_HTML),
            "empty",
            "an HTML table page must never grade empty"
        );
    }
}
