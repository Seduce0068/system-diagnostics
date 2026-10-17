use crate::diag::{
    error::DiagnosticError, sys_interface::SystemInterface, Diagnostic, DiagnosticData,
};
use serde_json::json;

pub struct ActiveUsersSshDiagnostic<S: SystemInterface> {
    pub sys: S,
}

impl<S: SystemInterface> Diagnostic for ActiveUsersSshDiagnostic<S> {
    fn id(&self) -> &str {
        "active_ssh_users"
    }
    fn run(&self) -> Result<DiagnosticData, DiagnosticError> {
        let who_output = self.sys.run_command("who", &[]);
        let user_count = match who_output {
            Ok(stdout) => stdout.trim().lines().count(),
            _ => 0,
        };

        let mut fail_count = 0;
        let mut auth_check_status = "ok".to_string();

        let is_systemd = std::path::Path::new("/run/systemd/system").exists();
        if !is_systemd {
            auth_check_status = "not_available".to_string();
        } else if !self.has_journal_permission() {
            auth_check_status = "permission_denied".to_string();
        } else {
            let ssh_log = self.sys.run_command(
                "journalctl",
                &["-u", "ssh", "-u", "sshd", "-n", "30", "--no-pager"],
            );

            if let Ok(stdout) = ssh_log {
                for line in stdout.lines() {
                    if line.contains("Failed password") || line.contains("Invalid user") {
                        fail_count += 1;
                    }
                }
            }
        }

        calculate_user_diag(user_count, fail_count, auth_check_status)
    }
}

impl<S: SystemInterface> ActiveUsersSshDiagnostic<S> {
    fn has_journal_permission(&self) -> bool {
        self.sys
            .run_command("journalctl", &["-n", "1", "--no-pager"])
            .is_ok()
    }
}

fn calculate_user_diag(
    user_count: usize,
    fail_count: u64,
    status: String,
) -> Result<DiagnosticData, DiagnosticError> {
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

    Ok(DiagnosticData { raw, readable })
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
