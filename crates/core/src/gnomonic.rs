//! Ellipsoidal gnomonic projection (Karney 2013, §8). Geodesics through the
//! centre are straight lines, and all geodesics are very nearly straight near
//! the centre, which makes it the right tool for "closest point on a geodesic".

use crate::geodesic;

const NUMIT: usize = 10;

/// Forward projection. Returns `None` when the point is more than ~90° away
/// (the projection is undefined there).
pub fn forward(lon0: f64, lat0: f64, lon: f64, lat: f64) -> Option<(f64, f64)> {
    let inv = geodesic::inverse(lon0, lat0, lon, lat);
    if inv.big_m12 <= 0.0 {
        return None;
    }
    let rho = inv.m12 / inv.big_m12;
    let az = inv.azi1.to_radians();
    Some((rho * az.sin(), rho * az.cos()))
}

/// Reverse projection.
pub fn reverse(lon0: f64, lat0: f64, x: f64, y: f64) -> Option<(f64, f64)> {
    let a = geodesic::wgs84().equatorial_radius();
    let azi0 = x.atan2(y).to_degrees();
    let mut rho = x.hypot(y);
    let mut s = a * (rho / a).atan();
    let little = rho <= a;
    if !little {
        rho = 1.0 / rho;
    }
    let eps = 0.01 * f64::EPSILON.sqrt();
    let mut trip = false;
    let mut out = None;
    for _ in 0..NUMIT {
        let d = geodesic::direct(lon0, lat0, azi0, s);
        out = Some((d.lon, d.lat));
        if trip {
            break;
        }
        let (m, big_m) = (d.m12, d.big_m12);
        let ds = if little {
            (m - rho * big_m) * big_m
        } else {
            (rho * m - big_m) * m
        };
        if !ds.is_finite() {
            return None;
        }
        s -= ds;
        if ds.abs() < eps * a {
            trip = true;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let (lon0, lat0) = (116.4, 39.9);
        for &(lon, lat) in &[(117.0, 40.5), (110.0, 30.0), (130.0, 50.0), (116.4, 39.9)] {
            let (x, y) = forward(lon0, lat0, lon, lat).unwrap();
            let (lo, la) = reverse(lon0, lat0, x, y).unwrap();
            assert!((lo - lon).abs() < 1e-11 && (la - lat).abs() < 1e-11, "{lo} {la}");
        }
    }
}
