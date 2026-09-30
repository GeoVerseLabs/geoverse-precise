use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("unsupported CRS: {0}")]
    UnsupportedCrs(String),
    #[error("unsupported unit: {0}")]
    UnsupportedUnit(String),
    #[error("invalid geometry: {0}")]
    InvalidGeometry(String),
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    #[error("geometry extent too large for a local plane ({0:.0} km from centre); split the input")]
    ExtentTooLarge(f64),
    #[error("geojson: {0}")]
    GeoJson(#[from] geojson::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
