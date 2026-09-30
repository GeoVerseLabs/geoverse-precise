//! Line-level operations: splitting, offsetting, overlap detection and a few
//! point-to-geometry measures.
//!
//! Metric work (offsets, tolerances) happens on the ellipsoid or in the local
//! plane; splitting and overlap are topological and stay in lon/lat, matching
//! the planar edge convention used elsewhere.

use geo::line_intersection::{line_intersection, LineIntersection};
use geo::{Coord, Geometry, Line, LineString, MultiLineString};

use crate::densify::{self, Edges};
use crate::local::LocalFrame;
use crate::{geodesic, measure, ops, Error, Result};

/// Every segment of a geometry as a two-point line.
pub fn line_segments(g: &Geometry) -> Vec<Line> {
    ops::lines_of(g)
}

/// Split a line wherever another geometry crosses or touches it.
pub fn line_split(line: &LineString, splitter: &Geometry) -> Vec<LineString> {
    let mut cuts: Vec<(usize, f64)> = Vec::new();
    let splitter_lines = ops::lines_of(splitter);
    let splitter_points: Vec<Coord> = match splitter {
        Geometry::Point(p) => vec![p.0],
        Geometry::MultiPoint(mp) => mp.0.iter().map(|p| p.0).collect(),
        _ => Vec::new(),
    };

    for (i, seg) in line.lines().enumerate() {
        let len2 = (seg.end.x - seg.start.x).powi(2) + (seg.end.y - seg.start.y).powi(2);
        let mut add = |c: Coord| {
            if len2 == 0.0 {
                return;
            }
            let t = ((c.x - seg.start.x) * (seg.end.x - seg.start.x)
                + (c.y - seg.start.y) * (seg.end.y - seg.start.y))
                / len2;
            if t > 1e-12 && t < 1.0 - 1e-12 {
                cuts.push((i, t));
            }
        };
        for other in &splitter_lines {
            match line_intersection(seg, *other) {
                Some(LineIntersection::SinglePoint { intersection, .. }) => add(intersection),
                Some(LineIntersection::Collinear { intersection }) => {
                    add(intersection.start);
                    add(intersection.end);
                }
                None => {}
            }
        }
        for p in &splitter_points {
            // a point splits the line when it sits on it
            let (_, d, _) = planar_point_segment(*p, seg.start, seg.end);
            if d < 1e-12 {
                add(*p);
            }
        }
    }

    cuts.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
    cuts.dedup_by(|a, b| a.0 == b.0 && (a.1 - b.1).abs() < 1e-12);

    let mut parts: Vec<LineString> = Vec::new();
    let mut current: Vec<Coord> = Vec::new();
    for (i, seg) in line.lines().enumerate() {
        if current.is_empty() {
            current.push(seg.start);
        }
        for (ci, t) in cuts.iter().filter(|(ci, _)| *ci == i) {
            let _ = ci;
            let c = Coord {
                x: seg.start.x + (seg.end.x - seg.start.x) * t,
                y: seg.start.y + (seg.end.y - seg.start.y) * t,
            };
            if current.last() != Some(&c) {
                current.push(c);
            }
            if current.len() >= 2 {
                parts.push(LineString(std::mem::take(&mut current)));
            }
            current.push(c);
        }
        if current.last() != Some(&seg.end) {
            current.push(seg.end);
        }
    }
    if current.len() >= 2 {
        parts.push(LineString(current));
    }
    if parts.is_empty() {
        parts.push(line.clone());
    }
    parts
}

#[inline]
fn planar_point_segment(p: Coord, a: Coord, b: Coord) -> (Coord, f64, f64) {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len2 = dx * dx + dy * dy;
    if len2 == 0.0 {
        return (a, (p.x - a.x).hypot(p.y - a.y), 0.0);
    }
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0);
    let q = Coord { x: a.x + t * dx, y: a.y + t * dy };
    ((q), (p.x - q.x).hypot(p.y - q.y), t)
}

/// Offset a line by `dist_m` metres (positive = left of the direction of
/// travel, like turf). Every generated vertex sits exactly at the requested
/// distance from its own segment, and convex corners get a rounded join.
///
/// Like every naive offset (turf's included), the inside of a tight corner can
/// fold over itself — run the result through [`crate::buffer`] if you need a
/// clean single-sided region.
pub fn line_offset(line: &LineString, dist_m: f64, edges: Edges, steps: usize) -> Result<LineString> {
    let pts: Vec<Coord> = {
        let mut v: Vec<Coord> = Vec::with_capacity(line.0.len());
        for c in &line.0 {
            if v.last() != Some(c) {
                v.push(*c);
            }
        }
        v
    };
    if pts.len() < 2 {
        return Err(Error::InvalidGeometry("line needs at least 2 distinct positions".into()));
    }
    let side = if dist_m >= 0.0 { -90.0 } else { 90.0 };
    let d = dist_m.abs();
    let segs: Vec<(Coord, Coord, f64, f64, f64)> = pts
        .windows(2)
        .map(|w| match edges {
            Edges::Geodesic => {
                let inv = geodesic::inverse(w[0].x, w[0].y, w[1].x, w[1].y);
                (w[0], w[1], inv.s12, inv.azi1, inv.azi2)
            }
            Edges::Planar => (
                w[0],
                w[1],
                measure::planar_edge_length(w[0], w[1]),
                densify::planar_azimuth(w[0], w[1], w[0]),
                densify::planar_azimuth(w[0], w[1], w[1]),
            ),
        })
        .collect();

    let offset = |p: Coord, azi: f64| {
        let (x, y) = geodesic::destination(p.x, p.y, azi + side, d);
        Coord { x, y }
    };

    let mut out: Vec<Coord> = Vec::with_capacity(pts.len() * 2);
    for (i, s) in segs.iter().enumerate() {
        if i == 0 {
            out.push(offset(s.0, s.3));
        } else {
            // joint between segment i-1 and i
            let prev = &segs[i - 1];
            let turn = geodesic::normalize_deg(s.3 - prev.4);
            let convex = if side < 0.0 { turn < 0.0 } else { turn > 0.0 };
            if convex && turn.abs() > 1e-9 {
                let sweep = turn.abs();
                let n = ((sweep / 360.0 * steps.max(4) as f64).ceil() as usize).max(1);
                let start = prev.4 + side;
                let dir = if turn > 0.0 { 1.0 } else { -1.0 };
                for k in 0..=n {
                    let az = start + dir * sweep * k as f64 / n as f64;
                    let (x, y) = geodesic::destination(s.0.x, s.0.y, az, d);
                    out.push(Coord { x, y });
                }
            } else {
                out.push(offset(s.0, prev.4));
                out.push(offset(s.0, s.3));
            }
        }
        out.push(offset(s.1, s.4));
    }
    Ok(LineString(out))
}

/// Portions where two line geometries run along each other (within `tolerance_m`).
pub fn line_overlap(a: &Geometry, b: &Geometry, tolerance_m: f64) -> Result<MultiLineString> {
    let frame = LocalFrame::for_geometries([a, b])?;
    let proj = |c: Coord| {
        let (x, y) = frame.project(c.x, c.y);
        Coord { x, y }
    };
    let asegs: Vec<(Coord, Coord)> = ops::lines_of(a).into_iter().map(|l| (proj(l.start), proj(l.end))).collect();
    let bsegs: Vec<(Coord, Coord)> = ops::lines_of(b).into_iter().map(|l| (proj(l.start), proj(l.end))).collect();
    let tol = tolerance_m.max(1e-9);

    let mut parts: Vec<LineString> = Vec::new();
    let mut current: Vec<Coord> = Vec::new();
    for (p, q) in &asegs {
        let dir = ((q.x - p.x), (q.y - p.y));
        let len = dir.0.hypot(dir.1);
        if len == 0.0 {
            continue;
        }
        // intervals of this segment that lie on some segment of b
        let mut spans: Vec<(f64, f64)> = Vec::new();
        for (r, s) in &bsegs {
            let (_, dr, tr) = planar_point_segment(*r, *p, *q);
            let (_, ds, ts) = planar_point_segment(*s, *p, *q);
            // both ends of b's segment must be close to a's segment (collinear)
            let near_r = dr <= tol;
            let near_s = ds <= tol;
            if !near_r && !near_s {
                continue;
            }
            // require near-parallel directions
            let bdir = (s.x - r.x, s.y - r.y);
            let blen = bdir.0.hypot(bdir.1);
            if blen == 0.0 {
                continue;
            }
            let cos = (dir.0 * bdir.0 + dir.1 * bdir.1).abs() / (len * blen);
            if cos < 0.995 {
                continue;
            }
            let (t0, t1) = if near_r && near_s {
                (tr.min(ts), tr.max(ts))
            } else {
                // clip against the part of a's segment covered by b
                let (_, _, t_p) = planar_point_segment(*p, *r, *s);
                let (_, _, t_q) = planar_point_segment(*q, *r, *s);
                let inside_p = t_p > 0.0 && t_p < 1.0;
                let inside_q = t_q > 0.0 && t_q < 1.0;
                let t_end = if near_r { tr } else { ts };
                if inside_p {
                    (0.0f64.min(t_end), 0.0f64.max(t_end))
                } else if inside_q {
                    (t_end.min(1.0), t_end.max(1.0))
                } else {
                    continue;
                }
            };
            if (t1 - t0) * len > tol {
                spans.push((t0.clamp(0.0, 1.0), t1.clamp(0.0, 1.0)));
            }
        }
        if spans.is_empty() {
            if current.len() >= 2 {
                parts.push(LineString(std::mem::take(&mut current)));
            } else {
                current.clear();
            }
            continue;
        }
        spans.sort_by(|x, y| x.0.total_cmp(&y.0));
        let mut merged: Vec<(f64, f64)> = vec![spans[0]];
        for s in &spans[1..] {
            let last = merged.last_mut().unwrap();
            if s.0 <= last.1 + tol / len {
                last.1 = last.1.max(s.1);
            } else {
                merged.push(*s);
            }
        }
        for (k, (t0, t1)) in merged.iter().enumerate() {
            let at = |t: f64| {
                let c = Coord { x: p.x + dir.0 * t, y: p.y + dir.1 * t };
                let (x, y) = frame.unproject(c.x, c.y);
                Coord { x, y }
            };
            let (s0, s1) = (at(*t0), at(*t1));
            let continues = k == 0 && current.last().is_some_and(|c| measure::distance(*c, s0) <= tol);
            if !continues && current.len() >= 2 {
                parts.push(LineString(std::mem::take(&mut current)));
            } else if !continues {
                current.clear();
            }
            if current.last() != Some(&s0) {
                current.push(s0);
            }
            current.push(s1);
        }
    }
    if current.len() >= 2 {
        parts.push(LineString(current));
    }
    Ok(MultiLineString(parts))
}

/// Which point of a set is closest to a line? Returns `(index, distance_m)`.
pub fn nearest_point_to_line(points: &[Coord], lines: &MultiLineString) -> Option<(usize, f64)> {
    points
        .iter()
        .enumerate()
        .filter_map(|(i, p)| measure::nearest_point_on_line_opts(lines, *p, false).map(|n| (i, n.dist)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

/// Distance (m) from a point to a polygon: negative when the point is inside.
pub fn point_to_polygon_distance(p: Coord, polygon: &Geometry, edges: Edges) -> Result<f64> {
    let dense = densify::prepare(polygon, edges, densify::DEFAULT_TOL);
    let rings = ops::polygon_to_line(&dense)?;
    let n = measure::nearest_point_on_line_opts(&rings, p, false)
        .ok_or_else(|| Error::InvalidGeometry("polygon has no boundary".into()))?;
    let inside = crate::predicates::point_in_polygon(p, polygon, false);
    Ok(if inside { -n.dist } else { n.dist })
}

/// Angle at `b` between `b→a` and `b→c`, in degrees.
///
/// Returns the smaller (interior) angle by default; `explementary` gives the
/// reflex angle instead.
pub fn angle(a: Coord, b: Coord, c: Coord, explementary: bool) -> f64 {
    let ba = measure::bearing(b, a, false);
    let bc = measure::bearing(b, c, false);
    let raw = geodesic::normalize_deg(bc - ba).abs();
    if explementary {
        360.0 - raw
    } else {
        raw
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo::{coord, line_string, polygon};

    #[test]
    fn splitting_a_line() {
        let line = line_string![(x: 0.0, y: 0.0), (x: 4.0, y: 0.0)];
        let splitter = Geometry::LineString(line_string![(x: 2.0, y: -1.0), (x: 2.0, y: 1.0)]);
        let parts = line_split(&line, &splitter);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].0.last().unwrap(), &coord! {x: 2.0, y: 0.0});
        assert_eq!(parts[1].0.first().unwrap(), &coord! {x: 2.0, y: 0.0});

        // a polygon boundary can split too
        let poly = Geometry::Polygon(polygon![(x: 1.0, y: -1.0), (x: 3.0, y: -1.0), (x: 3.0, y: 1.0), (x: 1.0, y: 1.0)]);
        assert_eq!(line_split(&line, &poly).len(), 3);

        // no intersection returns the original
        let away = Geometry::LineString(line_string![(x: 9.0, y: 9.0), (x: 10.0, y: 10.0)]);
        assert_eq!(line_split(&line, &away).len(), 1);
    }

    #[test]
    fn offsetting_keeps_the_distance() {
        let dense = |l: &LineString| MultiLineString(vec![LineString(densify::planar_path(&l.0, 0.01))]);
        let dist_to = |l: &MultiLineString, c: Coord| {
            measure::nearest_point_on_line_opts(l, c, false).unwrap().dist
        };

        // a straight line has no joints: every offset vertex is exactly d away
        let straight = line_string![(x: 120.0, y: 30.0), (x: 120.3, y: 30.0)];
        let ref_s = dense(&straight);
        for dist in [500.0, -500.0, 2_000.0] {
            let off = line_offset(&straight, dist, Edges::Planar, 32).unwrap();
            for c in &off.0 {
                let d = dist_to(&ref_s, *c);
                assert!((d - dist.abs()).abs() < 0.05, "straight: {d} vs {dist}");
            }
        }

        // a bend: the convex side is mitre-free (a d-radius arc about the
        // corner, so still exactly d), while the concave side keeps both
        // perpendicular feet without mitring, which puts the joint vertices at
        // d·cos(turn) — never further than d. That is the documented
        // behaviour: concave corners under-shoot rather than fold over.
        let bend = line_string![(x: 120.0, y: 30.0), (x: 120.2, y: 30.0), (x: 120.4, y: 30.02)];
        let ref_b = dense(&bend);
        let turn = {
            let a1 = densify::planar_azimuth(bend.0[0], bend.0[1], bend.0[1]);
            let a2 = densify::planar_azimuth(bend.0[1], bend.0[2], bend.0[1]);
            geodesic::normalize_deg(a2 - a1).abs()
        };
        assert!(turn > 1.0 && turn < 20.0, "turn {turn}");
        let d = 500.0;
        // heading east and turning left (north): left is the concave side
        let left = line_offset(&bend, d, Edges::Planar, 32).unwrap();
        let right = line_offset(&bend, -d, Edges::Planar, 32).unwrap();
        let (concave, convex) = (&left, &right);

        let floor = d * turn.to_radians().cos() - 0.05;
        for c in &concave.0 {
            let dd = dist_to(&ref_b, *c);
            assert!(dd <= d + 0.05 && dd >= floor, "concave: {dd} not in [{floor}, {d}]");
        }
        for c in &convex.0 {
            let dd = dist_to(&ref_b, *c);
            assert!((dd - d).abs() < 0.05, "convex: {dd} vs {d}");
        }

        // the sign picks the side: +d is left of travel (north, heading east)
        assert!(left.0[0].y > 30.0 && right.0[0].y < 30.0);
        // the convex side gets an arc, so it has more vertices than the input
        assert!(convex.0.len() > bend.0.len());
    }

    #[test]
    fn overlap_finds_shared_runs() {
        let a = Geometry::LineString(line_string![(x: 0.0, y: 0.0), (x: 0.01, y: 0.0)]);
        let b = Geometry::LineString(line_string![(x: 0.002, y: 0.0), (x: 0.006, y: 0.0)]);
        let ov = line_overlap(&a, &b, 0.5).unwrap();
        assert_eq!(ov.0.len(), 1);
        let len = measure::line_length(&ov.0[0]);
        let expect = measure::distance(coord! {x: 0.002, y: 0.0}, coord! {x: 0.006, y: 0.0});
        assert!((len - expect).abs() < 1.0, "{len} vs {expect}");

        // crossing lines are not overlaps
        let c = Geometry::LineString(line_string![(x: 0.005, y: -0.005), (x: 0.005, y: 0.005)]);
        assert_eq!(line_overlap(&a, &c, 0.5).unwrap().0.len(), 0);
    }

    #[test]
    fn point_measures() {
        let poly = Geometry::Polygon(polygon![
            (x: 120.0, y: 30.0), (x: 120.1, y: 30.0), (x: 120.1, y: 30.1), (x: 120.0, y: 30.1), (x: 120.0, y: 30.0)
        ]);
        let inside = point_to_polygon_distance(coord! {x: 120.05, y: 30.05}, &poly, Edges::Planar).unwrap();
        let outside = point_to_polygon_distance(coord! {x: 120.2, y: 30.05}, &poly, Edges::Planar).unwrap();
        assert!(inside < 0.0 && outside > 0.0);
        // nearest boundary is the east/west edge: 0.05° of longitude at 30°N
        assert!((inside.abs() - 4816.0).abs() < 50.0, "{inside}");

        let lines = MultiLineString(vec![line_string![(x: 0.0, y: 0.0), (x: 1.0, y: 0.0)]]);
        let pts = [coord! {x: 0.5, y: 0.5}, coord! {x: 0.5, y: 0.1}];
        assert_eq!(nearest_point_to_line(&pts, &lines).unwrap().0, 1);
    }

    #[test]
    fn angles() {
        let a = coord! {x: 0.0, y: 1.0};
        let b = coord! {x: 0.0, y: 0.0};
        let c = coord! {x: 1.0, y: 0.0};
        let deg = angle(a, b, c, false);
        assert!((deg - 90.0).abs() < 0.01, "{deg}");
        assert!((angle(a, b, c, true) - 270.0).abs() < 0.01);
    }
}
