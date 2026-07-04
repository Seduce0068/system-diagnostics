use anyhow::{Result, anyhow};
use std::fs;
use crate::diag::{Diagnostic, DiagnosticData};
use serde_json::json;

pub struct CpuVulnerabilityDiagnostic;

impl Diagnostic for CpuVulnerabilityDiagnostic {
    fn id(&self) -> &str { "cpu_vulnerability" }
    fn run(&self) -> Result<DiagnosticData> {
        let mut vuln_map = std::collections::HashMap::new();
        let vulns = vec!["meltdown", "spectre_v1", "spectre_v2", "mds", "l1tf"];
        
        for vuln in vulns {
            let path = format!("/sys/devices/system/cpu/vulnerabilities/{}", vuln);
            if let Ok(content) = fs::read_to_string(path) {
                vuln_map.insert(vuln.to_string(), content.trim().to_string());
            }
        }
        
        calculate_cpu_vuln_diag(vuln_map)
    }
}

fn calculate_cpu_vuln_diag(vuln_map: std::collections::HashMap<String, String>) -> Result<DiagnosticData> {
    if vuln_map.is_empty() {
        return Err(anyhow!("CPU脆弱性緩和ステータスを取得できませんでした"));
    }
    
    let mut status_lines = Vec::new();
    let mut statuses = Vec::new();
    
    let mut keys: Vec<_> = vuln_map.keys().collect();
    keys.sort();
    
    for vuln in keys {
        let text = vuln_map.get(vuln).unwrap();
        let short_text = if text.len() > 25 {
            format!("{}...", &text[0..22])
        } else {
            text.to_string()
        };
        
        let is_vuln = text.to_lowercase().contains("vulnerable") || text.to_lowercase().contains("susceptible");
        let mark = if is_vuln { "⚠️" } else { "🛡️" };
        
        status_lines.push(format!("{}{} ({})", mark, vuln, short_text));
        statuses.push(json!({
            "vulnerability": vuln,
            "status": text,
            "is_vulnerable": is_vuln
        }));
    }
    
    let raw = json!({
        "vulnerabilities": statuses
    });

    Ok(DiagnosticData {
        raw,
        readable: status_lines.join(" | "),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_calculate_cpu_vuln_diag_normal() {
        let mut map = HashMap::new();
        map.insert("meltdown".to_string(), "Mitigation: PTI".to_string());
        
        let result = calculate_cpu_vuln_diag(map);
        assert!(result.is_ok());
        let data = result.unwrap();
        assert!(data.readable.contains("🛡️meltdown"));
    }

    #[test]
    fn test_calculate_cpu_vuln_diag_empty() {
        let map = HashMap::new();
        let result = calculate_cpu_vuln_diag(map);
        assert!(result.is_err());
    }
}
