#[path = "network/api.rs"]
pub mod api;
#[path = "network/cache.rs"]
pub mod cache;
#[path = "network/download.rs"]
pub mod download;
#[path = "network/tap_formula.rs"]
pub mod tap_formula;

use rama::http::Response;

use crate::types::Error;

pub(crate) fn ensure_success(response: Response) -> Result<Response, Error> {
    if !response.status().is_success() {
        return Err(Error::NetworkFailure {
            message: format!("HTTP {}", response.status()),
        });
    }
    Ok(response)
}
