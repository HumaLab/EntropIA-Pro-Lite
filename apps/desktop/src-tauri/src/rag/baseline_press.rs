//! Caso de prensa del baseline: una página de *El Obrero* (Montevideo,
//! 26/08/1905, dominio público) con el artículo "La Mujer" al final.
//!
//! A diferencia del golden SOIP 1961 (`baseline.rs`), este caso corre en CI
//! sin red ni base privada: el texto OCR está versionado junto al golden.

/// Texto OCR exacto de la página, sin normalizar.
const EL_OBRERO_1905_08_26_P1: &str =
    include_str!("../../tests/fixtures/rag_golden/el_obrero_1905_08_26_p1.txt");

/// Escalares Unicode de la página. Si cambia, el archivo se corrompió.
const EL_OBRERO_CHARS: usize = 14_124;

/// Carácter (base 0, escalares Unicode) donde empieza `## La Mujer`.
const LA_MUJER_TITLE_CHAR: usize = 12_034;

const LA_MUJER_TITLE: &str = "## La Mujer";

/// Posición, en escalares Unicode, de la primera aparición de `needle`.
fn char_position(text: &str, needle: &str) -> Option<usize> {
    text.find(needle)
        .map(|byte_index| text[..byte_index].chars().count())
}

#[test]
fn el_obrero_fixture_keeps_its_exact_length_and_title_offset() {
    assert_eq!(EL_OBRERO_1905_08_26_P1.chars().count(), EL_OBRERO_CHARS);
    assert_eq!(
        char_position(EL_OBRERO_1905_08_26_P1, LA_MUJER_TITLE),
        Some(LA_MUJER_TITLE_CHAR)
    );
    assert_eq!(
        EL_OBRERO_1905_08_26_P1.matches(LA_MUJER_TITLE).count(),
        1,
        "the title marks exactly one section"
    );
}
