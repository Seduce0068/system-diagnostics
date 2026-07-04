#[path = "mtu_diag.rs"]
mod mtu_diag;

#[test]
fn test_build_icmp_echo_request_size() {
    let payload_size = 100;
    let packet = mtu_diag::build_icmp_echo_request(payload_size);
    // ICMP Header (8 bytes) + Payload (100 bytes) = 108 bytes
    assert_eq!(packet.len(), 108);
}

#[test]
fn test_icmp_header_type() {
    let packet = mtu_diag::build_icmp_echo_request(10);
    // ICMP Type (Echo Request = 8) is at index 0
    assert_eq!(packet[0], 8);
}
