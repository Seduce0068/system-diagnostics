use crate::diag::error::DiagnosticError;
use serde_json::Value;

pub struct DiagnosticData {
    pub raw: Value,
    pub readable: String,
}

pub trait Diagnostic {
    fn id(&self) -> &str;
    fn run(&self) -> Result<DiagnosticData, DiagnosticError>;
}

pub fn get_all_diagnostics() -> Vec<Box<dyn Diagnostic>> {
    vec![
        Box::new(system::UptimeLoadAvgDiagnostic { sys: sys_interface::RealSystem }),
        Box::new(cpu::CpuFrequencyDiagnostic { sys: sys_interface::RealSystem }),
        Box::new(cpu_vuln::CpuVulnerabilityDiagnostic { sys: sys_interface::RealSystem }),
        Box::new(net::LocalGatewayPingDiagnostic { sys: sys_interface::RealSystem }),
        Box::new(conn::ActiveConnectionDiagnostic { sys: sys_interface::RealSystem }),
        Box::new(user::ActiveUsersSshDiagnostic { sys: sys_interface::RealSystem }),
        Box::new(mtu::NetworkMTUDiagnostic { sys: sys_interface::RealSystem }),
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
pub mod error;

