//! ```cargo
//! [dependencies]
//! anyhow = "1.0"
//! ```

use std::process::Command;
use anyhow::{Result, anyhow};

trait Diagnostic {
    fn name(&self) -> &str;
    fn run(&self) -> Result<String>;
}

// ネットワークMTU診断
struct NetworkMTUDiagnostic;
impl Diagnostic for NetworkMTUDiagnostic {
    fn name(&self) -> &str { "Network MTU" }
    fn run(&self) -> Result<String> {
        let interface = "enp4s0";
        let target = "8.8.8.8";
        let mut low = 576;
        let mut high = 1500;
        let mut optimal = low;
        while low <= high {
            let mid = (low + high) / 2;
            let payload = mid - 28;
            let output = Command::new("ping")
                .args(&["-c", "1", "-W", "1", "-I", interface, "-s", &payload.to_string(), "-M", "do", target])
                .output()?;
            if output.status.success() { optimal = mid; low = mid + 1; } else { high = mid - 1; }
        }
        Ok(format!("推奨MTU: {}", optimal))
    }
}

// Bluetooth診断
struct BluetoothDiagnostic;
impl Diagnostic for BluetoothDiagnostic {
    fn name(&self) -> &str { "Bluetooth Status" }
    fn run(&self) -> Result<String> {
        let output = Command::new("bluetoothctl").arg("show").output()?;
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stdout.contains("Powered: yes") { Ok("正常".to_string()) } else { Ok("警告: オフ".to_string()) }
        } else { Err(anyhow!("失敗")) }
    }
}

// ハードウェア温度診断
struct TemperatureDiagnostic;
impl Diagnostic for TemperatureDiagnostic {
    fn name(&self) -> &str { "Hardware Temp" }
    fn run(&self) -> Result<String> {
        let output = Command::new("sensors").output()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let temp = stdout.lines()
            .find(|l| l.contains("Package id 0:"))
            .map(|l| l.split_whitespace().nth(3).unwrap_or("不明").to_string())
            .unwrap_or_else(|| "取得失敗".to_string());
        Ok(format!("パッケージ温度: {}", temp))
    }
}

// メモリ・ハングアップ予兆診断
struct MemoryPressureDiagnostic;
impl Diagnostic for MemoryPressureDiagnostic {
    fn name(&self) -> &str { "Memory/Hangout" }
    fn run(&self) -> Result<String> {
        let output = Command::new("free").args(&["-h"]).output()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mem_line = stdout.lines().find(|l| l.starts_with("Mem:")).unwrap_or("");
        let fields: Vec<&str> = mem_line.split_whitespace().collect();
        if fields.len() > 6 { Ok(format!("使用量: {} / {}", fields[2], fields[1])) } else { Err(anyhow!("解析失敗")) }
    }
}

// 簡易ディスク容量詳細
struct DetailedDiskUsageDiagnostic;
impl Diagnostic for DetailedDiskUsageDiagnostic {
    fn name(&self) -> &str { "Detailed Disk" }
    fn run(&self) -> Result<String> {
        let output = Command::new("df").args(&["-h", "--output=source,pcent,target"]).output()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(format!("\n{}", stdout.trim()))
    }
}

// サービス状態診断
struct ServiceDiagnostic { service_name: String }
impl Diagnostic for ServiceDiagnostic {
    fn name(&self) -> &str { "Service Status" }
    fn run(&self) -> Result<String> {
        let output = Command::new("systemctl").args(&["is-active", &self.service_name]).output()?;
        if output.status.success() { Ok(format!("{} はアクティブ", self.service_name)) } else { Ok(format!("{} は非アクティブ", self.service_name)) }
    }
}

// ログ監視診断
struct LogErrorDiagnostic;
impl Diagnostic for LogErrorDiagnostic {
    fn name(&self) -> &str { "Critical Logs" }
    fn run(&self) -> Result<String> {
        let output = Command::new("journalctl").args(&["-p", "3", "-n", "5", "--no-pager"]).output()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.trim().is_empty() { Ok("過去のクリティカルログなし".to_string()) } else { Ok(format!("エラー検知:\n{}", stdout)) }
    }
}

fn main() -> Result<()> {
    let diagnostics: Vec<Box<dyn Diagnostic>> = vec![
        Box::new(NetworkMTUDiagnostic),
        Box::new(BluetoothDiagnostic),
        Box::new(TemperatureDiagnostic),
        Box::new(MemoryPressureDiagnostic),
        Box::new(DetailedDiskUsageDiagnostic),
        Box::new(ServiceDiagnostic { service_name: "NetworkManager".to_string() }),
        Box::new(LogErrorDiagnostic),
    ];

    println!("=== 🔍 包括的システム診断フレームワーク ===");
    for d in diagnostics {
        print!("[{}] ", d.name());
        match d.run() {
            Ok(res) => println!("✅ {}", res),
            Err(e) => println!("❌ 診断失敗: {}", e),
        }
    }
    println!("=== ✅ 全ての診断完了 ===");
    Ok(())
}
