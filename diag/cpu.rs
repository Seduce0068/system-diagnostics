use anyhow::{Result, anyhow};
use std::fs;
use crate::diag::Diagnostic;

pub struct CpuFrequencyDiagnostic;

impl Diagnostic for CpuFrequencyDiagnostic {
    fn name(&self) -> &str { "CPU Freq & Throttle" }
    fn run(&self) -> Result<String> {
        let mut freqs = Vec::new();
        let mut cores = 0;
        
        if let Ok(entries) = fs::read_dir("/sys/devices/system/cpu") {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.starts_with("cpu") && name_str[3..].chars().all(|c| c.is_ascii_digit()) {
                    cores += 1;
                    let freq_path = entry.path().join("cpufreq/scaling_cur_freq");
                    if let Ok(freq_str) = fs::read_to_string(freq_path) {
                        if let Ok(freq_khz) = freq_str.trim().parse::<u64>() {
                            freqs.push(freq_khz as f64 / 1000.0 / 1000.0); // GHz
                        }
                    }
                }
            }
        }
        
        if freqs.is_empty() {
            return Err(anyhow!("CPU周波数情報を取得できませんでした"));
        }
        
        let sum: f64 = freqs.iter().sum();
        let avg = sum / freqs.len() as f64;
        let max = freqs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let min = freqs.iter().cloned().fold(f64::INFINITY, f64::min);
        
        let mut throttle_detected = false;
        let mut total_throttle_count = 0;
        if let Ok(entries) = fs::read_dir("/sys/devices/system/cpu") {
            for entry in entries.flatten() {
                let path = entry.path().join("thermal_throttle/core_throttle_count");
                if let Ok(content) = fs::read_to_string(path) {
                    if let Ok(count) = content.trim().parse::<u64>() {
                        if count > 0 {
                            throttle_detected = true;
                            total_throttle_count += count;
                        }
                    }
                }
            }
        }
        
        let throttle_msg = if throttle_detected {
            format!("⚠️ 熱制限(スロットリング)検知！(累積発生数: {})", total_throttle_count)
        } else {
            "熱制限なし (正常)".to_string()
        };
        
        Ok(format!(
            "コア数: {} | 周波数: 平均 {:.2} GHz [範囲: {:.2} - {:.2} GHz] | サーマル状態: {}",
            cores, avg, min, max, throttle_msg
        ))
    }
}
