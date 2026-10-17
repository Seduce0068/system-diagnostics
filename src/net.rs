use crate::diag::error::DiagnosticError;
use pnet::datalink;

pub fn get_default_interface() -> Result<String, DiagnosticError> {
    for interface in datalink::interfaces() {
        if interface.is_up() && !interface.is_loopback() && !interface.ips.is_empty() {
            return Ok(interface.name);
        }
    }
    Err(DiagnosticError::DiagnosticFailed(
        "アクティブなネットワークインターフェースが見つかりません".into(),
    ))
}
