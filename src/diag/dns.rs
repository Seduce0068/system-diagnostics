use anyhow::{Result, anyhow};
use crate::diag::{Diagnostic, DiagnosticData};
use serde_json::json;

pub struct DnsLatencyDiagnostic;
impl Diagnostic for DnsLatencyDiagnostic {
    fn name(&self) -> &str { "DNS Latency" }
    fn run(&self) -> Result<DiagnosticData> {
        use std::net::ToSocketAddrs;
        use std::time::Instant;
        
        let domains = vec!["google.com", "cloudflare.com", "yahoo.co.jp"];
        let mut latencies = std::collections::HashMap::new();
        
        for domain in domains {
            let start = Instant::now();
            let host_port = format!("{}:80", domain);
            match host_port.to_socket_addrs() {
                Ok(_) => {
                    let ms = start.elapsed().as_secs_f64() * 1000.0;
                    latencies.insert(domain.to_string(), Some(ms));
                }
                Err(_) => {
                    latencies.insert(domain.to_string(), None);
                }
            };
        }
        
        let mut dns_servers = Vec::new();
        if let Ok(resolv) = std::fs::read_to_string("/etc/resolv.conf") {
            for line in resolv.lines() {
                if line.starts_with("nameserver") {
                    if let Some(ns) = line.split_whitespace().nth(1) {
                        dns_servers.push(ns.to_string());
                    }
                }
            }
        }
        
        calculate_dns_diag(dns_servers, latencies)
    }
}

fn calculate_dns_diag(dns_servers: Vec<String>, latencies: std::collections::HashMap<String, Option<f64>>) -> Result<DiagnosticData> {
    if latencies.is_empty() {
        return Err(anyhow!("DNS解決結果がありません"));
    }
    
    let mut results = Vec::new();
    let mut domain_latencies = Vec::new();
    
    let mut domains: Vec<_> = latencies.keys().collect();
    domains.sort();
    
    for domain in domains {
        let latency = latencies.get(domain).unwrap();
        match latency {
            Some(ms) => {
                results.push(format!("{} ({:.1}ms)", domain, ms));
            }
            None => {
                results.push(format!("{} (失敗)", domain));
            }
        }
        domain_latencies.push(json!({
            "domain": domain,
            "latency_ms": latency
        }));
    }
    
    let raw = json!({
        "dns_servers": dns_servers,
        "latencies": domain_latencies
    });

    let readable = format!(
        "DNSサーバー: {} / 解決時間: {}",
        if dns_servers.is_empty() { "不明".to_string() } else { dns_servers.join(", ") },
        results.join(", ")
    );

    Ok(DiagnosticData {
        raw,
        readable,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_calculate_dns_diag_normal() {
        let mut latencies = HashMap::new();
        latencies.insert("google.com".to_string(), Some(10.0));
        
        let result = calculate_dns_diag(vec!["8.8.8.8".to_string()], latencies);
        assert!(result.is_ok());
        let data = result.unwrap();
        assert!(data.readable.contains("10.0ms"));
    }

    #[test]
    fn test_calculate_dns_diag_empty() {
        let latencies = HashMap::new();
        let result = calculate_dns_diag(vec![], latencies);
        assert!(result.is_err());
    }
}
