use std::process::Command;
use crate::diag::{Diagnostic, DiagnosticData, error::DiagnosticError};
use serde_json::json;

pub struct NtpTimeSyncDiagnostic;
impl Diagnostic for NtpTimeSyncDiagnostic {
    fn id(&self) -> &str { "ntp_time_sync" }
    fn run(&self) -> Result<DiagnosticData, DiagnosticError> {
        let mut ntp_active = false;
        let mut synchronized = false;
        let mut available = false;
        
        let is_systemd = std::path::Path::new("/run/systemd/system").exists();
        
        if is_systemd {
            available = true;
            if let Ok(out) = Command::new("timedatectl").arg("show").output() {
                if out.status.success() {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    for line in stdout.lines() {
                        let parts: Vec<&str> = line.split('=').collect();
                        if parts.len() == 2 {
                            let key = parts[0].trim();
                            let val = parts[1].trim();
                            if key == "NTP" {
                                ntp_active = val == "yes";
                            } else if key == "NTPSynchronized" {
                                synchronized = val == "yes";
                            }
                        }
                    }
                }
            }
        }
        
        calculate_ntp_diag(available, ntp_active, synchronized)
    }
}

fn calculate_ntp_diag(available: bool, ntp_active: bool, synchronized: bool) -> Result<DiagnosticData, DiagnosticError> {
    if !available {
        return Ok(DiagnosticData {
            raw: json!({"available": false}),
            readable: "非対応環境 (systemd非稼働のためスキップ)".to_string(),
        });
    }
    
    let sync_status = if synchronized {
        "同期済み (正常)".to_string()
    } else {
        "⚠️ 未同期 (警告)".to_string()
    };
    
    let raw = json!({
        "available": true,
        "ntp_active": ntp_active,
        "synchronized": synchronized
    });
    
    let readable = format!("NTPサービス: {} | 時刻同期状態: {}", if ntp_active { "有効" } else { "無効" }, sync_status);
        
    Ok(DiagnosticData {
        raw,
        readable,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_ntp_diag_normal() {
        let result = calculate_ntp_diag(true, true, true);
        assert!(result.is_ok());
        let data = result.unwrap();
        assert!(data.readable.contains("正常"));
    }

    #[test]
    fn test_calculate_ntp_diag_not_available() {
        let result = calculate_ntp_diag(false, false, false);
        assert!(result.is_ok());
        let data = result.unwrap();
        assert!(data.readable.contains("スキップ"));
    }
}
