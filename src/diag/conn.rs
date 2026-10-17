use crate::diag::{
    error::DiagnosticError, sys_interface::SystemInterface, Diagnostic, DiagnosticData,
};
use serde_json::Value;
use std::collections::HashMap;
use std::net::Ipv6Addr;

pub struct ActiveConnectionDiagnostic<S: SystemInterface> {
    pub sys: S,
}

impl<S: SystemInterface> Diagnostic for ActiveConnectionDiagnostic<S> {
    fn id(&self) -> &str {
        "active_connections"
    }
    fn run(&self) -> Result<DiagnosticData, DiagnosticError> {
        let mut connections = Vec::new();
        for path in &["/proc/net/tcp", "/proc/net/tcp6"] {
            if let Ok(content) = self.sys.read_to_string(path) {
                for line in content.lines().skip(1) {
                    let fields: Vec<&str> = line.split_whitespace().collect();
                    if fields.len() > 3 {
                        connections.push((fields[2].to_string(), fields[3].to_string()));
                    }
                }
            }
        }

        calculate_conn_diag(connections)
    }
}

fn calculate_conn_diag(
    connections: Vec<(String, String)>,
) -> Result<DiagnosticData, DiagnosticError> {
    let mut est_count = 0;
    let mut remote_ips = HashMap::new();

    for (remote_addr, state) in connections {
        if state == "01" {
            // ESTABLISHED
            est_count += 1;
            if let Some(ip_hex) = remote_addr.split(':').next() {
                let ip = hex_to_ip(ip_hex);
                *remote_ips.entry(ip).or_insert(0) += 1;
            }
        }
    }

    let mut ip_counts: Vec<(String, usize)> = remote_ips.into_iter().collect();
    ip_counts.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    let top_dest: Vec<Value> = ip_counts
        .iter()
        .take(3)
        .map(|(ip, count)| serde_json::json!({"ip": ip, "count": count}))
        .collect();

    let raw = serde_json::json!({
        "established_count": est_count,
        "top_destinations": top_dest
    });

    let top_dest_str = ip_counts
        .iter()
        .take(3)
        .map(|(ip, count)| format!("{} ({}本)", ip, count))
        .collect::<Vec<String>>()
        .join(", ");

    let readable = if est_count == 0 {
        "確立された外部接続はありません (正常)".to_string()
    } else {
        format!(
            "確立済み接続(ESTABLISHED): {}本 | 主要接続先: {}",
            est_count,
            if top_dest_str.is_empty() {
                "なし".to_string()
            } else {
                top_dest_str
            }
        )
    };

    Ok(DiagnosticData { raw, readable })
}

fn hex_to_ip(hex_ip: &str) -> String {
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

fn parse_ipv6_hex(hex: &str) -> Option<Ipv6Addr> {
    if hex.len() != 32 {
        return None;
    }
    let mut segments = [0u16; 8];
    for i in 0..4 {
        let part_hex = &hex[i * 8..(i + 1) * 8];
        let val = u32::from_str_radix(part_hex, 16).ok()?;
        let bytes = val.to_ne_bytes();
        segments[i * 2] = u16::from_be_bytes([bytes[1], bytes[0]]);
        segments[i * 2 + 1] = u16::from_be_bytes([bytes[3], bytes[2]]);
    }
    Some(Ipv6Addr::new(
        segments[0],
        segments[1],
        segments[2],
        segments[3],
        segments[4],
        segments[5],
        segments[6],
        segments[7],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_conn_diag_normal() {
        let conns = vec![("0100007F:0050".to_string(), "01".to_string())];
        let result = calculate_conn_diag(conns);
        assert!(result.is_ok());
        let data = result.unwrap();
        assert!(data.readable.contains("1本"));
    }
}
