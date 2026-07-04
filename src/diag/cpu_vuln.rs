use anyhow::Result;
use std::fs;
use crate::diag::Diagnostic;

pub struct CpuVulnerabilityDiagnostic;

impl Diagnostic for CpuVulnerabilityDiagnostic {
    fn name(&self) -> &str { "CPU Vulnerabilities" }
    fn run(&self) -> Result<String> {
        let mut status = Vec::new();
        let vulns = vec!["meltdown", "spectre_v1", "spectre_v2", "mds", "l1tf"];
        
        for vuln in vulns {
            let path = format!("/sys/devices/system/cpu/vulnerabilities/{}", vuln);
            if let Ok(content) = fs::read_to_string(path) {
                let text = content.trim();
                let short_text = if text.len() > 25 {
                    format!("{}...", &text[0..22])
                } else {
                    text.to_string()
                };
                
                let is_vuln = text.to_lowercase().contains("vulnerable") || text.to_lowercase().contains("susceptible");
                let mark = if is_vuln { "⚠️" } else { "🛡️" };
                
                status.push(format!("{}{} ({})", mark, vuln, short_text));
            }
        }
        
        if status.is_empty() {
            Ok("CPU脆弱性緩和ステータスを取得できませんでした".to_string())
        } else {
            Ok(status.join(" | "))
        }
    }
    fn report(&self) -> serde_json::Value {
        let vulns = vec!["meltdown", "spectre_v1", "spectre_v2", "mds", "l1tf"];
        let mut statuses = Vec::new();
        
        for vuln in vulns {
            let path = format!("/sys/devices/system/cpu/vulnerabilities/{}", vuln);
            if let Ok(content) = fs::read_to_string(path) {
                let text = content.trim();
                let is_vuln = text.to_lowercase().contains("vulnerable") || text.to_lowercase().contains("susceptible");
                statuses.push(serde_json::json!({
                    "vulnerability": vuln,
                    "status": text,
                    "is_vulnerable": is_vuln
                }));
            }
        }
        
        serde_json::json!({
            "name": self.name(),
            "vulnerabilities": statuses
        })
    }
}
