use mtu_diag_tool::net;
use anyhow::Result;

fn main() -> Result<()> {
    println!("Starting MTU diagnosis...");
    let interface = net::get_default_interface()?;
    println!("Interface: {}", interface);
    Ok(())
}
