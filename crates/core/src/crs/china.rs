//! GCJ-02 ("Mars coordinates") and BD-09 transforms.
//!
//! Forward transforms are the widely published algorithms. The inverse
//! transforms are solved with a fixed-point iteration down to ~1e-10° (~0.01 mm),
//! instead of the one-step approximation most JS libraries use (1–2 m residual).

use std::f64::consts::PI;

const KRASOVSKY_A: f64 = 6378245.0;
const KRASOVSKY_EE: f64 = 0.006_693_421_622_965_943;
const X_PI: f64 = PI * 3000.0 / 180.0;
const TOL_DEG: f64 = 1e-10;
const MAX_ITER: usize = 30;

/// Rough bounding box used by practically every implementation to decide
/// whether GCJ-02 obfuscation applies.
pub fn in_china_bbox(lon: f64, lat: f64) -> bool {
    (72.004..=137.8347).contains(&lon) && (0.8293..=55.8271).contains(&lat)
}

fn transform_lat(x: f64, y: f64) -> f64 {
    let mut r = -100.0 + 2.0 * x + 3.0 * y + 0.2 * y * y + 0.1 * x * y + 0.2 * x.abs().sqrt();
    r += (20.0 * (6.0 * x * PI).sin() + 20.0 * (2.0 * x * PI).sin()) * 2.0 / 3.0;
    r += (20.0 * (y * PI).sin() + 40.0 * (y / 3.0 * PI).sin()) * 2.0 / 3.0;
    r += (160.0 * (y / 12.0 * PI).sin() + 320.0 * (y * PI / 30.0).sin()) * 2.0 / 3.0;
    r
}

fn transform_lon(x: f64, y: f64) -> f64 {
    let mut r = 300.0 + x + 2.0 * y + 0.1 * x * x + 0.1 * x * y + 0.1 * x.abs().sqrt();
    r += (20.0 * (6.0 * x * PI).sin() + 20.0 * (2.0 * x * PI).sin()) * 2.0 / 3.0;
    r += (20.0 * (x * PI).sin() + 40.0 * (x / 3.0 * PI).sin()) * 2.0 / 3.0;
    r += (150.0 * (x / 12.0 * PI).sin() + 300.0 * (x / 30.0 * PI).sin()) * 2.0 / 3.0;
    r
}

fn gcj_offset(lon: f64, lat: f64) -> (f64, f64) {
    let dlat = transform_lat(lon - 105.0, lat - 35.0);
    let dlon = transform_lon(lon - 105.0, lat - 35.0);
    let radlat = lat / 180.0 * PI;
    let magic = 1.0 - KRASOVSKY_EE * radlat.sin().powi(2);
    let sqrtmagic = magic.sqrt();
    let dlat = (dlat * 180.0) / ((KRASOVSKY_A * (1.0 - KRASOVSKY_EE)) / (magic * sqrtmagic) * PI);
    let dlon = (dlon * 180.0) / (KRASOVSKY_A / sqrtmagic * radlat.cos() * PI);
    (dlon, dlat)
}

pub fn wgs84_to_gcj02(lon: f64, lat: f64) -> (f64, f64) {
    if !in_china_bbox(lon, lat) {
        return (lon, lat);
    }
    let (dlon, dlat) = gcj_offset(lon, lat);
    (lon + dlon, lat + dlat)
}

pub fn gcj02_to_wgs84(lon: f64, lat: f64) -> (f64, f64) {
    if !in_china_bbox(lon, lat) {
        return (lon, lat);
    }
    // Initial guess: subtract the offset evaluated at the GCJ point.
    let (dlon, dlat) = gcj_offset(lon, lat);
    let (mut wx, mut wy) = (lon - dlon, lat - dlat);
    for _ in 0..MAX_ITER {
        let (gx, gy) = wgs84_to_gcj02(wx, wy);
        let (ex, ey) = (gx - lon, gy - lat);
        wx -= ex;
        wy -= ey;
        if ex.abs() < TOL_DEG && ey.abs() < TOL_DEG {
            break;
        }
    }
    (wx, wy)
}

pub fn gcj02_to_bd09(lon: f64, lat: f64) -> (f64, f64) {
    let z = (lon * lon + lat * lat).sqrt() + 0.00002 * (lat * X_PI).sin();
    let theta = lat.atan2(lon) + 0.000003 * (lon * X_PI).cos();
    (z * theta.cos() + 0.0065, z * theta.sin() + 0.006)
}

pub fn bd09_to_gcj02(lon: f64, lat: f64) -> (f64, f64) {
    let x = lon - 0.0065;
    let y = lat - 0.006;
    let z = (x * x + y * y).sqrt() - 0.00002 * (y * X_PI).sin();
    let theta = y.atan2(x) - 0.000003 * (x * X_PI).cos();
    let (mut gx, mut gy) = (z * theta.cos(), z * theta.sin());
    // Refine the closed-form approximation.
    for _ in 0..MAX_ITER {
        let (bx, by) = gcj02_to_bd09(gx, gy);
        let (ex, ey) = (bx - lon, by - lat);
        gx -= ex;
        gy -= ey;
        if ex.abs() < TOL_DEG && ey.abs() < TOL_DEG {
            break;
        }
    }
    (gx, gy)
}

pub fn wgs84_to_bd09(lon: f64, lat: f64) -> (f64, f64) {
    let (x, y) = wgs84_to_gcj02(lon, lat);
    gcj02_to_bd09(x, y)
}

pub fn bd09_to_wgs84(lon: f64, lat: f64) -> (f64, f64) {
    let (x, y) = bd09_to_gcj02(lon, lat);
    gcj02_to_wgs84(x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gcj_roundtrip_sub_mm() {
        let mut worst: f64 = 0.0;
        let mut lon = 73.5;
        while lon < 135.0 {
            let mut lat = 18.0;
            while lat < 53.5 {
                let (gx, gy) = wgs84_to_gcj02(lon, lat);
                let (wx, wy) = gcj02_to_wgs84(gx, gy);
                worst = worst.max((wx - lon).abs()).max((wy - lat).abs());
                lat += 0.37;
            }
            lon += 0.41;
        }
        // 1e-9 deg ≈ 0.1 mm
        assert!(worst < 1e-9, "worst {worst}");
    }

    #[test]
    fn bd_roundtrip() {
        let (bx, by) = wgs84_to_bd09(116.397_128, 39.916_527);
        let (wx, wy) = bd09_to_wgs84(bx, by);
        assert!((wx - 116.397_128).abs() < 1e-9);
        assert!((wy - 39.916_527).abs() < 1e-9);
    }

    #[test]
    fn outside_china_untouched() {
        assert_eq!(wgs84_to_gcj02(-122.0, 37.0), (-122.0, 37.0));
    }
}
