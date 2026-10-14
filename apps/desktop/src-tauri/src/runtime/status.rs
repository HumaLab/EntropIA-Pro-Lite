use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeState {
    Healthy,
    Repairing,
    Checking,
    Downloading,
    Hydrating,
    Verifying,
    Damaged,
    Fixture,
    Incompatible,
    BlockedOffline,
    BlockedSourceUnavailable,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeOperationKind {
    Bootstrap,
    Repair,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeOperationStage {
    Checking,
    PlanningDownload,
    Downloading,
    Hydrating,
    Verifying,
    Activating,
    Blocked,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeOperation {
    pub kind: RuntimeOperationKind,
    pub stage: RuntimeOperationStage,
    pub summary: String,
    pub progress_percent: Option<u8>,
    pub downloaded_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    pub retryable: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeCapability {
    Ocr,
    Transcription,
    Nlp,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatus {
    pub state: RuntimeState,
    pub pack_version: Option<String>,
    pub repair_needed: bool,
    pub repair_available: bool,
    pub summary: String,
    pub blocked_capabilities: Vec<RuntimeCapability>,
    pub details: Vec<String>,
    #[serde(default)]
    pub guidance: Vec<String>,
    #[serde(default)]
    pub bootstrap_eligible: bool,
    #[serde(default)]
    pub bootstrap_required: bool,
    #[serde(default)]
    pub active_operation: Option<RuntimeOperation>,
}

/// Bytes that must be free before starting a remote bootstrap, mirrored from
/// `download.rs::required_free_bytes` (T3 status-level pre-flight). `download.rs`
/// is out of T3 scope so the 3x formula is intentionally duplicated here for
/// explanatory status only; the in-download check stays authoritative and is
/// never moved.
pub(crate) fn bootstrap_required_free_bytes(archive_size: u64) -> u64 {
    archive_size.saturating_mul(3)
}

/// Status-level disk pre-flight: returns an explanatory Spanish message when
/// `free_bytes` is known and below `required_bytes`, so the UI can explain
/// BEFORE the first byte. Returns `None` when free space is unknown (skip
/// rather than block) or sufficient. Copy mirrors the in-download error in
/// `download.rs` (`Espacio en disco insuficiente para ... Liberá espacio ...`).
pub(crate) fn disk_preflight_message(
    free_bytes: Option<u64>,
    required_bytes: u64,
) -> Option<String> {
    let free = free_bytes?;
    if free >= required_bytes {
        return None;
    }
    Some(format!(
        "Espacio en disco insuficiente para descargar el runtime de IA: se requieren ~{:.1} GB libres y hay {:.1} GB disponibles. Liberá espacio antes de iniciar la descarga.",
        required_bytes as f64 / 1_000_000_000.0,
        free as f64 / 1_000_000_000.0
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn damaged_status_can_report_blocked_capabilities() {
        let status = RuntimeStatus {
            state: RuntimeState::Damaged,
            pack_version: Some("2026.05.0".to_string()),
            repair_needed: true,
            repair_available: true,
            summary: "Runtime requiere reparación".to_string(),
            blocked_capabilities: vec![RuntimeCapability::Ocr, RuntimeCapability::Transcription],
            details: vec!["Checksum inválido en python/python.exe".to_string()],
            guidance: vec![
                "Ejecutá la reparación del runtime desde Ajustes > Dependencias.".to_string(),
            ],
            bootstrap_eligible: true,
            bootstrap_required: true,
            active_operation: Some(RuntimeOperation {
                kind: RuntimeOperationKind::Bootstrap,
                stage: RuntimeOperationStage::PlanningDownload,
                summary: "Planificando bootstrap".to_string(),
                progress_percent: Some(5),
                downloaded_bytes: None,
                total_bytes: None,
                retryable: true,
            }),
        };

        assert_eq!(status.state, RuntimeState::Damaged);
        assert!(status.repair_needed);
        assert!(status.repair_available);
        assert_eq!(
            status.blocked_capabilities,
            vec![RuntimeCapability::Ocr, RuntimeCapability::Transcription]
        );
        assert_eq!(status.guidance.len(), 1);
        assert!(status.bootstrap_eligible);
        assert!(status.bootstrap_required);
        assert_eq!(
            status
                .active_operation
                .as_ref()
                .map(|operation| &operation.stage),
            Some(&RuntimeOperationStage::PlanningDownload)
        );
    }

    #[test]
    fn blocked_source_unavailable_status_exposes_bootstrap_progress_contract() {
        let status = RuntimeStatus {
            state: RuntimeState::BlockedSourceUnavailable,
            pack_version: Some("2026.05.0".to_string()),
            repair_needed: false,
            repair_available: false,
            summary: "No hay una fuente confiable disponible".to_string(),
            blocked_capabilities: vec![RuntimeCapability::Ocr],
            details: vec!["Manifest remoto todavía no configurado".to_string()],
            guidance: vec!["Reintentá cuando EntropIA publique una fuente firmada".to_string()],
            bootstrap_eligible: false,
            bootstrap_required: true,
            active_operation: Some(RuntimeOperation {
                kind: RuntimeOperationKind::Bootstrap,
                stage: RuntimeOperationStage::Blocked,
                summary: "Bootstrap bloqueado por fuente no disponible".to_string(),
                progress_percent: None,
                downloaded_bytes: None,
                total_bytes: None,
                retryable: true,
            }),
        };

        assert_eq!(status.state, RuntimeState::BlockedSourceUnavailable);
        assert!(status.bootstrap_required);
        assert!(!status.bootstrap_eligible);
        assert_eq!(
            status
                .active_operation
                .as_ref()
                .map(|operation| operation.retryable),
            Some(true)
        );
    }
}
