//! ```cargo
//! [dependencies]
//! anyhow = "1.0"
//! ```

use std::fs;
use std::process::Command;
use anyhow::{Result, anyhow, Context};

trait Diagnostic {
    fn name(&self) -> &str;
    fn run(&self) -> Result<String>;
}

// 16進数ゲートウェイIPのデコード
fn parse_gateway_ip(hex_ip: &str) -> Option<String> {
    if hex_ip.len() == 8 && hex_ip != "00000000" {
        let bytes = (0..4)
            .map(|i| u8::from_str_radix(&hex_ip[i * 2..i * 2 + 2], 16).unwrap_or(0))
            .collect::<Vec<u8>>();
        Some(format!("{}.{}.{}.{}", bytes[3], bytes[2], bytes[1], bytes[0]))
    } else {
        None
    }
}

// デフォルトゲートウェイのIPとインターフェース名を取得するヘルパー
fn get_default_gateway() -> Result<(String, String)> {
    let content = fs::read_to_string("/proc/net/route")
        .context("Failed to read /proc/net/route")?;
    for line in content.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() > 2 && fields[1] == "00000000" {
            let iface = fields[0].to_string();
            if let Some(gw_ip) = parse_gateway_ip(fields[2]) {
                return Ok((iface, gw_ip));
            }
        }
    }
    Err(anyhow!("デフォルトゲートウェイが見つかりません"))
}

// デフォルトネットワークインターフェースを動的に取得するヘルパー
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

// systemd が稼働しているかを検知するヘルパー
fn is_systemd_available() -> bool {
    std::path::Path::new("/run/systemd/system").exists()
}

// journalctl の読み取り権限があるかをチェックするヘルパー
fn has_journal_permission() -> bool {
    if let Ok(output) = Command::new("journalctl").args(&["-n", "1", "--no-pager"]).output() {
        output.status.success() && !String::from_utf8_lossy(&output.stderr).contains("No journal files were found")
    } else {
        false
    }
}

// 1. 稼働時間＆システム負荷診断
struct UptimeLoadAvgDiagnostic;
impl Diagnostic for UptimeLoadAvgDiagnostic {
    fn name(&self) -> &str { "Uptime & Load" }
    fn run(&self) -> Result<String> {
        let uptime_raw = fs::read_to_string("/proc/uptime")
            .context("Failed to read /proc/uptime")?;
        let uptime_secs: f64 = uptime_raw.split_whitespace().next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);
            
        let days = (uptime_secs / 86400.0) as u32;
        let hours = ((uptime_secs % 86400.0) / 3600.0) as u32;
        let minutes = ((uptime_secs % 3600.0) / 60.0) as u32;
        
        let loadavg_raw = fs::read_to_string("/proc/loadavg")
            .context("Failed to read /proc/loadavg")?;
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

// 2. CPU周波数＆スロットリング診断
struct CpuFrequencyDiagnostic;
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

// 3. CPU脆弱性緩和ステータス監査
struct CpuVulnerabilityDiagnostic;
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
}

// 4. ネットワークMTU診断
struct NetworkMTUDiagnostic;
impl Diagnostic for NetworkMTUDiagnostic {
    fn name(&self) -> &str { "Network MTU" }
    fn run(&self) -> Result<String> {
        let interface = get_default_interface()?;
        let target = "8.8.8.8";
        let mut low = 576;
        let mut high = 1500;
        let mut optimal = low;
        
        let check_ping = Command::new("ping").arg("-V").output();
        if check_ping.is_err() {
            return Err(anyhow!("'ping' コマンドがシステムに見つかりません"));
        }

        while low <= high {
            let mid = (low + high) / 2;
            let payload = mid - 28;
            let output = Command::new("ping")
                .args(&["-c", "1", "-W", "1", "-I", &interface, "-s", &payload.to_string(), "-M", "do", target])
                .output()?;
            if output.status.success() {
                optimal = mid;
                low = mid + 1;
            } else {
                high = mid - 1;
            }
        }
        Ok(format!("推奨MTU (via {}): {}", interface, optimal))
    }
}

// 5. ローカルゲートウェイ応答＆パケットロス診断
struct LocalGatewayPingDiagnostic;
impl Diagnostic for LocalGatewayPingDiagnostic {
    fn name(&self) -> &str { "Local Gateway Ping" }
    fn run(&self) -> Result<String> {
        let (interface, gateway_ip) = get_default_gateway()?;
        
        if Command::new("ping").arg("-V").output().is_err() {
            return Err(anyhow!("'ping' コマンドがシステムに見つかりません"));
        }
        
        let output = Command::new("ping")
            .args(&["-c", "3", "-W", "1", "-I", &interface, &gateway_ip])
            .output()?;
            
        let stdout = String::from_utf8_lossy(&output.stdout);
        
        let mut loss = "100%".to_string();
        for line in stdout.lines() {
            if line.contains("packet loss") {
                if let Some(pct_idx) = line.find("%") {
                    let start = line[..pct_idx].rfind(' ').unwrap_or(0);
                    loss = line[start..pct_idx+1].trim().to_string();
                }
            }
        }
        
        let mut rtt = "計測不可".to_string();
        for line in stdout.lines() {
            if line.starts_with("rtt min/avg/max") {
                let parts: Vec<&str> = line.split('=').collect();
                if parts.len() == 2 {
                    rtt = parts[1].trim().to_string();
                }
            }
        }
        
        Ok(format!(
            "ゲートウェイ: {} (via {}) | ロス: {} | RTT min/avg/max/mdev: {}",
            gateway_ip, interface, loss, rtt
        ))
    }
}

// 6. DNS名前解決遅延診断
struct DnsLatencyDiagnostic;
impl Diagnostic for DnsLatencyDiagnostic {
    fn name(&self) -> &str { "DNS Latency" }
    fn run(&self) -> Result<String> {
        use std::net::ToSocketAddrs;
        use std::time::Instant;
        
        let domains = vec!["google.com", "cloudflare.com", "yahoo.co.jp"];
        let mut results = Vec::new();
        
        for domain in domains {
            let start = Instant::now();
            let host_port = format!("{}:80", domain);
            match host_port.to_socket_addrs() {
                Ok(_) => {
                    let duration = start.elapsed();
                    results.push(format!("{} ({:.1}ms)", domain, duration.as_secs_f64() * 1000.0));
                }
                Err(e) => {
                    results.push(format!("{} (失敗: {})", domain, e));
                }
            }
        }
        
        let mut dns_servers = Vec::new();
        if let Ok(resolv) = fs::read_to_string("/etc/resolv.conf") {
            for line in resolv.lines() {
                if line.starts_with("nameserver") {
                    if let Some(ns) = line.split_whitespace().nth(1) {
                        dns_servers.push(ns.to_string());
                    }
                }
            }
        }
        
        Ok(format!(
            "DNSサーバー: {} / 解決時間: {}",
            if dns_servers.is_empty() { "不明".to_string() } else { dns_servers.join(", ") },
            results.join(", ")
        ))
    }
}

// 7. 時刻同期＆NTPステータス診断
struct NtpTimeSyncDiagnostic;
impl Diagnostic for NtpTimeSyncDiagnostic {
    fn name(&self) -> &str { "Time Sync (NTP)" }
    fn run(&self) -> Result<String> {
        if !is_systemd_available() {
            return Ok("非対応環境 (systemd非稼働のためスキップ)".to_string());
        }
        
        let output = Command::new("timedatectl").arg("show").output();
        match output {
            Ok(out) if out.status.success() => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let mut ntp_active = "unknown";
                let mut synchronized = "unknown";
                
                for line in stdout.lines() {
                    let parts: Vec<&str> = line.split('=').collect();
                    if parts.len() == 2 {
                        let key = parts[0].trim();
                        let val = parts[1].trim();
                        if key == "NTP" {
                            ntp_active = val;
                        } else if key == "NTPSynchronized" {
                            synchronized = val;
                        }
                    }
                }
                
                let sync_status = if synchronized == "yes" {
                    "同期済み (正常)".to_string()
                } else {
                    "⚠️ 未同期 (警告)".to_string()
                };
                
                Ok(format!("NTPサービス: {} | 時刻同期状態: {}", ntp_active, sync_status))
            }
            _ => Ok("警告: timedatectl からNTP状態を取得できませんでした".to_string())
        }
    }
}

// IPv6 16進文字列(32文字)をパースして標準表記にするヘルパー
fn parse_ipv6_hex(hex: &str) -> Option<std::net::Ipv6Addr> {
    if hex.len() != 32 { return None; }
    let mut segments = [0u16; 8];
    for i in 0..4 {
        let part_hex = &hex[i * 8..(i + 1) * 8];
        let val = u32::from_str_radix(part_hex, 16).ok()?;
        let bytes = val.to_ne_bytes();
        segments[i * 2] = u16::from_be_bytes([bytes[1], bytes[0]]);
        segments[i * 2 + 1] = u16::from_be_bytes([bytes[3], bytes[2]]);
    }
    Some(std::net::Ipv6Addr::new(
        segments[0], segments[1], segments[2], segments[3],
        segments[4], segments[5], segments[6], segments[7]
    ))
}

// 8. 確立済みのアクティブ接続統計診断
struct ActiveConnectionDiagnostic;
impl ActiveConnectionDiagnostic {
    fn hex_to_ip(&self, hex_ip: &str) -> String {
        if hex_ip.len() == 8 {
            let bytes = (0..4)
                .map(|i| u8::from_str_radix(&hex_ip[i * 2..i * 2 + 2], 16).unwrap_or(0))
                .collect::<Vec<u8>>();
            format!("{}.{}.{}.{}", bytes[3], bytes[2], bytes[1], bytes[0])
        } else if hex_ip.len() == 32 {
            if let Some(ipv6) = parse_ipv6_hex(hex_ip) {
                ipv6.to_string()
            } else {
                "IPv6(不正形式)".to_string()
            }
        } else {
            "不明".to_string()
        }
    }
}
impl Diagnostic for ActiveConnectionDiagnostic {
    fn name(&self) -> &str { "Active Connections" }
    fn run(&self) -> Result<String> {
        let paths = vec!["/proc/net/tcp", "/proc/net/tcp6"];
        let mut est_count = 0;
        let mut remote_ips = std::collections::HashMap::new();
        
        for path in paths {
            if let Ok(content) = fs::read_to_string(path) {
                for line in content.lines().skip(1) {
                    let fields: Vec<&str> = line.split_whitespace().collect();
                    if fields.len() > 3 {
                        let state = fields[3];
                        if state == "01" { // ESTABLISHED
                            est_count += 1;
                            let remote_addr = fields[2];
                            if let Some(ip_hex) = remote_addr.split(':').next() {
                                let ip = self.hex_to_ip(ip_hex);
                                *remote_ips.entry(ip).or_insert(0) += 1;
                            }
                        }
                    }
                }
            }
        }
        
        let mut ip_counts: Vec<(String, usize)> = remote_ips.into_iter().collect();
        ip_counts.sort_by(|a, b| b.1.cmp(&a.1));
        let top_dest = ip_counts.iter()
            .take(3)
            .map(|(ip, count)| format!("{} ({}本)", ip, count))
            .collect::<Vec<String>>()
            .join(", ");
            
        if est_count == 0 {
            Ok("確立された外部接続はありません (正常)".to_string())
        } else {
            Ok(format!(
                "確立済み接続(ESTABLISHED): {}本 | 主要接続先: {}",
                est_count,
                if top_dest.is_empty() { "なし".to_string() } else { top_dest }
            ))
        }
    }
}

// 9. ログイン中のユーザー＆SSH認証失敗スキャン
struct ActiveUsersSshDiagnostic;
impl Diagnostic for ActiveUsersSshDiagnostic {
    fn name(&self) -> &str { "Users & SSH Auth" }
    fn run(&self) -> Result<String> {
        let who_output = Command::new("who").output();
        let user_count = match who_output {
            Ok(out) if out.status.success() => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                stdout.trim().lines().count()
            }
            _ => 0,
        };
        
        if !is_systemd_available() {
            return Ok(format!("ログイン中のアクティブユーザー: {}名 | SSH監視: スキップ (systemd非稼働)", user_count));
        }

        if !has_journal_permission() {
            return Ok(format!("ログイン中のアクティブユーザー: {}名 | SSH監視: ⚠️ 権限不足のためスキップ", user_count));
        }
        
        let ssh_log = Command::new("journalctl")
            .args(&["-u", "ssh", "-u", "sshd", "-n", "30", "--no-pager"])
            .output();
            
        let mut fail_count = 0;
        if let Ok(out) = ssh_log {
            if out.status.success() {
                let stdout = String::from_utf8_lossy(&out.stdout);
                for line in stdout.lines() {
                    if line.contains("Failed password") || line.contains("Invalid user") {
                        fail_count += 1;
                    }
                }
            }
        }
        
        let ssh_msg = if fail_count > 0 {
            format!("⚠️ 直近の認証失敗ログ検知: {}件", fail_count)
        } else {
            "SSH認証失敗なし (正常)".to_string()
        };
        
        Ok(format!("ログイン中のアクティブユーザー: {}名 | {}", user_count, ssh_msg))
    }
}

// 10. ネットワークI/Fエラー・ドロップ診断
struct NetworkInterfaceErrorDiagnostic;
impl Diagnostic for NetworkInterfaceErrorDiagnostic {
    fn name(&self) -> &str { "Network Errors" }
    fn run(&self) -> Result<String> {
        let content = fs::read_to_string("/proc/net/dev")
            .context("Failed to read /proc/net/dev")?;
        let mut alerts = Vec::new();
        
        for line in content.lines().skip(2) {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() == 2 {
                let iface = parts[0].trim();
                if iface == "lo" { continue; }
                
                let stats: Vec<&str> = parts[1].split_whitespace().collect();
                if stats.len() >= 12 {
                    let rx_err: u64 = stats[2].parse().unwrap_or(0);
                    let rx_drop: u64 = stats[3].parse().unwrap_or(0);
                    let tx_err: u64 = stats[10].parse().unwrap_or(0);
                    let tx_drop: u64 = stats[11].parse().unwrap_or(0);
                    
                    if rx_err > 0 || rx_drop > 0 || tx_err > 0 || tx_drop > 0 {
                        alerts.push(format!(
                            "{}(Rx-err:{}/Rx-drop:{} | Tx-err:{}/Tx-drop:{})",
                            iface, rx_err, rx_drop, tx_err, tx_drop
                        ));
                    }
                }
            }
        }
        
        if alerts.is_empty() {
            Ok("すべてのネットワークインターフェースでエラー・ドロップなし (正常)".to_string())
        } else {
            Ok(format!("⚠️ パケットエラー/ドロップ検知: {}", alerts.join(", ")))
        }
    }
}

// 11. OSセキュリティモジュール監査 (SELinux & AppArmor)
struct SecurityModuleDiagnostic;
impl Diagnostic for SecurityModuleDiagnostic {
    fn name(&self) -> &str { "Security Modules" }
    fn run(&self) -> Result<String> {
        let mut status = Vec::new();
        
        if let Ok(enforce) = fs::read_to_string("/sys/fs/selinux/enforce") {
            let mode = if enforce.trim() == "1" { "Enforcing" } else { "Permissive/Disabled" };
            status.push(format!("SELinux: {}", mode));
        } else {
            status.push("SELinux: 非アクティブ".to_string());
        }
        
        if let Ok(enabled) = fs::read_to_string("/sys/module/apparmor/parameters/enabled") {
            let mode = if enabled.trim() == "Y" { "Enabled" } else { "Disabled" };
            status.push(format!("AppArmor: {}", mode));
        } else {
            status.push("AppArmor: 非アクティブ".to_string());
        }
        
        Ok(status.join(" | "))
    }
}

// 12. Bluetooth診断
struct BluetoothDiagnostic;
impl Diagnostic for BluetoothDiagnostic {
    fn name(&self) -> &str { "Bluetooth Status" }
    fn run(&self) -> Result<String> {
        let output = Command::new("bluetoothctl").arg("show").output();
        match output {
            Ok(out) => {
                if out.status.success() {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    if stdout.contains("Powered: yes") {
                        Ok("正常 (有効)".to_string())
                    } else {
                        Ok("警告: オフ (無効)".to_string())
                    }
                } else {
                    Err(anyhow!("bluetoothctl がエラーを返しました"))
                }
            }
            Err(_) => Ok("Bluetoothモジュール非搭載またはツール未インストール".to_string()),
        }
    }
}

// 13. USB/PCI デバイス接続状態診断
struct HardwareBusDiagnostic;
impl Diagnostic for HardwareBusDiagnostic {
    fn name(&self) -> &str { "Hardware Bus" }
    fn run(&self) -> Result<String> {
        let mut usb_devices = Vec::new();
        let mut pci_count = 0;
        
        if let Ok(entries) = fs::read_dir("/sys/bus/usb/devices") {
            for entry in entries.flatten() {
                let path = entry.path();
                let product_path = path.join("product");
                let manufacturer_path = path.join("manufacturer");
                
                if product_path.exists() {
                    let prod = fs::read_to_string(product_path)
                        .map(|s| s.trim().to_string())
                        .unwrap_or_default();
                    let manu = fs::read_to_string(manufacturer_path)
                        .map(|s| s.trim().to_string())
                        .unwrap_or_default();
                        
                    if !prod.is_empty() {
                        let name = if manu.is_empty() { prod } else { format!("{} {}", manu, prod) };
                        usb_devices.push(name);
                    }
                }
            }
        }
        
        if let Ok(entries) = fs::read_dir("/sys/bus/pci/devices") {
            pci_count = entries.count();
        }
        
        usb_devices.sort();
        usb_devices.dedup();
        
        let usb_summary = if usb_devices.is_empty() {
            "USBデバイスなし".to_string()
        } else {
            let total = usb_devices.len();
            usb_devices.truncate(3);
            let display = usb_devices.join(", ");
            if total > 3 {
                format!("USBデバイス {}件 ({} など)", total, display)
            } else {
                format!("USBデバイス {}件 ({})", total, display)
            }
        };
        
        Ok(format!("PCIデバイス: {}件 | USBバス: {}", pci_count, usb_summary))
    }
}

// 14. バッテリ劣化＆給電状態診断
struct BatteryPowerDiagnostic;
impl Diagnostic for BatteryPowerDiagnostic {
    fn name(&self) -> &str { "Battery & Power" }
    fn run(&self) -> Result<String> {
        let mut power_status = "不明".to_string();
        let mut battery_msg = "バッテリー非搭載または検知不可".to_string();
        
        if let Ok(entries) = fs::read_dir("/sys/class/power_supply") {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                
                if name_str.starts_with("AC") || name_str.starts_with("ADP") {
                    if let Ok(online_str) = fs::read_to_string(path.join("online")) {
                        power_status = if online_str.trim() == "1" {
                            "AC電源接続中".to_string()
                        } else {
                            "バッテリー駆動中".to_string()
                        };
                    }
                } else if name_str.starts_with("BAT") {
                    let cap_str = fs::read_to_string(path.join("capacity"))
                        .unwrap_or_default();
                    let cap = cap_str.trim();
                    
                    let full = fs::read_to_string(path.join("energy_full"))
                        .or_else(|_| fs::read_to_string(path.join("charge_full")))
                        .ok()
                        .and_then(|s| s.trim().parse::<f64>().ok());
                        
                    let design = fs::read_to_string(path.join("energy_full_design"))
                        .or_else(|_| fs::read_to_string(path.join("charge_full_design")))
                        .ok()
                        .and_then(|s| s.trim().parse::<f64>().ok());
                        
                    let health = match (full, design) {
                        (Some(f), Some(d)) if d > 0.0 => {
                            format!("{:.1}%", (f / d) * 100.0)
                        }
                        _ => "不明".to_string(),
                    };
                    
                    battery_msg = format!("バッテリー残量: {}% | 健康度: {}", cap, health);
                }
            }
        }
        
        Ok(format!("電源状態: {} | {}", power_status, battery_msg))
    }
}

// 15. ハードウェア温度診断 (/sys/class/hwmon から直接読取)
struct TemperatureDiagnostic;
impl Diagnostic for TemperatureDiagnostic {
    fn name(&self) -> &str { "Hardware Temp" }
    fn run(&self) -> Result<String> {
        let mut best_temp = None;
        let mut fallback_temp = None;

        if let Ok(entries) = fs::read_dir("/sys/class/hwmon") {
            for entry in entries.flatten() {
                let path = entry.path();
                let name_path = path.join("name");
                let sensor_name = fs::read_to_string(name_path)
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default();

                if let Ok(sub_entries) = fs::read_dir(&path) {
                    for sub_entry in sub_entries.flatten() {
                        let sub_path = sub_entry.path();
                        if let Some(filename) = sub_path.file_name().and_then(|n| n.to_str()) {
                            if filename.starts_with("temp") && filename.ends_with("_input") {
                                if let Ok(temp_str) = fs::read_to_string(&sub_path) {
                                    if let Ok(temp_raw) = temp_str.trim().parse::<i32>() {
                                        let temp_c = temp_raw as f32 / 1000.0;
                                        
                                        if sensor_name == "k10temp" || sensor_name == "coretemp" {
                                            if best_temp.is_none() || filename == "temp1_input" {
                                                best_temp = Some((sensor_name.clone(), filename.to_string(), temp_c));
                                            }
                                        } else if fallback_temp.is_none() {
                                            fallback_temp = Some((sensor_name.clone(), filename.to_string(), temp_c));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        if let Some((name, file, temp)) = best_temp.or(fallback_temp) {
            Ok(format!("{} ({}): {:.1}°C", name, file, temp))
        } else {
            Err(anyhow!("温度センサー情報が見つかりません"))
        }
    }
}

// 16. メモリ・ハングアップ予兆診断 (/proc/meminfo から直接読取)
struct MemoryPressureDiagnostic;
impl Diagnostic for MemoryPressureDiagnostic {
    fn name(&self) -> &str { "Memory/Hangout" }
    fn run(&self) -> Result<String> {
        let content = fs::read_to_string("/proc/meminfo")
            .context("Failed to read /proc/meminfo")?;
        let mut mem_total = None;
        let mut mem_available = None;

        for line in content.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                if parts[0] == "MemTotal:" {
                    mem_total = parts[1].parse::<u64>().ok();
                } else if parts[0] == "MemAvailable:" {
                    mem_available = parts[1].parse::<u64>().ok();
                }
            }
        }

        if let (Some(total), Some(avail)) = (mem_total, mem_available) {
            let used = total - avail;
            let total_gb = total as f64 / 1024.0 / 1024.0;
            let used_gb = used as f64 / 1024.0 / 1024.0;
            Ok(format!("使用量: {:.2} GB / {:.2} GB ({:.1}%)", used_gb, total_gb, (used as f64 / total as f64) * 100.0))
        } else {
            Err(anyhow!("メモリ情報のパースに失敗しました"))
        }
    }
}

// ドロップ時に確実に一時ファイルを削除するクリーンアップ構造体
struct TempFileCleaner {
    path: std::path::PathBuf,
}
impl Drop for TempFileCleaner {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

// 17. ディスクI/Oパフォーマンス診断
struct DiskIoPerformanceDiagnostic;
impl Diagnostic for DiskIoPerformanceDiagnostic {
    fn name(&self) -> &str { "Disk I/O Performance" }
    fn run(&self) -> Result<String> {
        use std::fs::OpenOptions;
        use std::io::{Write, Read};
        use std::time::Instant;
        
        let temp_dir = std::env::temp_dir();
        let test_file_path = temp_dir.join("system_diag_io_test_temp.bin");
        
        let _cleaner = TempFileCleaner { path: test_file_path.clone() };
        
        let data_size = 8 * 1024 * 1024; // 8MB
        let data = vec![0u8; data_size];
        
        let start_write = Instant::now();
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&test_file_path)
            .context("テスト用一時ファイルのオープンに失敗しました")?;
            
        file.write_all(&data)?;
        file.sync_all()?;
        let write_duration = start_write.elapsed().as_secs_f64();
        let write_speed = (data_size as f64 / 1024.0 / 1024.0) / write_duration;
        
        let start_read = Instant::now();
        let mut file_read = fs::File::open(&test_file_path)?;
        let mut buffer = vec![0u8; data_size];
        file_read.read_exact(&mut buffer)?;
        let read_duration = start_read.elapsed().as_secs_f64();
        let read_speed = (data_size as f64 / 1024.0 / 1024.0) / read_duration;
        
        Ok(format!(
            "書き込み: {:.1} MB/s ({:.3}s) | 読み込み: {:.1} MB/s ({:.3}s) [TempDir]",
            write_speed, write_duration, read_speed, read_duration
        ))
    }
}

// 18. スワップ圧迫度＆OOM Killerプロセス殺戮履歴スキャン
struct SwapOomDiagnostic;
impl Diagnostic for SwapOomDiagnostic {
    fn name(&self) -> &str { "Swap & OOM Killer" }
    fn run(&self) -> Result<String> {
        let content = fs::read_to_string("/proc/meminfo")
            .context("Failed to read /proc/meminfo")?;
        let mut swap_total = None;
        let mut swap_free = None;
        
        for line in content.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                if parts[0] == "SwapTotal:" {
                    swap_total = parts[1].parse::<u64>().ok();
                } else if parts[0] == "SwapFree:" {
                    swap_free = parts[1].parse::<u64>().ok();
                }
            }
        }
        
        let swap_msg = match (swap_total, swap_free) {
            (Some(total), Some(free)) if total > 0 => {
                let used = total - free;
                let used_pct = (used as f64 / total as f64) * 100.0;
                format!(
                    "使用量: {:.2} GB / {:.2} GB ({:.1}%)",
                    used as f64 / 1024.0 / 1024.0,
                    total as f64 / 1024.0 / 1024.0,
                    used_pct
                )
            }
            _ => "スワップ未割り当て、または使用不可".to_string(),
        };
        
        if !is_systemd_available() {
            return Ok(format!("スワップ: {} | OOM監視: スキップ (systemd非稼働)", swap_msg));
        }

        if !has_journal_permission() {
            return Ok(format!("スワップ: {} | OOM監視: ⚠️ 権限不足のためスキップ", swap_msg));
        }
        
        let oom_check = Command::new("journalctl")
            .args(&["-p", "3", "-n", "100", "--no-pager"])
            .output();
            
        let mut oom_events = Vec::new();
        if let Ok(out) = oom_check {
            if out.status.success() {
                let stdout = String::from_utf8_lossy(&out.stdout);
                for line in stdout.lines() {
                    if line.to_lowercase().contains("out of memory") || line.to_lowercase().contains("killed process") {
                        oom_events.push(line.trim().to_string());
                    }
                }
            }
        }
        
        let oom_msg = if oom_events.is_empty() {
            "OOM Killer履歴なし (正常)".to_string()
        } else {
            oom_events.truncate(2);
            format!("⚠️ OOM Killer発生検知:\n{}", oom_events.join("\n"))
        };
        
        Ok(format!("スワップ: {} | {}", swap_msg, oom_msg))
    }
}

// 19. 簡易ディスク容量詳細
struct DetailedDiskUsageDiagnostic;
impl Diagnostic for DetailedDiskUsageDiagnostic {
    fn name(&self) -> &str { "Detailed Disk" }
    fn run(&self) -> Result<String> {
        let output = Command::new("df").args(&["-h", "--output=source,pcent,target"]).output()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(format!("\n{}", stdout.trim()))
    }
}

// 20. ファイルシステム inode 使用率診断
struct InodeUsageDiagnostic;
impl Diagnostic for InodeUsageDiagnostic {
    fn name(&self) -> &str { "FS Inodes" }
    fn run(&self) -> Result<String> {
        let output = Command::new("df").args(&["-i", "--output=source,ipcent,target"]).output()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(format!("\n{}", stdout.trim()))
    }
}

// 21. 不要な公開ポート（リッスンポート）のセキュリティ監査
struct ListenPortDiagnostic;
impl Diagnostic for ListenPortDiagnostic {
    fn parse_ports(&self, path: &str, is_udp: bool) -> Result<Vec<(String, u16)>> {
        let content = fs::read_to_string(path)?;
        let mut open_ports = Vec::new();
        for line in content.lines().skip(1) {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() > 3 {
                let local_addr = fields[1];
                let parts: Vec<&str> = local_addr.split(':').collect();
                if parts.len() == 2 {
                    let port = u16::from_str_radix(parts[1], 16).unwrap_or(0);
                    let ip_hex = parts[0];
                    let state = fields[3];
                    
                    if !is_udp && state != "0A" {
                        continue;
                    }
                    
                    let is_local = ip_hex == "0100007F" || ip_hex == "00000000000000000000000001000000";
                    let ip_desc = if is_local {
                        "Local-only".to_string()
                    } else {
                        "Public/NIC-bound".to_string()
                    };
                    
                    open_ports.push((ip_desc, port));
                }
            }
        }
        Ok(open_ports)
    }
}

impl Diagnostic for ListenPortDiagnostic {
    fn name(&self) -> &str { "Listen Ports" }
    fn run(&self) -> Result<String> {
        let mut public_ports = Vec::new();
        
        let scans = vec![
            ("/proc/net/tcp", false, "TCP"),
            ("/proc/net/tcp6", false, "TCP6"),
            ("/proc/net/udp", true, "UDP"),
            ("/proc/net/udp6", true, "UDP6"),
        ];

        for (path, is_udp, proto) in scans {
            if let Ok(ports) = self.parse_ports(path, is_udp) {
                for (ip, port) in ports {
                    if ip == "Public/NIC-bound" {
                        public_ports.push(format!("{}:{}", proto, port));
                    }
                }
            }
        }

        public_ports.sort();
        public_ports.dedup();

        if public_ports.is_empty() {
            Ok("外部公開されているポートはありません (安全)".to_string())
        } else {
            Ok(format!("⚠️ 外部公開中(NICバインド含む): {}", public_ports.join(", ")))
        }
    }
}

// 22. CPU/メモリ高負荷プロセスおよびハングプロセス（D状態/ゾンビ）の検出 & 自動クリーンアップ
struct ProcessLoadDiagnostic;
impl Diagnostic for ProcessLoadDiagnostic {
    fn name(&self) -> &str { "Process Hangs" }
    fn run(&self) -> Result<String> {
        let mut zombies = Vec::new();
        let mut io_blocked = Vec::new();
        
        if let Ok(entries) = fs::read_dir("/proc") {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.chars().all(|c| c.is_ascii_digit()) {
                    let stat_path = entry.path().join("stat");
                    let cmdline_path = entry.path().join("cmdline");
                    
                    if let Ok(stat_content) = fs::read_to_string(stat_path) {
                        if let Some(last_paren) = stat_content.rfind(')') {
                            let after_name = &stat_content[last_paren+1..];
                            let fields: Vec<&str> = after_name.split_whitespace().collect();
                            if fields.len() > 1 {
                                let state = fields[0];
                                let ppid = fields[1].to_string();
                                let comm = stat_content[stat_content.find('(').unwrap_or(0)+1..last_paren].to_string();
                                
                                let cmdline = fs::read_to_string(cmdline_path)
                                    .map(|c| c.replace('\0', " ").trim().to_string())
                                    .unwrap_or_default();
                                
                                let raw_name = if cmdline.is_empty() { comm } else { cmdline };
                                let truncated_name = if raw_name.len() > 20 {
                                    format!("{}...", &raw_name[0..17])
                                } else {
                                    raw_name
                                };

                                if state == "Z" {
                                    let parent_name = fs::read_to_string(format!("/proc/{}/stat", ppid))
                                        .map(|s| {
                                            s.rfind(')')
                                                .map(|idx| {
                                                    let start = s.find('(').unwrap_or(0) + 1;
                                                    s[start..idx].to_string()
                                                })
                                                .unwrap_or_else(|| "不明".to_string())
                                        })
                                        .unwrap_or_else(|_| "終了済み".to_string());

                                    // 親に SIGCHLD を送信して自動回収を試みる
                                    let cleanup_status = match Command::new("kill").args(&["-s", "CHLD", &ppid]).status() {
                                        Ok(status) if status.success() => "SIGCHLD送信済",
                                        _ => "回収要求不可",
                                    };

                                    zombies.push(format!(
                                        "{} (PID:{}, 親:{} PPID:{}, {})",
                                        truncated_name, name_str, parent_name, ppid, cleanup_status
                                    ));
                                } else if state == "D" {
                                    io_blocked.push(format!("{} (PID:{})", truncated_name, name_str));
                                }
                            }
                        }
                    }
                }
            }
        }
        
        let mut results = Vec::new();
        if !zombies.is_empty() {
            let total = zombies.len();
            zombies.truncate(3);
            let display = zombies.join(", ");
            if total > 3 {
                results.push(format!("ゾンビ: {} (他 {}件) ", display, total - 3));
            } else {
                results.push(format!("ゾンビ: {} ", display));
            }
        }
        if !io_blocked.is_empty() {
            let total = io_blocked.len();
            io_blocked.truncate(3);
            let display = io_blocked.join(", ");
            if total > 3 {
                results.push(format!("D状態(I/O待機): {} (他 {}件)", display, total - 3));
            } else {
                results.push(format!("D状態(I/O待機): {}", display));
            }
        }
        
        if results.is_empty() {
            Ok("ゾンビやハング状態(D状態)のプロセスはありません (正常)".to_string())
        } else {
            Ok(results.join(" | "))
        }
    }
}

// 23. 致命的カーネルエラー (dmesg / journalctlフォールバック) 診断
struct KernelErrorDiagnostic;
impl Diagnostic for KernelErrorDiagnostic {
    fn name(&self) -> &str { "Kernel Errors" }
    fn run(&self) -> Result<String> {
        let output = Command::new("dmesg")
            .args(&["--level=emerg,alert,crit,err", "-x"])
            .output();
            
        match output {
            Ok(out) => {
                if out.status.success() {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    let lines: Vec<&str> = stdout.trim().lines().collect();
                    if lines.is_empty() {
                        Ok("カーネルの致命的エラーは検出されませんでした (正常)".to_string())
                    } else {
                        let total = lines.len();
                        let display_lines: Vec<&str> = lines.into_iter().rev().take(3).collect();
                        Ok(format!(
                            "⚠️ 致命的カーネルエラーを検知 (計{}件) 直近:\n{}",
                            total,
                            display_lines.join("\n")
                        ))
                    }
                } else {
                    if !is_systemd_available() {
                        return Ok("スキップ (systemd非稼働のためカーネルログ監査不可)".to_string());
                    }
                    if !has_journal_permission() {
                        return Ok("⚠️ 権限不足のためカーネルログ監査スキップ (dmesg/journalctl読取不可)".to_string());
                    }
                    
                    let fallback = Command::new("journalctl")
                        .args(&["-k", "-p", "3", "-n", "3", "--no-pager"])
                        .output();
                    if let Ok(f_out) = fallback {
                        if f_out.status.success() {
                            let stdout = String::from_utf8_lossy(&f_out.stdout);
                            if stdout.trim().is_empty() {
                                return Ok("致命的カーネルエラーなし (journalctlフォールバック)".to_string());
                            } else {
                                return Ok(format!("⚠️ journalctlによりカーネルエラー検出:\n{}", stdout.trim()));
                            }
                        }
                    }
                    Ok("警告: カーネルログ(dmesg)のチェックに必要な権限がありません".to_string())
                }
            }
            Err(_) => Err(anyhow!("dmesg コマンドが利用できません")),
        }
    }
}

// 24. サービス状態診断
struct ServiceDiagnostic { service_name: String }
impl Diagnostic for ServiceDiagnostic {
    fn name(&self) -> &str { "Service Status" }
    fn run(&self) -> Result<String> {
        let output = Command::new("systemctl").args(&["is-active", &self.service_name]).output();
        match output {
            Ok(out) => {
                let status = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if out.status.success() || status == "inactive" || status == "failed" {
                    Ok(format!("{} は {}", self.service_name, status))
                } else {
                    Ok(format!("{} は取得不可 (ステータス: {})", self.service_name, status))
                }
            }
            Err(_) => Err(anyhow!("systemctl コマンドが実行できません")),
        }
    }
}

// 25. ログ監視診断
struct LogErrorDiagnostic;
impl Diagnostic for LogErrorDiagnostic {
    fn name(&self) -> &str { "Critical Logs" }
    fn run(&self) -> Result<String> {
        if !is_systemd_available() {
            return Ok("スキップ (systemd非稼働のためジャーナルログなし)".to_string());
        }
        if !has_journal_permission() {
            return Ok("⚠️ 権限不足のためログ監視スキップ".to_string());
        }
        
        let output = Command::new("journalctl").args(&["-p", "3", "-n", "5", "--no-pager"]).output();
        match output {
            Ok(out) => {
                if out.status.success() {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    if stdout.trim().is_empty() {
                        Ok("過去のクリティカルログなし".to_string())
                    } else {
                        Ok(format!("エラー検知:\n{}", stdout.trim()))
                    }
                } else {
                    Ok("警告: journalctl の実行中にエラーが発生しました。".to_string())
                }
            }
            Err(_) => Err(anyhow!("journalctl コマンドが見つかりません")),
        }
    }
}

fn main() -> Result<()> {
    let diagnostics: Vec<Box<dyn Diagnostic>> = vec![
        Box::new(UptimeLoadAvgDiagnostic),
        Box::new(CpuFrequencyDiagnostic),
        Box::new(CpuVulnerabilityDiagnostic),
        Box::new(NetworkMTUDiagnostic),
        Box::new(LocalGatewayPingDiagnostic),
        Box::new(DnsLatencyDiagnostic),
        Box::new(NtpTimeSyncDiagnostic),
        Box::new(ActiveConnectionDiagnostic),
        Box::new(ActiveUsersSshDiagnostic),
        Box::new(NetworkInterfaceErrorDiagnostic),
        Box::new(SecurityModuleDiagnostic),
        Box::new(BluetoothDiagnostic),
        Box::new(HardwareBusDiagnostic),
        Box::new(BatteryPowerDiagnostic),
        Box::new(TemperatureDiagnostic),
        Box::new(MemoryPressureDiagnostic),
        Box::new(DiskIoPerformanceDiagnostic),
        Box::new(SwapOomDiagnostic),
        Box::new(DetailedDiskUsageDiagnostic),
        Box::new(InodeUsageDiagnostic),
        Box::new(ListenPortDiagnostic),
        Box::new(ProcessLoadDiagnostic),
        Box::new(KernelErrorDiagnostic),
        Box::new(ServiceDiagnostic { service_name: "NetworkManager".to_string() }),
        Box::new(LogErrorDiagnostic),
    ];

    println!("=== 🔍 包括的システム診断フレームワーク (堅牢・アルティメット多機能版) ===");
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
