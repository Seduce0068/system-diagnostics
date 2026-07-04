use thiserror::Error;

#[derive(Error, Debug)]
pub enum DiagnosticError {
    #[error("IOエラー: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("パースエラー: {0}")]
    Parse(String),
    
    #[error("権限エラー: {0}")]
    PermissionDenied(String),
    
    #[error("診断エラー: {0}")]
    DiagnosticFailed(String),

    #[error("不明なエラー: {0}")]
    Unknown(String),
}
