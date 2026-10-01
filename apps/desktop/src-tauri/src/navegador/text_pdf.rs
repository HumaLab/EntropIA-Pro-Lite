//! Rendering captured text into a PDF.
//!
//! A page or selection capture has no file the corpus can open, so a copy of it
//! into a collection is a PDF made from its text: a provenance header followed
//! by the body. The PDF is a derivative, a plain text rendition with no layout
//! or images of the original page.
//!
//! The PDF uses the three built-in Helvetica faces with `WinAnsiEncoding`, so no
//! font is embedded and the text stays a real text layer (the corpus extracts it
//! natively, with no OCR). That encoding is Windows-1252: it covers every
//! character of Spanish and English, the other Western European languages and
//! typographic punctuation. Text is normalised to it first: accents written as a
//! base letter plus a combining mark are composed, typographic look-alikes are
//! mapped (non-breaking and thin spaces, other hyphens and quotes, ligatures,
//! a few arrows and comparisons) and zero-width characters are dropped. Anything
//! else (other scripts, emoji, Latin letters outside Windows-1252 such as `ł`)
//! becomes `?`, and the PDF's header says how many characters were replaced, so
//! the loss is never silent.

use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Document as Pdf, Object, Stream};

/// A4 in PDF points.
const PAGE_WIDTH: f32 = 595.0;
const PAGE_HEIGHT: f32 = 842.0;
const MARGIN_X: f32 = 56.0;
const MARGIN_TOP: f32 = 60.0;
const MARGIN_BOTTOM: f32 = 60.0;

/// What a character that cannot be written stands for.
pub const PLACEHOLDER: u8 = b'?';

/// One block of the body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// Running text: blank lines separate paragraphs, line breaks are kept.
    Text(String),
    /// Lighter text that surrounds a quote.
    Context(String),
    /// A quotation, set apart with a bar and an oblique face.
    Quote(String),
}

/// What to render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub title: String,
    /// Label and value, one line each (the value wraps).
    pub meta: Vec<(String, String)>,
    pub body: Vec<Block>,
}

/// A rendered PDF.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    pub bytes: Vec<u8>,
    pub pages: usize,
    /// Characters that could not be written and became [`PLACEHOLDER`].
    pub replaced: usize,
}

/// What one character becomes.
enum Mapped {
    Byte(u8),
    Text(&'static [u8]),
    Drop,
    Replace,
}

/// Letters that take a combining mark, as `(mark, bases, composed)`; every
/// composed letter is in Latin-1, so its Windows-1252 byte is its code point.
const COMPOSITIONS: [(char, &str, &str); 6] = [
    ('\u{300}', "AEIOUaeiou", "ÀÈÌÒÙàèìòù"),
    ('\u{301}', "AEIOUYaeiouy", "ÁÉÍÓÚÝáéíóúý"),
    ('\u{302}', "AEIOUaeiou", "ÂÊÎÔÛâêîôû"),
    ('\u{303}', "ANOano", "ÃÑÕãñõ"),
    ('\u{308}', "AEIOUaeiouy", "ÄËÏÖÜäëïöüÿ"),
    ('\u{327}', "Cc", "Çç"),
];

fn compose(base: char, mark: char) -> Option<u8> {
    let (_, bases, composed) = COMPOSITIONS.iter().find(|(m, _, _)| *m == mark)?;
    let at = bases.chars().position(|b| b == base)?;
    composed.chars().nth(at).map(|c| c as u32 as u8)
}

/// The Windows-1252 byte of the characters it holds outside Latin-1.
fn cp1252_extra(c: char) -> Option<u8> {
    Some(match c {
        '€' => 0x80,
        '‚' => 0x82,
        'ƒ' => 0x83,
        '„' => 0x84,
        '…' => 0x85,
        '†' => 0x86,
        '‡' => 0x87,
        'ˆ' => 0x88,
        '‰' => 0x89,
        'Š' => 0x8A,
        '‹' => 0x8B,
        'Œ' => 0x8C,
        'Ž' => 0x8E,
        '‘' => 0x91,
        '’' => 0x92,
        '“' => 0x93,
        '”' => 0x94,
        '•' => 0x95,
        '–' => 0x96,
        '—' => 0x97,
        '˜' => 0x98,
        '™' => 0x99,
        'š' => 0x9A,
        '›' => 0x9B,
        'œ' => 0x9C,
        'ž' => 0x9E,
        'Ÿ' => 0x9F,
        _ => return None,
    })
}

fn map_char(c: char) -> Mapped {
    match c {
        '\n' | '\u{2028}' | '\u{2029}' => Mapped::Byte(b'\n'),
        '\t' => Mapped::Byte(b' '),
        '\r' => Mapped::Drop,
        '\u{20}'..='\u{7e}' => Mapped::Byte(c as u8),
        // C0/C1 controls and DEL.
        '\u{0}'..='\u{1f}' | '\u{7f}'..='\u{9f}' => Mapped::Drop,
        '\u{a0}' => Mapped::Byte(b' '),
        '\u{ad}' => Mapped::Drop,
        '\u{a1}'..='\u{ff}' => Mapped::Byte(c as u32 as u8),
        '\u{2000}'..='\u{200a}' | '\u{202f}' | '\u{205f}' | '\u{3000}' => Mapped::Byte(b' '),
        '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}' | '\u{feff}' => Mapped::Drop,
        // Combining marks that composed with nothing: keep the letter, lose the mark.
        '\u{300}'..='\u{36f}' => Mapped::Drop,
        '\u{2010}'..='\u{2012}' | '\u{2043}' | '\u{2212}' => Mapped::Byte(b'-'),
        '\u{2015}' => Mapped::Byte(0x97),
        '\u{201b}' => Mapped::Byte(0x91),
        '\u{201f}' => Mapped::Byte(0x93),
        '\u{2032}' => Mapped::Byte(b'\''),
        '\u{2033}' => Mapped::Byte(b'"'),
        '\u{2044}' => Mapped::Byte(b'/'),
        '\u{fb00}' => Mapped::Text(b"ff"),
        '\u{fb01}' => Mapped::Text(b"fi"),
        '\u{fb02}' => Mapped::Text(b"fl"),
        '\u{fb03}' => Mapped::Text(b"ffi"),
        '\u{fb04}' => Mapped::Text(b"ffl"),
        '\u{2190}' => Mapped::Text(b"<-"),
        '\u{2192}' => Mapped::Text(b"->"),
        '\u{2264}' => Mapped::Text(b"<="),
        '\u{2265}' => Mapped::Text(b">="),
        '\u{2260}' => Mapped::Text(b"!="),
        '\u{2248}' => Mapped::Byte(b'~'),
        _ => match cp1252_extra(c) {
            Some(byte) => Mapped::Byte(byte),
            None => Mapped::Replace,
        },
    }
}

/// `text` as Windows-1252 bytes, with a count of the characters that had no
/// place in it. Line breaks stay `\n`; tabs become spaces; other control
/// characters are dropped.
pub fn to_win_ansi(text: &str) -> (Vec<u8>, usize) {
    let mut out = Vec::with_capacity(text.len());
    let mut replaced = 0;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if let Some(composed) = chars.peek().and_then(|mark| compose(c, *mark)) {
            out.push(composed);
            chars.next();
            continue;
        }
        match map_char(c) {
            Mapped::Byte(byte) => out.push(byte),
            Mapped::Text(bytes) => out.extend_from_slice(bytes),
            Mapped::Drop => {}
            Mapped::Replace => {
                out.push(PLACEHOLDER);
                replaced += 1;
            }
        }
    }
    (out, replaced)
}

// --- metrics ---------------------------------------------------------------

/// Helvetica advance widths (thousandths of an em) for bytes 32..=126.
const ASCII_WIDTHS: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278,
    278, // space .. /
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, // 0 .. ?
    1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, // @ .. O
    667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556, // P .. _
    333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, // ` .. o
    556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584, // p .. ~
];

/// Advance width of a Windows-1252 byte in Helvetica. Accented letters take the
/// width of their base letter.
fn width_of(byte: u8) -> u16 {
    match byte {
        32..=126 => ASCII_WIDTHS[usize::from(byte) - 32],
        0x80 | 0x83 | 0x86 | 0x87 | 0x96 => 556,
        0x82 | 0x91 | 0x92 => 222,
        0x84 | 0x88 | 0x8B | 0x93 | 0x94 | 0x98 | 0x9B => 333,
        0x85 | 0x89 | 0x8C | 0x97 | 0x99 => 1000,
        0x8A | 0x9F => 667,
        0x8E => 611,
        0x95 => 350,
        0x9A | 0x9E => 500,
        0x9C => 944,
        0xA0 => 278,
        0xA1 | 0xA8 | 0xAD | 0xAF | 0xB4 | 0xB8 | 0xB2 | 0xB3 | 0xB9 => 333,
        0xA2..=0xA5 | 0xA7 | 0xAB | 0xB5 | 0xBB => 556,
        0xA6 => 260,
        0xA9 | 0xAE => 737,
        0xAA => 370,
        0xAC | 0xB1 | 0xD7 | 0xF7 => 584,
        0xB0 => 400,
        0xB6 => 537,
        0xB7 => 278,
        0xBA => 365,
        0xBC..=0xBE => 834,
        0xBF | 0xDF => 611,
        0xC0..=0xC5 | 0xC8..=0xCB | 0xDD | 0xDE => 667,
        0xC6 => 1000,
        0xC7 | 0xD0 | 0xD1 | 0xD9..=0xDC => 722,
        0xCC..=0xCF | 0xEC..=0xEF => 278,
        0xD2..=0xD6 | 0xD8 => 778,
        0xE0..=0xE5 | 0xE8..=0xEB | 0xF0..=0xF6 | 0xF9..=0xFC | 0xFE => 556,
        0xE6 => 889,
        0xE7 | 0xFD | 0xFF => 500,
        0xF8 => 611,
        _ => 556,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Face {
    Regular,
    Bold,
    Oblique,
}

impl Face {
    fn resource(self) -> &'static [u8] {
        match self {
            Face::Regular => b"F1",
            Face::Bold => b"F2",
            Face::Oblique => b"F3",
        }
    }

    fn base_font(self) -> &'static str {
        match self {
            Face::Regular => "Helvetica",
            Face::Bold => "Helvetica-Bold",
            Face::Oblique => "Helvetica-Oblique",
        }
    }

    /// Bold is a little wider than the regular table says; it is only used for
    /// short labels and the title, so a flat allowance is enough.
    fn allowance(self) -> f32 {
        match self {
            Face::Bold => 1.08,
            _ => 1.0,
        }
    }
}

fn text_width(bytes: &[u8], face: Face, size: f32) -> f32 {
    let thousandths: u32 = bytes.iter().map(|b| u32::from(width_of(*b))).sum();
    thousandths as f32 * size / 1000.0 * face.allowance()
}

/// Greedy word wrap of `text` (which may hold `\n`). An empty entry is a blank
/// line. The first line of each hard line has `first` points, the rest `rest`; a
/// word wider than a line is broken where it overflows, never cut off.
fn wrap(text: &[u8], face: Face, size: f32, first: f32, rest: f32) -> Vec<Vec<u8>> {
    // Estimates are exact for the regular tables; this slack covers rounding.
    const SLACK: f32 = 0.985;
    let mut lines: Vec<Vec<u8>> = Vec::new();
    for hard in text.split(|b| *b == b'\n') {
        let mut current: Vec<u8> = Vec::new();
        let mut first_line = true;
        let mut limit = first * SLACK;
        let fits = |bytes: &[u8], limit: f32| text_width(bytes, face, size) <= limit;
        for word in hard.split(|b| *b == b' ').filter(|w| !w.is_empty()) {
            let mut candidate = current.clone();
            if !candidate.is_empty() {
                candidate.push(b' ');
            }
            candidate.extend_from_slice(word);
            if fits(&candidate, limit) {
                current = candidate;
                continue;
            }
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
                if first_line {
                    first_line = false;
                    limit = rest * SLACK;
                }
            }
            let mut remaining = word;
            while !fits(remaining, limit) {
                let mut take = 1;
                while take < remaining.len() && fits(&remaining[..take + 1], limit) {
                    take += 1;
                }
                lines.push(remaining[..take].to_vec());
                remaining = &remaining[take..];
                if first_line {
                    first_line = false;
                    limit = rest * SLACK;
                }
            }
            current = remaining.to_vec();
        }
        lines.push(current);
    }
    lines
}

// --- layout ----------------------------------------------------------------

struct Run {
    face: Face,
    size: f32,
    gray: f32,
    bytes: Vec<u8>,
}

struct Placed {
    x: f32,
    /// Baseline, from the top of the page.
    y: f32,
    runs: Vec<Run>,
}

struct Rect {
    x: f32,
    /// Top edge, from the top of the page.
    y: f32,
    w: f32,
    h: f32,
    gray: f32,
}

#[derive(Default)]
struct Page {
    lines: Vec<Placed>,
    rects: Vec<Rect>,
}

struct Layout {
    pages: Vec<Page>,
    /// Top of the next line, from the top of the page.
    y: f32,
}

impl Layout {
    fn new() -> Self {
        Layout {
            pages: vec![Page::default()],
            y: MARGIN_TOP,
        }
    }

    fn page(&mut self) -> &mut Page {
        self.pages.last_mut().expect("there is always a page")
    }

    /// Start a new page when `height` more does not fit.
    fn room(&mut self, height: f32) {
        if self.y + height > PAGE_HEIGHT - MARGIN_BOTTOM && self.y > MARGIN_TOP {
            self.pages.push(Page::default());
            self.y = MARGIN_TOP;
        }
    }

    fn gap(&mut self, height: f32) {
        self.y += height;
    }

    /// One line of `leading` points; `bar` also draws a bar beside it.
    fn line(&mut self, x: f32, leading: f32, runs: Vec<Run>, bar: Option<(f32, f32)>) {
        self.room(leading);
        let top = self.y;
        if let Some((bar_x, gray)) = bar {
            self.page().rects.push(Rect {
                x: bar_x,
                y: top,
                w: 2.5,
                h: leading,
                gray,
            });
        }
        self.page().lines.push(Placed {
            x,
            y: top + leading * 0.78,
            runs,
        });
        self.y += leading;
    }

    fn rule(&mut self, gray: f32) {
        self.room(1.0);
        let y = self.y;
        self.page().rects.push(Rect {
            x: MARGIN_X,
            y,
            w: PAGE_WIDTH - 2.0 * MARGIN_X,
            h: 0.75,
            gray,
        });
        self.y += 0.75;
    }
}

struct Style {
    face: Face,
    size: f32,
    leading: f32,
    gray: f32,
    indent: f32,
    bar: Option<f32>,
}

impl Layout {
    fn paragraphs(&mut self, text: &[u8], style: &Style) {
        let width = PAGE_WIDTH - 2.0 * MARGIN_X - style.indent;
        for line in wrap(text, style.face, style.size, width, width) {
            if line.is_empty() {
                self.gap(style.leading * 0.6);
                continue;
            }
            let run = Run {
                face: style.face,
                size: style.size,
                gray: style.gray,
                bytes: line,
            };
            self.line(
                MARGIN_X + style.indent,
                style.leading,
                vec![run],
                style.bar.map(|gray| (MARGIN_X + 2.0, gray)),
            );
        }
    }
}

fn operand(value: f32) -> Object {
    Object::Real(value)
}

fn page_content(page: &Page) -> Content {
    let mut operations = Vec::new();
    for rect in &page.rects {
        operations.push(Operation::new("q", vec![]));
        operations.push(Operation::new("g", vec![operand(rect.gray)]));
        operations.push(Operation::new(
            "re",
            vec![
                operand(rect.x),
                operand(PAGE_HEIGHT - rect.y - rect.h),
                operand(rect.w),
                operand(rect.h),
            ],
        ));
        operations.push(Operation::new("f", vec![]));
        operations.push(Operation::new("Q", vec![]));
    }
    operations.push(Operation::new("BT", vec![]));
    for line in &page.lines {
        operations.push(Operation::new(
            "Tm",
            vec![
                operand(1.0),
                operand(0.0),
                operand(0.0),
                operand(1.0),
                operand(line.x),
                operand(PAGE_HEIGHT - line.y),
            ],
        ));
        for run in &line.runs {
            operations.push(Operation::new(
                "Tf",
                vec![
                    Object::Name(run.face.resource().to_vec()),
                    operand(run.size),
                ],
            ));
            operations.push(Operation::new("g", vec![operand(run.gray)]));
            operations.push(Operation::new(
                "Tj",
                vec![Object::string_literal(run.bytes.clone())],
            ));
        }
    }
    operations.push(Operation::new("ET", vec![]));
    Content { operations }
}

/// Render `document` as an A4 PDF.
pub fn render(document: &Document) -> Result<Rendered, String> {
    let mut replaced = 0;
    let mut encode = |text: &str| {
        let (bytes, count) = to_win_ansi(text);
        replaced += count;
        bytes
    };
    let title = encode(&document.title);
    let mut meta: Vec<(Vec<u8>, Vec<u8>)> = document
        .meta
        .iter()
        .map(|(label, value)| (encode(label), encode(value)))
        .collect();
    let body: Vec<(&Block, Vec<u8>)> = document
        .body
        .iter()
        .map(|block| {
            let text = match block {
                Block::Text(text) | Block::Context(text) | Block::Quote(text) => text,
            };
            (block, encode(text))
        })
        .collect();
    if replaced > 0 {
        let note = format!(
            "{replaced} caracteres no representables se sustituyeron por «?» en esta copia"
        );
        meta.push((to_win_ansi("Nota").0, to_win_ansi(&note).0));
    }

    let content_width = PAGE_WIDTH - 2.0 * MARGIN_X;
    let mut layout = Layout::new();

    layout.paragraphs(
        &title,
        &Style {
            face: Face::Bold,
            size: 17.0,
            leading: 22.0,
            gray: 0.0,
            indent: 0.0,
            bar: None,
        },
    );
    layout.gap(6.0);
    for (label, value) in &meta {
        if value.is_empty() {
            continue;
        }
        let mut label = label.clone();
        label.extend_from_slice(b": ");
        let label_width = text_width(&label, Face::Bold, 9.5);
        let lines = wrap(
            value,
            Face::Regular,
            9.5,
            content_width - label_width,
            content_width,
        );
        for (index, text) in lines.into_iter().enumerate() {
            let mut runs = Vec::new();
            if index == 0 {
                runs.push(Run {
                    face: Face::Bold,
                    size: 9.5,
                    gray: 0.25,
                    bytes: label.clone(),
                });
            }
            runs.push(Run {
                face: Face::Regular,
                size: 9.5,
                gray: 0.25,
                bytes: text,
            });
            layout.line(MARGIN_X, 13.0, runs, None);
        }
    }
    layout.gap(8.0);
    layout.rule(0.6);
    layout.gap(14.0);

    for (block, text) in &body {
        let style = match block {
            Block::Text(_) => Style {
                face: Face::Regular,
                size: 11.0,
                leading: 15.0,
                gray: 0.0,
                indent: 0.0,
                bar: None,
            },
            Block::Context(_) => Style {
                face: Face::Regular,
                size: 10.0,
                leading: 14.0,
                gray: 0.45,
                indent: 0.0,
                bar: None,
            },
            Block::Quote(_) => Style {
                face: Face::Oblique,
                size: 12.0,
                leading: 17.0,
                gray: 0.0,
                indent: 14.0,
                bar: Some(0.55),
            },
        };
        layout.paragraphs(text, &style);
        layout.gap(8.0);
    }

    let mut pdf = Pdf::with_version("1.5");
    let pages_id = pdf.new_object_id();
    let mut fonts = lopdf::Dictionary::new();
    for face in [Face::Regular, Face::Bold, Face::Oblique] {
        let font_id = pdf.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => face.base_font(),
            "Encoding" => "WinAnsiEncoding",
        });
        fonts.set(face.resource().to_vec(), font_id);
    }
    let resources_id = pdf.add_object(dictionary! { "Font" => fonts });

    let mut page_ids = Vec::new();
    for page in &layout.pages {
        let encoded = page_content(page)
            .encode()
            .map_err(|error| format!("pdf_content: {error}"))?;
        let mut stream = Stream::new(dictionary! {}, encoded);
        // Best effort: an uncompressed stream is just as valid.
        let _ = stream.compress();
        let content_id = pdf.add_object(stream);
        page_ids.push(pdf.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), PAGE_WIDTH.into(), PAGE_HEIGHT.into()],
            "Resources" => resources_id,
            "Contents" => content_id,
        }));
    }
    pdf.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Count" => page_ids.len() as i64,
            "Kids" => page_ids.iter().map(|id| Object::Reference(*id)).collect::<Vec<Object>>(),
        }),
    );
    let catalog_id = pdf.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    let info_id = pdf.add_object(dictionary! {
        "Title" => Object::string_literal(title.clone()),
        "Producer" => Object::string_literal("EntropIA Navegador"),
    });
    pdf.trailer.set("Root", catalog_id);
    pdf.trailer.set("Info", info_id);

    let mut bytes = Vec::new();
    pdf.save_to(&mut bytes)
        .map_err(|error| format!("pdf_save: {error}"))?;
    Ok(Rendered {
        bytes,
        pages: layout.pages.len(),
        replaced,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocr::pdf::is_quality_text;

    fn doc(body: Vec<Block>) -> Document {
        Document {
            title: "Educación y ñandú".into(),
            meta: vec![
                ("URL".into(), "https://example.com/a".into()),
                ("Consultada (UTC)".into(), "2026-09-30T12:00:00Z".into()),
            ],
            body,
        }
    }

    fn extract(bytes: &[u8]) -> String {
        pdf_extract::extract_text_from_mem(bytes).expect("the text layer is readable")
    }

    fn squash(text: &str) -> String {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn spanish_and_english_text_survives_the_round_trip() {
        let text = "¿Qué pasó con la educación? ¡Ñandú! “Hola” – dijo — it’s fine… 5 € ©";
        let rendered = render(&doc(vec![Block::Text(text.into())])).unwrap();

        let back = squash(&extract(&rendered.bytes));

        assert!(back.contains("Educación y ñandú"), "{back}");
        assert!(back.contains("¿Qué pasó con la educación?"), "{back}");
        assert!(back.contains("¡Ñandú!"), "{back}");
        assert!(back.contains("“Hola” – dijo — it’s fine… 5 € ©"), "{back}");
        assert_eq!(rendered.replaced, 0);
    }

    #[test]
    fn the_header_carries_the_provenance_lines() {
        let rendered = render(&doc(vec![Block::Text("cuerpo".into())])).unwrap();

        let back = squash(&extract(&rendered.bytes));

        assert!(back.contains("URL: https://example.com/a"), "{back}");
        assert!(
            back.contains("Consultada (UTC): 2026-09-30T12:00:00Z"),
            "{back}"
        );
        assert!(back.contains("cuerpo"), "{back}");
    }

    #[test]
    fn a_quote_and_its_context_come_in_reading_order() {
        let rendered = render(&doc(vec![
            Block::Context("antes de la cita".into()),
            Block::Quote("la cita exacta".into()),
            Block::Context("después de la cita".into()),
        ]))
        .unwrap();

        let back = squash(&extract(&rendered.bytes));

        let before = back.find("antes de la cita").expect("context before");
        let quote = back.find("la cita exacta").expect("quote");
        let after = back.find("después de la cita").expect("context after");
        assert!(before < quote && quote < after, "{back}");
    }

    #[test]
    fn long_text_flows_onto_more_pages_without_losing_a_word() {
        let paragraph = (0..60)
            .map(|n| format!("palabra{n}"))
            .collect::<Vec<_>>()
            .join(" ");
        let text = vec![paragraph; 40].join("\n\n");

        let rendered = render(&doc(vec![Block::Text(text)])).unwrap();

        assert!(rendered.pages >= 3, "{} pages", rendered.pages);
        let parsed = lopdf::Document::load_mem(&rendered.bytes).unwrap();
        assert_eq!(parsed.get_pages().len(), rendered.pages);
        let words = squash(&extract(&rendered.bytes));
        let words: Vec<&str> = words.split(' ').collect();
        assert_eq!(words.iter().filter(|w| **w == "palabra0").count(), 40);
        assert_eq!(words.iter().filter(|w| **w == "palabra59").count(), 40);
    }

    #[test]
    fn a_word_wider_than_a_line_is_broken_and_not_cut_off() {
        let long = "x".repeat(400);
        let rendered = render(&doc(vec![Block::Text(long.clone())])).unwrap();

        let back: String = extract(&rendered.bytes)
            .split_whitespace()
            .filter(|word| word.chars().all(|c| c == 'x'))
            .collect();

        assert_eq!(back, long);
    }

    #[test]
    fn a_short_capture_still_passes_the_native_text_quality_check() {
        let rendered = render(&Document {
            title: "T".into(),
            meta: vec![
                ("URL".into(), "https://example.com/a".into()),
                ("SHA-256".into(), "a".repeat(64)),
            ],
            body: vec![Block::Quote("ok".into())],
        })
        .unwrap();

        assert!(is_quality_text(&extract(&rendered.bytes)));
    }

    #[test]
    fn what_win_ansi_cannot_hold_becomes_a_question_mark_and_is_counted() {
        let (bytes, replaced) = to_win_ansi("a ł 漢字 😀 b");

        assert_eq!(bytes, b"a ? ?? ? b");
        assert_eq!(replaced, 4);
    }

    #[test]
    fn unrepresentable_text_yields_a_valid_pdf_that_says_so() {
        let rendered = render(&doc(vec![Block::Text("漢字 😀 text".into())])).unwrap();

        assert_eq!(rendered.replaced, 3);
        assert!(lopdf::Document::load_mem(&rendered.bytes).is_ok());
        let back = squash(&extract(&rendered.bytes));
        assert!(back.contains("3 caracteres"), "{back}");
        assert!(back.contains("?? ? text"), "{back}");
    }

    #[test]
    fn look_alikes_are_mapped_and_invisible_characters_dropped() {
        let (bytes, replaced) =
            to_win_ansi("a\u{a0}b\u{2009}c\u{2011}d\u{200b}e\u{feff}f\u{fb01}g\t.");

        assert_eq!(bytes, b"a b c-deffig .");
        assert_eq!(replaced, 0);
    }

    #[test]
    fn a_base_letter_and_a_combining_mark_become_the_accented_letter() {
        let (bytes, replaced) = to_win_ansi("cafe\u{301} nin\u{303}o u\u{308}");

        assert_eq!(bytes, b"caf\xe9 ni\xf1o \xfc");
        assert_eq!(replaced, 0);
    }

    #[test]
    fn windows_1252_punctuation_has_its_own_bytes() {
        let (bytes, _) = to_win_ansi("“a” ‘b’ – — … € •");

        assert_eq!(bytes, b"\x93a\x94 \x91b\x92 \x96 \x97 \x85 \x80 \x95");
    }

    #[test]
    fn a_document_with_no_body_is_still_one_page() {
        let rendered = render(&doc(vec![])).unwrap();

        assert_eq!(rendered.pages, 1);
    }

    #[test]
    fn parentheses_and_backslashes_are_text_not_syntax() {
        let rendered = render(&doc(vec![Block::Text("f(x) = a\\b (cierra))".into())])).unwrap();

        let back = squash(&extract(&rendered.bytes));

        assert!(back.contains("f(x) = a\\b (cierra))"), "{back}");
    }
}
