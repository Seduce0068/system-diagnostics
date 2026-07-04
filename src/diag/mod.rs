use anyhow::Result;
use serde_json::Value;

pub struct DiagnosticData {
    pub raw: Value,
    pub readable: String,
}

pub trait Diagnostic {
    fn id(&self) -> &str;
    fn run(&self) -> Result<DiagnosticData>;
}

pub fn get_all_diagnostics() -> Vec<Box<dyn Diagnostic>> {
    vec![
        Box::new(system::UptimeLoadAvgDiagnostic),
        Box::new(cpu::CpuFrequencyDiagnostic { sys: sys_interface::RealSystem }),
        Box::new(cpu_vuln::CpuVulnerabilityDiagnostic),
        Box::new(net::LocalGatewayPingDiagnostic),
        Box::new(conn::ActiveConnectionDiagnostic),
        Box::new(user::ActiveUsersSshDiagnostic),
        Box::new(mtu::NetworkMTUDiagnostic),
        Box::new(time::NtpTimeSyncDiagnostic),
        Box::new(dns::DnsLatencyDiagnostic),
    ]
}

pub mod system;
pub mod cpu;
pub mod cpu_vuln;
pub mod net;
pub mod mtu;
pub mod dns;
pub mod time;
pub mod conn;
pub mod user;
pub mod sys_interface;
