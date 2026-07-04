#[path = "mtu_diag.rs"]
mod mtu_diag;
use mtu_diag::{PingStatus, build_icmp_echo_request};

#[test]
fn test_build_icmp_echo_request_size() {
    let payload_size = 100;
    let src = "127.0.0.1".parse().unwrap();
    let dst = "8.8.8.8".parse().unwrap();
    let packet = build_icmp_echo_request(payload_size, false, &src, &dst);
    // ICMP Header (8 bytes) + Payload (100 bytes) = 108 bytes
    assert_eq!(packet.len(), 108);
}

#[test]
fn test_icmp_header_type() {
    let src = "127.0.0.1".parse().unwrap();
    let dst = "8.8.8.8".parse().unwrap();
    let packet = build_icmp_echo_request(10, false, &src, &dst);
    // ICMP Type (Echo Request = 8) is at index 0
    assert_eq!(packet[0], 8);
}
