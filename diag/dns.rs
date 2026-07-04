use anyhow::Result;
use crate::diag::Diagnostic;

pub struct DnsLatencyDiagnostic;
impl Diagnostic for DnsLatencyDiagnostic {
    fn name(&self) -> &str { "DNS Latency" }
    fn run(&self) -> Result<String> {
        use std::net::ToSocketAddrs;
        use std::time::Instant;
        
        let domains = vec!["google.com", "cloudflare.com", "yahoo.co.jp"];
        let mut results = Vec::new();
        
        for domain in domains {
            let start = Instant::now();
            let host_port = format!("{}:80", domain);
            match host_port.to_socket_addrs() {
                Ok(_) => {
                    let duration = start.elapsed();
                    results.push(format!("{} ({:.1}ms)", domain, duration.as_secs_f64() * 1000.0));
                }
                Err(e) => {
                    results.push(format!("{} (失敗: {})", domain, e));
                }
            }
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
        
        Ok(format!(
            "DNSサーバー: {} / 解決時間: {}",
            if dns_servers.is_empty() { "不明".to_string() } else { dns_servers.join(", ") },
            results.join(", ")
        ))
    }
}
