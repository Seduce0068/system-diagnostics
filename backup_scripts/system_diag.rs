//! ```cargo
//! [dependencies]
//! anyhow = "1.0"
//! ```

mod diag;

use diag::Diagnostic;
use diag::system::UptimeLoadAvgDiagnostic;
use diag::cpu::CpuFrequencyDiagnostic;
use diag::cpu_vuln::CpuVulnerabilityDiagnostic;
use diag::net::LocalGatewayPingDiagnostic;
use diag::mtu::NetworkMTUDiagnostic;
use diag::dns::DnsLatencyDiagnostic;
use diag::time::NtpTimeSyncDiagnostic;
use diag::conn::ActiveConnectionDiagnostic;
use diag::user::ActiveUsersSshDiagnostic;

fn main() -> anyhow::Result<()> {
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
    ];

    println!("--- システム診断開始 ---");
    for d in diagnostics {
        match d.run() {
            Ok(result) => println!("[{}] {}", d.name(), result),
            Err(e) => println!("[{}] ⚠️ 診断失敗: {}", d.name(), e),
        }
    }
    println!("--- 診断完了 ---");
    Ok(())
}
