//! ```cargo
//! [dependencies]
//! anyhow = "1.0"
//! ```

use std::process::Command;
use anyhow::{Result, anyhow};

// 診断トレイトの定義
trait Diagnostic {
    fn name(&self) -> &str;
    fn run(&self) -> Result<String>;
}

// ハードウェア温度診断
struct TemperatureDiagnostic;
impl Diagnostic for TemperatureDiagnostic {
    fn name(&self) -> &str { "Hardware Temp" }
    fn run(&self) -> Result<String> {
        let output = Command::new("sensors").output()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        // パッケージ温度を探す (簡易的)
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
        if fields.len() > 6 {
            Ok(format!("使用量: {} / {}", fields[2], fields[1]))
        } else {
            Err(anyhow!("free出力解析失敗"))
        }
    }
}

// 簡易ディスク容量詳細 (全パーティション)
struct DetailedDiskUsageDiagnostic;
impl Diagnostic for DetailedDiskUsageDiagnostic {
    fn name(&self) -> &str { "Detailed Disk" }
    fn run(&self) -> Result<String> {
        let output = Command::new("df").args(&["-h", "--output=source,pcent,target"]).output()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(format!("\n{}", stdout.trim()))
    }
}

// (既存のものは省略... 以下 main で連結)
// NetworkMTUDiagnostic, BluetoothDiagnostic, ServiceDiagnostic, LogErrorDiagnostic は定義済みとみなす

fn main() -> Result<()> {
    // 省略されていた既存の診断構造体もここに定義が必要
    // 実際にはこのファイルを完全なファイルとして上書きする
    
    // ... (定義を連結)
    Ok(())
}
