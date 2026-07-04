use crate::diag::error::DiagnosticError;
use std::path::Path;
use std::process::Command;

pub trait SystemInterface: Send + Sync {
    fn read_to_string<P: AsRef<Path>>(&self, path: P) -> Result<String, DiagnosticError>;
    fn read_dir<P: AsRef<Path>>(&self, path: P) -> Result<Vec<std::path::PathBuf>, DiagnosticError>;
    fn run_command(&self, program: &str, args: &[&str]) -> Result<String, DiagnosticError>;
}

pub struct RealSystem;

impl SystemInterface for RealSystem {
    fn read_to_string<P: AsRef<Path>>(&self, path: P) -> Result<String, DiagnosticError> {
        Ok(std::fs::read_to_string(path).map_err(DiagnosticError::Io)?)
    }
    fn read_dir<P: AsRef<Path>>(&self, path: P) -> Result<Vec<std::path::PathBuf>, DiagnosticError> {
        let mut paths = Vec::new();
        for entry in std::fs::read_dir(path).map_err(DiagnosticError::Io)? {
            paths.push(entry.map_err(DiagnosticError::Io)?.path());
        }
        Ok(paths)
    }
    fn run_command(&self, program: &str, args: &[&str]) -> Result<String, DiagnosticError> {
        let output = Command::new(program)
            .args(args)
            .output()
            .map_err(|e| DiagnosticError::DiagnosticFailed(format!("Command '{}' failed: {}", program, e)))?;
            
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}
