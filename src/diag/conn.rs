use anyhow::Result;
use std::fs;
use std::collections::HashMap;
use std::net::Ipv6Addr;
use crate::diag::Diagnostic;

// IPv6 16進文字列(32文字)をパースして標準表記にするヘルパー
fn parse_ipv6_hex(hex: &str) -> Option<Ipv6Addr> {
    if hex.len() != 32 { return None; }
    let mut segments = [0u16; 8];
    for i in 0..4 {
        let part_hex = &hex[i * 8..(i + 1) * 8];
        let val = u32::from_str_radix(part_hex, 16).ok()?;
        let bytes = val.to_ne_bytes();
        segments[i * 2] = u16::from_be_bytes([bytes[1], bytes[0]]);
        segments[i * 2 + 1] = u16::from_be_bytes([bytes[3], bytes[2]]);
    }
    Some(Ipv6Addr::new(
        segments[0], segments[1], segments[2], segments[3],
        segments[4], segments[5], segments[6], segments[7]
    ))
}

pub struct ActiveConnectionDiagnostic;
impl ActiveConnectionDiagnostic {
    fn hex_to_ip(&self, hex_ip: &str) -> String {
        if hex_ip.len() == 8 {
            let bytes = (0..4)
                .map(|i| u8::from_str_radix(&hex_ip[i * 2..i * 2 + 2], 16).unwrap_or(0))
                .collect::<Vec<u8>>();
            format!("{}.{}.{}.{}", bytes[3], bytes[2], bytes[1], bytes[0])
        } else if hex_ip.len() == 32 {
            if let Some(ipv6) = parse_ipv6_hex(hex_ip) {
                ipv6.to_string()
            } else {
                "IPv6(不正形式)".to_string()
            }
        } else {
            "不明".to_string()
        }
    }
}
impl Diagnostic for ActiveConnectionDiagnostic {
    fn name(&self) -> &str { "Active Connections" }
    fn run(&self) -> Result<String> {
        let paths = vec!["/proc/net/tcp", "/proc/net/tcp6"];
        let mut est_count = 0;
        let mut remote_ips = HashMap::new();
        
        for path in paths {
            if let Ok(content) = fs::read_to_string(path) {
                for line in content.lines().skip(1) {
                    let fields: Vec<&str> = line.split_whitespace().collect();
                    if fields.len() > 3 {
                        let state = fields[3];
                        if state == "01" { // ESTABLISHED
                            est_count += 1;
                            let remote_addr = fields[2];
                            if let Some(ip_hex) = remote_addr.split(':').next() {
                                let ip = self.hex_to_ip(ip_hex);
                                *remote_ips.entry(ip).or_insert(0) += 1;
                            }
                        }
                    }
                }
            }
        }
        
        let mut ip_counts: Vec<(String, usize)> = remote_ips.into_iter().collect();
        ip_counts.sort_by(|a, b| b.1.cmp(&a.1));
        let top_dest = ip_counts.iter()
            .take(3)
            .map(|(ip, count)| format!("{} ({}本)", ip, count))
            .collect::<Vec<String>>()
            .join(", ");
            
        if est_count == 0 {
            Ok("確立された外部接続はありません (正常)".to_string())
        } else {
            Ok(format!(
                "確立済み接続(ESTABLISHED): {}本 | 主要接続先: {}",
                est_count,
                if top_dest.is_empty() { "なし".to_string() } else { top_dest }
            ))
        }
    }
    fn report(&self) -> serde_json::Value {
        let paths = vec!["/proc/net/tcp", "/proc/net/tcp6"];
        let mut est_count = 0;
        let mut remote_ips = HashMap::new();
        
        for path in paths {
            if let Ok(content) = fs::read_to_string(path) {
                for line in content.lines().skip(1) {
                    let fields: Vec<&str> = line.split_whitespace().collect();
                    if fields.len() > 3 {
                        let state = fields[3];
                        if state == "01" { // ESTABLISHED
                            est_count += 1;
                            let remote_addr = fields[2];
                            if let Some(ip_hex) = remote_addr.split(':').next() {
                                let ip = self.hex_to_ip(ip_hex);
                                *remote_ips.entry(ip).or_insert(0) += 1;
                            }
                        }
                    }
                }
            }
        }
        
        let mut ip_counts: Vec<(String, usize)> = remote_ips.into_iter().collect();
        ip_counts.sort_by(|a, b| b.1.cmp(&a.1));
        let top_dest: Vec<serde_json::Value> = ip_counts.iter()
            .take(3)
            .map(|(ip, count)| serde_json::json!({"ip": ip, "count": count}))
            .collect();
            
        serde_json::json!({
            "name": self.name(),
            "established_count": est_count,
            "top_destinations": top_dest
        })
    }
}
