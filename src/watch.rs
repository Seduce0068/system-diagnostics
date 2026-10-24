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
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
/// 子プロセス終了後に stdout/stderr を回収する猶予。
const DRAIN_LIMIT: Duration = Duration::from_millis(500);
/// `ip` 1 回あたりの上限。
const IP_TIMEOUT: Duration = Duration::from_secs(1);
/// DNS 解決確認 1 回あたりの上限。
const DNS_TIMEOUT: Duration = Duration::from_secs(2);
/// ICMP 確認 1 回あたりの上限。`ping -W 1` より十分に長く取る。
const PING_TIMEOUT: Duration = Duration::from_secs(2);
/// 証跡採取コマンド 1 回あたりの上限。
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(2);
/// タイムゾーン取得 1 回あたりの上限。
const DATE_TIMEOUT: Duration = Duration::from_secs(1);

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
    /// 監視器側の失敗（コマンド欠落・タイムアウト等）で判定できない。
    Unknown,
}

impl Health {
    /// ログ出力用の短い表現。
    pub fn as_str(&self) -> &'static str {
        match self {
            Health::Ok => "ok",
            Health::Degraded => "degraded",
            Health::Down => "down",
            Health::Unknown => "unknown",
        }
    }
}

/// 1 回の測定結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub interface: String,
    /// `/sys/class/net/<interface>` が存在するか。
    pub interface_present: bool,
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
    /// 監視器側の測定失敗理由。空なら測定は完了している。
    pub probe_errors: Vec<String>,
    /// 必須チェックが監視器側の理由で測定できなかった場合 true。
    pub measurement_failed: bool,
}

impl Snapshot {
    /// 測定結果からヘルス状態を判定する。
    pub fn health(&self) -> Health {
        if !self.interface_present || !self.carrier {
            return Health::Down;
        }
        if self.measurement_failed {
            return Health::Unknown;
        }
        if self.ipv4.is_none() || !self.default_route {
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
        if !self.interface_present {
            return "interface-missing".to_string();
        }
        if !self.carrier {
            return "carrier-down".to_string();
        }
        if self.measurement_failed {
            return if self.probe_errors.is_empty() {
                "probe-error".to_string()
            } else {
                self.probe_errors.join(",")
            };
        }

        let mut reasons: Vec<&str> = Vec::new();
        if self.ipv4.is_none() {
            reasons.push("no-ipv4");
        }
        if !self.default_route {
            reasons.push("no-default-route");
        }
        if self.ipv4.is_some() && self.default_route {
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
            "interface_present": self.interface_present,
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
            "probe_errors": self.probe_errors,
            "measurement_failed": self.measurement_failed,
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
///
/// `out` は既存の jq クエリとの互換用の要約で、stdout が上限を超えると
/// stderr/exit が切れる。`stdout`/`stderr`/`exit_code`/`timed_out` は
/// それぞれ独立に保持する。
#[derive(Debug, Serialize)]
struct Evidence {
    cmd: String,
    out: String,
    stdout: String,
    stderr: String,
    exit_code: Option<i32>,
    signal: Option<i32>,
    timed_out: bool,
    elapsed_ms: u64,
}

impl Evidence {
    fn sysfs(cmd: String, value: &str) -> Self {
        let text = truncate_utf8(value, EVIDENCE_LIMIT_BYTES);
        Self {
            cmd,
            out: text.clone(),
            stdout: text,
            stderr: String::new(),
            exit_code: None,
            signal: None,
            timed_out: false,
            elapsed_ms: 0,
        }
    }
}

fn interface_exists(interface: &str) -> bool {
    !interface.is_empty()
        && !interface.contains('/')
        && interface != "."
        && interface != ".."
        && Path::new("/sys/class/net").join(interface).is_dir()
}

/// 監視ループを実行する。状態が変化するまで待機し続ける。
pub fn run(cfg: &WatchConfig) -> std::io::Result<()> {
    if !interface_exists(&cfg.interface) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("network interface not found: {}", cfg.interface),
        ));
    }

    let offset_seconds = parse_offset_seconds(&local_utc_offset());
    let mut previous: Option<(Health, String)> = None;
    let mut cycle: u64 = 0;
    let mut last_event_epoch: u64 = 0;
    let mut warned_watchdog = false;

    loop {
        let cycle_start = Instant::now();
        let snapshot = probe(&cfg.interface);
        let health = snapshot.health();
        let reason = snapshot.reason();
        let changed = previous
            .as_ref()
            .is_none_or(|(h, r)| *h != health || *r != reason);
        let first = cycle == 0;

        if changed || cfg.once || first {
            let epoch = now_epoch();
            let evidence = capture(&cfg.interface);
            let event = if cfg.once {
                "once"
            } else if first {
                "started"
            } else {
                "changed"
            };
            write_event(
                cfg,
                &EventRecord {
                    event,
                    epoch,
                    offset_seconds,
                    health,
                    reason: &reason,
                    snapshot: &snapshot,
                    evidence: &evidence,
                    duration_ms: cycle_start.elapsed().as_millis() as u64,
                },
            )?;
            last_event_epoch = epoch;
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

        let duration_ms = cycle_start.elapsed().as_millis() as u64;
        write_heartbeat(
            cfg,
            now_epoch(),
            offset_seconds,
            cycle,
            duration_ms,
            last_event_epoch,
        )?;
        if let Err(err) = notify_watchdog() {
            if !warned_watchdog {
                eprintln!("watchdog 通知に失敗: {}", err);
                warned_watchdog = true;
            }
        }
        cycle = cycle.saturating_add(1);

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
    let interface_present = interface_exists(interface);
    let carrier = sysfs_value(interface, "carrier")
        .map(|v| v == "1")
        .unwrap_or(false);
    let operstate = sysfs_value(interface, "operstate").unwrap_or_else(|| "unknown".to_string());
    let speed_mbps = sysfs_value(interface, "speed");
    let mtu = sysfs_value(interface, "mtu");

    let mut probe_errors = Vec::new();
    let ipv4 = interface_ipv4(interface, &mut probe_errors);
    let gateway = default_gateway(&mut probe_errors);
    let default_route = gateway.is_some();
    let gateway_reachable = gateway
        .as_deref()
        .map(|gw| ping_ok(gw, &mut probe_errors))
        .unwrap_or(false);
    let internet_reachable = carrier && ping_ok(PING_TARGET, &mut probe_errors);
    let dns_ok = carrier && resolver_ok(DNS_PROBE_HOST, &mut probe_errors);
    let app_tcp_ok = carrier && tcp_ok(TCP_PROBE_ADDR);
    let direct_tcp_ok = carrier && tcp_ok(TCP_DIRECT_PROBE_ADDR);
    let measurement_failed = !probe_errors.is_empty();

    Snapshot {
        interface: interface.to_string(),
        interface_present,
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
        probe_errors,
        measurement_failed,
    }
}

struct EventRecord<'a> {
    event: &'a str,
    epoch: u64,
    offset_seconds: i64,
    health: Health,
    reason: &'a str,
    snapshot: &'a Snapshot,
    evidence: &'a [Evidence],
    duration_ms: u64,
}

fn write_event(cfg: &WatchConfig, record: &EventRecord<'_>) -> std::io::Result<()> {
    if let Some(parent) = cfg.out.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }

    let payload = json!({
        "schema_version": 2,
        "event": record.event,
        "at_epoch": record.epoch,
        "at_local": format_rfc3339(record.epoch, record.offset_seconds),
        "health": record.health,
        "reason": record.reason,
        "duration_ms": record.duration_ms,
        "snapshot": record.snapshot.to_json(),
        "evidence": record.evidence,
    });

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&cfg.out)?;
    writeln!(file, "{}", serde_json::to_string(&payload)?)?;
    file.flush()
}

/// JSONL とは別に、毎周期の生存記録を 1 ファイルへ上書きする。
fn heartbeat_path(out: &Path) -> PathBuf {
    out.with_extension("heartbeat")
}

fn write_heartbeat(
    cfg: &WatchConfig,
    epoch: u64,
    offset_seconds: i64,
    cycle: u64,
    duration_ms: u64,
    last_event_epoch: u64,
) -> std::io::Result<()> {
    let path = heartbeat_path(&cfg.out);
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }

    let record = json!({
        "schema_version": 2,
        "at_epoch": epoch,
        "at_local": format_rfc3339(epoch, offset_seconds),
        "pid": std::process::id(),
        "cycle": cycle,
        "duration_ms": duration_ms,
        "last_event_epoch": last_event_epoch,
    });

    // 同一ディレクトリ内の rename は atomic。読み手が空ファイルを
    // 見ることはない。
    let tmp = path.with_extension(format!("heartbeat.tmp.{}", std::process::id()));
    {
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp)?;
        writeln!(file, "{}", serde_json::to_string(&record)?)?;
        file.flush()?;
    }
    if let Err(err) = fs::rename(&tmp, &path) {
        let _ = fs::remove_file(&tmp);
        return Err(err);
    }
    Ok(())
}

/// systemd の `WatchdogSec=` に keep-alive を送る。
///
/// `Type=simple` のままでよい。`WatchdogSec=` を設定すると
/// `NotifyAccess=main` が暗黙に適用される。
fn notify_watchdog() -> std::io::Result<()> {
    let Some(socket) = std::env::var_os("NOTIFY_SOCKET") else {
        return Ok(());
    };
    let path = PathBuf::from(socket);
    // std は抽象名前空間 (先頭が '@') を扱えない。user unit では
    // 通常 /run/user/<uid>/systemd/notify が渡される。
    if path.as_os_str().to_string_lossy().starts_with('@') {
        return Ok(());
    }
    let datagram = std::os::unix::net::UnixDatagram::unbound()?;
    datagram.send_to(b"WATCHDOG=1", &path)?;
    Ok(())
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
            items.push(Evidence::sysfs(path, value.trim_end()));
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
                stdout: String::new(),
                stderr: String::new(),
                exit_code: None,
                signal: None,
                timed_out: false,
                elapsed_ms: 0,
            }
        }
    };

    match run_command(program, rest, CAPTURE_TIMEOUT) {
        CommandOutcome::Completed {
            status,
            stdout,
            stderr,
            elapsed,
        } => {
            let exit_code = status.code();
            let signal = status.signal();
            Evidence {
                cmd: args.join(" "),
                out: summarize(&stdout, &stderr, exit_code, signal),
                stdout: truncate_utf8(&stdout, EVIDENCE_LIMIT_BYTES),
                stderr: truncate_utf8(&stderr, EVIDENCE_LIMIT_BYTES),
                exit_code,
                signal,
                timed_out: false,
                elapsed_ms: elapsed.as_millis() as u64,
            }
        }
        CommandOutcome::TimedOut { elapsed } => Evidence {
            cmd: args.join(" "),
            out: "[timeout]".to_string(),
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
            signal: None,
            timed_out: true,
            elapsed_ms: elapsed.as_millis() as u64,
        },
        CommandOutcome::SpawnFailed { message, elapsed } => Evidence {
            cmd: args.join(" "),
            out: format!("[spawn-error] {}", message),
            stdout: String::new(),
            stderr: message,
            exit_code: None,
            signal: None,
            timed_out: false,
            elapsed_ms: elapsed.as_millis() as u64,
        },
    }
}

fn summarize(stdout: &str, stderr: &str, exit_code: Option<i32>, signal: Option<i32>) -> String {
    let mut text = stdout.to_string();
    if !stderr.trim().is_empty() {
        text.push_str("\n[stderr] ");
        text.push_str(stderr.trim_end());
    }
    if let Some(code) = exit_code {
        if code != 0 {
            text.push_str(&format!("\n[exit] {}", code));
        }
    }
    if let Some(sig) = signal {
        text.push_str(&format!("\n[signal] {}", sig));
    }
    truncate_utf8(&text, EVIDENCE_LIMIT_BYTES)
}

/// 外部コマンドの結果。タイムアウトと起動失敗を正常終了から分離する。
#[derive(Debug)]
enum CommandOutcome {
    Completed {
        status: ExitStatus,
        stdout: String,
        stderr: String,
        elapsed: Duration,
    },
    TimedOut {
        elapsed: Duration,
    },
    SpawnFailed {
        message: String,
        elapsed: Duration,
    },
}

/// パイプを読み、読めた分を逐次送るスレッドを起動する。
fn spawn_reader<T>(pipe: Option<T>) -> Option<Receiver<Vec<u8>>>
where
    T: Read + Send + 'static,
{
    pipe.map(|mut pipe| {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match pipe.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        });
        rx
    })
}

/// 期限内に到着した出力を回収する。
///
/// 子が終了しても、パイプを保持した孫プロセスがいると EOF は来ない。
/// 監視ループを止めないため、回収自体にも期限を設ける。
fn collect_output(rx: Option<Receiver<Vec<u8>>>, deadline: Instant) -> Vec<u8> {
    let Some(rx) = rx else {
        return Vec::new();
    };
    let mut buf = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match rx.recv_timeout(remaining) {
            Ok(chunk) => buf.extend_from_slice(&chunk),
            Err(_) => break,
        }
    }
    buf
}

/// 子プロセスを上限時間で打ち切り、stdout/stderr を回収する。
///
/// `Command::output()` は待ち時間を制限できないため使わない。
/// パイプを読み切らないと子が終われないので、読み取りは専用
/// スレッドで行う。
fn run_command(program: &str, args: &[String], limit: Duration) -> CommandOutcome {
    let start = Instant::now();
    let mut child = match Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            return CommandOutcome::SpawnFailed {
                message: err.to_string(),
                elapsed: start.elapsed(),
            }
        }
    };

    let out_handle = spawn_reader(child.stdout.take());
    let err_handle = spawn_reader(child.stderr.take());

    let mut timed_out = false;
    let mut wait_error = None;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if start.elapsed() >= limit {
                    let _ = child.kill();
                    let _ = child.wait();
                    timed_out = true;
                    break None;
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(err) => {
                let _ = child.kill();
                let _ = child.wait();
                wait_error = Some(err.to_string());
                break None;
            }
        }
    };

    let drain_deadline = Instant::now() + DRAIN_LIMIT;
    let stdout = collect_output(out_handle, drain_deadline);
    let stderr = collect_output(err_handle, drain_deadline);
    let elapsed = start.elapsed();

    if timed_out {
        return CommandOutcome::TimedOut { elapsed };
    }
    if let Some(message) = wait_error {
        return CommandOutcome::SpawnFailed { message, elapsed };
    }
    match status {
        Some(status) => CommandOutcome::Completed {
            status,
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
            elapsed,
        },
        None => CommandOutcome::SpawnFailed {
            message: "command returned no status".to_string(),
            elapsed,
        },
    }
}

fn sysfs_value(interface: &str, name: &str) -> Option<String> {
    fs::read_to_string(format!("/sys/class/net/{}/{}", interface, name))
        .ok()
        .map(|v| v.trim().to_string())
}

fn interface_ipv4(interface: &str, errors: &mut Vec<String>) -> Option<String> {
    let args = words(&["-4", "-o", "addr", "show", "dev", interface]);
    match run_command("ip", &args, IP_TIMEOUT) {
        CommandOutcome::Completed { status, stdout, .. } if status.success() => {
            parse_ipv4_addr(&stdout)
        }
        CommandOutcome::Completed { status, .. } => {
            errors.push(format!("ip-addr-exit({})", status));
            None
        }
        CommandOutcome::TimedOut { .. } => {
            errors.push("ip-addr-timeout".to_string());
            None
        }
        CommandOutcome::SpawnFailed { message, .. } => {
            errors.push(format!("ip-addr-spawn: {}", message));
            None
        }
    }
}

fn default_gateway(errors: &mut Vec<String>) -> Option<String> {
    let args = words(&["-4", "route", "show", "default"]);
    match run_command("ip", &args, IP_TIMEOUT) {
        CommandOutcome::Completed { status, stdout, .. } if status.success() => {
            parse_default_gateway(&stdout)
        }
        CommandOutcome::Completed { status, .. } => {
            errors.push(format!("ip-route-exit({})", status));
            None
        }
        CommandOutcome::TimedOut { .. } => {
            errors.push("ip-route-timeout".to_string());
            None
        }
        CommandOutcome::SpawnFailed { message, .. } => {
            errors.push(format!("ip-route-spawn: {}", message));
            None
        }
    }
}

fn ping_ok(target: &str, errors: &mut Vec<String>) -> bool {
    let args = words(&["-n", "-c", "1", "-W", "1", target]);
    match run_command("ping", &args, PING_TIMEOUT) {
        CommandOutcome::Completed { status, .. } => status.success(),
        CommandOutcome::TimedOut { .. } => {
            errors.push(format!("ping-timeout({})", target));
            false
        }
        CommandOutcome::SpawnFailed { message, .. } => {
            errors.push(format!("ping-spawn: {}", message));
            false
        }
    }
}

fn resolver_ok(host: &str, errors: &mut Vec<String>) -> bool {
    let args = words(&["ahostsv4", host]);
    match run_command("getent", &args, DNS_TIMEOUT) {
        CommandOutcome::Completed { status, .. } => status.success(),
        CommandOutcome::TimedOut { .. } => {
            errors.push("dns-timeout".to_string());
            false
        }
        CommandOutcome::SpawnFailed { message, .. } => {
            errors.push(format!("getent-spawn: {}", message));
            false
        }
    }
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
    let args = words(&["+%:z"]);
    match run_command("date", &args, DATE_TIMEOUT) {
        CommandOutcome::Completed { status, stdout, .. } if status.success() => {
            let value = stdout.trim().to_string();
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
            interface_present: true,
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
            probe_errors: Vec::new(),
            measurement_failed: false,
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

    #[test]
    fn test_health_unknown_on_measurement_failure() {
        let mut snap = snapshot(true, Some("192.168.128.111"), true, true, true, true, true);
        snap.measurement_failed = true;
        snap.probe_errors.push("dns-timeout".to_string());
        assert_eq!(snap.health(), Health::Unknown);
        assert_eq!(snap.reason(), "dns-timeout");
    }

    #[test]
    fn test_health_interface_missing() {
        let mut snap = snapshot(true, Some("192.168.128.111"), true, true, true, true, true);
        snap.interface_present = false;
        assert_eq!(snap.health(), Health::Down);
        assert_eq!(snap.reason(), "interface-missing");
    }

    #[test]
    fn test_interface_exists() {
        assert!(interface_exists("lo"));
        assert!(!interface_exists("definitely-not-an-interface"));
        assert!(!interface_exists(""));
        assert!(!interface_exists("../lo"));
        assert!(!interface_exists("."));
        assert!(!interface_exists(".."));
    }

    #[test]
    fn test_run_rejects_missing_interface() {
        let cfg = WatchConfig {
            interface: "definitely-not-an-interface".to_string(),
            interval: Duration::from_secs(1),
            out: std::env::temp_dir().join("system-diag-unused.jsonl"),
            once: true,
            quiet: true,
        };
        let err = run(&cfg).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn test_heartbeat_path() {
        assert_eq!(
            heartbeat_path(Path::new("/tmp/net-watch.jsonl")),
            PathBuf::from("/tmp/net-watch.heartbeat")
        );
    }

    #[test]
    fn test_run_command_timeout() {
        let outcome = run_command("/bin/sleep", &["2".to_string()], Duration::from_millis(100));
        match outcome {
            CommandOutcome::TimedOut { elapsed } => {
                assert!(elapsed >= Duration::from_millis(100));
                assert!(elapsed < Duration::from_secs(2));
            }
            other => panic!("unexpected outcome: {:?}", other),
        }
    }

    #[test]
    fn test_run_command_completed() {
        let outcome = run_command(
            "/bin/sh",
            &[
                "-c".to_string(),
                "echo out; echo err >&2; exit 3".to_string(),
            ],
            Duration::from_secs(2),
        );
        match outcome {
            CommandOutcome::Completed {
                status,
                stdout,
                stderr,
                ..
            } => {
                assert_eq!(stdout, "out\n");
                assert_eq!(stderr, "err\n");
                assert_eq!(status.code(), Some(3));
            }
            other => panic!("unexpected outcome: {:?}", other),
        }
    }

    #[test]
    fn test_run_command_spawn_failed() {
        let outcome = run_command("/nonexistent/command", &[], Duration::from_secs(1));
        match outcome {
            CommandOutcome::SpawnFailed { .. } => {}
            other => panic!("unexpected outcome: {:?}", other),
        }
    }

    #[test]
    fn test_run_command_bounds_inherited_pipe() {
        // 子 (sh) は即終了するが、孫 (sleep) が stdout を保持し続ける。
        // EOF 待ちで監視ループが止まらないことを確認する。
        let outcome = run_command(
            "/bin/sh",
            &["-c".to_string(), "sleep 2 & echo hi".to_string()],
            Duration::from_secs(5),
        );
        match outcome {
            CommandOutcome::Completed {
                stdout, elapsed, ..
            } => {
                assert_eq!(stdout, "hi\n");
                assert!(
                    elapsed < Duration::from_millis(1500),
                    "elapsed too long: {:?}",
                    elapsed
                );
            }
            other => panic!("unexpected outcome: {:?}", other),
        }
    }

    #[test]
    fn test_run_command_large_output_does_not_deadlock() {
        let outcome = run_command(
            "/bin/sh",
            &["-c".to_string(), "yes x | head -c 200000".to_string()],
            Duration::from_secs(5),
        );
        match outcome {
            CommandOutcome::Completed { status, stdout, .. } => {
                assert!(status.success());
                assert_eq!(stdout.len(), 200_000);
            }
            other => panic!("unexpected outcome: {:?}", other),
        }
    }

    #[test]
    fn test_write_heartbeat_is_valid_json() {
        let dir = std::env::temp_dir().join(format!("system-diag-test-{}", std::process::id()));
        let out = dir.join("net-watch.jsonl");
        let cfg = WatchConfig {
            interface: "lo".to_string(),
            interval: Duration::from_secs(5),
            out: out.clone(),
            once: false,
            quiet: true,
        };

        write_heartbeat(&cfg, 1_700_000_000, 32_400, 7, 42, 1_699_999_999).unwrap();

        let text = std::fs::read_to_string(heartbeat_path(&out)).unwrap();
        let value: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(value["schema_version"], json!(2));
        assert_eq!(value["cycle"], json!(7));
        assert_eq!(value["duration_ms"], json!(42));
        assert_eq!(value["last_event_epoch"], json!(1_699_999_999_u64));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
