//! `bibliography-profile-v1`: the canonical text of one verified work (E3b-WU1).
//!
//! The profile is derived purely from catalog metadata — never from
//! attachments — so a work without a PDF profiles identically to one with
//! full text (plan §"Construir el perfil aunque no haya adjunto"). The
//! builder is a pure function over [`ProfileInput`]: labeled lines for the
//! fields present, creator order preserved, tags sorted stably, whitespace
//! collapsed, and an input hash over the template version plus the canonical
//! text. Nothing administrative (ids, formatted citations) enters the text.
//!
//! Publication lives in [`super::profile_repository`]; this module only
//! derives, so template evolution stays testable without a database.

use sha2::{Digest, Sha256};

/// Versioned template stamped into every profile row and mixed into the
/// hash: changing the template re-profiles every work.
pub const BIBLIOGRAPHY_PROFILE_TEMPLATE_V1: &str = "bibliography-profile-v1";

/// Catalog metadata the template consumes. `creators` keeps family/given
/// pairs in stored order; `tags` is normalized (sorted, deduplicated) by the
/// builder so import order never changes the canonical text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProfileInput {
    pub title: String,
    pub creators: Vec<(String, String)>,
    pub year: Option<i64>,
    pub item_type: String,
    pub publication: String,
    pub abstract_text: String,
    pub tags: Vec<String>,
}

/// The derived profile: canonical text, its provenance per line, and the
/// input hash stamped into `bibliographic_semantic_profiles`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltProfile {
    pub canonical_text: String,
    pub input_hash: String,
    /// `(field, line)` pairs in render order — the durable provenance of
    /// what entered the text and how.
    pub field_provenance: Vec<(String, String)>,
}

/// Collapses whitespace runs to single spaces and trims: Zotero metadata
/// frequently carries newlines inside abstracts, and the profile text must
/// be stable regardless of import formatting.
fn normalize_field(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Formats one creator as `Family, Given`, preserving catalog order.
fn format_creator(family: &str, given: &str) -> String {
    let family = normalize_field(family);
    let given = normalize_field(given);
    if given.is_empty() {
        family
    } else if family.is_empty() {
        given
    } else {
        format!("{family}, {given}")
    }
}

impl ProfileInput {
    fn normalized(self: &ProfileInput) -> ProfileInput {
        ProfileInput {
            title: normalize_field(&self.title),
            creators: self
                .creators
                .iter()
                .map(|(family, given)| (normalize_field(family), normalize_field(given)))
                .collect(),
            year: self.year,
            item_type: normalize_field(&self.item_type),
            publication: normalize_field(&self.publication),
            abstract_text: normalize_field(&self.abstract_text),
            // Stable tag identity: sort and deduplicate so neither Zotero's
            // arbitrary order nor duplicate tags can change the text.
            tags: {
                let mut tags: Vec<String> =
                    self.tags.iter().map(|tag| normalize_field(tag)).collect();
                tags.retain(|tag| !tag.is_empty());
                tags.sort();
                tags.dedup();
                tags
            },
        }
    }
}

/// Derives the `bibliography-profile-v1` canonical text. Empty fields are
/// omitted entirely (no empty labeled lines); a work with only a title
/// still profiles.
pub fn build_profile(input: &ProfileInput) -> BuiltProfile {
    let input = input.normalized();
    let mut lines: Vec<String> = Vec::new();
    let mut provenance: Vec<(String, String)> = Vec::new();
    let mut push = |field: &str, label: &str, value: String| {
        if value.is_empty() {
            return;
        }
        let line = format!("{label}: {value}");
        provenance.push((field.to_string(), line.clone()));
        lines.push(line);
    };
    push("title", "Título", input.title.clone());
    if !input.creators.is_empty() {
        let creators = input
            .creators
            .iter()
            .map(|(family, given)| format_creator(family, given))
            .filter(|creator| !creator.is_empty())
            .collect::<Vec<_>>()
            .join("; ");
        push("creators", "Autores", creators);
    }
    if let Some(year) = input.year {
        push("year", "Año", year.to_string());
    }
    push("item_type", "Tipo", input.item_type.clone());
    push(
        "publication",
        "Publicación o editorial",
        input.publication.clone(),
    );
    push("abstract", "Resumen", input.abstract_text.clone());
    if !input.tags.is_empty() {
        push("tags", "Palabras clave y etiquetas", input.tags.join("; "));
    }
    let canonical_text = lines.join("\n");
    let input_hash = profile_input_hash(&canonical_text);
    BuiltProfile {
        canonical_text,
        input_hash,
        field_provenance: provenance,
    }
}

/// `sha256(template_version \n canonical_text)`: the template participates
/// in the hash so a template bump re-profiles every work, and equal texts
/// from different templates never share identity.
pub fn profile_input_hash(canonical_text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(BIBLIOGRAPHY_PROFILE_TEMPLATE_V1.as_bytes());
    hasher.update(b"\n");
    hasher.update(canonical_text.as_bytes());
    let digest = hasher.finalize();
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

// ── Catalog-backed extraction (E3b-WU2) ────────────────────────────────────

use crate::bibliography::repository::BibliographyError;
use crate::bibliography::repository::BibliographyResult;
use rusqlite::OptionalExtension as _;

fn json_str(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string()
}

fn normalize_csl_creators(value: &serde_json::Value) -> Vec<(String, String)> {
    match value.get("author").and_then(|value| value.as_array()) {
        Some(creators) => creators
            .iter()
            .map(|creator| (json_str(creator, "family"), json_str(creator, "given")))
            .filter(|(family, given)| !family.is_empty() || !given.is_empty())
            .collect(),
        None => Vec::new(),
    }
}

fn csl_year(value: &serde_json::Value) -> Option<i64> {
    let parts = value.get("issued")?.get("date-parts")?.as_array()?;
    let first = parts.first()?.as_array()?;
    first.first()?.as_i64()
}

fn native_tags(value: &serde_json::Value) -> Vec<String> {
    match value.get("tags").and_then(|value| value.as_array()) {
        Some(tags) => tags
            .iter()
            .filter_map(|tag| match tag {
                serde_json::Value::String(text) => Some(text.clone()),
                other => other
                    .get("tag")
                    .and_then(|text| text.as_str())
                    .map(String::from),
            })
            .collect(),
        None => Vec::new(),
    }
}

/// Reads one verified catalog row into the profile template input. `None`
/// when the work does not exist. Fields come from the trusted snapshots:
/// CSL carries title/creators/year/type/publication/abstract, the native
/// snapshot carries Zotero tags. Malformed JSON degrades to empty fields —
/// the snapshots are validated at write time, so this is defensive depth,
/// never a silent acceptance path.
pub fn profile_input_for_item(
    conn: &rusqlite::Connection,
    item_id: &str,
) -> BibliographyResult<Option<ProfileInput>> {
    let row: Option<(String, Option<String>, String, String)> = conn
        .query_row(
            "SELECT csl_json_snapshot, creators_json, native_json_snapshot, title
             FROM bibliographic_items WHERE id = ?1",
            [item_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(|error| BibliographyError::sql("Failed to read catalog item", error))?;
    let Some((csl_json, creators_json, native_json, title)) = row else {
        return Ok(None);
    };
    let csl: serde_json::Value = serde_json::from_str(&csl_json).unwrap_or(serde_json::Value::Null);
    let native: serde_json::Value =
        serde_json::from_str(&native_json).unwrap_or(serde_json::Value::Null);
    let mut creators = normalize_csl_creators(&csl);
    if creators.is_empty() {
        if let Ok(serde_json::Value::Array(creators_json)) =
            serde_json::from_str::<serde_json::Value>(creators_json.as_deref().unwrap_or("[]"))
        {
            creators = creators_json
                .iter()
                .map(|creator| (json_str(creator, "family"), json_str(creator, "given")))
                .filter(|(family, given)| !family.is_empty() || !given.is_empty())
                .collect();
        }
    }
    let title = {
        let csl_title = json_str(&csl, "title");
        if csl_title.is_empty() {
            title
        } else {
            csl_title
        }
    };
    Ok(Some(ProfileInput {
        title,
        creators,
        year: csl_year(&csl),
        item_type: json_str(&csl, "type"),
        publication: json_str(&csl, "publisher"),
        abstract_text: json_str(&csl, "abstract"),
        tags: native_tags(&native),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_input() -> ProfileInput {
        ProfileInput {
            title: "Revoluciones y cultura   política".to_string(),
            creators: vec![
                ("Pérez".to_string(), "Ana".to_string()),
                ("Gómez".to_string(), "Luis".to_string()),
            ],
            year: Some(2018),
            item_type: "book".to_string(),
            publication: "Editorial Universitaria".to_string(),
            abstract_text: "Estudio de asociaciones y prácticas políticas en el siglo XIX."
                .to_string(),
            tags: vec![
                "siglo XIX".to_string(),
                "asociaciones".to_string(),
                "cultura política".to_string(),
            ],
        }
    }

    #[test]
    fn full_profile_renders_labeled_lines_in_template_order() {
        let built = build_profile(&full_input());
        assert_eq!(
            built.canonical_text,
            "Título: Revoluciones y cultura política\n\
             Autores: Pérez, Ana; Gómez, Luis\n\
             Año: 2018\n\
             Tipo: book\n\
             Publicación o editorial: Editorial Universitaria\n\
             Resumen: Estudio de asociaciones y prácticas políticas en el siglo XIX.\n\
             Palabras clave y etiquetas: asociaciones; cultura política; siglo XIX"
        );
        assert_eq!(built.field_provenance.len(), 7);
        assert_eq!(built.field_provenance[0].0, "title");
        assert_eq!(
            built.field_provenance[6].0, "tags",
            "provenance keeps render order"
        );
    }

    #[test]
    fn title_only_works_profile_without_attachment_or_empty_lines() {
        let built = build_profile(&ProfileInput {
            title: "Obra huérfana".to_string(),
            ..Default::default()
        });
        assert_eq!(built.canonical_text, "Título: Obra huérfana");
        assert_eq!(built.field_provenance.len(), 1);
        assert!(
            !built.canonical_text.contains(": \n"),
            "no empty labeled lines may appear"
        );
    }

    #[test]
    fn tags_sort_stably_and_whitespace_never_changes_the_hash() {
        let mut a = full_input();
        a.tags = vec!["zeta".to_string(), "alpha".to_string(), "alpha".to_string()];
        a.title = "  Espacios   múltiples  ".to_string();
        a.abstract_text = "línea uno\nlínea  dos".to_string();
        let built_a = build_profile(&a);
        let mut b = full_input();
        b.tags = vec!["alpha".to_string(), "zeta".to_string()];
        b.title = "Espacios múltiples".to_string();
        b.abstract_text = "línea uno línea dos".to_string();
        let built_b = build_profile(&b);
        assert_eq!(
            built_a.canonical_text, built_b.canonical_text,
            "import formatting and tag order must not change the text"
        );
        assert_eq!(built_a.input_hash, built_b.input_hash);
        assert!(
            built_a
                .canonical_text
                .ends_with("Palabras clave y etiquetas: alpha; zeta"),
            "tags render sorted and deduplicated"
        );
    }

    #[test]
    fn hash_binds_template_and_text() {
        let built = build_profile(&full_input());
        assert_eq!(
            built.input_hash,
            profile_input_hash(&built.canonical_text),
            "the stored hash is the template+text hash"
        );
        let mut changed = full_input();
        changed.year = Some(2019);
        assert_ne!(
            build_profile(&changed).input_hash,
            built.input_hash,
            "a metadata edit must change the hash"
        );
        let digest = built.input_hash;
        assert_eq!(digest.len(), 64, "the hash is a hex sha256 digest");
    }
}
