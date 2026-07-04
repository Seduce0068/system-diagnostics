use anyhow::{Result, anyhow, Context};
use std::process::Command;
use crate::diag::Diagnostic;

// デフォルトネットワークインターフェースを動的に取得するヘルパー
fn get_default_interface() -> Result<String> {
    let content = std::fs::read_to_string("/proc/net/route")
        .context("Failed to read /proc/net/route")?;
    for line in content.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() > 1 && fields[1] == "00000000" {
            return Ok(fields[0].to_string());
        }
    }
    if let Ok(entries) = std::fs::read_dir("/sys/class/net") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name != "lo" {
                return Ok(name);
            }
        }
    }
    Err(anyhow!("アクティブなネットワークインターフェースが見つかりません"))
}

pub struct NetworkMTUDiagnostic;
impl Diagnostic for NetworkMTUDiagnostic {
    fn name(&self) -> &str { "Network MTU" }
    fn run(&self) -> Result<String> {
        let interface = get_default_interface()?;
        let target = "8.8.8.8";
        let mut low = 576;
        let mut high = 1500;
        let mut optimal = low;
        
        let check_ping = Command::new("ping").arg("-V").output();
        if check_ping.is_err() {
            return Err(anyhow!("'ping' コマンドがシステムに見つかりません"));
        }

        while low <= high {
            let mid = (low + high) / 2;
            let payload = mid - 28;
            let output = Command::new("ping")
                .args(&["-c", "1", "-W", "1", "-I", &interface, "-s", &payload.to_string(), "-M", "do", target])
                .output()?;
            if output.status.success() {
                optimal = mid;
                low = mid + 1;
            } else {
                high = mid - 1;
            }
        }
        Ok(format!("推奨MTU (via {}): {}", interface, optimal))
    }
    fn report(&self) -> serde_json::Value {
        let interface = get_default_interface().unwrap_or_else(|_| "unknown".to_string());
        let target = "8.8.8.8";
        let mut low = 576;
        let mut high = 1500;
        let mut optimal = low;
        
        // ping check
        if Command::new("ping").arg("-V").output().is_ok() {
            while low <= high {
                let mid = (low + high) / 2;
                let payload = mid - 28;
                if let Ok(output) = Command::new("ping")
                    .args(&["-c", "1", "-W", "1", "-I", &interface, "-s", &payload.to_string(), "-M", "do", target])
                    .output()
                {
                    if output.status.success() {
                        optimal = mid;
                        low = mid + 1;
                    } else {
                        high = mid - 1;
                    }
                } else {
                    high = mid - 1;
                }
            }
        }
        
        serde_json::json!({
            "name": self.name(),
            "interface": interface,
            "optimal_mtu": optimal
        })
    }
}
