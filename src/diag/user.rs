use anyhow::Result;
use std::process::Command;
use crate::diag::{Diagnostic, DiagnosticData};
use serde_json::json;

pub struct ActiveUsersSshDiagnostic;
impl Diagnostic for ActiveUsersSshDiagnostic {
    fn id(&self) -> &str { "active_ssh_users" }
    fn run(&self) -> Result<DiagnosticData> {
        let who_output = Command::new("who").output();
        let user_count = match who_output {
            Ok(out) if out.status.success() => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                stdout.trim().lines().count()
            }
            _ => 0,
        };
        
        let mut fail_count = 0;
        let mut auth_check_status = "ok".to_string();
        
        let is_systemd = std::path::Path::new("/run/systemd/system").exists();
        if !is_systemd {
            auth_check_status = "not_available".to_string();
        } else if !has_journal_permission() {
            auth_check_status = "permission_denied".to_string();
        } else {
            let ssh_log = Command::new("journalctl")
                .args(["-u", "ssh", "-u", "sshd", "-n", "30", "--no-pager"])
                .output();
                
            if let Ok(out) = ssh_log {
                if out.status.success() {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    for line in stdout.lines() {
                        if line.contains("Failed password") || line.contains("Invalid user") {
                            fail_count += 1;
                        }
                    }
                }
            }
        }
        
        calculate_user_diag(user_count, fail_count, auth_check_status)
    }
}

fn has_journal_permission() -> bool {
    if let Ok(output) = Command::new("journalctl").args(["-n", "1", "--no-pager"]).output() {
        output.status.success() && !String::from_utf8_lossy(&output.stderr).contains("No journal files were found")
    } else {
        false
    }
}

fn calculate_user_diag(user_count: usize, fail_count: u64, status: String) -> Result<DiagnosticData> {
    let raw = json!({
        "active_users": user_count,
        "ssh_auth_failures": fail_count,
        "ssh_auth_status": status
    });
    
    let readable = format!(
        "ログイン中のアクティブユーザー: {}名 | SSH認証失敗数: {}件 {}",
        user_count,
        fail_count,
        match status.as_str() {
            "not_available" => "| SSH監視: スキップ (systemd非稼働)".to_string(),
            "permission_denied" => "| SSH監視: ⚠️ 権限不足のためスキップ".to_string(),
            _ => "".to_string(),
        }
    );
    
    Ok(DiagnosticData {
        raw,
        readable,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_user_diag_normal() {
        let result = calculate_user_diag(1, 0, "ok".to_string());
        assert!(result.is_ok());
        let data = result.unwrap();
        assert!(data.readable.contains("1名"));
    }
}
