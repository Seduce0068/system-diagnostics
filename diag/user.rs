use anyhow::Result;
use std::process::Command;
use crate::diag::Diagnostic;

// systemd が稼働しているかを検知するヘルパー
fn is_systemd_available() -> bool {
    std::path::Path::new("/run/systemd/system").exists()
}

// journalctl の読み取り権限があるかをチェックするヘルパー
fn has_journal_permission() -> bool {
    if let Ok(output) = Command::new("journalctl").args(&["-n", "1", "--no-pager"]).output() {
        output.status.success() && !String::from_utf8_lossy(&output.stderr).contains("No journal files were found")
    } else {
        false
    }
}

pub struct ActiveUsersSshDiagnostic;
impl Diagnostic for ActiveUsersSshDiagnostic {
    fn name(&self) -> &str { "Users & SSH Auth" }
    fn run(&self) -> Result<String> {
        let who_output = Command::new("who").output();
        let user_count = match who_output {
            Ok(out) if out.status.success() => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                stdout.trim().lines().count()
            }
            _ => 0,
        };
        
        if !is_systemd_available() {
            return Ok(format!("ログイン中のアクティブユーザー: {}名 | SSH監視: スキップ (systemd非稼働)", user_count));
        }

        if !has_journal_permission() {
            return Ok(format!("ログイン中のアクティブユーザー: {}名 | SSH監視: ⚠️ 権限不足のためスキップ", user_count));
        }
        
        let ssh_log = Command::new("journalctl")
            .args(&["-u", "ssh", "-u", "sshd", "-n", "30", "--no-pager"])
            .output();
            
        let mut fail_count = 0;
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
        
        Ok(format!("ログイン中のアクティブユーザー: {}名 | SSH認証失敗数: {}件", user_count, fail_count))
    }
}
