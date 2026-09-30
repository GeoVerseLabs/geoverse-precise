//! Rhumb lines (loxodromes) on the WGS84 ellipsoid.
//!
//! A rhumb line crosses every meridian at the same angle: it is the path you
//! follow with a fixed compass bearing, and a straight line on a Mercator map.
//! It is longer than the geodesic, which is why turf keeps both.
//!
//! The meridian arc is integrated with Gauss–Legendre quadrature (exact to
//! roundoff for the spans involved) rather than a truncated series, and the
//! inverse problem is solved with Newton's method on that integral.

use geo::Coord;

use crate::geodesic::normalize_deg;

const A: f64 = 6_378_137.0;
const E2: f64 = 0.006_694_379_990_141_316;

/// 7-point Gauss–Legendre nodes / weights on [0, 1].
const GL_X: [f64; 7] = [
    0.025_446_043_828_620_7,
    0.129_234_407_200_302_78,
    0.297_077_424_311_301_4,
    0.5,
    0.702_922_575_688_698_6,
    0.870_765_592_799_697_2,
    0.974_553_956_171_379_3,
];
const GL_W: [f64; 7] = [
    0.064_742_483_084_434_85,
    0.139_852_695_744_638_34,
    0.190_915_025_252_559_47,
    0.208_979_591_836_734_7,
    0.190_915_025_252_559_47,
    0.139_852_695_744_638_34,
    0.064_742_483_084_434_85,
];

/// Meridional radius of curvature at latitude φ (radians).
#[inline]
fn meridional_radius(lat: f64) -> f64 {
    let s = lat.sin();
    let w = 1.0 - E2 * s * s;
    A * (1.0 - E2) / (w * w.sqrt())
}

/// Normal (prime vertical) radius of curvature.
#[inline]
fn normal_radius(lat: f64) -> f64 {
    let s = lat.sin();
    A / (1.0 - E2 * s * s).sqrt()
}

/// Meridian arc length between two latitudes (radians), in metres.
fn meridian_arc(lat1: f64, lat2: f64) -> f64 {
    let span = (lat2 - lat1).abs().to_degrees();
    let parts = ((span / 2.0).ceil() as usize).max(1);
    let d = lat2 - lat1;
    let mut total = 0.0;
    for k in 0..parts {
        for (x, w) in GL_X.iter().zip(GL_W.iter()) {
            let t = (k as f64 + x) / parts as f64;
            total += w * meridional_radius(lat1 + d * t);
        }
    }
    total * d / parts as f64
}

/// Isometric latitude (the Mercator y coordinate, in radians).
#[inline]
fn isometric_latitude(lat: f64) -> f64 {
    let e = E2.sqrt();
    let s = lat.sin();
    lat.tan().asinh() - e * (e * s).atanh()
}

/// Bearing of the rhumb line from `a` to `b`, degrees in (-180, 180].
pub fn bearing(a: Coord, b: Coord) -> f64 {
    let (lat1, lat2) = (a.y.to_radians(), b.y.to_radians());
    let dlon = normalize_deg(b.x - a.x).to_radians();
    let dpsi = isometric_latitude(lat2) - isometric_latitude(lat1);
    normalize_deg(dlon.atan2(dpsi).to_degrees())
}

/// Length of the rhumb line from `a` to `b`, in metres.
pub fn distance(a: Coord, b: Coord) -> f64 {
    let (lat1, lat2) = (a.y.to_radians(), b.y.to_radians());
    let dlon = normalize_deg(b.x - a.x).to_radians();
    let dpsi = isometric_latitude(lat2) - isometric_latitude(lat1);
    let alpha = dlon.atan2(dpsi);
    if alpha.cos().abs() > 1e-12 {
        (meridian_arc(lat1, lat2) / alpha.cos()).abs()
    } else {
        // along a parallel
        let lat_m = 0.5 * (lat1 + lat2);
        (dlon * normal_radius(lat_m) * lat_m.cos()).abs()
    }
}

/// Destination after travelling `dist_m` metres from `origin` on a constant
/// bearing of `bearing_deg`.
pub fn destination(origin: Coord, dist_m: f64, bearing_deg: f64) -> Coord {
    let lat1 = origin.y.to_radians();
    let alpha = bearing_deg.to_radians();
    let (sa, ca) = alpha.sin_cos();
    if ca.abs() < 1e-12 {
        // pure east/west: stay on the parallel
        let dlon = dist_m * sa.signum() / (normal_radius(lat1) * lat1.cos());
        return Coord { x: normalize_deg(origin.x + dlon.to_degrees()), y: origin.y };
    }
    // solve meridian_arc(lat1, lat2) = dist * cos(alpha) for lat2
    let target = dist_m * ca;
    let mut lat2 = lat1 + target / meridional_radius(lat1);
    for _ in 0..8 {
        let f = meridian_arc(lat1, lat2) - target;
        let d = meridional_radius(lat2);
        let step = f / d;
        lat2 -= step;
        lat2 = lat2.clamp(-std::f64::consts::FRAC_PI_2 + 1e-12, std::f64::consts::FRAC_PI_2 - 1e-12);
        if step.abs() < 1e-14 {
            break;
        }
    }
    let dpsi = isometric_latitude(lat2) - isometric_latitude(lat1);
    let dlon = if sa.abs() < 1e-15 { 0.0 } else { dpsi * (sa / ca) };
    Coord {
        x: normalize_deg(origin.x + dlon.to_degrees()),
        y: lat2.to_degrees(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{geodesic, measure};
    use geo::coord;

    /// Walking a constant bearing in small geodesic steps traces the rhumb
    /// line, which gives an independent check of both length and endpoint.
    fn simulate(origin: Coord, dist: f64, bearing_deg: f64, steps: usize) -> Coord {
        let mut p = origin;
        let step = dist / steps as f64;
        for _ in 0..steps {
            let (x, y) = geodesic::destination(p.x, p.y, bearing_deg, step);
            p = Coord { x, y };
        }
        p
    }

    #[test]
    fn destination_matches_small_step_simulation() {
        // Stepping along geodesics only approximates a constant bearing: the
        // azimuth drifts within each step, so the simulation carries a
        // first-order error. Check that it converges onto our closed form.
        for (origin, dist, brg) in [
            (coord! {x: 116.4, y: 39.9}, 500_000.0, 45.0),
            (coord! {x: 116.4, y: 39.9}, 100_000.0, 135.0),
            (coord! {x: -60.0, y: -30.0}, 2_000_000.0, 300.0),
            (coord! {x: 10.0, y: 60.0}, 800_000.0, 270.0),
            (coord! {x: 10.0, y: 0.0}, 800_000.0, 0.0),
        ] {
            let exact = destination(origin, dist, brg);
            let coarse = measure::distance(exact, simulate(origin, dist, brg, 5_000));
            let fine = measure::distance(exact, simulate(origin, dist, brg, 20_000));
            assert!(
                fine <= coarse * 0.35 + 1e-6,
                "bearing {brg}: no convergence ({coarse} m → {fine} m)"
            );
            // a 4× refinement should quarter the error; a bias in our closed
            // form would instead leave it stuck at a constant
            assert!(fine < dist * 2e-5, "bearing {brg}: {fine} m off over {dist} m");
        }
    }

    #[test]
    fn distance_and_bearing_round_trip() {
        for (origin, dist, brg) in [
            (coord! {x: 116.4, y: 39.9}, 500_000.0, 45.0),
            (coord! {x: 0.0, y: 10.0}, 1_500_000.0, 200.0),
            (coord! {x: 120.0, y: -20.0}, 300_000.0, 89.0),
        ] {
            let dest = destination(origin, dist, brg);
            let back = distance(origin, dest);
            assert!((back - dist).abs() < 1e-3, "{back} vs {dist}");
            let b = bearing(origin, dest);
            assert!((normalize_deg(b - brg)).abs() < 1e-9, "{b} vs {brg}");
        }
    }

    #[test]
    fn rhumb_is_longer_than_the_geodesic() {
        let a = coord! {x: 0.0, y: 50.0};
        let b = coord! {x: 60.0, y: 50.0};
        let rhumb = distance(a, b);
        let geo = measure::distance(a, b);
        assert!(rhumb > geo, "{rhumb} vs {geo}");
        // along a parallel the rhumb line stays on 50°N
        let mid = destination(a, rhumb / 2.0, bearing(a, b));
        assert!((mid.y - 50.0).abs() < 1e-9, "{mid:?}");
    }

    #[test]
    fn due_north_matches_the_meridian_arc() {
        let a = coord! {x: 30.0, y: 10.0};
        let b = coord! {x: 30.0, y: 40.0};
        let rhumb = distance(a, b);
        let geo = measure::distance(a, b);
        // a meridian is both a geodesic and a rhumb line
        assert!((rhumb - geo).abs() < 1e-6, "{rhumb} vs {geo}");
        assert!((bearing(a, b) - 0.0).abs() < 1e-12);
    }
}
