//! ```cargo
//! [dependencies]
//! anyhow = "1.0"
//! socket2 = "0.5"
//! libc = "0.2"
//! ```

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;
use anyhow::{Result, anyhow};
use socket2::{Socket, Domain, Type, Protocol};

fn main() -> Result<()> {
    println!("=== 堅牢なMTU 診断ツール ===");
    let interface = "enp4s0";
    let targets = vec!["8.8.8.8", "1.1.1.1"]; // Google, Cloudflare
    
    for target in targets {
        println!("Testing target: {}", target);
        match detect_optimal_mtu(interface, target) {
            Ok(mtu) => println!("推奨MTU (via {}): {}", target, mtu),
            Err(e) => println!("診断失敗 (via {}): {}", target, e),
        }
    }
    Ok(())
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
