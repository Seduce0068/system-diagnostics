use anyhow::Result;
use std::path::Path;

pub trait SystemInterface {
    fn read_to_string<P: AsRef<Path>>(&self, path: P) -> Result<String>;
    fn read_dir<P: AsRef<Path>>(&self, path: P) -> Result<Vec<std::path::PathBuf>>;
}

pub struct RealSystem;

impl SystemInterface for RealSystem {
    fn read_to_string<P: AsRef<Path>>(&self, path: P) -> Result<String> {
        Ok(std::fs::read_to_string(path)?)
    }
    fn read_dir<P: AsRef<Path>>(&self, path: P) -> Result<Vec<std::path::PathBuf>> {
        let mut paths = Vec::new();
        for entry in std::fs::read_dir(path)? {
            paths.push(entry?.path());
        }
        Ok(paths)
    }
}
