use anyhow::{Result, anyhow, Context};
use std::process::Command;
use crate::diag::Diagnostic;

// デフォルトゲートウェイのIPとインターフェース名を取得するヘルパー
fn get_default_gateway() -> Result<(String, String)> {
    let content = std::fs::read_to_string("/proc/net/route")
        .context("Failed to read /proc/net/route")?;
    for line in content.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() > 2 && fields[1] == "00000000" {
            let iface = fields[0].to_string();
            if let Some(gw_ip) = parse_gateway_ip(fields[2]) {
                return Ok((iface, gw_ip));
            }
        }
    }
    Err(anyhow!("デフォルトゲートウェイが見つかりません"))
}

// 16進数ゲートウェイIPのデコード
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

pub struct LocalGatewayPingDiagnostic;
impl Diagnostic for LocalGatewayPingDiagnostic {
    fn name(&self) -> &str { "Local Gateway Ping" }
    fn run(&self) -> Result<String> {
        let (interface, gateway_ip) = get_default_gateway()?;
        
        if Command::new("ping").arg("-V").output().is_err() {
            return Err(anyhow!("'ping' コマンドがシステムに見つかりません"));
        }
        
        let output = Command::new("ping")
            .args(&["-c", "3", "-W", "1", "-I", &interface, &gateway_ip])
            .output()?;
            
        let stdout = String::from_utf8_lossy(&output.stdout);
        
        let mut loss = "100%".to_string();
        for line in stdout.lines() {
            if line.contains("packet loss") {
                if let Some(pct_idx) = line.find("%") {
                    let start = line[..pct_idx].rfind(' ').unwrap_or(0);
                    loss = line[start..pct_idx+1].trim().to_string();
                }
            }
        }
        
        let mut rtt = "計測不可".to_string();
        for line in stdout.lines() {
            if line.starts_with("rtt min/avg/max") {
                let parts: Vec<&str> = line.split('=').collect();
                if parts.len() == 2 {
                    rtt = parts[1].trim().to_string();
                }
            }
        }
        
        Ok(format!(
            "ゲートウェイ: {} (via {}) | ロス: {} | RTT min/avg/max/mdev: {}",
            gateway_ip, interface, loss, rtt
        ))
    }
}
