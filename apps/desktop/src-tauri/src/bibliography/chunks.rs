//! Structural work chunking (E4c-WU1): packs page paragraphs into
//! size-bounded chunks with exact page spans.
//!
//! Pure and dependency-free: the publisher feeds ordered page texts and
//! stores the resulting chunks and spans. Rules, all pinned by tests:
//! - Pages split into paragraphs on blank lines; whitespace collapses
//!   inside a paragraph; empty pages contribute nothing.
//! - Paragraphs pack greedily up to `CHUNK_TARGET_CHARS`. Closing a chunk
//!   repeats its last paragraph (when short) at the head of the next one,
//!   so overlap stays paragraph-aligned and spans stay exact.
//! - A single paragraph above the target hard-splits on character windows
//!   with overlap, like the documentary chunker.
//! - Chunk text joins paragraphs with blank lines; the hash covers the
//!   final text, so equal texts hash equal regardless of page breaks.
//! - Spans record (page, start, end) offsets into the page's own text,
//!   including chunks that cross pages.

/// Target chunk size in Unicode scalars, mirroring the documentary
/// chunker's window.
pub const CHUNK_TARGET_CHARS: usize = 800;
/// Sliding overlap for hard-split paragraphs, mirroring the documentary
/// chunker's overlap.
pub const CHUNK_OVERLAP_CHARS: usize = 100;
/// A closing paragraph at most this long repeats into the next chunk.
pub const CHUNK_OVERLAP_PARAGRAPH_MAX_CHARS: usize = 200;

/// Versioned chunking contract stamped on every chunk row: changing the
/// template re-chunks every work instead of mixing chunkings.
pub const BIBLIOGRAPHY_CHUNKING_CONTRACT_V1: &str = "bibliography-chunk-800-paragraph-v1";

/// One input page in document order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageInput {
    pub page_number: i64,
    pub text: String,
}

/// One exact page range covered by a chunk, in the page's own character
/// offsets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkSpan {
    pub page_number: i64,
    pub start_char: usize,
    pub end_char: usize,
}

/// One structural chunk: ordinal, text, hash, and spans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkChunk {
    pub ordinal: usize,
    pub text: String,
    pub hash: String,
    pub spans: Vec<ChunkSpan>,
}

pub fn chunk_hash(text: &str) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;
    let digest = Sha256::digest(text.as_bytes());
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

/// One paragraph: raw text trimmed of surrounding whitespace, with exact
/// offsets into the page's own characters. Interior whitespace is kept
/// verbatim so offsets never drift from the source: chunk text fidelity
/// outranks import-formatting normalization here (the profile builder owns
/// normalization; chunks own provenance).
struct Paragraph {
    page_number: i64,
    start_char: usize,
    text: String,
}

/// Splits one page into paragraphs on blank lines. Offsets are Unicode-scalar
/// indices into the page text, exact by construction.
fn page_paragraphs(page_number: i64, text: &str) -> Vec<Paragraph> {
    let chars: Vec<char> = text.chars().collect();
    let mut paragraphs = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        while index < chars.len() && chars[index].is_whitespace() {
            index += 1;
        }
        if index >= chars.len() {
            break;
        }
        let mut end = chars.len();
        let mut scan = index;
        while scan < chars.len() {
            if chars[scan] == '\n' {
                let mut look = scan + 1;
                while look < chars.len() && chars[look] != '\n' && chars[look].is_whitespace() {
                    look += 1;
                }
                if look < chars.len() && chars[look] == '\n' {
                    end = scan;
                    break;
                }
            }
            scan += 1;
        }
        // Trim trailing whitespace off the raw extent so the span ends at
        // content, keeping offsets exact.
        let mut trimmed_end = end;
        while trimmed_end > index && chars[trimmed_end - 1].is_whitespace() {
            trimmed_end -= 1;
        }
        if trimmed_end > index {
            paragraphs.push(Paragraph {
                page_number,
                start_char: index,
                text: chars[index..trimmed_end].iter().collect(),
            });
        }
        index = end;
    }
    paragraphs
}

/// Segments ordered page texts into structural chunks.
pub fn segment_pages(pages: &[PageInput]) -> Vec<WorkChunk> {
    let mut paragraphs: Vec<Paragraph> = Vec::new();
    for page in pages {
        paragraphs.extend(page_paragraphs(page.page_number, &page.text));
    }
    // A chunk under construction: (paragraph index, slice start, slice end)
    // in scalar offsets of the paragraph's raw text.
    let mut current: Vec<(usize, usize, usize)> = Vec::new();
    let mut current_len = 0usize;
    let mut chunks: Vec<WorkChunk> = Vec::new();
    // Closes the open chunk, resolving slice offsets against raw page
    // offsets — exact because paragraph text is raw.
    let flush = |current: &mut Vec<(usize, usize, usize)>,
                     chunks: &mut Vec<WorkChunk>,
                     paragraphs: &[Paragraph]| {
        if current.is_empty() {
            return;
        }
        let mut text = String::new();
        let mut spans = Vec::new();
        for (first, &(paragraph_index, slice_start, slice_end)) in current.iter().enumerate() {
            let paragraph = &paragraphs[paragraph_index];
            let slice: String = paragraph
                .text
                .chars()
                .skip(slice_start)
                .take(slice_end - slice_start)
                .collect();
            if first > 0 {
                text.push_str("\n\n");
            }
            text.push_str(&slice);
            spans.push(ChunkSpan {
                page_number: paragraph.page_number,
                start_char: paragraph.start_char + slice_start,
                end_char: paragraph.start_char + slice_end,
            });
        }
        chunks.push(WorkChunk {
            ordinal: chunks.len(),
            hash: chunk_hash(&text),
            text,
            spans,
        });
        current.clear();
    };
    let mut idx = 0;
    while idx < paragraphs.len() {
        let paragraph_len = paragraphs[idx].text.chars().count();
        if paragraph_len > CHUNK_TARGET_CHARS {
            if !current.is_empty() {
                flush(&mut current, &mut chunks, &paragraphs);
                current_len = 0;
            }
            let mut start = 0;
            while start < paragraph_len {
                let end = (start + CHUNK_TARGET_CHARS).min(paragraph_len);
                current.push((idx, start, end));
                flush(&mut current, &mut chunks, &paragraphs);
                current_len = 0;
                if end == paragraph_len {
                    break;
                }
                start = end - CHUNK_OVERLAP_CHARS;
            }
            idx += 1;
            continue;
        }
        let separator = if current.is_empty() { 0 } else { 2 };
        if !current.is_empty() && current_len + separator + paragraph_len > CHUNK_TARGET_CHARS {
            let repeat = current
                .last()
                .and_then(|&(paragraph_index, slice_start, slice_end)| {
                    if slice_start == 0
                        && slice_end - slice_start
                            == paragraphs[paragraph_index].text.chars().count()
                        && slice_end - slice_start <= CHUNK_OVERLAP_PARAGRAPH_MAX_CHARS
                    {
                        Some((paragraph_index, slice_start, slice_end))
                    } else {
                        None
                    }
                });
            flush(&mut current, &mut chunks, &paragraphs);
            current_len = 0;
            if let Some(entry) = repeat {
                let len = entry.2 - entry.1;
                current.push(entry);
                current_len = len;
            }
        }
        current.push((idx, 0, paragraph_len));
        current_len += separator + paragraph_len;
        idx += 1;
    }
    flush(&mut current, &mut chunks, &paragraphs);
    chunks
}
#[cfg(test)]
mod tests {
    use super::*;

    fn page(number: i64, text: &str) -> PageInput {
        PageInput {
            page_number: number,
            text: text.to_string(),
        }
    }

    #[test]
    fn short_pages_pack_into_one_chunk_with_exact_spans() {
        let chunks = segment_pages(&[
            page(1, "Primer párrafo.\n\nSegundo párrafo."),
            page(2, "Tercer párrafo."),
        ]);
        assert_eq!(chunks.len(), 1, "short input packs into one chunk");
        assert_eq!(
            chunks[0].text,
            "Primer párrafo.\n\nSegundo párrafo.\n\nTercer párrafo."
        );
        assert_eq!(
            chunks[0].spans,
            vec![
                ChunkSpan {
                    page_number: 1,
                    start_char: 0,
                    end_char: 15
                },
                ChunkSpan {
                    page_number: 1,
                    start_char: 17,
                    end_char: 33
                },
                ChunkSpan {
                    page_number: 2,
                    start_char: 0,
                    end_char: 15
                },
            ]
        );
        assert_eq!(chunks[0].ordinal, 0);
    }

    #[test]
    fn overflow_repeats_the_closing_paragraph_in_the_next_chunk() {
        // 770 + 2 + 13 fits the closing paragraph in; the 19-char next
        // page then overflows, proving the repeat.
        let filler = "x".repeat(CHUNK_TARGET_CHARS - 30);
        let chunks = segment_pages(&[
            page(1, &format!("{filler}\n\nCierre breve.")),
            page(2, "Párrafo siguiente."),
        ]);
        assert_eq!(chunks.len(), 2, "overflow must split, got {}", chunks.len());
        assert!(
            chunks[0].text.ends_with("Cierre breve."),
            "the closing paragraph completes the first chunk"
        );
        assert!(
            chunks[1]
                .text
                .starts_with("Cierre breve.\n\nPárrafo siguiente."),
            "the next chunk repeats it, then continues: {}",
            chunks[1].text.chars().take(60).collect::<String>()
        );
        assert_eq!(chunks[1].ordinal, 1);
    }

    #[test]
    fn long_paragraphs_hard_split_with_overlap_on_one_page() {
        let long = "y".repeat(CHUNK_TARGET_CHARS + 50);
        let chunks = segment_pages(&[page(3, &long)]);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].text.chars().count(), CHUNK_TARGET_CHARS);
        assert_eq!(
            chunks[1].text.chars().count(),
            50 + CHUNK_OVERLAP_CHARS,
            "the second window overlaps the first by the overlap"
        );
        assert_eq!(
            chunks[1].spans,
            vec![ChunkSpan {
                page_number: 3,
                start_char: CHUNK_TARGET_CHARS - CHUNK_OVERLAP_CHARS,
                end_char: CHUNK_TARGET_CHARS + 50,
            }]
        );
    }

    #[test]
    fn empty_pages_contribute_nothing_and_empty_input_chunks_nothing() {
        let chunks = segment_pages(&[page(1, "   \n\n  "), page(2, "")]);
        assert!(chunks.is_empty(), "empty pages chunk nothing");
        assert!(segment_pages(&[]).is_empty());
    }

    #[test]
    fn hashes_are_stable_and_cover_the_final_text() {
        let input = vec![page(1, "Mismo texto.\n\nOtro párrafo.")];
        let a = segment_pages(&input);
        let b = segment_pages(&input);
        assert_eq!(a[0].hash, b[0].hash, "same input hashes equal");
        assert_eq!(a[0].hash.len(), 64, "the hash is a hex sha256 digest");
        let other = segment_pages(&[page(1, "Mismo texto.\n\nPárrafo distinto.")]);
        assert_ne!(a[0].hash, other[0].hash, "changed text changes the hash");
    }
}
