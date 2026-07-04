use anyhow::{Result, anyhow};
use pnet::datalink;

pub fn get_default_interface() -> Result<String> {
    for interface in datalink::interfaces() {
        if interface.is_up() && !interface.is_loopback() && !interface.ips.is_empty() {
            return Ok(interface.name);
        }
    }
    Err(anyhow!("アクティブなネットワークインターフェースが見つかりません"))
}
