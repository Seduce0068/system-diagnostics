use anyhow::Result;
use std::process::Command;
use crate::diag::Diagnostic;

// systemd が稼働しているかを検知するヘルパー
fn is_systemd_available() -> bool {
    std::path::Path::new("/run/systemd/system").exists()
}

pub struct NtpTimeSyncDiagnostic;
impl Diagnostic for NtpTimeSyncDiagnostic {
    fn name(&self) -> &str { "Time Sync (NTP)" }
    fn run(&self) -> Result<String> {
        if !is_systemd_available() {
            return Ok("非対応環境 (systemd非稼働のためスキップ)".to_string());
        }
        
        let output = Command::new("timedatectl").arg("show").output();
        match output {
            Ok(out) if out.status.success() => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let mut ntp_active = "unknown";
                let mut synchronized = "unknown";
                
                for line in stdout.lines() {
                    let parts: Vec<&str> = line.split('=').collect();
                    if parts.len() == 2 {
                        let key = parts[0].trim();
                        let val = parts[1].trim();
                        if key == "NTP" {
                            ntp_active = val;
                        } else if key == "NTPSynchronized" {
                            synchronized = val;
                        }
                    }
                }
                
                let sync_status = if synchronized == "yes" {
                    "同期済み (正常)".to_string()
                } else {
                    "⚠️ 未同期 (警告)".to_string()
                };
                
                Ok(format!("NTPサービス: {} | 時刻同期状態: {}", ntp_active, sync_status))
            }
            _ => Ok("警告: timedatectl からNTP状態を取得できませんでした".to_string())
        }
    }
    fn report(&self) -> serde_json::Value {
        let mut ntp_active = false;
        let mut synchronized = false;
        let mut available = false;
        
        if is_systemd_available() {
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
        
        serde_json::json!({
            "name": self.name(),
            "available": available,
            "ntp_active": ntp_active,
            "synchronized": synchronized
        })
    }
}
