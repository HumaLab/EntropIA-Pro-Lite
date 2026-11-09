//! Researcher-written collection context shared by the four AI entry points.

use rusqlite::Connection;

/// Includes each selected collection once, preserving the author's text.
pub fn load(conn: &Connection, collection_ids: &[String]) -> String {
    if collection_ids.is_empty() {
        return String::new();
    }
    let Ok(mut statement) = conn.prepare(
        "SELECT id, name, ai_context FROM collections WHERE ai_context IS NOT NULL ORDER BY name, id",
    ) else {
        return String::new();
    };
    let Ok(rows) = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    }) else {
        return String::new();
    };
    let mut lines = Vec::new();
    for row in rows.flatten() {
        let (id, name, context) = row;
        if collection_ids.contains(&id) && !context.trim().is_empty() {
            lines.push(format!("- «{name}»: {context}"));
        }
    }
    if lines.is_empty() {
        String::new()
    } else {
        format!("Contexto del proyecto (escrito por quien investiga; usalo para interpretar nombres, grafías, período y convenciones):\n{}\n\n", lines.join("\n"))
    }
}

/// OCR context follows the asset's owning item.
pub fn for_asset(conn: &Connection, asset_id: &str) -> String {
    let collection_id = conn.query_row(
        "SELECT i.collection_id FROM assets a JOIN items i ON i.id = a.item_id WHERE a.id = ?1",
        [asset_id],
        |row| row.get::<_, String>(0),
    );
    match collection_id {
        Ok(id) => load(conn, &[id]),
        Err(_) => String::new(),
    }
}

/// Manuscripts inherit context only from their primary collection (S3/L7).
pub fn for_manuscript(conn: &Connection, document_id: &str) -> String {
    let Ok(mut statement) = conn.prepare(
        "SELECT collection_id FROM writing_document_collections WHERE document_id = ?1 AND is_primary = 1",
    ) else {
        return String::new();
    };
    let Ok(rows) = statement.query_map([document_id], |row| row.get::<_, String>(0)) else {
        return String::new();
    };
    let ids = rows.flatten().collect::<Vec<_>>();
    load(conn, &ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn database() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::migrations::apply_migrations(&mut conn).unwrap();
        conn.execute_batch(
            "INSERT INTO collections (id,name,ai_context,created_at,updated_at) VALUES
             ('b','Beta','  grafía original\nsegunda línea  ',0,0),
             ('a','Alfa','Siglo XIX',0,0),
             ('blank','Vacía',' \n\t ',0,0),
             ('null','Sin contexto',NULL,0,0);",
        )
        .unwrap();
        conn
    }

    #[test]
    fn context_is_labelled_sorted_unique_and_preserves_multiline_text() {
        let conn = database();
        assert_eq!(
            load(
                &conn,
                &["b".into(), "a".into(), "b".into(), "blank".into(), "null".into()]
            ),
            "Contexto del proyecto (escrito por quien investiga; usalo para interpretar nombres, grafías, período y convenciones):\n- «Alfa»: Siglo XIX\n- «Beta»:   grafía original\nsegunda línea  \n\n"
        );
    }

    #[test]
    fn assets_use_their_items_collection_and_missing_assets_add_nothing() {
        let conn = database();
        conn.execute_batch(
            "INSERT INTO items (id,title,collection_id,created_at,updated_at)
             VALUES ('item','Item','a',0,0);
             INSERT INTO assets (id,item_id,path,type,created_at)
             VALUES ('asset','item','fixture.png','image',0);",
        )
        .unwrap();
        assert_eq!(for_asset(&conn, "asset"), load(&conn, &["a".into()]));
        assert_eq!(for_asset(&conn, "missing"), "");
    }

    #[test]
    fn manuscripts_use_only_the_primary_collection() {
        let conn = database();
        conn.execute_batch(
            "INSERT INTO writing_documents
             (id,title,document_type,schema_version,current_content_json,created_at,updated_at)
             VALUES ('doc','Manuscrito','article',1,'{}',0,0);
             INSERT INTO writing_document_collections
             (document_id,collection_id,is_primary,created_at)
             VALUES ('doc','a',0,0),('doc','b',1,0);",
        )
        .unwrap();
        assert_eq!(for_manuscript(&conn, "doc"), load(&conn, &["b".into()]));
        assert_eq!(for_manuscript(&conn, "missing"), "");
    }

    #[test]
    fn missing_null_blank_or_unselected_context_adds_nothing() {
        let conn = database();
        assert_eq!(load(&conn, &[]), "");
        assert_eq!(
            load(&conn, &["missing".into(), "blank".into(), "null".into()]),
            ""
        );
    }
}
