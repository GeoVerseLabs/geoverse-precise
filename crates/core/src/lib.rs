//! # geoverse-precise-core
//!
//! Ellipsoid-accurate spatial analysis for client-side use (compiled to WASM by
//! `geoverse-precise-wasm`, but this crate has no JS dependency).
//!
//! * [`geodesic`] / [`measure`] – Karney geodesics on WGS84: distance, bearing,
//!   destination, length, area, along, nearest point on line.
//! * [`crs`] – WGS84 / CGCS2000 / GCJ-02 / BD-09 / Web Mercator / UTM /
//!   CGCS2000 Gauss-Krüger transforms.
//! * [`buffer`] – geodesic buffer (exact vertices) and fast projected buffer.
//! * [`overlay`] – intersection / union / difference in a local conformal plane.
//! * [`predicates`] – point in polygon etc.
//!
//! All geometries are `geo-types` with `x = longitude`, `y = latitude` in degrees
//! unless stated otherwise.

pub mod api;
pub mod api_ext;
pub mod buffer;
pub mod cluster;
pub mod crs;
pub mod densify;
pub mod error;
pub mod geodesic;
pub mod geojson_util;
pub mod gnomonic;
pub mod grids;
pub mod index;
pub mod interp;
pub mod lines;
pub mod local;
pub mod measure;
pub mod ops;
pub mod overlay;
pub mod predicates;
pub mod rhumb;
pub mod shapes;
pub mod stats;
pub mod topology;
pub mod units;
pub mod validate;

pub use error::{Error, Result};
pub use geo;
pub use geojson;
