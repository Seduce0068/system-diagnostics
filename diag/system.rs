use anyhow::{Result, anyhow};
use std::fs;
use crate::diag::Diagnostic;

pub struct UptimeLoadAvgDiagnostic;

impl Diagnostic for UptimeLoadAvgDiagnostic {
    fn name(&self) -> &str { "Uptime & Load" }
    fn run(&self) -> Result<String> {
        let uptime_raw = fs::read_to_string("/proc/uptime")
            .map_err(|_| anyhow!("Failed to read /proc/uptime"))?;
        let uptime_secs: f64 = uptime_raw.split_whitespace().next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);
            
        let days = (uptime_secs / 86400.0) as u32;
        let hours = ((uptime_secs % 86400.0) / 3600.0) as u32;
        let minutes = ((uptime_secs % 3600.0) / 60.0) as u32;
        
        let loadavg_raw = fs::read_to_string("/proc/loadavg")
            .map_err(|_| anyhow!("Failed to read /proc/loadavg"))?;
        let loads: Vec<&str> = loadavg_raw.split_whitespace().collect();
        if loads.len() < 3 {
            return Err(anyhow!("loadavgのフォーマットが異常です"));
        }
        
        let load_1: f64 = loads[0].parse().unwrap_or(0.0);
        let load_5: f64 = loads[1].parse().unwrap_or(0.0);
        let load_15: f64 = loads[2].parse().unwrap_or(0.0);
        
        let cpu_cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
            
        let load_status = if load_1 > cpu_cores as f64 {
            format!("⚠️ 高負荷 (1分平均: {} > コア数: {})", load_1, cpu_cores)
        } else {
            format!("正常 (1分平均: {})", load_1)
        };
        
        Ok(format!(
            "稼働時間: {}日 {}時間 {}分 | 負荷平均: {} [5分], {} [15分] | CPUステータス: {}",
            days, hours, minutes, load_5, load_15, load_status
        ))
    }
}
