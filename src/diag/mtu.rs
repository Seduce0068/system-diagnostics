use std::process::Command;
use crate::diag::{Diagnostic, DiagnosticData, error::DiagnosticError};
use serde_json::json;

pub struct NetworkMTUDiagnostic;
impl Diagnostic for NetworkMTUDiagnostic {
    fn id(&self) -> &str { "network_mtu" }
    fn run(&self) -> Result<DiagnosticData, DiagnosticError> {
        let interface = crate::net::get_default_interface()?;
        let target = "8.8.8.8";
        
        // ping check
        if Command::new("ping").arg("-V").output().is_err() {
            return Err(DiagnosticError::DiagnosticFailed("'ping' コマンドがシステムに見つかりません".into()));
        }

        let mut low = 576;
        let mut high = 1500;
        let mut optimal = low;
        
        while low <= high {
            let mid = (low + high) / 2;
            let payload = mid - 28;
            let output = Command::new("ping")
                .args(["-c", "1", "-W", "1", "-I", &interface, "-s", &payload.to_string(), "-M", "do", target])
                .output()
                .map_err(|e| DiagnosticError::Io(e))?;
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

fn calculate_mtu_diag(interface: String, mtu: u64) -> Result<DiagnosticData, DiagnosticError> {
    if mtu < 576 {
        return Err(DiagnosticError::DiagnosticFailed(format!("MTUの値が異常です: {}", mtu)));
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
