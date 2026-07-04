use anyhow::{Result, anyhow, Context};
use std::fs;
use crate::diag::{Diagnostic, DiagnosticData};

pub struct UptimeLoadAvgDiagnostic;

impl Diagnostic for UptimeLoadAvgDiagnostic {
    fn name(&self) -> &str { "Uptime & Load" }
    fn run(&self) -> Result<DiagnosticData> {
        let uptime_raw = fs::read_to_string("/proc/uptime")
            .context("Failed to read /proc/uptime")?;
        let loadavg_raw = fs::read_to_string("/proc/loadavg")
            .context("Failed to read /proc/loadavg")?;
            
        calculate_uptime_load(&uptime_raw, &loadavg_raw)
    }
}

fn calculate_uptime_load(uptime_raw: &str, loadavg_raw: &str) -> Result<DiagnosticData> {
    let uptime_secs: f64 = uptime_raw.split_whitespace().next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| anyhow!("Failed to parse /proc/uptime"))?;
        
    let loads: Vec<&str> = loadavg_raw.split_whitespace().collect();
    if loads.len() < 3 {
        return Err(anyhow!("Invalid /proc/loadavg format"));
    }
    
    let load_1: f64 = loads[0].parse().context("Failed to parse load_1")?;
    let load_5: f64 = loads[1].parse().context("Failed to parse load_5")?;
    let load_15: f64 = loads[2].parse().context("Failed to parse load_15")?;
    
    let cpu_cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
        
    let days = (uptime_secs / 86400.0) as u32;
    let hours = ((uptime_secs % 86400.0) / 3600.0) as u32;
    let minutes = ((uptime_secs % 3600.0) / 60.0) as u32;
    
    let load_status = if load_1 > cpu_cores as f64 {
        format!("⚠️ 高負荷 (1分平均: {} > コア数: {})", load_1, cpu_cores)
    } else {
        format!("正常 (1分平均: {})", load_1)
    };
    
    Ok(DiagnosticData {
        readable: format!(
            "稼働時間: {}日 {}時間 {}分 | 負荷平均: {} [5分], {} [15分] | CPUステータス: {}",
            days, hours, minutes, load_5, load_15, load_status
        ),
        raw: serde_json::json!({
            "uptime_seconds": uptime_secs,
            "load_avg": {
                "1min": load_1,
                "5min": load_5,
                "15min": load_15
            },
            "cpu_cores": cpu_cores
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_uptime_load_normal() {
        let result = calculate_uptime_load("100.0 200.0", "0.1 0.2 0.3");
        assert!(result.is_ok());
        let data = result.unwrap();
        assert!(data.readable.contains("稼働時間"));
        assert_eq!(data.raw["load_avg"]["1min"], 0.1);
    }

    #[test]
    fn test_calculate_uptime_load_invalid_format() {
        let result = calculate_uptime_load("abc", "0.1 0.2");
        assert!(result.is_err());
    }
}
