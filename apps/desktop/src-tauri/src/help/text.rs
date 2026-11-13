//! Shared Spanish text helpers for the help assistant: accent folding and the
//! stopword list the local selection drops from queries. Moved from
//! `manual.rs` so the index query builder shares the exact same list.

/// Spanish function words that carry no retrieval signal.
pub(crate) const STOPWORDS: &[&str] = &[
    "que", "como", "para", "por", "con", "los", "las", "una", "uno", "unos", "unas", "del", "mis",
    "sus", "este", "esta", "esto", "estos", "estas", "ese", "esa", "eso", "hay", "puedo", "quiero",
    "hago", "hacer", "donde", "cuando", "porque", "pero", "sin", "sobre", "entre", "todo", "todos",
    "tiene", "tengo", "cual", "mas", "muy", "ya", "les", "le", "me", "mi", "se", "si", "no", "es",
    "son", "al", "el", "la", "lo", "de", "en", "y", "o", "un", "a", "the", "and", "for", "how",
    "what", "can", "entropia", "app",
];

/// Lowercases and drops Spanish accents, so «Transcripción» meets
/// «transcripcion» before either reaches the index or the query builder.
pub(crate) fn fold(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'á' | 'à' | 'ä' | 'â' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            other => other,
        })
        .collect()
}
