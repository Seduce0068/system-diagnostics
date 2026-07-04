//! ```cargo
//! [dependencies]
//! anyhow = "1.0"
//! socket2 = "0.5"
//! nix = { version = "0.27", features = ["net"] }
//! serde = { version = "1.0", features = ["derive"] }
//! serde_json = "1.0"
//! ```

use std::fs;
use std::time::Duration;
use anyhow::{Result, anyhow, Context};
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Debug)]
enum PingStatus {
    Success,
    TimedOut,
    FragmentationNeeded,
    Error(String),
}

#[derive(Serialize, Deserialize, Debug)]
struct PingResult {
    target: String,
    mtu: u32,
    status: PingStatus,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct IcmpHeader {
    icmp_type: u8,
    icmp_code: u8,
    icmp_checksum: u16,
    icmp_id: u16,
    icmp_seq: u16,
}

fn get_default_gateway() -> Result<String> {
    let content = fs::read_to_string("/proc/net/route")?;
    for line in content.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() > 2 && fields[1] != "00000000" {
            let gw_hex = u32::from_str_radix(fields[2], 16)?;
            let ip = std::net::Ipv4Addr::from(gw_hex.to_le());
            return Ok(ip.to_string());
        }
    }
    Err(anyhow!("デフォルトゲートウェイが見つかりません"))
}

#[cfg(not(test))]
fn main() -> Result<()> {
    let interface = get_default_interface()?;
    let mut targets = vec![];
    if let Ok(gw) = get_default_gateway() {
        targets.push(gw);
    }
    targets.extend(vec!["8.8.8.8".to_string(), "1.1.1.1".to_string()]);

    let mut results = vec![];
    for target in targets {
        let mtu = detect_optimal_mtu(&interface, &target)?;
        results.push(PingResult { target, mtu, status: PingStatus::Success });
    }
    
    println!("{}", serde_json::to_string_pretty(&results)?);
    Ok(())
}

fn get_default_interface() -> Result<String> {
    use nix::ifaddrs::getifaddrs;
    use nix::net::if_::InterfaceFlags;

    let addrs = getifaddrs()?;
    for ifaddr in addrs {
        if ifaddr.flags.contains(InterfaceFlags::IFF_UP)
            && ifaddr.flags.contains(InterfaceFlags::IFF_RUNNING)
            && !ifaddr.flags.contains(InterfaceFlags::IFF_LOOPBACK)
        {
            return Ok(ifaddr.interface_name);
        }
    }
    Err(anyhow!("アクティブなネットワークインターフェースが見つかりません"))
}

fn calculate_checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut chunks = data.chunks_exact(2);
    while let Some(chunk) = chunks.next() {
        sum += u16::from_be_bytes([chunk[0], chunk[1]]) as u32;
    }
    if let Some(&last) = chunks.remainder().first() {
        sum += (last as u32) << 8;
    }
    while (sum >> 16) != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

pub fn build_icmp_echo_request(payload_size: u32, is_ipv6: bool, src: &std::net::IpAddr, dst: &std::net::IpAddr) -> Vec<u8> {
    let mut packet = vec![0u8; (std::mem::size_of::<IcmpHeader>() + payload_size as usize)];
    let header = IcmpHeader {
        icmp_type: if is_ipv6 { 128 } else { 8 },
        icmp_code: 0,
        icmp_checksum: 0,
        icmp_id: std::process::id() as u16,
        icmp_seq: 1,
    };

    unsafe {
        std::ptr::copy_nonoverlapping(
            &header as *const _ as *const u8,
            packet.as_mut_ptr(),
            std::mem::size_of::<IcmpHeader>(),
        );
    }

    if is_ipv6 {
        let mut pseudo_header = Vec::new();
        if let std::net::IpAddr::V6(s) = src { pseudo_header.extend(s.octets()); }
        if let std::net::IpAddr::V6(d) = dst { pseudo_header.extend(d.octets()); }
        pseudo_header.extend((packet.len() as u32).to_be_bytes());
        pseudo_header.extend([0, 0, 0, 58]);
        
        pseudo_header.extend(&packet);
        let checksum = calculate_checksum(&pseudo_header);
        let checksum_bytes = checksum.to_be_bytes();
        packet[2] = checksum_bytes[0];
        packet[3] = checksum_bytes[1];
    } else {
        let checksum = calculate_checksum(&packet);
        let checksum_bytes = checksum.to_be_bytes();
        packet[2] = checksum_bytes[0];
        packet[3] = checksum_bytes[1];
    }
    packet
}

fn detect_optimal_mtu(interface: &str, target: &str) -> Result<u32> {
    let mut low = 576; 
    let mut high = 1500;
    let mut optimal = low;

    while low <= high {
        let mid = (low + high) / 2;
        let payload = mid - 28;
        
        let res = ping_with_retry(target, payload, 3, interface)?;
        if matches!(res.status, PingStatus::Success) {
            optimal = mid;
            low = mid + 1;
        } else {
            high = mid - 1;
        }
    }
    
    Ok(optimal)
}

fn ping_with_retry(target: &str, payload_size: u32, retries: usize, interface: &str) -> Result<PingResult> {
    let mut last_result = Err(anyhow!("No retries"));
    for _ in 0..retries {
        let res = perform_ping(target, payload_size, interface)?;
        if matches!(res.status, PingStatus::Success) {
            return Ok(res);
        }
        last_result = Ok(res);
        std::thread::sleep(Duration::from_millis(100));
    }
    last_result
}

fn perform_ping(target: &str, payload_size: u32, interface: &str) -> Result<PingResult> {
    use socket2::{Socket, Domain, Type, Protocol, SockAddr};
    use std::net::{IpAddr, SocketAddr};
    use std::time::Duration;

    let target_ip: IpAddr = target.parse().context("Invalid target IP")?;
    let is_ipv6 = target_ip.is_ipv6();
    
    let domain = if is_ipv6 { Domain::IPV6 } else { Domain::IPV4 };
    let protocol = if is_ipv6 { Protocol::ICMPV6 } else { Protocol::ICMPV4 };

    let socket = Socket::new(domain, Type::RAW, Some(protocol))?;
    if !is_ipv6 {
        socket.set_ip_mtu_discover(socket2::IpMtuDiscover::Do)?;
    }
    
    socket.bind_device(Some(interface.as_bytes()))
        .context("Failed to bind to interface")?;
    
    socket.set_read_timeout(Some(Duration::from_secs(1)))?;

    let dest_addr = SockAddr::from(SocketAddr::new(target_ip, 0));
    let packet = build_icmp_echo_request(payload_size, is_ipv6, &"127.0.0.1".parse().unwrap(), &target_ip);

    socket.send_to(&packet, &dest_addr)?;

    let mut buf = [0u8; 1024];
    match socket.recv_from(&mut unsafe { std::mem::transmute(&mut buf) }) {
        Ok((size, _)) => {
            if is_ipv6 {
                if buf[0] == 129 {
                    Ok(PingResult { target: target.to_string(), mtu: payload_size + 48, status: PingStatus::Success })
                } else {
                    Ok(PingResult { target: target.to_string(), mtu: payload_size + 48, status: PingStatus::Error("Unknown ICMPv6".to_string()) })
                }
            } else {
                let ip_header_len = ((buf[0] & 0x0F) as usize) * 4;
                if size >= ip_header_len + std::mem::size_of::<IcmpHeader>() {
                    let icmp_type = buf[ip_header_len];
                    if icmp_type == 0 {
                        Ok(PingResult { target: target.to_string(), mtu: payload_size + 28, status: PingStatus::Success })
                    } else if icmp_type == 3 && buf[ip_header_len + 1] == 4 {
                        Ok(PingResult { target: target.to_string(), mtu: payload_size + 28, status: PingStatus::FragmentationNeeded })
                    } else {
                        Ok(PingResult { target: target.to_string(), mtu: payload_size + 28, status: PingStatus::Error(format!("ICMP Type: {}", icmp_type)) })
                    }
                } else {
                    Ok(PingResult { target: target.to_string(), mtu: payload_size + 28, status: PingStatus::Error("Truncated".to_string()) })
                }
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {
            Ok(PingResult { target: target.to_string(), mtu: payload_size + 28, status: PingStatus::TimedOut })
        },
        Err(e) => Err(e.into()),
    }
}
