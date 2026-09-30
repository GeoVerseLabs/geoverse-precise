//! Cross-checks against GeographicLib (Python) and PROJ (pyproj).
//! Regenerate with `python bench/gen_fixtures.py`.

use geo::{Coord, LineString};
use geoprecise_core::crs::{Crs, Transformer};
use geoprecise_core::{geodesic, measure};
use serde_json::Value;

fn load(name: &str) -> Vec<Value> {
    let path = format!("{}/../../bench/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    let s = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    serde_json::from_str(&s).unwrap()
}

fn f(v: &Value, k: &str) -> f64 {
    v[k].as_f64().unwrap()
}

fn angle_diff(a: f64, b: f64) -> f64 {
    geodesic::normalize_deg(a - b).abs()
}

#[test]
fn inverse_matches_geographiclib() {
    let mut worst: f64 = 0.0;
    for r in load("inverse.json") {
        let inv = geodesic::inverse(f(&r, "lon1"), f(&r, "lat1"), f(&r, "lon2"), f(&r, "lat2"));
        let e = (inv.s12 - f(&r, "s12")).abs();
        worst = worst.max(e);
        assert!(e < 1e-6, "{r}: s12 {} err {e}", inv.s12);
        if f(&r, "s12") > 1.0 {
            assert!(angle_diff(inv.azi1, f(&r, "azi1")) < 1e-9, "{r}");
            assert!(angle_diff(inv.azi2, f(&r, "azi2")) < 1e-9, "{r}");
        }
    }
    eprintln!("inverse worst distance error: {worst:e} m");
}

#[test]
fn direct_matches_geographiclib() {
    for r in load("direct.json") {
        let (lon, lat) = geodesic::destination(f(&r, "lon"), f(&r, "lat"), f(&r, "azi"), f(&r, "s"));
        let e = geodesic::distance(lon, lat, f(&r, "lon2"), f(&r, "lat2"));
        assert!(e < 1e-6, "{r}: err {e} m");
    }
}

#[test]
fn area_matches_geographiclib() {
    for r in load("area.json") {
        let ring: Vec<Coord> = r["ring"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| Coord {
                x: p[0].as_f64().unwrap(),
                y: p[1].as_f64().unwrap(),
            })
            .collect();
        let ls = LineString(ring);
        let a = measure::ring_area(&ls);
        let expect = f(&r, "area");
        assert!((a - expect).abs() <= 1e-6 * expect.max(1.0), "area {a} vs {expect}");
        let p = measure::line_length(&ls);
        assert!((p - f(&r, "perimeter")).abs() < 1e-6, "perimeter");
    }
}

#[test]
fn projections_match_proj() {
    let mut worst: f64 = 0.0;
    for r in load("projections.json") {
        let src = Crs::parse(r["src"].as_str().unwrap()).unwrap();
        let dst = Crs::parse(r["dst"].as_str().unwrap()).unwrap();
        let fwd = Transformer::new(&src, &dst);
        let (x, y) = fwd.apply(f(&r, "lon"), f(&r, "lat"));
        let e = (x - f(&r, "x")).hypot(y - f(&r, "y"));
        worst = worst.max(e);
        // PROJ uses a different (also sub-mm) series; agreement to 1 mm is the bar.
        assert!(e < 1e-3, "{r}: got ({x}, {y}) err {e} m");
        let inv = Transformer::new(&dst, &src);
        let (lon, lat) = inv.apply(x, y);
        assert!(
            (lon - f(&r, "lon")).abs() < 1e-10 && (lat - f(&r, "lat")).abs() < 1e-10,
            "{r} inverse"
        );
    }
    eprintln!("projection worst error vs PROJ: {worst:e} m");
}
