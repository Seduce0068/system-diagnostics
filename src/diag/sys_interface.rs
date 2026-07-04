use crate::diag::error::DiagnosticError;
use std::path::Path;

pub trait SystemInterface {
    fn read_to_string<P: AsRef<Path>>(&self, path: P) -> Result<String, DiagnosticError>;
    fn read_dir<P: AsRef<Path>>(&self, path: P) -> Result<Vec<std::path::PathBuf>, DiagnosticError>;
}

pub struct RealSystem;

impl SystemInterface for RealSystem {
    fn read_to_string<P: AsRef<Path>>(&self, path: P) -> Result<String, DiagnosticError> {
        Ok(std::fs::read_to_string(path).map_err(|e| DiagnosticError::Io(e))?)
    }
    fn read_dir<P: AsRef<Path>>(&self, path: P) -> Result<Vec<std::path::PathBuf>, DiagnosticError> {
        let mut paths = Vec::new();
        for entry in std::fs::read_dir(path).map_err(|e| DiagnosticError::Io(e))? {
            paths.push(entry.map_err(|e| DiagnosticError::Io(e))?.path());
        }
        Ok(paths)
    }
}
