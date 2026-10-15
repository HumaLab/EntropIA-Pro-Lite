use serde::{Deserialize, Serialize};

use crate::runtime::bootstrap_types::BootstrapUnavailableReason;

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
    #[serde(default)]
    pub blocked_reason: Option<BootstrapUnavailableReason>,
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
            blocked_reason: None,
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
            blocked_reason: None,
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
