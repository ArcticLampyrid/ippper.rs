use thiserror::Error;
use ipp::model::StatusCode;

#[derive(Error, Debug, Clone)]
#[error("{code} {msg:?}")]
pub struct IppError {
    pub code: StatusCode,
    pub msg: String,
}