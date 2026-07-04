//! ```cargo
//! [dependencies]
//! anyhow = "1.0"
//! ```

use std::fs;
use std::time::Duration;
use anyhow::{Result, anyhow, Context};

fn main() -> Result<()> {
    println!("=== 堅牢なMTU 診断ツール ===");
    
    // ping コマンドの存在確認
    if std::process::Command::new("ping").arg("-V").output().is_err() {
        return Err(anyhow!("'ping' コマンドがシステムに見つかりません"));
    }

    let interface = get_default_interface()?;
    let targets = vec!["8.8.8.8", "1.1.1.1"]; // Google, Cloudflare
    
    for target in targets {
        println!("Testing target: {}", target);
        match detect_optimal_mtu(&interface, target) {
            Ok(mtu) => println!("推奨MTU (via {} - {}): {}", interface, target, mtu),
            Err(e) => println!("診断失敗 (via {} - {}): {}", interface, target, e),
        }
    }
    Ok(())
}

fn get_default_interface() -> Result<String> {
    let content = fs::read_to_string("/proc/net/route")
        .context("Failed to read /proc/net/route")?;
    for line in content.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() > 1 && fields[1] == "00000000" {
            return Ok(fields[0].to_string());
        }
    }
    if let Ok(entries) = fs::read_dir("/sys/class/net") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name != "lo" {
                return Ok(name);
            }
        }
    }
    Err(anyhow!("アクティブなネットワークインターフェースが見つかりません"))
}

fn detect_optimal_mtu(interface: &str, target: &str) -> Result<u32> {
    let mut low = 576; 
    let mut high = 1500;
    let mut optimal = low;

    while low <= high {
        let mid = (low + high) / 2;
        let payload = mid - 28;
        
        if ping_with_retry(target, payload, 3, interface)? {
            optimal = mid;
            low = mid + 1;
        } else {
            high = mid - 1;
        }
    }
    
    Ok(optimal)
}

fn ping_with_retry(target: &str, payload_size: u32, retries: usize, interface: &str) -> Result<bool> {
    for _ in 0..retries {
        if perform_ping(target, payload_size, interface)? {
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(false)
}

// 堅牢にインターフェースを指定してpingを実行する
fn perform_ping(target: &str, payload_size: u32, interface: &str) -> Result<bool> {
    use std::process::Command;
    let output = Command::new("ping")
        .args(&[
            "-c", "1", 
            "-W", "1", 
            "-I", interface, 
            "-s", &payload_size.to_string(), 
            "-M", "do", 
            target
        ])
        .output()?;
    Ok(output.status.success())
}
