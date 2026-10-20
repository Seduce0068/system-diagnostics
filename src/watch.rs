//! ネットワークの状態変化を監視し、変化時に証跡を JSONL で記録する。
//!
//! 「PC だけインターネットに出られない」事象の切り分けでは、
//! 発生時点のリンク状態（carrier/operstate）、IPv4 アドレス、経路、
//! ゲートウェイ・上位網・DNS・TCP の到達性を同時に採取する必要がある。
//! 本モジュールは一定間隔でこれらを測定し、状態が変化した瞬間の
//! コマンド出力一式を追記型のログへ保存する。
//!
//! 外部送信は行わない。書き込み先はローカルファイルのみ。

use serde::Serialize;
use serde_json::json;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// 上位網の到達性確認に使う ICMP 宛先。
const PING_TARGET: &str = "8.8.8.8";
/// DNS 解決確認に使うホスト名。
const DNS_PROBE_HOST: &str = "www.google.com";
/// TCP 到達性確認に使うアドレス。
///
/// AdGuard CLI の透過プロキシ（`filtered_ports: 80:5221,5300:49151`）を
/// 経由するアプリ経路。ユーザーが実際に使う経路の健全性を測る。
const TCP_PROBE_ADDR: &str = "1.1.1.1:443";
/// 透過プロキシを経由しない直接 TCP の確認先。
///
/// ポート 53 は AdGuard の `filtered_ports` の範囲外にあり、
/// OS とルーターの TCP 経路そのものを測る。`tcp_ok` と分けて
/// 記録することで、プロキシ層とネットワーク層を切り分けられる。
const TCP_DIRECT_PROBE_ADDR: &str = "1.1.1.1:53";
/// 証跡 1 件あたりの最大保存バイト数。
const EVIDENCE_LIMIT_BYTES: usize = 4096;

/// 監視対象のヘルス状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Health {
    /// ゲートウェイ・上位網・DNS・TCP のすべてが成功。
    Ok,
    /// リンクと IPv4 は正常だが、一部の到達性が失敗。
    Degraded,
    /// carrier 断、IPv4 不在、既定経路不在のいずれか。
    Down,
}

impl Health {
    /// ログ出力用の短い表現。
    pub fn as_str(&self) -> &'static str {
        match self {
            Health::Ok => "ok",
            Health::Degraded => "degraded",
            Health::Down => "down",
        }
    }
}

/// 1 回の測定結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub interface: String,
    pub carrier: bool,
    pub operstate: String,
    pub speed_mbps: Option<String>,
    pub mtu: Option<String>,
    pub ipv4: Option<String>,
    pub gateway: Option<String>,
    pub default_route: bool,
    pub gateway_reachable: bool,
    pub internet_reachable: bool,
    pub dns_ok: bool,
    /// アプリ経路（AdGuard CLI の透過プロキシ対象）の TCP 到達性。
    pub tcp_ok: bool,
    /// 透過プロキシを経由しない直接 TCP の到達性。
    pub direct_tcp_ok: bool,
}

impl Snapshot {
    /// 測定結果からヘルス状態を判定する。
    pub fn health(&self) -> Health {
        if !self.carrier || self.ipv4.is_none() || !self.default_route {
            return Health::Down;
        }
        if self.gateway_reachable
            && self.internet_reachable
            && self.dns_ok
            && self.tcp_ok
            && self.direct_tcp_ok
        {
            Health::Ok
        } else {
            Health::Degraded
        }
    }

    /// 判定理由を `,` 区切りで返す。全成功時は `ok`。
    pub fn reason(&self) -> String {
        let mut reasons: Vec<&str> = Vec::new();
        if !self.carrier {
            reasons.push("carrier-down");
        }
        if self.ipv4.is_none() {
            reasons.push("no-ipv4");
        }
        if !self.default_route {
            reasons.push("no-default-route");
        }
        if self.carrier && self.ipv4.is_some() && self.default_route {
            if !self.gateway_reachable {
                reasons.push("gateway-unreachable");
            }
            if !self.internet_reachable {
                reasons.push("internet-unreachable");
            }
            if !self.dns_ok {
                reasons.push("dns-failed");
            }
            if !self.tcp_ok {
                reasons.push("tcp-failed");
            }
            if !self.direct_tcp_ok {
                reasons.push("direct-tcp-failed");
            }
        }
        if reasons.is_empty() {
            "ok".to_string()
        } else {
            reasons.join(",")
        }
    }

    fn to_json(&self) -> serde_json::Value {
        json!({
            "interface": self.interface,
            "carrier": self.carrier,
            "operstate": self.operstate,
            "speed_mbps": self.speed_mbps,
            "mtu": self.mtu,
            "ipv4": self.ipv4,
            "gateway": self.gateway,
            "default_route": self.default_route,
            "gateway_reachable": self.gateway_reachable,
            "internet_reachable": self.internet_reachable,
            "dns_ok": self.dns_ok,
            "tcp_ok": self.tcp_ok,
            "direct_tcp_ok": self.direct_tcp_ok,
        })
    }
}

/// 監視設定。
pub struct WatchConfig {
    pub interface: String,
    pub interval: Duration,
    pub out: PathBuf,
    pub once: bool,
    pub quiet: bool,
}

/// 証跡 1 件（実行したコマンドと出力）。
#[derive(Debug, Serialize)]
struct Evidence {
    cmd: String,
    out: String,
}

/// 監視ループを実行する。状態が変化するまで待機し続ける。
pub fn run(cfg: &WatchConfig) -> std::io::Result<()> {
    let offset_seconds = parse_offset_seconds(&local_utc_offset());
    let mut previous: Option<(Health, String)> = None;

    loop {
        let snapshot = probe(&cfg.interface);
        let health = snapshot.health();
        let reason = snapshot.reason();
        let changed = previous
            .as_ref()
            .is_none_or(|(h, r)| *h != health || *r != reason);

        if changed || cfg.once {
            let epoch = now_epoch();
            let evidence = capture(&cfg.interface);
            write_event(
                cfg,
                epoch,
                offset_seconds,
                health,
                &reason,
                &snapshot,
                &evidence,
            )?;
            if !cfg.quiet {
                println!(
                    "{} [{}] {} (log: {})",
                    format_rfc3339(epoch, offset_seconds),
                    health.as_str(),
                    reason,
                    cfg.out.display()
                );
            }
            previous = Some((health, reason));
        }

        if cfg.once {
            return Ok(());
        }
        thread::sleep(cfg.interval);
    }
}

/// 既定のログ保存先を返す。
pub fn default_log_path() -> PathBuf {
    if let Ok(state) = std::env::var("XDG_STATE_HOME") {
        if !state.is_empty() {
            return PathBuf::from(state)
                .join("system-diag")
                .join("net-watch.jsonl");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            return PathBuf::from(home)
                .join(".local")
                .join("state")
                .join("system-diag")
                .join("net-watch.jsonl");
        }
    }
    PathBuf::from("net-watch.jsonl")
}

/// 1 回分の測定を実行する。
pub fn probe(interface: &str) -> Snapshot {
    let carrier = sysfs_value(interface, "carrier")
        .map(|v| v == "1")
        .unwrap_or(false);
    let operstate = sysfs_value(interface, "operstate").unwrap_or_else(|| "unknown".to_string());
    let speed_mbps = sysfs_value(interface, "speed");
    let mtu = sysfs_value(interface, "mtu");
    let ipv4 = interface_ipv4(interface);
    let gateway = default_gateway();
    let default_route = gateway.is_some();
    let gateway_reachable = gateway.as_deref().map(ping_ok).unwrap_or(false);
    let internet_reachable = carrier && ping_ok(PING_TARGET);
    let dns_ok = carrier && resolver_ok(DNS_PROBE_HOST);
    let app_tcp_ok = carrier && tcp_ok(TCP_PROBE_ADDR);
    let direct_tcp_ok = carrier && tcp_ok(TCP_DIRECT_PROBE_ADDR);

    Snapshot {
        interface: interface.to_string(),
        carrier,
        operstate,
        speed_mbps,
        mtu,
        ipv4,
        gateway,
        default_route,
        gateway_reachable,
        internet_reachable,
        dns_ok,
        tcp_ok: app_tcp_ok,
        direct_tcp_ok,
    }
}

fn write_event(
    cfg: &WatchConfig,
    epoch: u64,
    offset_seconds: i64,
    health: Health,
    reason: &str,
    snapshot: &Snapshot,
    evidence: &[Evidence],
) -> std::io::Result<()> {
    if let Some(parent) = cfg.out.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }

    let record = json!({
        "at_epoch": epoch,
        "at_local": format_rfc3339(epoch, offset_seconds),
        "health": health,
        "reason": reason,
        "snapshot": snapshot.to_json(),
        "evidence": evidence,
    });

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&cfg.out)?;
    writeln!(file, "{}", serde_json::to_string(&record)?)?;
    file.flush()
}

fn capture(interface: &str) -> Vec<Evidence> {
    let mut items = Vec::new();

    for name in [
        "carrier",
        "operstate",
        "speed",
        "duplex",
        "mtu",
        "statistics/rx_bytes",
        "statistics/tx_bytes",
        "statistics/rx_dropped",
        "statistics/tx_dropped",
    ] {
        let path = format!("/sys/class/net/{}/{}", interface, name);
        if let Ok(value) = fs::read_to_string(&path) {
            items.push(Evidence {
                cmd: path,
                out: truncate_utf8(value.trim_end(), EVIDENCE_LIMIT_BYTES),
            });
        }
    }

    let mut commands: Vec<Vec<String>> = vec![
        words(&["ip", "-br", "addr"]),
        words(&["ip", "-4", "route"]),
        words(&["ip", "-6", "route"]),
        words(&["ip", "neigh"]),
        words(&["ip", "-s", "link", "show", interface]),
        words(&[
            "nmcli",
            "-t",
            "-f",
            "GENERAL.STATE,GENERAL.CONNECTION,IP4.ADDRESS,IP4.GATEWAY,IP4.DNS",
            "device",
            "show",
            interface,
        ]),
        words(&["journalctl", "-k", "-n", "30", "--no-pager"]),
        words(&[
            "journalctl",
            "-u",
            "NetworkManager",
            "-n",
            "30",
            "--no-pager",
        ]),
    ];
    commands.retain(|c| !c.is_empty());

    for command in commands {
        items.push(run_evidence(&command));
    }

    items
}

fn words(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| (*a).to_string()).collect()
}

fn run_evidence(args: &[String]) -> Evidence {
    let (program, rest) = match args.split_first() {
        Some(parts) => parts,
        None => {
            return Evidence {
                cmd: String::new(),
                out: "[empty-command]".to_string(),
            }
        }
    };

    match Command::new(program).args(rest).output() {
        Ok(output) => {
            let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.trim().is_empty() {
                text.push_str("\n[stderr] ");
                text.push_str(stderr.trim_end());
            }
            if !output.status.success() {
                text.push_str(&format!("\n[exit] {}", output.status));
            }
            Evidence {
                cmd: args.join(" "),
                out: truncate_utf8(&text, EVIDENCE_LIMIT_BYTES),
            }
        }
        Err(err) => Evidence {
            cmd: args.join(" "),
            out: format!("[spawn-error] {}", err),
        },
    }
}

fn sysfs_value(interface: &str, name: &str) -> Option<String> {
    fs::read_to_string(format!("/sys/class/net/{}/{}", interface, name))
        .ok()
        .map(|v| v.trim().to_string())
}

fn interface_ipv4(interface: &str) -> Option<String> {
    let output = Command::new("ip")
        .args(["-4", "-o", "addr", "show", "dev", interface])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_ipv4_addr(&String::from_utf8_lossy(&output.stdout))
}

fn default_gateway() -> Option<String> {
    let output = Command::new("ip")
        .args(["-4", "route", "show", "default"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_default_gateway(&String::from_utf8_lossy(&output.stdout))
}

fn ping_ok(target: &str) -> bool {
    Command::new("ping")
        .args(["-n", "-c", "1", "-W", "1", target])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn resolver_ok(host: &str) -> bool {
    Command::new("getent")
        .args(["ahostsv4", host])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn tcp_ok(addr: &str) -> bool {
    let parsed: Result<SocketAddr, _> = addr.parse();
    match parsed {
        Ok(socket) => TcpStream::connect_timeout(&socket, Duration::from_secs(2)).is_ok(),
        Err(_) => false,
    }
}

fn parse_ipv4_addr(output: &str) -> Option<String> {
    for line in output.lines() {
        let mut tokens = line.split_whitespace();
        while let Some(token) = tokens.next() {
            if token == "inet" {
                if let Some(addr) = tokens.next() {
                    return Some(addr.split('/').next().unwrap_or(addr).to_string());
                }
            }
        }
    }
    None
}

fn parse_default_gateway(output: &str) -> Option<String> {
    for line in output.lines() {
        if !line.starts_with("default") {
            continue;
        }
        let tokens: Vec<&str> = line.split_whitespace().collect();
        if let Some(index) = tokens.iter().position(|t| *t == "via") {
            if let Some(gateway) = tokens.get(index + 1) {
                return Some((*gateway).to_string());
            }
        }
    }
    None
}

fn truncate_utf8(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[truncated]", &text[..end])
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn local_utc_offset() -> String {
    match Command::new("date").arg("+%:z").output() {
        Ok(output) if output.status.success() => {
            let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if value.len() == 6 {
                value
            } else {
                "+00:00".to_string()
            }
        }
        _ => "+00:00".to_string(),
    }
}

fn parse_offset_seconds(offset: &str) -> i64 {
    if offset.len() != 6 {
        return 0;
    }
    let sign = match offset.as_bytes()[0] {
        b'+' => 1,
        b'-' => -1,
        _ => return 0,
    };
    let hours: i64 = offset[1..3].parse().unwrap_or(0);
    let minutes: i64 = offset[4..6].parse().unwrap_or(0);
    sign * (hours * 3600 + minutes * 60)
}

fn format_offset(offset_seconds: i64) -> String {
    let sign = if offset_seconds < 0 { '-' } else { '+' };
    let abs = offset_seconds.abs();
    format!("{}{:02}:{:02}", sign, abs / 3600, (abs % 3600) / 60)
}

fn format_rfc3339(epoch: u64, offset_seconds: i64) -> String {
    let local = epoch as i64 + offset_seconds;
    let days = local.div_euclid(86_400);
    let seconds = local.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds / 3600;
    let minute = (seconds % 3600) / 60;
    let second = seconds % 60;
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{}",
        year,
        month,
        day,
        hour,
        minute,
        second,
        format_offset(offset_seconds)
    )
}

/// Unix epoch からの経過日数を暦日へ変換する (Howard Hinnant のアルゴリズム)。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = (z - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(
        carrier: bool,
        ipv4: Option<&str>,
        default_route: bool,
        gateway_reachable: bool,
        internet_reachable: bool,
        dns_ok: bool,
        tcp_ok: bool,
    ) -> Snapshot {
        Snapshot {
            interface: "enp4s0".to_string(),
            carrier,
            operstate: "up".to_string(),
            speed_mbps: Some("1000".to_string()),
            mtu: Some("1280".to_string()),
            ipv4: ipv4.map(|v| v.to_string()),
            gateway: Some("192.168.128.1".to_string()),
            default_route,
            gateway_reachable,
            internet_reachable,
            dns_ok,
            tcp_ok,
            direct_tcp_ok: tcp_ok,
        }
    }

    #[test]
    fn test_health_all_ok() {
        let snap = snapshot(true, Some("192.168.128.111"), true, true, true, true, true);
        assert_eq!(snap.health(), Health::Ok);
        assert_eq!(snap.reason(), "ok");
    }

    #[test]
    fn test_health_carrier_down() {
        let snap = snapshot(
            false,
            Some("192.168.128.111"),
            true,
            false,
            false,
            false,
            false,
        );
        assert_eq!(snap.health(), Health::Down);
        assert!(snap.reason().contains("carrier-down"));
    }

    #[test]
    fn test_health_no_ipv4_is_down() {
        let snap = snapshot(true, None, true, false, false, false, false);
        assert_eq!(snap.health(), Health::Down);
        assert!(snap.reason().contains("no-ipv4"));
    }

    #[test]
    fn test_health_degraded_gateway_unreachable() {
        let snap = snapshot(
            true,
            Some("192.168.128.111"),
            true,
            false,
            false,
            false,
            false,
        );
        assert_eq!(snap.health(), Health::Degraded);
        assert!(snap.reason().contains("gateway-unreachable"));
    }

    #[test]
    fn test_health_degraded_upstream_only() {
        let snap = snapshot(true, Some("192.168.128.111"), true, true, false, true, true);
        assert_eq!(snap.health(), Health::Degraded);
        assert_eq!(snap.reason(), "internet-unreachable");
    }

    #[test]
    fn test_health_degraded_dns_only() {
        let snap = snapshot(true, Some("192.168.128.111"), true, true, true, false, true);
        assert_eq!(snap.health(), Health::Degraded);
        assert_eq!(snap.reason(), "dns-failed");
    }

    #[test]
    fn test_health_degraded_proxy_tcp_only() {
        // 直接 TCP は成功し、プロキシ経由の TCP だけ失敗する場合。
        let mut snap = snapshot(true, Some("192.168.128.111"), true, true, true, true, true);
        snap.tcp_ok = false;
        assert_eq!(snap.health(), Health::Degraded);
        assert_eq!(snap.reason(), "tcp-failed");
    }

    #[test]
    fn test_health_degraded_direct_tcp_only() {
        // プロキシ経由の TCP は成功し、直接 TCP だけ失敗する場合。
        let mut snap = snapshot(true, Some("192.168.128.111"), true, true, true, true, true);
        snap.direct_tcp_ok = false;
        assert_eq!(snap.health(), Health::Degraded);
        assert_eq!(snap.reason(), "direct-tcp-failed");
    }

    #[test]
    fn test_parse_default_gateway() {
        let output =
            "default via 192.168.128.1 dev enp4s0 proto dhcp src 192.168.128.111 metric 100 \n";
        assert_eq!(
            parse_default_gateway(output),
            Some("192.168.128.1".to_string())
        );
        assert_eq!(parse_default_gateway(""), None);
    }

    #[test]
    fn test_parse_ipv4_addr() {
        let output =
            "2: enp4s0    inet 192.168.128.111/24 brd 192.168.128.255 scope global dynamic enp4s0\n";
        assert_eq!(parse_ipv4_addr(output), Some("192.168.128.111".to_string()));
        assert_eq!(
            parse_ipv4_addr("2: enp4s0    inet6 fe80::1/64 scope link\n"),
            None
        );
    }

    #[test]
    fn test_parse_offset_seconds() {
        assert_eq!(parse_offset_seconds("+09:00"), 32_400);
        assert_eq!(parse_offset_seconds("-05:30"), -19_800);
        assert_eq!(parse_offset_seconds("bad"), 0);
    }

    #[test]
    fn test_format_rfc3339() {
        assert_eq!(format_rfc3339(0, 32_400), "1970-01-01T09:00:00+09:00");
        assert_eq!(
            format_rfc3339(1_000_000_000, 32_400),
            "2001-09-09T10:46:40+09:00"
        );
        assert_eq!(
            format_rfc3339(1_000_000_000, 0),
            "2001-09-09T01:46:40+00:00"
        );
    }

    #[test]
    fn test_format_rfc3339_negative_offset() {
        assert_eq!(format_rfc3339(0, -19_800), "1969-12-31T18:30:00-05:30");
    }

    #[test]
    fn test_truncate_utf8_keeps_boundary() {
        let text = "日本語のログ出力";
        let truncated = truncate_utf8(text, 4);
        assert!(truncated.starts_with('日'));
        assert!(truncated.ends_with("[truncated]"));
        assert_eq!(truncate_utf8("short", 100), "short");
    }

    #[test]
    fn test_default_log_path_is_absolute_or_relative_file() {
        let path = default_log_path();
        assert!(path.to_string_lossy().ends_with("net-watch.jsonl"));
    }
}
