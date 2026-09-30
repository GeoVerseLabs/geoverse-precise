//! Thin, allocation-free wrapper around Karney's geodesic algorithms
//! (geographiclib-rs) on the WGS84 ellipsoid.
//!
//! Coordinates are `(lon, lat)` in degrees, matching GeoJSON order.

use std::sync::OnceLock;

use geographiclib_rs::{capability as caps, DirectGeodesic, Geodesic, GeodesicLine};

pub fn wgs84() -> &'static Geodesic {
    static G: OnceLock<Geodesic> = OnceLock::new();
    G.get_or_init(Geodesic::wgs84)
}

/// Result of an inverse problem.
#[derive(Debug, Clone, Copy)]
pub struct Inverse {
    /// Distance in metres.
    pub s12: f64,
    /// Azimuth at point 1, degrees clockwise from north, (-180, 180].
    pub azi1: f64,
    /// Forward azimuth at point 2.
    pub azi2: f64,
    /// Reduced length (m).
    pub m12: f64,
    /// Geodesic scale of point 2 relative to point 1.
    pub big_m12: f64,
}

/// Solve the inverse geodesic problem between `(lon1, lat1)` and `(lon2, lat2)`.
pub fn inverse(lon1: f64, lat1: f64, lon2: f64, lat2: f64) -> Inverse {
    let mask = caps::DISTANCE | caps::AZIMUTH | caps::REDUCEDLENGTH | caps::GEODESICSCALE;
    let (_a12, s12, azi1, azi2, m12, big_m12, _m21, _s) = wgs84()._gen_inverse_azi(lat1, lon1, lat2, lon2, mask);
    Inverse {
        s12,
        azi1,
        azi2,
        m12,
        big_m12,
    }
}

/// Distance in metres.
pub fn distance(lon1: f64, lat1: f64, lon2: f64, lat2: f64) -> f64 {
    let mask = caps::DISTANCE;
    let (_a12, s12, ..) = wgs84()._gen_inverse(lat1, lon1, lat2, lon2, mask);
    s12
}

/// Result of a direct problem.
#[derive(Debug, Clone, Copy)]
pub struct Direct {
    pub lon: f64,
    pub lat: f64,
    /// Forward azimuth at the destination.
    pub azi: f64,
    pub m12: f64,
    pub big_m12: f64,
}

/// Solve the direct problem: start at `(lon, lat)`, head `azi` degrees, travel `s` metres.
pub fn direct(lon: f64, lat: f64, azi: f64, s: f64) -> Direct {
    let (lat2, lon2, azi2, m12, big_m12, _m21): (f64, f64, f64, f64, f64, f64) = wgs84().direct(lat, lon, azi, s);
    Direct {
        lon: lon2,
        lat: lat2,
        azi: azi2,
        m12,
        big_m12,
    }
}

/// Destination point only (faster, fewer outputs).
pub fn destination(lon: f64, lat: f64, azi: f64, s: f64) -> (f64, f64) {
    let (lat2, lon2): (f64, f64) = wgs84().direct(lat, lon, azi, s);
    (lon2, lat2)
}

/// Point at fraction `t` (0..=1) along the geodesic between two points.
pub fn interpolate(lon1: f64, lat1: f64, lon2: f64, lat2: f64, t: f64) -> (f64, f64) {
    let inv = inverse(lon1, lat1, lon2, lat2);
    destination(lon1, lat1, inv.azi1, inv.s12 * t)
}

/// Normalise an angle to [-180, 180).
pub fn normalize_deg(a: f64) -> f64 {
    let mut x = (a + 180.0) % 360.0;
    if x < 0.0 {
        x += 360.0;
    }
    x - 180.0
}

/// Normalise a bearing to [0, 360).
pub fn to_bearing360(a: f64) -> f64 {
    let x = a % 360.0;
    if x < 0.0 {
        x + 360.0
    } else {
        x
    }
}

/// A geodesic line from a point with a given azimuth, for repeated positions.
pub struct Line {
    inner: GeodesicLine,
}

impl Line {
    pub fn new(lon: f64, lat: f64, azi: f64) -> Line {
        let mask = caps::LATITUDE | caps::LONGITUDE | caps::AZIMUTH | caps::DISTANCE_IN;
        Line {
            inner: GeodesicLine::new(wgs84(), lat, lon, azi, Some(mask), None, None),
        }
    }

    /// `(lon, lat, azi)` at distance `s` metres.
    pub fn position(&self, s: f64) -> (f64, f64, f64) {
        let mask = caps::LATITUDE | caps::LONGITUDE | caps::AZIMUTH;
        let (_a12, lat2, lon2, azi2, ..) = self.inner._gen_position(false, s, mask);
        (lon2, lat2, azi2)
    }
}
