use ipp::model::StatusCode;
use thiserror::Error;

#[derive(Error, Debug, Clone)]
#[error("{code} {msg:?}")]
pub struct IppError {
    pub code: StatusCode,
    pub msg: String,
}
