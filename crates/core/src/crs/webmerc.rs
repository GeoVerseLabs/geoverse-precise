//! Spherical ("Web") Mercator, EPSG:3857.

use std::f64::consts::FRAC_PI_4;

pub const R: f64 = 6378137.0;
pub const MAX_LAT: f64 = 85.051_128_779_806_59;

pub fn forward(lon: f64, lat: f64) -> (f64, f64) {
    let lat = lat.clamp(-MAX_LAT, MAX_LAT);
    let x = R * lon.to_radians();
    let y = R * (FRAC_PI_4 + lat.to_radians() / 2.0).tan().ln();
    (x, y)
}

pub fn reverse(x: f64, y: f64) -> (f64, f64) {
    let lon = (x / R).to_degrees();
    let lat = (2.0 * (y / R).exp().atan() - std::f64::consts::FRAC_PI_2).to_degrees();
    (lon, lat)
}
