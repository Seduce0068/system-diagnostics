use pnet::packet::icmp::{echo_request::MutableEchoRequestPacket, IcmpPacket, IcmpTypes};
use pnet::packet::ipv4::Ipv4Packet;
use pnet::packet::Packet;
use anyhow::{Result, anyhow};
use socket2::{Socket, Domain, Protocol, SockAddr, Type};
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;
use std::mem::MaybeUninit;
use libc;


pub fn build_icmp_packet(payload_size: usize, identifier: u16, sequence: u16) -> Vec<u8> {
    let mut buf = vec![0u8; 8 + payload_size];
    let mut packet = MutableEchoRequestPacket::new(&mut buf).unwrap();
    packet.set_icmp_type(IcmpTypes::EchoRequest);
    packet.set_identifier(identifier);
    packet.set_sequence_number(sequence);
    
    let payload = vec![0u8; payload_size];
    packet.set_payload(&payload);
    
    buf
}

pub fn send_and_receive_icmp(target: IpAddr, ttl: u8) -> Result<IpAddr> {
    let domain = if target.is_ipv6() { Domain::IPV6 } else { Domain::IPV4 };
    let protocol = if target.is_ipv6() { Protocol::ICMPV6 } else { Protocol::ICMPV4 };

    // socket2 0.5+ requires explicit Type
    let socket = Socket::new(domain, Type::from(libc::SOCK_RAW), Some(protocol))?;
    
    if target.is_ipv4() {
        socket.set_ttl(ttl as u32)?;
    } else {
        socket.set_multicast_hops_v6(ttl as u32)?;
    }
    
    socket.set_read_timeout(Some(Duration::from_secs(1)))?;

    let packet = build_icmp_packet(32, std::process::id() as u16, ttl as u16);
    let dest_addr = SockAddr::from(SocketAddr::new(target, 0));
    
    socket.send_to(&packet, &dest_addr)?;

    let mut buf: [MaybeUninit<u8>; 1024] = unsafe { MaybeUninit::uninit().assume_init() };
    let (size, _) = socket.recv_from(&mut buf)?;
    
    // Convert MaybeUninit to u8
    let buf = unsafe { std::mem::transmute::<[MaybeUninit<u8>; 1024], [u8; 1024]>(buf) };
    
    // Parse IPv4 packet to get inner ICMP
    let ipv4_packet = Ipv4Packet::new(&buf[..size]).ok_or_else(|| anyhow!("Not an IPv4 packet"))?;
    let icmp_packet = IcmpPacket::new(ipv4_packet.payload()).ok_or_else(|| anyhow!("Not an ICMP packet"))?;
    
    if icmp_packet.get_icmp_type() == IcmpTypes::TimeExceeded || icmp_packet.get_icmp_type() == IcmpTypes::EchoReply {
        Ok(IpAddr::V4(ipv4_packet.get_source()))
    } else {
        Err(anyhow!("Received unexpected ICMP type: {:?}", icmp_packet.get_icmp_type()))
    }
}
