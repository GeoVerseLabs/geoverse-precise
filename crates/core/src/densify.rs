//! Edge interpretation and adaptive densification.
//!
//! GeoJSON (RFC 7946 §3.1.1) defines an edge as a straight line in lon/lat,
//! which is also what turf assumes. Internally geoprecise works with geodesic
//! segments, so a `Planar` edge is first split (only where needed) until the
//! geodesic between consecutive vertices stays within `tol` metres of the
//! lon/lat straight line.
//!
//! When geometry is moved into the local transverse Mercator plane, curves
//! (geodesics, offset curves) are sampled adaptively so that straight chords in
//! the plane stay within `tol` of the true curve.

use geo::{Coord, Geometry, LineString, MultiLineString, MultiPolygon, Polygon};

use crate::geodesic;
use crate::local::LocalFrame;

/// How an edge between two vertices is interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Edges {
    /// Straight line in lon/lat (GeoJSON / turf semantics). Default.
    #[default]
    Planar,
    /// Shortest path on the ellipsoid.
    Geodesic,
}

impl Edges {
    pub fn parse(s: &str) -> crate::Result<Edges> {
        match s {
            "" | "planar" => Ok(Edges::Planar),
            "geodesic" => Ok(Edges::Geodesic),
            o => Err(crate::Error::InvalidArgument(format!(
                "unknown edge interpretation `{o}`"
            ))),
        }
    }
}

/// Default geometric tolerance (m).
pub const DEFAULT_TOL: f64 = 0.01;
const R: f64 = 6_371_008.8;

#[inline]
fn approx_len(a: Coord, b: Coord) -> f64 {
    let lat = ((a.y + b.y) * 0.5).to_radians();
    let dx = crate::geodesic::normalize_deg(b.x - a.x).to_radians() * lat.cos();
    let dy = (b.y - a.y).to_radians();
    R * dx.hypot(dy)
}

/// Number of equal lon/lat pieces needed so that each piece's geodesic stays
/// within `tol` of the lon/lat straight line.
///
/// A lon/lat straight line with azimuth α has geodesic curvature
/// κ ≈ sinα·(1 + cos²α)·tanφ / R (sphere: rhumb term sinα·tanφ/R plus the
/// azimuth drift of a plate-carrée line), so its sagitta over a chord of length L is
/// L²κ/8. The bound uses the larger |tanφ| of the endpoints, the smallest
/// radius of curvature of WGS84 and a 25 % margin.
fn planar_pieces(a: Coord, b: Coord, tol: f64) -> usize {
    let lat_max = a.y.abs().max(b.y.abs()).min(89.9).to_radians();
    let len = approx_len(a, b);
    let dx = crate::geodesic::normalize_deg(b.x - a.x).to_radians() * ((a.y + b.y) * 0.5).to_radians().cos();
    let sin_a = if len > 0.0 { (dx * R / len).abs().min(1.0) } else { 0.0 };
    let kappa = sin_a * (2.0 - sin_a * sin_a) * lat_max.tan() / R_MIN;
    let sag = 1.25 * len * len * kappa / 8.0;
    if sag <= tol {
        1
    } else {
        ((sag / tol).sqrt().ceil() as usize).min(1 << 16)
    }
}

/// Split planar edges so that geodesic segments approximate them within `tol`.
pub fn planar_path(pts: &[Coord], tol: f64) -> Vec<Coord> {
    let mut out = Vec::with_capacity(pts.len());
    for (i, &p) in pts.iter().enumerate() {
        if i > 0 {
            let a = pts[i - 1];
            let n = planar_pieces(a, p, tol);
            let dx = crate::geodesic::normalize_deg(p.x - a.x);
            for k in 1..n {
                let t = k as f64 / n as f64;
                out.push(Coord {
                    x: a.x + dx * t,
                    y: a.y + (p.y - a.y) * t,
                });
            }
        }
        out.push(p);
    }
    out
}

fn planar_ls(ls: &LineString, tol: f64) -> LineString {
    LineString(planar_path(&ls.0, tol))
}

fn planar_poly(p: &Polygon, tol: f64) -> Polygon {
    Polygon::new(
        planar_ls(p.exterior(), tol),
        p.interiors().iter().map(|r| planar_ls(r, tol)).collect(),
    )
}

/// Return a geometry whose geodesic edges reproduce the requested interpretation.
pub fn prepare(g: &Geometry, edges: Edges, tol: f64) -> Geometry {
    if edges == Edges::Geodesic {
        return g.clone();
    }
    match g {
        Geometry::Point(_) | Geometry::MultiPoint(_) => g.clone(),
        Geometry::Line(l) => Geometry::LineString(LineString(planar_path(&[l.start, l.end], tol))),
        Geometry::LineString(ls) => Geometry::LineString(planar_ls(ls, tol)),
        Geometry::MultiLineString(m) => {
            Geometry::MultiLineString(MultiLineString(m.0.iter().map(|l| planar_ls(l, tol)).collect()))
        }
        Geometry::Polygon(p) => Geometry::Polygon(planar_poly(p, tol)),
        Geometry::MultiPolygon(mp) => {
            Geometry::MultiPolygon(MultiPolygon(mp.0.iter().map(|p| planar_poly(p, tol)).collect()))
        }
        Geometry::Rect(r) => Geometry::Polygon(planar_poly(&r.to_polygon(), tol)),
        Geometry::Triangle(t) => Geometry::Polygon(planar_poly(&t.to_polygon(), tol)),
        Geometry::GeometryCollection(gc) => Geometry::GeometryCollection(geo::GeometryCollection(
            gc.0.iter().map(|g| prepare(g, edges, tol)).collect(),
        )),
    }
}

// ------------------------------------------------------------------ plane

/// Smallest radius of curvature of WGS84 (meridional, at the equator).
const R_MIN: f64 = 6_335_439.0;
const WGS84_A: f64 = 6_378_137.0;
const WGS84_E2: f64 = 0.006_694_379_990_141_316;

/// Upper bound of the geodesic curvature of a lon/lat straight line near
/// latitude `lat_abs_max` (see [`planar_pieces`]).
pub fn planar_curvature(lat_abs_max: f64) -> f64 {
    1.09 * lat_abs_max.min(89.9).to_radians().tan() / R_MIN
}

/// Number of equal pieces so that chords in the local plane stay within `tol`
/// of a curve of length `len` located at |x| ≤ `x_abs`.
///
/// Geodesics in a transverse Mercator plane curve by ≈ |x|/R²; `extra` is the
/// curve's own geodesic curvature (offset curves, lon/lat lines). A 100 km
/// margin covers the remaining terms.
pub fn plane_pieces(len: f64, x_abs: f64, extra: f64, tol: f64) -> usize {
    let kappa = (x_abs + 100_000.0) / (R * R) + extra;
    let l = (8.0 * tol / kappa).sqrt();
    if len <= l {
        1
    } else {
        ((len / l).ceil() as usize).min(1 << 16)
    }
}

/// Exact azimuth (degrees) of the tangent of the lon/lat straight line a→b at `p`.
pub fn planar_azimuth(a: Coord, b: Coord, p: Coord) -> f64 {
    let dlon = crate::geodesic::normalize_deg(b.x - a.x).to_radians();
    let dlat = (b.y - a.y).to_radians();
    let (s, c) = p.y.to_radians().sin_cos();
    let w = 1.0 - WGS84_E2 * s * s;
    let n = WGS84_A / w.sqrt();
    let m = WGS84_A * (1.0 - WGS84_E2) / (w * w.sqrt());
    (dlon * n * c).atan2(dlat * m).to_degrees()
}

/// Point at parameter `t` on the lon/lat straight line a→b.
#[inline]
pub fn planar_lerp(a: Coord, b: Coord, t: f64) -> Coord {
    let dx = crate::geodesic::normalize_deg(b.x - a.x);
    Coord {
        x: a.x + dx * t,
        y: a.y + (b.y - a.y) * t,
    }
}

/// Project a path into the plane, sampling each edge (interpreted per `edges`)
/// so that chords stay within `tol` of the true edge.
pub fn project_path(frame: &LocalFrame, pts: &[Coord], edges: Edges, tol: f64) -> Vec<Coord> {
    let mut out = Vec::with_capacity(pts.len());
    let mut prev: Option<(Coord, Coord)> = None;
    for &c in pts {
        let (x, y) = frame.project(c.x, c.y);
        let pc = Coord { x, y };
        if let Some((a, pa)) = prev {
            let chord = (pc.x - pa.x).hypot(pc.y - pa.y);
            let x_abs = pa.x.abs().max(pc.x.abs()) + chord;
            let extra = match edges {
                Edges::Planar => planar_curvature(a.y.abs().max(c.y.abs())),
                Edges::Geodesic => 0.0,
            };
            let n = plane_pieces(chord, x_abs, extra, tol);
            if n > 1 {
                match edges {
                    Edges::Planar => {
                        for k in 1..n {
                            let q = planar_lerp(a, c, k as f64 / n as f64);
                            let (x, y) = frame.project(q.x, q.y);
                            out.push(Coord { x, y });
                        }
                    }
                    Edges::Geodesic => {
                        let inv = geodesic::inverse(a.x, a.y, c.x, c.y);
                        let line = geodesic::Line::new(a.x, a.y, inv.azi1);
                        for k in 1..n {
                            let (lon, lat, _) = line.position(inv.s12 * k as f64 / n as f64);
                            let (x, y) = frame.project(lon, lat);
                            out.push(Coord { x, y });
                        }
                    }
                }
            }
        }
        out.push(pc);
        prev = Some((c, pc));
    }
    out
}

/// Geodesic-edge version of [`project_path`].
pub fn project_geodesic_path(frame: &LocalFrame, pts: &[Coord], tol: f64) -> Vec<Coord> {
    project_path(frame, pts, Edges::Geodesic, tol)
}

pub fn project_polygon(frame: &LocalFrame, p: &Polygon, edges: Edges, tol: f64) -> Polygon {
    Polygon::new(
        LineString(project_path(frame, &p.exterior().0, edges, tol)),
        p.interiors()
            .iter()
            .map(|r| LineString(project_path(frame, &r.0, edges, tol)))
            .collect(),
    )
}

/// Project any geometry into the plane with adaptive sampling of its edges.
pub fn project_geometry(frame: &LocalFrame, g: &Geometry, edges: Edges, tol: f64) -> Geometry {
    let ls = |l: &LineString| LineString(project_path(frame, &l.0, edges, tol));
    match g {
        Geometry::LineString(l) => Geometry::LineString(ls(l)),
        Geometry::MultiLineString(m) => Geometry::MultiLineString(MultiLineString(m.0.iter().map(ls).collect())),
        Geometry::Polygon(p) => Geometry::Polygon(project_polygon(frame, p, edges, tol)),
        Geometry::MultiPolygon(mp) => Geometry::MultiPolygon(MultiPolygon(
            mp.0.iter().map(|p| project_polygon(frame, p, edges, tol)).collect(),
        )),
        Geometry::GeometryCollection(gc) => Geometry::GeometryCollection(geo::GeometryCollection(
            gc.0.iter().map(|g| project_geometry(frame, g, edges, tol)).collect(),
        )),
        other => frame.project_geom(other),
    }
}

// ------------------------------------------------------------ simplify

fn on_planar_segment(a: Coord, b: Coord, c: Coord, tol: f64) -> bool {
    // Local metric around b.
    let k = b.y.to_radians().cos() * 111_320.0;
    let (ax, ay) = ((a.x - b.x) * k, (a.y - b.y) * 110_574.0);
    let (cx, cy) = ((c.x - b.x) * k, (c.y - b.y) * 110_574.0);
    let (dx, dy) = (cx - ax, cy - ay);
    let len2 = dx * dx + dy * dy;
    if len2 == 0.0 {
        return true;
    }
    let t = -(ax * dx + ay * dy) / len2;
    if !(0.0..=1.0).contains(&t) {
        return false;
    }
    let (px, py) = (ax + t * dx, ay + t * dy);
    px.hypot(py) <= tol
}

fn on_geodesic_segment(a: Coord, b: Coord, c: Coord, tol: f64) -> bool {
    let ab = geodesic::inverse(a.x, a.y, b.x, b.y);
    let ac = geodesic::inverse(a.x, a.y, c.x, c.y);
    if ab.s12 > ac.s12 {
        return false;
    }
    let dz = geodesic::normalize_deg(ab.azi1 - ac.azi1).to_radians();
    dz.cos() >= 0.0 && (dz.sin() * ab.s12).abs() <= tol
}

fn simplify_ring(ring: &LineString, edges: Edges, tol: f64) -> LineString {
    let pts = &ring.0;
    if pts.len() <= 4 {
        return ring.clone();
    }
    let test = |a: Coord, b: Coord, c: Coord| match edges {
        Edges::Planar => on_planar_segment(a, b, c, tol),
        Edges::Geodesic => on_geodesic_segment(a, b, c, tol),
    };
    let open = &pts[..pts.len() - 1];
    let mut out: Vec<Coord> = Vec::with_capacity(open.len());
    for &p in open {
        while out.len() >= 2 && test(out[out.len() - 2], out[out.len() - 1], p) {
            out.pop();
        }
        out.push(p);
    }
    // Wrap-around.
    while out.len() > 3 && test(out[out.len() - 2], out[out.len() - 1], out[0]) {
        out.pop();
    }
    while out.len() > 3 && test(out[out.len() - 1], out[0], out[1]) {
        out.remove(0);
    }
    out.push(out[0]);
    LineString(out)
}

/// Remove vertices that only exist because of densification.
pub fn simplify_collinear(mp: &MultiPolygon, edges: Edges, tol: f64) -> MultiPolygon {
    MultiPolygon(
        mp.0.iter()
            .map(|p| {
                Polygon::new(
                    simplify_ring(p.exterior(), edges, tol),
                    p.interiors().iter().map(|r| simplify_ring(r, edges, tol)).collect(),
                )
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planar_edge_split_follows_parallel() {
        let pts = planar_path(&[Coord { x: 120.0, y: 30.0 }, Coord { x: 121.0, y: 30.0 }], 0.01);
        assert!(pts.len() > 50, "{}", pts.len());
        // every geodesic piece stays on the parallel within tolerance
        for w in pts.windows(2) {
            let (_, my) = geodesic::interpolate(w[0].x, w[0].y, w[1].x, w[1].y, 0.5);
            assert!(geodesic::distance(0.0, my, 0.0, 30.0) <= 0.01);
        }
    }

    #[test]
    fn planar_bound_holds_for_random_edges() {
        // Deterministic LCG
        let mut seed = 12345u64;
        let mut rnd = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        let mut worst: f64 = 0.0;
        for _ in 0..300 {
            let a = Coord {
                x: 70.0 + rnd() * 70.0,
                y: -60.0 + rnd() * 140.0,
            };
            let len = 10f64.powf(2.0 + rnd() * 3.5); // 100 m .. 300 km
            let az = rnd() * 360.0;
            let (bx, by) = geodesic::destination(a.x, a.y, az, len);
            let b = Coord { x: bx, y: by };
            let pts = planar_path(&[a, b], 0.01);
            for w in pts.windows(2) {
                for t in [0.25, 0.5, 0.75] {
                    let p = Coord {
                        x: w[0].x + (w[1].x - w[0].x) * t,
                        y: w[0].y + (w[1].y - w[0].y) * t,
                    };
                    // cross-track distance from the lon/lat line to the geodesic piece
                    let (_, dev, _) = crate::measure::closest_on_segment(w[0], w[1], p);
                    worst = worst.max(dev);
                }
            }
        }
        assert!(worst <= 0.01, "worst deviation {worst}");
    }

    #[test]
    fn plane_chords_within_tolerance() {
        let frame = LocalFrame::new(105.0, 35.0);
        let mut seed = 99u64;
        let mut rnd = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        let mut worst: f64 = 0.0;
        for _ in 0..200 {
            let a = Coord {
                x: 75.0 + rnd() * 60.0,
                y: 18.0 + rnd() * 35.0,
            };
            let (bx, by) = geodesic::destination(a.x, a.y, rnd() * 360.0, 10f64.powf(3.0 + rnd() * 3.0));
            let b = Coord { x: bx, y: by };
            let proj = project_geodesic_path(&frame, &[a, b], 0.01);
            let inv = geodesic::inverse(a.x, a.y, b.x, b.y);
            // sample the true geodesic and measure distance to the projected polyline
            for i in 0..=50 {
                let (lon, lat) = geodesic::destination(a.x, a.y, inv.azi1, inv.s12 * i as f64 / 50.0);
                let (x, y) = frame.project(lon, lat);
                let d = proj
                    .windows(2)
                    .map(|w| {
                        let (dx, dy) = (w[1].x - w[0].x, w[1].y - w[0].y);
                        let t = (((x - w[0].x) * dx + (y - w[0].y) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
                        (x - w[0].x - t * dx).hypot(y - w[0].y - t * dy)
                    })
                    .fold(f64::INFINITY, f64::min);
                worst = worst.max(d);
            }
        }
        assert!(worst <= 0.01, "worst {worst}");
    }

    #[test]
    fn short_edges_untouched() {
        let src = [Coord { x: 120.0, y: 30.0 }, Coord { x: 120.001, y: 30.001 }];
        assert_eq!(planar_path(&src, 0.01).len(), 2);
    }
}
