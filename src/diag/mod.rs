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
        Box::new(system::UptimeLoadAvgDiagnostic {
            sys: sys_interface::RealSystem,
        }),
        Box::new(cpu::CpuFrequencyDiagnostic {
            sys: sys_interface::RealSystem,
        }),
        Box::new(cpu_vuln::CpuVulnerabilityDiagnostic {
            sys: sys_interface::RealSystem,
        }),
        Box::new(net::LocalGatewayPingDiagnostic {
            sys: sys_interface::RealSystem,
        }),
        Box::new(conn::ActiveConnectionDiagnostic {
            sys: sys_interface::RealSystem,
        }),
        Box::new(user::ActiveUsersSshDiagnostic {
            sys: sys_interface::RealSystem,
        }),
        Box::new(mtu::NetworkMTUDiagnostic {
            sys: sys_interface::RealSystem,
        }),
        Box::new(time::NtpTimeSyncDiagnostic),
        Box::new(dns::DnsLatencyDiagnostic),
        Box::new(net_driver::LanDriverDiagnostic {
            sys: sys_interface::RealSystem,
        }),
    ]
}

pub mod conn;
pub mod cpu;
pub mod cpu_vuln;
pub mod dns;
pub mod error;
pub mod mtu;
pub mod net;
pub mod net_driver;
pub mod sys_interface;
pub mod system;
pub mod time;
pub mod user;
