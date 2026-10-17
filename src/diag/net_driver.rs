use crate::diag::{
    error::DiagnosticError, sys_interface::SystemInterface, Diagnostic, DiagnosticData,
};
use serde_json::json;

pub struct LanDriverDiagnostic<S: SystemInterface> {
    pub sys: S,
}

impl<S: SystemInterface> Diagnostic for LanDriverDiagnostic<S> {
    fn id(&self) -> &str {
        "lan_driver"
    }

    fn run(&self) -> Result<DiagnosticData, DiagnosticError> {
        let mut interfaces_data = Vec::new();
        let mut warnings = Vec::new();
        let mut normals = Vec::new();

        let blacklist_active = check_blacklist_r8169(&self.sys);

        if let Ok(entries) = self.sys.read_dir("/sys/class/net") {
            for entry in entries {
                let iface_name = match entry.file_name() {
                    Some(name) => name.to_string_lossy().into_owned(),
                    None => continue,
                };

                let uevent_path = entry.join("device/uevent");
                if let Ok(uevent_content) = self.sys.read_to_string(&uevent_path) {
                    let mut driver = None;
                    let mut pci_id = None;

                    for line in uevent_content.lines() {
                        if let Some(value) = line.strip_prefix("DRIVER=") {
                            driver = Some(value.trim().to_string());
                        } else if let Some(value) = line.strip_prefix("PCI_ID=") {
                            pci_id = Some(value.trim().to_string());
                        }
                    }

                    if let Some(drv) = driver {
                        let mut issue_detected = false;
                        let mut recommendation = None;

                        if drv == "r8168" {
                            issue_detected = true;
                            recommendation = Some("不具合報告が多い Realtek 独自ドライバ r8168 が使用されています。標準の r8169 ドライバへの移行（r8168-dkmsパッケージの削除や設定解除）を強く推奨します。".to_string());
                            warnings.push(format!(
                                "⚠️ 警告: 有線LAN ({}) で不具合の多い独自ドライバ (r8168) が使用されています。ルータ再起動時やスリープ復帰時に接続が切れる原因となります。安定した標準ドライバ (r8169) への移行を推奨します。",
                                iface_name
                            ));
                        } else {
                            normals
                                .push(format!("有線LAN ({}) ドライバ: {} (正常)", iface_name, drv));
                        }

                        interfaces_data.push(json!({
                            "name": iface_name,
                            "driver": drv,
                            "pci_id": pci_id,
                            "issue_detected": issue_detected,
                            "recommendation": recommendation,
                        }));
                    }
                }
            }
        }

        let readable = if !warnings.is_empty() {
            warnings.join("\n")
        } else if !normals.is_empty() {
            normals.join("\n")
        } else {
            "有線LANインターフェースが検出されませんでした。".to_string()
        };

        let raw = json!({
            "interfaces": interfaces_data,
            "blacklist_r8169_active": blacklist_active,
        });

        Ok(DiagnosticData { raw, readable })
    }
}

fn check_blacklist_r8169<S: SystemInterface>(sys: &S) -> bool {
    if let Ok(entries) = sys.read_dir("/etc/modprobe.d") {
        for entry in entries {
            if let Ok(content) = sys.read_to_string(&entry) {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with("blacklist") {
                        let parts: Vec<&str> = trimmed.split_whitespace().collect();
                        if parts.len() >= 2 && parts[1] == "r8169" {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    struct MockSystem {
        files: HashMap<PathBuf, String>,
        dirs: HashMap<PathBuf, Vec<PathBuf>>,
    }

    impl SystemInterface for MockSystem {
        fn read_to_string<P: AsRef<Path>>(&self, path: P) -> Result<String, DiagnosticError> {
            let p = path.as_ref().to_path_buf();
            self.files.get(&p).cloned().ok_or_else(|| {
                DiagnosticError::Io(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "file not found",
                ))
            })
        }

        fn read_dir<P: AsRef<Path>>(&self, path: P) -> Result<Vec<PathBuf>, DiagnosticError> {
            let p = path.as_ref().to_path_buf();
            self.dirs.get(&p).cloned().ok_or_else(|| {
                DiagnosticError::Io(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "dir not found",
                ))
            })
        }

        fn run_command(&self, _program: &str, _args: &[&str]) -> Result<String, DiagnosticError> {
            Ok(String::new())
        }
    }

    #[test]
    fn test_lan_driver_diagnostic_r8168_warning() {
        let mut files = HashMap::new();
        let mut dirs = HashMap::new();

        dirs.insert(
            PathBuf::from("/sys/class/net"),
            vec![
                PathBuf::from("/sys/class/net/enp4s0"),
                PathBuf::from("/sys/class/net/lo"),
            ],
        );

        files.insert(
            PathBuf::from("/sys/class/net/enp4s0/device/uevent"),
            "DRIVER=r8168\nPCI_ID=10EC:8168\n".to_string(),
        );

        dirs.insert(
            PathBuf::from("/etc/modprobe.d"),
            vec![PathBuf::from("/etc/modprobe.d/blacklist-r8169.conf")],
        );

        files.insert(
            PathBuf::from("/etc/modprobe.d/blacklist-r8169.conf"),
            "blacklist r8169\n".to_string(),
        );

        let sys = MockSystem { files, dirs };
        let diag = LanDriverDiagnostic { sys };
        let res = diag.run().unwrap();

        assert!(res.raw["blacklist_r8169_active"].as_bool().unwrap());
        assert_eq!(
            res.raw["interfaces"][0]["driver"].as_str().unwrap(),
            "r8168"
        );
        assert!(res.raw["interfaces"][0]["issue_detected"]
            .as_bool()
            .unwrap());
        assert!(res.readable.contains(
            "⚠️ 警告: 有線LAN (enp4s0) で不具合の多い独自ドライバ (r8168) が使用されています"
        ));
    }

    #[test]
    fn test_lan_driver_diagnostic_r8169_normal() {
        let mut files = HashMap::new();
        let mut dirs = HashMap::new();

        dirs.insert(
            PathBuf::from("/sys/class/net"),
            vec![PathBuf::from("/sys/class/net/enp4s0")],
        );

        files.insert(
            PathBuf::from("/sys/class/net/enp4s0/device/uevent"),
            "DRIVER=r8169\nPCI_ID=10EC:8168\n".to_string(),
        );

        dirs.insert(PathBuf::from("/etc/modprobe.d"), vec![]);

        let sys = MockSystem { files, dirs };
        let diag = LanDriverDiagnostic { sys };
        let res = diag.run().unwrap();

        assert!(!res.raw["blacklist_r8169_active"].as_bool().unwrap());
        assert_eq!(
            res.raw["interfaces"][0]["driver"].as_str().unwrap(),
            "r8169"
        );
        assert!(!res.raw["interfaces"][0]["issue_detected"]
            .as_bool()
            .unwrap());
        assert!(res
            .readable
            .contains("有線LAN (enp4s0) ドライバ: r8169 (正常)"));
    }
}
