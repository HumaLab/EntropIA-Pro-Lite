//! Sends one Escritura document to the hlab.com.ar blog (hlab-platform
//! `PUT /api/escritura/posts/{documento}`). The first send creates a hidden
//! draft; later sends update title and body, live if it was already published.
//! The frontend renders the HTML with the same exporter as "Descargar HTML".

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::db::commands::run_blocking_db_task;
use crate::db::state::AppDbState;
use crate::settings::HLAB_PUBLISH_KEY;

const HLAB_URL: &str = "https://hlab.com.ar";
/// Debug builds only: points publishing at a local copy of the site.
const DEV_HLAB_URL_VAR: &str = "ENTROPIA_DEV_HLAB_URL";

#[derive(Debug, Serialize, Deserialize)]
pub struct PublishedPost {
    pub url: String,
    pub admin_url: String,
    pub is_active: bool,
    pub created: bool,
}

/// The site to publish to. A dev profile never reaches the real site: it needs
/// the local one named explicitly.
fn hlab_base() -> Result<String, String> {
    let local = if cfg!(debug_assertions) {
        std::env::var(DEV_HLAB_URL_VAR)
            .ok()
            .filter(|v| !v.trim().is_empty())
    } else {
        None
    };
    match (local, crate::dev_profile::active()) {
        (Some(url), _) => Ok(url.trim_end_matches('/').to_string()),
        (None, Some(_)) => Err(format!(
            "hlab_disabled_in_dev_profile: definí {DEV_HLAB_URL_VAR} para publicar en el sitio local"
        )),
        (None, None) => Ok(HLAB_URL.to_string()),
    }
}

#[tauri::command]
pub async fn writing_publish_hlab(
    document_id: String,
    title: String,
    html: String,
    db: State<'_, AppDbState>,
) -> Result<PublishedPost, String> {
    if document_id.is_empty()
        || !document_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err(format!("invalid document id {document_id:?}"));
    }
    let db = db.inner().clone();
    let key = run_blocking_db_task(move || {
        crate::settings::get_secret_setting_unlocked(
            &db.ui_conn,
            HLAB_PUBLISH_KEY,
            &crate::settings::KeyringSecretStore,
        )
    })
    .await?
    .filter(|key| !key.trim().is_empty())
    .ok_or_else(|| "hlab_key_missing: falta la clave para publicar en hlab.com.ar".to_string())?;

    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|_| "Could not build the HTTP client".to_string())?
        .put(format!(
            "{}/api/escritura/posts/{document_id}",
            hlab_base()?
        ))
        .header("X-Publicar-Clave", key.trim())
        .header("Accept", "application/json")
        .json(&serde_json::json!({ "title": title, "html": html }))
        .send()
        .await
        .map_err(|e| format!("No se pudo conectar con hlab.com.ar: {e}"))?;

    match response.status().as_u16() {
        200 | 201 => response
            .json::<PublishedPost>()
            .await
            .map_err(|e| format!("Respuesta inesperada de hlab.com.ar: {e}")),
        401 => Err("hlab_key_rejected: hlab.com.ar no aceptó la clave".to_string()),
        status => {
            let body = response.text().await.unwrap_or_default();
            Err(format!(
                "hlab.com.ar respondió {status}: {}",
                body.chars().take(300).collect::<String>()
            ))
        }
    }
}
