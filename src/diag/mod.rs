pub trait Diagnostic {
    fn name(&self) -> &str;
    fn run(&self) -> anyhow::Result<String>;
    fn report(&self) -> serde_json::Value;
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
