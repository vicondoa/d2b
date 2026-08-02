//! Admin-only Zone audit export support.

use d2b_audit::{ExportLine, export_segments};

/// A bounded audit-export request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditExportRequest {
    /// Target Zone name.
    pub zone: String,
    /// Optional lower segment boundary.
    pub after: Option<String>,
    /// Optional upper segment boundary.
    pub before: Option<String>,
}

/// The only grant accepted by the audit export service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditExportGrant {
    zone: String,
}

impl AuditExportGrant {
    /// Admit the admin-only session verb for one exact Zone.
    pub fn admit(is_admin: bool, zone: impl Into<String>) -> Result<Self, ZoneAuditError> {
        let zone = zone.into();
        if !is_admin {
            return Err(ZoneAuditError::AdminRequired);
        }
        if zone.is_empty() || zone.len() > 63 || !valid_name(&zone) {
            return Err(ZoneAuditError::ZoneInvalid);
        }
        Ok(Self { zone })
    }

    /// Exact Zone selected by the session grant.
    pub fn zone(&self) -> &str {
        &self.zone
    }
}

/// Audit export failure.
#[derive(Debug)]
pub enum ZoneAuditError {
    /// The caller did not hold the admin-only session verb.
    AdminRequired,
    /// The Zone name was not canonical.
    ZoneInvalid,
    /// The requested Zone did not match the grant.
    ZoneMismatch,
    /// Segment read or verification failed.
    ReadFailed,
}

impl core::fmt::Display for ZoneAuditError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::AdminRequired => "audit-export-admin-required",
            Self::ZoneInvalid => "audit-export-zone-invalid",
            Self::ZoneMismatch => "audit-export-zone-mismatch",
            Self::ReadFailed => "audit-export-read-failed",
        })
    }
}

impl std::error::Error for ZoneAuditError {}

/// Export verified NDJSON lines for the granted Zone.
pub fn export_ndjson(
    grant: &AuditExportGrant,
    request: &AuditExportRequest,
    audit_directory: impl AsRef<std::path::Path>,
) -> Result<Vec<String>, ZoneAuditError> {
    if grant.zone != request.zone {
        return Err(ZoneAuditError::ZoneMismatch);
    }
    let lines = export_segments(audit_directory).map_err(|_| ZoneAuditError::ReadFailed)?;
    Ok(lines.into_iter().map(|line| line.to_json()).collect())
}

/// Return a line as a stable export string.
pub fn render_line(line: &ExportLine) -> String {
    line.to_json()
}

fn valid_name(value: &str) -> bool {
    value.bytes().enumerate().all(|(index, byte)| {
        (byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            && (index != 0 || byte.is_ascii_lowercase())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_grant_is_admin_only_and_zone_bound() {
        assert!(matches!(
            AuditExportGrant::admit(false, "work"),
            Err(ZoneAuditError::AdminRequired)
        ));
        let grant = AuditExportGrant::admit(true, "work").unwrap();
        let request = AuditExportRequest {
            zone: "other".to_owned(),
            after: None,
            before: None,
        };
        assert!(matches!(
            export_ndjson(&grant, &request, "/nonexistent"),
            Err(ZoneAuditError::ZoneMismatch)
        ));
    }
}
