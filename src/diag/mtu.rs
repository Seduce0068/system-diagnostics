use anyhow::{Result, anyhow, Context};
use std::process::Command;
use crate::diag::{Diagnostic, DiagnosticData};
use serde_json::json;

pub struct NetworkMTUDiagnostic;
impl Diagnostic for NetworkMTUDiagnostic {
    fn id(&self) -> &str { "network_mtu" }
    fn run(&self) -> Result<DiagnosticData> {
        let interface = get_default_interface().context("Failed to get interface")?;
        let target = "8.8.8.8";
        
        // ping check
        if Command::new("ping").arg("-V").output().is_err() {
            return Err(anyhow!("'ping' コマンドがシステムに見つかりません"));
        }

        let mut low = 576;
        let mut high = 1500;
        let mut optimal = low;
        
        while low <= high {
            let mid = (low + high) / 2;
            let payload = mid - 28;
            let output = Command::new("ping")
                .args(["-c", "1", "-W", "1", "-I", &interface, "-s", &payload.to_string(), "-M", "do", target])
                .output()?;
            if output.status.success() {
                optimal = mid;
                low = mid + 1;
            } else {
                high = mid - 1;
            }
        }
        
        calculate_mtu_diag(interface, optimal)
    }
}

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

fn calculate_mtu_diag(interface: String, mtu: u64) -> Result<DiagnosticData> {
    if mtu < 576 {
        return Err(anyhow!("MTUの値が異常です: {}", mtu));
    }
    
    let raw = json!({
        "interface": interface,
        "optimal_mtu": mtu
    });

    let readable = format!("推奨MTU (via {}): {}", interface, mtu);

    Ok(DiagnosticData {
        raw,
        readable,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_mtu_diag_normal() {
        let result = calculate_mtu_diag("eth0".to_string(), 1500);
        assert!(result.is_ok());
        let data = result.unwrap();
        assert!(data.readable.contains("1500"));
    }

    #[test]
    fn test_calculate_mtu_diag_low() {
        let result = calculate_mtu_diag("eth0".to_string(), 500);
        assert!(result.is_err());
    }
}
