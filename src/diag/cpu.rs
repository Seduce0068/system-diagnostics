use crate::diag::{Diagnostic, DiagnosticData, error::DiagnosticError, sys_interface::SystemInterface};
use serde_json::json;

pub struct CpuFrequencyDiagnostic<S: SystemInterface> {
    pub sys: S,
}

impl<S: SystemInterface> Diagnostic for CpuFrequencyDiagnostic<S> {
    fn id(&self) -> &str { "cpu_frequency" }
    fn run(&self) -> Result<DiagnosticData, DiagnosticError> {
        let cpu_dir = "/sys/devices/system/cpu";
        
        let mut freq_data = Vec::new();
        let mut cores = 0;
        if let Ok(entries) = self.sys.read_dir(cpu_dir) {
            for path in entries {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name.starts_with("cpu") && name[3..].chars().all(|c| c.is_ascii_digit()) {
                    cores += 1;
                    let freq_path = path.join("cpufreq/scaling_cur_freq");
                    if let Ok(freq_str) = self.sys.read_to_string(freq_path) {
                        if let Ok(freq_khz) = freq_str.trim().parse::<u64>() {
                            freq_data.push(freq_khz);
                        }
                    }
                }
            }
        }

        let mut total_throttle_count = 0;
        if let Ok(entries) = self.sys.read_dir(cpu_dir) {
            for path in entries {
                let throttle_path = path.join("thermal_throttle/core_throttle_count");
                if let Ok(content) = self.sys.read_to_string(throttle_path) {
                    if let Ok(count) = content.trim().parse::<u64>() {
                        total_throttle_count += count;
                    }
                }
            }
        }
        
        calculate_cpu_diag(cores, freq_data, total_throttle_count)
    }
}

fn calculate_cpu_diag(cores: u64, freq_khz_list: Vec<u64>, throttle_count: u64) -> Result<DiagnosticData, DiagnosticError> {
    if cores == 0 {
        return Err(DiagnosticError::DiagnosticFailed("CPUコア情報を取得できませんでした".into()));
    }
    
    let freqs_ghz: Vec<f64> = freq_khz_list.iter().map(|&khz| khz as f64 / 1000.0 / 1000.0).collect();
    
    let avg = if freqs_ghz.is_empty() { 0.0 } else { freqs_ghz.iter().sum::<f64>() / freqs_ghz.len() as f64 };
    let max = if freqs_ghz.is_empty() { 0.0 } else { freqs_ghz.iter().cloned().fold(f64::NEG_INFINITY, f64::max) };
    let min = if freqs_ghz.is_empty() { 0.0 } else { freqs_ghz.iter().cloned().fold(f64::INFINITY, f64::min) };
        
    let raw = json!({
        "cores": cores,
        "frequency_ghz": {
            "avg": avg,
            "min": min,
            "max": max
        },
        "thermal_throttle_count": throttle_count
    });

    let readable = format!(
        "コア数: {} | 周波数: 平均 {:.2} GHz [範囲: {:.2} - {:.2} GHz] | サーマル状態: {}",
        cores, avg, min, max,
        if throttle_count > 0 {
            format!("⚠️ 熱制限(スロットリング)検知！(累積発生数: {})", throttle_count)
        } else {
            "熱制限なし (正常)".to_string()
        }
    );

    Ok(DiagnosticData {
        raw,
        readable,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    struct MockSystem {
        files: std::collections::HashMap<String, String>,
        dirs: std::collections::HashMap<String, Vec<PathBuf>>,
    }

    impl SystemInterface for MockSystem {
        fn read_to_string<P: AsRef<Path>>(&self, path: P) -> Result<String, DiagnosticError> {
            self.files.get(path.as_ref().to_str().unwrap())
                .cloned()
                .ok_or_else(|| DiagnosticError::DiagnosticFailed("File not found".into()))
        }
        fn read_dir<P: AsRef<Path>>(&self, path: P) -> Result<Vec<PathBuf>, DiagnosticError> {
            self.dirs.get(path.as_ref().to_str().unwrap())
                .cloned()
                .ok_or_else(|| DiagnosticError::DiagnosticFailed("Dir not found".into()))
        }
    }

    #[test]
    fn test_cpu_diag_mock() {
        let mut files = std::collections::HashMap::new();
        files.insert("/sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq".to_string(), "3000000".to_string());
        files.insert("/sys/devices/system/cpu/cpu1/cpufreq/scaling_cur_freq".to_string(), "3000000".to_string());
        files.insert("/sys/devices/system/cpu/cpu0/thermal_throttle/core_throttle_count".to_string(), "0".to_string());
        files.insert("/sys/devices/system/cpu/cpu1/thermal_throttle/core_throttle_count".to_string(), "0".to_string());

        let mut dirs = std::collections::HashMap::new();
        dirs.insert("/sys/devices/system/cpu".to_string(), vec![
            PathBuf::from("/sys/devices/system/cpu/cpu0"),
            PathBuf::from("/sys/devices/system/cpu/cpu1"),
        ]);

        let diag = CpuFrequencyDiagnostic { sys: MockSystem { files, dirs } };
        let result = diag.run();
        assert!(result.is_ok());
        let data = result.unwrap();
        assert_eq!(data.raw["cores"], 2);
    }
}
