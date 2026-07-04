//! ```cargo
//! [dependencies]
//! anyhow = "1.0"
//! socket2 = "0.5"
//! nix = { version = "0.27", features = ["net"] }
//! ```

use std::fs;
use std::time::Duration;
use anyhow::{Result, anyhow, Context};

fn get_default_gateway() -> Result<String> {
    let content = fs::read_to_string("/proc/net/route")?;
    for line in content.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        // ゲートウェイのIPが00000000以外のレコードを探す
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
    println!("=== 堅牢なMTU 診断ツール ===");

    let interface = get_default_interface()?;
    println!("診断対象インターフェース: {}", interface);

    let mut targets = vec![];
    if let Ok(gw) = get_default_gateway() {
        println!("ゲートウェイ検出: {}", gw);
        targets.push(gw);
    }
    targets.extend(vec!["8.8.8.8".to_string(), "1.1.1.1".to_string()]);

    for target in targets {
        println!("Testing target: {}", target);
        match detect_optimal_mtu(&interface, &target) {
            Ok(mtu) => println!("推奨MTU (via {} - {}): {}", interface, target, mtu),
            Err(e) => println!("診断失敗 (via {} - {}): {}", interface, target, e),
        }
    }
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

pub fn build_icmp_echo_request(payload_size: u32) -> Vec<u8> {
    #[repr(C)]
    #[derive(Debug, Clone, Copy)]
    struct IcmpHeader {
        icmp_type: u8,
        icmp_code: u8,
        icmp_checksum: u16,
        icmp_id: u16,
        icmp_seq: u16,
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

    let mut packet = vec![0u8; (std::mem::size_of::<IcmpHeader>() + payload_size as usize)];
    let header = IcmpHeader {
        icmp_type: 8, // Echo Request
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

    let checksum = calculate_checksum(&packet);
    let checksum_bytes = checksum.to_be_bytes();
    packet[2] = checksum_bytes[0];
    packet[3] = checksum_bytes[1];
    packet
}

fn detect_optimal_mtu(interface: &str, target: &str) -> Result<u32> {
    let mut low = 576; 
    let mut high = 1500;
    let mut optimal = low;

    while low <= high {
        let mid = (low + high) / 2;
        let payload = mid - 28;
        
        if ping_with_retry(target, payload, 3, interface)? {
            optimal = mid;
            low = mid + 1;
        } else {
            high = mid - 1;
        }
    }
    
    Ok(optimal)
}

fn ping_with_retry(target: &str, payload_size: u32, retries: usize, interface: &str) -> Result<bool> {
    for _ in 0..retries {
        if perform_ping(target, payload_size, interface)? {
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(false)
}

// 堅牢にインターフェースを指定してICMP Echo Requestを実行する
// 注意: Raw Socketを使用するため、実行にはroot権限が必要です。
fn perform_ping(target: &str, payload_size: u32, interface: &str) -> Result<bool> {
    use socket2::{Socket, Domain, Type, Protocol, SockAddr};
    use std::net::{IpAddr, SocketAddr};
    use std::time::Duration;

    let socket = Socket::new(Domain::IPV4, Type::RAW, Some(Protocol::ICMPV4))?;
    // DF (Don't Fragment) ビットを有効にする
    socket.set_ip_mtu_discover(socket2::IpMtuDiscover::Do)?;
    socket.bind_device(Some(interface.as_bytes()))
        .context("Failed to bind to interface")?;
    
    // タイムアウトを1秒に設定
    socket.set_read_timeout(Some(Duration::from_secs(1)))?;

    let target_ip: IpAddr = target.parse().context("Invalid target IP")?;
    let dest_addr = SockAddr::from(SocketAddr::new(target_ip, 0));

    // ICMPパケットの構築
    let packet = build_icmp_echo_request(payload_size);

    // 送信
    socket.send_to(&packet, &dest_addr)?;

    // 受信確認
    let mut buf = [0u8; 1024];
    match socket.recv_from(&mut unsafe { std::mem::transmute(&mut buf) }) {
        Ok((size, _)) => {
            // 受信バッファの最初のバイトの後半4ビット * 4 がIPヘッダーの長さ
            let ip_header_len = ((buf[0] & 0x0F) as usize) * 4;
            if size >= ip_header_len + std::mem::size_of::<IcmpHeader>() {
                let icmp_type = buf[ip_header_len];
                return Ok(icmp_type == 0); // Echo Reply
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {
            return Ok(false);
        },
        Err(e) => return Err(e.into()),
    }

    Ok(false)
}
