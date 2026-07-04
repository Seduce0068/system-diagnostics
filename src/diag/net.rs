use std::process::Command;
use crate::diag::{Diagnostic, DiagnosticData, error::DiagnosticError};
use serde_json::json;

pub struct LocalGatewayPingDiagnostic;
impl Diagnostic for LocalGatewayPingDiagnostic {
    fn id(&self) -> &str { "local_gateway_ping" }
    fn run(&self) -> Result<DiagnosticData, DiagnosticError> {
        let content = std::fs::read_to_string("/proc/net/route")
            .map_err(|e| DiagnosticError::Io(e))?;
        
        let mut iface_gw = None;
        for line in content.lines().skip(1) {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() > 2 && fields[1] == "00000000" {
                let iface = fields[0].to_string();
                if let Some(gw_ip) = parse_gateway_ip(fields[2]) {
                    iface_gw = Some((iface, gw_ip));
                    break;
                }
            }
        }
        
        let (interface, gateway_ip) = iface_gw.ok_or_else(|| DiagnosticError::DiagnosticFailed("デフォルトゲートウェイが見つかりません".into()))?;
        
        let output = Command::new("ping")
            .args(["-c", "3", "-W", "1", "-I", &interface, &gateway_ip])
            .output()
            .map_err(|e| DiagnosticError::DiagnosticFailed(format!("Ping command failed: {}", e)))?;
            
        let stdout = String::from_utf8_lossy(&output.stdout);
        
        calculate_net_diag(interface, gateway_ip, &stdout)
    }
}

fn parse_gateway_ip(hex_ip: &str) -> Option<String> {
    if hex_ip.len() == 8 && hex_ip != "00000000" {
        let bytes = (0..4)
            .map(|i| u8::from_str_radix(&hex_ip[i * 2..i * 2 + 2], 16).unwrap_or(0))
            .collect::<Vec<u8>>();
        Some(format!("{}.{}.{}.{}", bytes[3], bytes[2], bytes[1], bytes[0]))
    } else {
        None
    }
}

fn calculate_net_diag(interface: String, gateway_ip: String, ping_stdout: &str) -> Result<DiagnosticData, DiagnosticError> {
    let mut loss = "100%".to_string();
    for line in ping_stdout.lines() {
        if line.contains("packet loss") {
            if let Some(pct_idx) = line.find("%") {
                let start = line[..pct_idx].rfind(' ').unwrap_or(0);
                loss = line[start..pct_idx+1].trim().to_string();
            }
        }
    }
    
    let mut rtt_min = None;
    let mut rtt_avg = None;
    let mut rtt_max = None;
    let mut rtt_mdev = None;
    let mut rtt_str = "計測不可".to_string();
    
    for line in ping_stdout.lines() {
        if line.starts_with("rtt min/avg/max") {
            let parts: Vec<&str> = line.split('=').collect();
            if parts.len() == 2 {
                rtt_str = parts[1].trim().to_string();
                let rtt_vals: Vec<&str> = rtt_str.split('/').collect();
                if rtt_vals.len() == 4 {
                    rtt_min = rtt_vals[0].parse::<f64>().ok();
                    rtt_avg = rtt_vals[1].parse::<f64>().ok();
                    rtt_max = rtt_vals[2].parse::<f64>().ok();
                    rtt_mdev = rtt_vals[3].split(' ').next().and_then(|v| v.parse::<f64>().ok());
                }
            }
        }
    }
    
    let raw = json!({
        "gateway": gateway_ip,
        "interface": interface,
        "packet_loss": loss,
        "rtt_ms": {
            "min": rtt_min,
            "avg": rtt_avg,
            "max": rtt_max,
            "mdev": rtt_mdev
        }
    });

    let readable = format!(
        "ゲートウェイ: {} (via {}) | ロス: {} | RTT min/avg/max/mdev: {}",
        gateway_ip, interface, loss, rtt_str
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
    fn test_calculate_net_diag_normal() {
        let stdout = "--- 192.168.1.1 ping statistics ---\n3 packets transmitted, 3 received, 0% packet loss\nrtt min/avg/max/mdev = 1.0/1.5/2.0/0.5 ms";
        let result = calculate_net_diag("eth0".to_string(), "192.168.1.1".to_string(), stdout);
        assert!(result.is_ok());
        let data = result.unwrap();
        assert!(data.readable.contains("0%"));
    }

    #[test]
    fn test_calculate_net_diag_loss() {
        let stdout = "--- 192.168.1.1 ping statistics ---\n3 packets transmitted, 0 received, 100% packet loss";
        let result = calculate_net_diag("eth0".to_string(), "192.168.1.1".to_string(), stdout);
        assert!(result.is_ok());
        let data = result.unwrap();
        assert!(data.readable.contains("100%"));
    }
}
