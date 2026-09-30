//! Ellipsoidal measurement on WGS84 (all lengths in metres, areas in m²).

use geo::{Coord, Geometry, LineString, MultiLineString, Polygon};
use geographiclib_rs::{PolygonArea, Winding};

use crate::geodesic::{self, to_bearing360};
use crate::gnomonic;

/// Geodesic distance in metres.
pub fn distance(a: Coord, b: Coord) -> f64 {
    geodesic::distance(a.x, a.y, b.x, b.y)
}

/// Initial bearing from `a` to `b` in degrees (-180, 180], turf-compatible.
/// With `final_bearing`, returns the bearing on arrival at `b`.
pub fn bearing(a: Coord, b: Coord, final_bearing: bool) -> f64 {
    let inv = geodesic::inverse(a.x, a.y, b.x, b.y);
    if final_bearing {
        // turf's final bearing is in [0, 360)
        to_bearing360(inv.azi2)
    } else {
        inv.azi1
    }
}

/// Destination from `origin` after `dist_m` metres heading `bearing_deg`.
pub fn destination(origin: Coord, dist_m: f64, bearing_deg: f64) -> Coord {
    let (x, y) = geodesic::destination(origin.x, origin.y, bearing_deg, dist_m);
    Coord { x, y }
}

/// Geodesic midpoint.
pub fn midpoint(a: Coord, b: Coord) -> Coord {
    let (x, y) = geodesic::interpolate(a.x, a.y, b.x, b.y, 0.5);
    Coord { x, y }
}

pub fn line_length(ls: &LineString) -> f64 {
    ls.0.windows(2).map(|w| distance(w[0], w[1])).sum()
}

/// Total length of all linear components (polygon rings included, like turf).
pub fn length(g: &Geometry) -> f64 {
    match g {
        Geometry::Line(l) => distance(l.start, l.end),
        Geometry::LineString(ls) => line_length(ls),
        Geometry::MultiLineString(m) => m.0.iter().map(line_length).sum(),
        Geometry::Polygon(p) => polygon_rings(p).map(line_length).sum(),
        Geometry::MultiPolygon(mp) => mp.0.iter().flat_map(polygon_rings).map(line_length).sum(),
        Geometry::Rect(r) => length(&Geometry::Polygon(r.to_polygon())),
        Geometry::Triangle(t) => length(&Geometry::Polygon(t.to_polygon())),
        Geometry::GeometryCollection(gc) => gc.0.iter().map(length).sum(),
        Geometry::Point(_) | Geometry::MultiPoint(_) => 0.0,
    }
}

fn polygon_rings(p: &Polygon) -> impl Iterator<Item = &LineString> {
    std::iter::once(p.exterior()).chain(p.interiors())
}

/// Absolute geodesic area enclosed by a ring (m²).
pub fn ring_area(ring: &LineString) -> f64 {
    let pts = &ring.0;
    if pts.len() < 3 {
        return 0.0;
    }
    let n = if pts.first() == pts.last() {
        pts.len() - 1
    } else {
        pts.len()
    };
    if n < 3 {
        return 0.0;
    }
    let mut pa = PolygonArea::new(geodesic::wgs84(), Winding::CounterClockwise);
    for c in &pts[..n] {
        pa.add_point(c.y, c.x);
    }
    let (_perim, area, _) = pa.compute(true);
    area.abs()
}

pub fn polygon_area(p: &Polygon) -> f64 {
    let outer = ring_area(p.exterior());
    let holes: f64 = p.interiors().iter().map(ring_area).sum();
    (outer - holes).max(0.0)
}

/// Geodesic area in m² (0 for non-areal geometries).
pub fn area(g: &Geometry) -> f64 {
    match g {
        Geometry::Polygon(p) => polygon_area(p),
        Geometry::MultiPolygon(mp) => mp.0.iter().map(polygon_area).sum(),
        Geometry::Rect(r) => polygon_area(&r.to_polygon()),
        Geometry::Triangle(t) => polygon_area(&t.to_polygon()),
        Geometry::GeometryCollection(gc) => gc.0.iter().map(area).sum(),
        _ => 0.0,
    }
}

/// Point `dist_m` metres along a line (clamped to the ends).
pub fn along(ls: &LineString, dist_m: f64) -> Option<Coord> {
    let pts = &ls.0;
    let first = *pts.first()?;
    if dist_m <= 0.0 {
        return Some(first);
    }
    let mut travelled = 0.0;
    for w in pts.windows(2) {
        let inv = geodesic::inverse(w[0].x, w[0].y, w[1].x, w[1].y);
        if travelled + inv.s12 >= dist_m {
            let (x, y) = geodesic::destination(w[0].x, w[0].y, inv.azi1, dist_m - travelled);
            return Some(Coord { x, y });
        }
        travelled += inv.s12;
    }
    pts.last().copied()
}

/// Result of [`nearest_point_on_line`].
#[derive(Debug, Clone, Copy)]
pub struct Nearest {
    pub point: Coord,
    /// Distance from the query point (m).
    pub dist: f64,
    /// Distance along the (multi)line from the start of its part (m).
    pub location: f64,
    /// Index of the segment's start vertex.
    pub index: usize,
    /// Index of the part within a MultiLineString.
    pub multi_index: usize,
}

const MAX_GNOMONIC_SEG: f64 = 2_000_000.0;

/// Closest point to `p` on the geodesic segment `a`–`b`.
/// Returns `(point, distance_from_p, distance_from_a)`.
pub fn closest_on_segment(a: Coord, b: Coord, p: Coord) -> (Coord, f64, f64) {
    closest_on_segment_hinted(a, b, p, None)
}

/// One gnomonic iteration from `hint`: the position is only approximate, but
/// the *distance* is stationary at the optimum, so it is accurate to well
/// below a millimetre. Used to rank candidate segments cheaply.
pub fn closest_on_segment_scored(a: Coord, b: Coord, p: Coord, hint: Coord) -> f64 {
    let (Some(pa), Some(pb), Some(pp)) = (
        gnomonic::forward(hint.x, hint.y, a.x, a.y),
        gnomonic::forward(hint.x, hint.y, b.x, b.y),
        gnomonic::forward(hint.x, hint.y, p.x, p.y),
    ) else {
        return distance(hint, p);
    };
    let (dx, dy) = (pb.0 - pa.0, pb.1 - pa.1);
    let len2 = dx * dx + dy * dy;
    if len2 == 0.0 {
        return distance(a, p);
    }
    let t = (((pp.0 - pa.0) * dx + (pp.1 - pa.1) * dy) / len2).clamp(0.0, 1.0);
    if t <= 0.0 {
        return distance(a, p);
    }
    if t >= 1.0 {
        return distance(b, p);
    }
    match gnomonic::reverse(hint.x, hint.y, pa.0 + t * dx, pa.1 + t * dy) {
        Some((x, y)) => distance(Coord { x, y }, p),
        None => distance(hint, p),
    }
}

/// As [`closest_on_segment`], with an optional starting estimate (a spherical
/// solution, say), which usually cuts the iteration count to one or two.
pub fn closest_on_segment_hinted(a: Coord, b: Coord, p: Coord, hint: Option<Coord>) -> (Coord, f64, f64) {
    let seg = geodesic::inverse(a.x, a.y, b.x, b.y);
    if seg.s12 == 0.0 {
        return (a, distance(a, p), 0.0);
    }
    if seg.s12 > MAX_GNOMONIC_SEG {
        // Split long segments so the gnomonic stays well-conditioned.
        let parts = (seg.s12 / MAX_GNOMONIC_SEG).ceil() as usize;
        let mut best = (a, f64::INFINITY, 0.0);
        let mut prev = a;
        for i in 1..=parts {
            let s = seg.s12 * i as f64 / parts as f64;
            let (x, y) = geodesic::destination(a.x, a.y, seg.azi1, s);
            let next = Coord { x, y };
            let (q, d, loc) = closest_on_segment_hinted(prev, next, p, None);
            if d < best.1 {
                best = (q, d, seg.s12 * (i - 1) as f64 / parts as f64 + loc);
            }
            prev = next;
        }
        return best;
    }

    // Karney's interception: iterate gnomonic projections centred on the estimate.
    let mut c = match hint {
        Some(h) => h,
        None => {
            let (mx, my) = geodesic::destination(a.x, a.y, seg.azi1, seg.s12 * 0.5);
            Coord { x: mx, y: my }
        }
    };
    let mut result: Option<Coord> = None;
    for _ in 0..10 {
        let (Some(pa), Some(pb), Some(pp)) = (
            gnomonic::forward(c.x, c.y, a.x, a.y),
            gnomonic::forward(c.x, c.y, b.x, b.y),
            gnomonic::forward(c.x, c.y, p.x, p.y),
        ) else {
            break;
        };
        let (dx, dy) = (pb.0 - pa.0, pb.1 - pa.1);
        let len2 = dx * dx + dy * dy;
        let t = (((pp.0 - pa.0) * dx + (pp.1 - pa.1) * dy) / len2).clamp(0.0, 1.0);
        let next = if t <= 0.0 {
            a
        } else if t >= 1.0 {
            b
        } else {
            match gnomonic::reverse(c.x, c.y, pa.0 + t * dx, pa.1 + t * dy) {
                Some((x, y)) => Coord { x, y },
                None => break,
            }
        };
        // 1e-11° ≈ 1 µm: the distance is stationary here, so this is far below
        // the accuracy of the distance itself.
        let converged = (next.x - c.x).abs() < 1e-11 && (next.y - c.y).abs() < 1e-11;
        c = next;
        result = Some(c);
        if converged {
            break;
        }
    }
    match result {
        Some(q) => (q, distance(q, p), distance(a, q)),
        None => {
            // Point is too far for the gnomonic; sample the segment instead.
            let mut best = (a, distance(a, p), 0.0);
            for i in 1..=256 {
                let s = seg.s12 * i as f64 / 256.0;
                let (x, y) = geodesic::destination(a.x, a.y, seg.azi1, s);
                let q = Coord { x, y };
                let d = distance(q, p);
                if d < best.1 {
                    best = (q, d, s);
                }
            }
            best
        }
    }
}

/// Exposed for profiling only.
#[doc(hidden)]
pub fn unit_vec_for_profiling(c: Coord) -> [f64; 3] {
    unit_vec(c)
}

/// Exposed for profiling only.
#[doc(hidden)]
pub fn to_lonlat_for_profiling(v: [f64; 3]) -> Coord {
    to_lonlat(v)
}

/// Exposed for profiling only.
#[doc(hidden)]
pub fn approx_seg_distance_for_profiling(a: [f64; 3], b: [f64; 3], p: [f64; 3]) -> (f64, [f64; 3]) {
    approx_seg_distance(a, b, p)
}

/// Exposed for the prepared-geometry index.
#[doc(hidden)]
pub fn approx_seg_distance_at(a: [f64; 3], b: [f64; 3], p: [f64; 3], r: f64) -> (f64, [f64; 3]) {
    approx_seg_distance_r(a, b, p, r)
}

/// Exposed for the prepared-geometry index.
#[doc(hidden)]
pub fn local_radius_at(lat_deg: f64) -> f64 {
    local_radius(lat_deg)
}

/// Unit vector on the sphere (for cheap candidate filtering).
#[inline]
fn unit_vec(c: Coord) -> [f64; 3] {
    let (slat, clat) = c.y.to_radians().sin_cos();
    let (slon, clon) = c.x.to_radians().sin_cos();
    [clat * clon, clat * slon, slat]
}

#[inline]
fn to_lonlat(v: [f64; 3]) -> Coord {
    Coord {
        x: v[1].atan2(v[0]).to_degrees(),
        y: v[2].atan2(v[0].hypot(v[1])).to_degrees(),
    }
}

#[inline]
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[inline]
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[inline]
fn chord2(a: [f64; 3], b: [f64; 3]) -> f64 {
    let (dx, dy, dz) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
    dx * dx + dy * dy + dz * dz
}

/// Approximate distance (m) from `p` to the great-circle segment `a`–`b`, plus
/// the approximate closest point. Sphere-based, so within a fraction of a
/// percent of the geodesic distance; used to shortlist segments and to seed the
/// exact solver.
#[inline]
pub(crate) fn approx_seg_distance(pa: [f64; 3], pb: [f64; 3], pp: [f64; 3]) -> (f64, [f64; 3]) {
    approx_seg_distance_r(pa, pb, pp, 6_371_008.8)
}

/// Gaussian radius of curvature at a latitude — using it instead of a mean
/// sphere radius cuts the approximation error roughly fivefold, which lets the
/// candidate filter prune much harder.
#[inline]
pub(crate) fn local_radius(lat_deg: f64) -> f64 {
    const A: f64 = 6_378_137.0;
    const E2: f64 = 0.006_694_379_990_141_316;
    let s = lat_deg.to_radians().sin();
    let w = 1.0 - E2 * s * s;
    A * (1.0 - E2).sqrt() / w
}

#[inline]
pub(crate) fn approx_seg_distance_r(pa: [f64; 3], pb: [f64; 3], pp: [f64; 3], r: f64) -> (f64, [f64; 3]) {
    let nearer_end = |pa: [f64; 3], pb: [f64; 3], pp: [f64; 3]| -> (f64, [f64; 3]) {
        let (ca, cb) = (chord2(pp, pa), chord2(pp, pb));
        let c = ca.min(cb).sqrt();
        (
            2.0 * (c * 0.5).clamp(-1.0, 1.0).asin() * r,
            if ca <= cb { pa } else { pb },
        )
    };
    let n = cross(pa, pb);
    let nn = dot(n, n).sqrt();
    if nn < 1e-12 {
        return nearer_end(pa, pb, pp);
    }
    let inv = 1.0 / nn;
    let nu = [n[0] * inv, n[1] * inv, n[2] * inv];
    // foot inside the segment? (sign tests only, no trig)
    if dot(cross(pa, pp), nu) < 0.0 || dot(cross(pp, pb), nu) < 0.0 {
        return nearer_end(pa, pb, pp);
    }
    let d = dot(pp, nu);
    let f = [pp[0] - d * nu[0], pp[1] - d * nu[1], pp[2] - d * nu[2]];
    let fnorm = dot(f, f).sqrt();
    if fnorm < 1e-12 {
        return nearer_end(pa, pb, pp);
    }
    let finv = 1.0 / fnorm;
    (
        d.abs().clamp(0.0, 1.0).asin() * r,
        [f[0] * finv, f[1] * finv, f[2] * finv],
    )
}

/// Closest point on a (multi)line, including `location` (the distance along
/// the line), which costs one extra pass over the segments.
pub fn nearest_point_on_line(lines: &MultiLineString, p: Coord) -> Option<Nearest> {
    nearest_point_on_line_opts(lines, p, true)
}

/// Closest point on a (multi)line.
///
/// A cheap spherical pass scores every segment first; the exact solver then
/// runs only for the shortlist, seeded with the spherical solution. The result
/// is identical to solving every segment exactly.
///
/// With `want_location = false` the `location` field is left as NaN, which
/// avoids summing the length of every preceding segment.
pub fn nearest_point_on_line_opts(lines: &MultiLineString, p: Coord, want_location: bool) -> Option<Nearest> {
    let pp = unit_vec(p);
    let r_local = local_radius(p.y);
    let mut approx: Vec<(usize, usize, f64, Coord)> = Vec::new();
    let mut best_approx = f64::INFINITY;
    for (mi, ls) in lines.0.iter().enumerate() {
        if ls.0.len() == 1 {
            let d = distance(ls.0[0], p);
            best_approx = best_approx.min(d);
            approx.push((mi, usize::MAX, d, ls.0[0]));
            continue;
        }
        let mut va = ls.0.first().map(|c| unit_vec(*c));
        for (i, w) in ls.0.windows(2).enumerate() {
            let a = va.unwrap_or_else(|| unit_vec(w[0]));
            let b = unit_vec(w[1]);
            let (d, hint) = approx_seg_distance_r(a, b, pp, r_local);
            best_approx = best_approx.min(d);
            approx.push((mi, i, d, to_lonlat(hint)));
            va = Some(b);
        }
    }
    if !best_approx.is_finite() {
        return None;
    }

    let mut best: Option<Nearest> = None;
    // Refine the best spherical candidate first, then use its exact distance to
    // prune the rest (sphere and ellipsoid differ by well under 1 %).
    let refine = |mi: usize, i: usize, hint: Coord, best: &mut Option<Nearest>| {
        let ls = &lines.0[mi];
        if i == usize::MAX {
            let d = distance(ls.0[0], p);
            if best.is_none_or(|b: Nearest| d < b.dist) {
                *best = Some(Nearest {
                    point: ls.0[0],
                    dist: d,
                    location: 0.0,
                    index: 0,
                    multi_index: mi,
                });
            }
            return;
        }
        let (q, d, loc) = closest_on_segment_hinted(ls.0[i], ls.0[i + 1], p, Some(hint));
        if best.is_none_or(|b: Nearest| d < b.dist) {
            *best = Some(Nearest {
                point: q,
                dist: d,
                location: loc,
                index: i,
                multi_index: mi,
            });
        }
    };

    let first = approx
        .iter()
        .enumerate()
        .min_by(|a, b| a.1 .2.total_cmp(&b.1 .2))
        .map(|(idx, _)| idx)?;
    {
        let (mi, i, _, hint) = approx[first];
        refine(mi, i, hint, &mut best);
    }
    for (idx, (mi, i, a, hint)) in approx.iter().copied().enumerate() {
        if idx == first {
            continue;
        }
        let bound = best.map_or(f64::INFINITY, |b| b.dist * 1.004 + 0.5);
        if a > bound {
            continue;
        }
        refine(mi, i, hint, &mut best);
    }

    // `location` needs the length of everything before the winning segment.
    if let Some(b) = best.as_mut() {
        if want_location {
            let ls = &lines.0[b.multi_index];
            let before: f64 = ls.0.windows(2).take(b.index).map(|w| distance(w[0], w[1])).sum();
            b.location += before;
        } else {
            b.location = f64::NAN;
        }
    }
    best
}

pub fn densify_line(ls: &LineString, max_seg_m: f64) -> LineString {
    if max_seg_m <= 0.0 || ls.0.len() < 2 {
        return ls.clone();
    }
    let mut out = Vec::with_capacity(ls.0.len());
    for w in ls.0.windows(2) {
        out.push(w[0]);
        let inv = geodesic::inverse(w[0].x, w[0].y, w[1].x, w[1].y);
        let n = (inv.s12 / max_seg_m).ceil() as usize;
        for i in 1..n {
            let (x, y) = geodesic::destination(w[0].x, w[0].y, inv.azi1, inv.s12 * i as f64 / n as f64);
            out.push(Coord { x, y });
        }
    }
    out.push(*ls.0.last().unwrap());
    LineString(out)
}

pub fn densify_polygon(p: &Polygon, max_seg_m: f64) -> Polygon {
    Polygon::new(
        densify_line(p.exterior(), max_seg_m),
        p.interiors().iter().map(|r| densify_line(r, max_seg_m)).collect(),
    )
}

pub fn densify(g: &Geometry, max_seg_m: f64) -> Geometry {
    if max_seg_m <= 0.0 {
        return g.clone();
    }
    match g {
        Geometry::LineString(ls) => Geometry::LineString(densify_line(ls, max_seg_m)),
        Geometry::MultiLineString(m) => Geometry::MultiLineString(MultiLineString(
            m.0.iter().map(|l| densify_line(l, max_seg_m)).collect(),
        )),
        Geometry::Polygon(p) => Geometry::Polygon(densify_polygon(p, max_seg_m)),
        Geometry::MultiPolygon(mp) => Geometry::MultiPolygon(geo::MultiPolygon(
            mp.0.iter().map(|p| densify_polygon(p, max_seg_m)).collect(),
        )),
        Geometry::GeometryCollection(gc) => Geometry::GeometryCollection(geo::GeometryCollection(
            gc.0.iter().map(|g| densify(g, max_seg_m)).collect(),
        )),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo::{coord, line_string, polygon};

    #[test]
    fn karney_reference_distance() {
        // GeographicLib reference: JFK -> SIN
        let d = distance(coord! {x: -73.78, y: 40.64}, coord! {x: 103.99, y: 1.36});
        assert!((d - 15347512.940_512_94).abs() < 1e-6, "{d}");
    }

    #[test]
    fn nearest_on_short_segment() {
        let ls = MultiLineString(vec![line_string![(x: 116.0, y: 40.0), (x: 117.0, y: 40.0)]]);
        let p = coord! {x: 116.5, y: 40.1};
        let n = nearest_point_on_line(&ls, p).unwrap();
        // Brute-force check along the geodesic.
        let mut best = f64::INFINITY;
        let seg = geodesic::inverse(116.0, 40.0, 117.0, 40.0);
        for i in 0..=200_000 {
            let (x, y) = geodesic::destination(116.0, 40.0, seg.azi1, seg.s12 * i as f64 / 200_000.0);
            best = best.min(distance(Coord { x, y }, p));
        }
        assert!((n.dist - best).abs() < 1e-3, "{} vs {}", n.dist, best);
    }

    #[test]
    fn area_with_hole() {
        let p = polygon!(
            exterior: [(x: 0.0, y: 0.0), (x: 1.0, y: 0.0), (x: 1.0, y: 1.0), (x: 0.0, y: 1.0), (x: 0.0, y: 0.0)],
            interiors: [[(x: 0.2, y: 0.2), (x: 0.2, y: 0.8), (x: 0.8, y: 0.8), (x: 0.8, y: 0.2), (x: 0.2, y: 0.2)]],
        );
        let outer = ring_area(p.exterior());
        assert!((outer - 12308778361.469452).abs() < 1.0, "{outer}");
        assert!(polygon_area(&p) < outer);
    }
}

// ---------------------------------------------------------------- planar edges

const WGS84_A: f64 = 6_378_137.0;
const WGS84_E2: f64 = 0.006_694_379_990_141_316;

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

#[inline]
fn quad_parts(span_deg: f64) -> usize {
    ((span_deg.abs() / 2.0).ceil() as usize).max(1)
}

/// Area between the equator and latitude φ, per radian of longitude (m²).
#[inline]
fn area_below(lat_rad: f64) -> f64 {
    let s = lat_rad.sin();
    let e = WGS84_E2.sqrt();
    WGS84_A * WGS84_A * (1.0 - WGS84_E2) / 2.0
        * (s / (1.0 - WGS84_E2 * s * s) + (1.0 / (2.0 * e)) * ((1.0 + e * s) / (1.0 - e * s)).ln())
}

/// Length (m) of an edge that is a straight line in lon/lat.
///
/// Integrates the ellipsoid line element with Gauss–Legendre quadrature, so it
/// is exact to roundoff without densifying the edge.
pub fn planar_edge_length(a: Coord, b: Coord) -> f64 {
    let dlon = crate::geodesic::normalize_deg(b.x - a.x).to_radians();
    let dlat = (b.y - a.y).to_radians();
    if dlon == 0.0 && dlat == 0.0 {
        return 0.0;
    }
    let lat0 = a.y.to_radians();
    let parts = quad_parts((b.y - a.y).abs().max(crate::geodesic::normalize_deg(b.x - a.x).abs()));
    let mut total = 0.0;
    for k in 0..parts {
        for (x, w) in GL_X.iter().zip(GL_W.iter()) {
            let t = (k as f64 + x) / parts as f64;
            let lat = lat0 + dlat * t;
            let s = lat.sin();
            let den = 1.0 - WGS84_E2 * s * s;
            let m = WGS84_A * (1.0 - WGS84_E2) / (den * den.sqrt());
            let n = WGS84_A / den.sqrt();
            total += w * ((m * dlat).powi(2) + (n * lat.cos() * dlon).powi(2)).sqrt();
        }
    }
    total / parts as f64
}

pub fn planar_line_length(ls: &LineString) -> f64 {
    ls.0.windows(2).map(|w| planar_edge_length(w[0], w[1])).sum()
}

/// Signed area (m²) enclosed by a ring whose edges are straight in lon/lat
/// (positive for counter-clockwise rings). Exact to roundoff.
pub fn planar_ring_signed_area(ring: &LineString) -> f64 {
    let pts = &ring.0;
    if pts.len() < 3 {
        return 0.0;
    }
    let n = if pts.first() == pts.last() {
        pts.len() - 1
    } else {
        pts.len()
    };
    if n < 3 {
        return 0.0;
    }
    let mut total = 0.0;
    for i in 0..n {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        let dlon = crate::geodesic::normalize_deg(b.x - a.x).to_radians();
        if dlon == 0.0 {
            continue;
        }
        let lat0 = a.y.to_radians();
        let dlat = (b.y - a.y).to_radians();
        let parts = quad_parts(b.y - a.y);
        let mut edge = 0.0;
        for k in 0..parts {
            for (x, w) in GL_X.iter().zip(GL_W.iter()) {
                let t = (k as f64 + x) / parts as f64;
                edge += w * area_below(lat0 + dlat * t);
            }
        }
        total += edge / parts as f64 * dlon;
    }
    // Green's theorem gives ∮ Q dλ = −A for a counter-clockwise ring.
    -total
}

pub fn planar_ring_area(ring: &LineString) -> f64 {
    planar_ring_signed_area(ring).abs()
}

fn planar_polygon_area(p: &Polygon) -> f64 {
    let outer = planar_ring_area(p.exterior());
    let holes: f64 = p.interiors().iter().map(planar_ring_area).sum();
    (outer - holes).max(0.0)
}

/// Total length with the requested edge interpretation.
pub fn length_with(g: &Geometry, edges: crate::densify::Edges) -> f64 {
    if edges == crate::densify::Edges::Geodesic {
        return length(g);
    }
    match g {
        Geometry::Line(l) => planar_edge_length(l.start, l.end),
        Geometry::LineString(ls) => planar_line_length(ls),
        Geometry::MultiLineString(m) => m.0.iter().map(planar_line_length).sum(),
        Geometry::Polygon(p) => polygon_rings(p).map(planar_line_length).sum(),
        Geometry::MultiPolygon(mp) => mp.0.iter().flat_map(polygon_rings).map(planar_line_length).sum(),
        Geometry::Rect(r) => length_with(&Geometry::Polygon(r.to_polygon()), edges),
        Geometry::Triangle(t) => length_with(&Geometry::Polygon(t.to_polygon()), edges),
        Geometry::GeometryCollection(gc) => gc.0.iter().map(|g| length_with(g, edges)).sum(),
        Geometry::Point(_) | Geometry::MultiPoint(_) => 0.0,
    }
}

/// Area with the requested edge interpretation.
pub fn area_with(g: &Geometry, edges: crate::densify::Edges) -> f64 {
    if edges == crate::densify::Edges::Geodesic {
        return area(g);
    }
    match g {
        Geometry::Polygon(p) => planar_polygon_area(p),
        Geometry::MultiPolygon(mp) => mp.0.iter().map(planar_polygon_area).sum(),
        Geometry::Rect(r) => planar_polygon_area(&r.to_polygon()),
        Geometry::Triangle(t) => planar_polygon_area(&t.to_polygon()),
        Geometry::GeometryCollection(gc) => gc.0.iter().map(|g| area_with(g, edges)).sum(),
        _ => 0.0,
    }
}

#[cfg(test)]
mod planar_tests {
    use super::*;
    use crate::densify::{self, Edges};
    use geo::{coord, line_string};

    #[test]
    fn planar_length_matches_dense_geodesic() {
        for (a, b) in [
            (coord! {x: 120.0, y: 30.0}, coord! {x: 121.0, y: 30.0}),
            (coord! {x: 20.0, y: 55.0}, coord! {x: 26.0, y: 60.0}),
            (coord! {x: 100.0, y: -20.0}, coord! {x: 100.0, y: 20.0}),
        ] {
            let exact = planar_edge_length(a, b);
            let dense = densify::planar_path(&[a, b], 1e-4);
            let ref_len: f64 = dense.windows(2).map(|w| distance(w[0], w[1])).sum();
            assert!((exact - ref_len).abs() / exact < 1e-9, "{exact} vs {ref_len}");
        }
    }

    #[test]
    fn planar_area_matches_dense_geodesic() {
        let ring = line_string![
            (x: 120.0, y: 30.0), (x: 121.0, y: 30.0), (x: 121.5, y: 31.0),
            (x: 120.2, y: 31.4), (x: 120.0, y: 30.0)
        ];
        let exact = planar_ring_area(&ring);
        let dense = densify::planar_path(&ring.0, 1e-4);
        let ref_area = ring_area(&LineString(dense));
        assert!((exact - ref_area).abs() / exact < 1e-9, "{exact} vs {ref_area}");
    }

    #[test]
    fn planar_rectangle_area_closed_form() {
        // A lon/lat rectangle: two meridians (geodesics) and two parallels.
        let ring =
            line_string![(x: 0.0, y: 0.0), (x: 1.0, y: 0.0), (x: 1.0, y: 1.0), (x: 0.0, y: 1.0), (x: 0.0, y: 0.0)];
        let area = planar_ring_area(&ring);
        // (Q(1°) − Q(0°)) · 1° in radians
        let expect = (area_below(1f64.to_radians()) - area_below(0.0)) * 1f64.to_radians();
        assert!((area - expect).abs() / expect < 1e-12);
        // and it is slightly smaller than the geodesic-edge polygon
        let geo_area = area_with(&Geometry::Polygon(Polygon::new(ring, vec![])), Edges::Geodesic);
        assert!(area < geo_area && area > geo_area * 0.9999, "{area} vs {geo_area}");
    }
}
