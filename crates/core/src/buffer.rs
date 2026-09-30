//! Buffers with true metric distances on the WGS84 ellipsoid.
//!
//! `Geodesic` (default): every vertex that determines the buffer distance is
//! computed with the geodesic direct problem. The pieces — offset strips along
//! each segment, wedges on the convex side of interior vertices, circles at
//! line ends, and the polygon itself — are merged in a local transverse
//! Mercator plane with a robust non-zero union. Curves are sampled adaptively
//! so that straight chords in the plane stay within `tolerance` of the true
//! curve; the plane is only a topology workspace.
//!
//! Coverage: for any polyline, strips ∪ convex-side wedges ∪ end circles equal
//! the full distance-d neighbourhood (a point whose foot falls outside a
//! segment is strictly closer to that segment's end vertex, so induction over
//! vertices terminates at a strip, a wedge or an end circle).
//!
//! `Projected`: project into the local plane and run a planar buffer. Faster,
//! accurate for small extents (relative error ≈ r²/2R², r = distance from centre).

use std::f64::consts::PI;

use geo::algorithm::buffer::{BufferStyle, LineCap, LineJoin};
use geo::orient::Direction;
use geo::{unary_union, BooleanOps, BoundingRect, Buffer, Coord, Geometry, LineString, MultiPolygon, Orient, Polygon};

use crate::densify::{self, Edges, DEFAULT_TOL};
use crate::geodesic::{self, normalize_deg};
use crate::local::LocalFrame;
use crate::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferMethod {
    Geodesic,
    Projected,
}

#[derive(Debug, Clone, Copy)]
pub struct BufferOptions {
    /// Segments per full circle.
    pub circle_segments: usize,
    pub method: BufferMethod,
    /// Interpretation of input edges.
    pub edges: Edges,
    /// Maximum deviation (m) of chords from the true curves.
    pub tolerance: f64,
}

impl Default for BufferOptions {
    fn default() -> Self {
        BufferOptions {
            circle_segments: 64,
            method: BufferMethod::Geodesic,
            edges: Edges::Planar,
            tolerance: DEFAULT_TOL,
        }
    }
}

/// Geodesic circle as a lon/lat polygon (counter-clockwise, like turf.circle).
pub fn circle(center: Coord, radius_m: f64, segments: usize) -> Polygon {
    let n = segments.max(3);
    let mut pts: Vec<Coord> = (0..n)
        .map(|i| {
            let az = -360.0 * i as f64 / n as f64;
            let (x, y) = geodesic::destination(center.x, center.y, az, radius_m);
            Coord { x, y }
        })
        .collect();
    pts.push(pts[0]);
    Polygon::new(LineString(pts), vec![])
}

pub fn buffer(g: &Geometry, dist_m: f64, opts: &BufferOptions) -> Result<MultiPolygon> {
    if !dist_m.is_finite() {
        return Err(crate::Error::InvalidArgument("buffer distance must be finite".into()));
    }
    if dist_m == 0.0 {
        return Ok(polygonal_part(g));
    }
    if dist_m < 0.0 && polygonal_part(g).0.is_empty() {
        return Ok(MultiPolygon(vec![]));
    }
    if let (Geometry::Point(p), true) = (g, dist_m > 0.0) {
        return Ok(MultiPolygon(vec![circle(p.0, dist_m, opts.circle_segments)]));
    }
    let tol = if opts.tolerance > 0.0 {
        opts.tolerance
    } else {
        DEFAULT_TOL
    };
    let frame = LocalFrame::for_geometries([g])?;
    if let Some(r) = g.bounding_rect() {
        frame.check_extent(&r, dist_m.abs())?;
    }
    match opts.method {
        BufferMethod::Geodesic => buffer_geodesic(&frame, g, dist_m, opts.circle_segments, opts.edges, tol),
        BufferMethod::Projected => buffer_projected(&frame, g, dist_m, opts.circle_segments, opts.edges, tol),
    }
}

fn polygonal_part(g: &Geometry) -> MultiPolygon {
    match g {
        Geometry::Polygon(p) => MultiPolygon(vec![p.clone()]),
        Geometry::MultiPolygon(mp) => mp.clone(),
        Geometry::Rect(r) => MultiPolygon(vec![r.to_polygon()]),
        Geometry::Triangle(t) => MultiPolygon(vec![t.to_polygon()]),
        Geometry::GeometryCollection(gc) => MultiPolygon(gc.0.iter().flat_map(|g| polygonal_part(g).0).collect()),
        _ => MultiPolygon(vec![]),
    }
}

fn buffer_projected(
    frame: &LocalFrame,
    g: &Geometry,
    dist_m: f64,
    segments: usize,
    edges: Edges,
    tol: f64,
) -> Result<MultiPolygon> {
    let projected = densify::project_geometry(frame, g, edges, tol);
    let angle = 2.0 * PI / segments.max(4) as f64;
    let style = BufferStyle::new(dist_m)
        .line_join(LineJoin::Round(angle))
        .line_cap(LineCap::Round(angle));
    let out = projected.buffer_with_style(style);
    Ok(frame.unproject_geom(&out))
}

struct Collector<'a> {
    frame: &'a LocalFrame,
    d: f64,
    segments: usize,
    edges: Edges,
    tol: f64,
    pieces: Vec<Polygon>,
}

/// An input edge with its length and the azimuths of its tangent at both ends.
#[derive(Clone, Copy)]
struct Seg {
    a: Coord,
    b: Coord,
    len: f64,
    azi1: f64,
    azi2: f64,
}

impl<'a> Collector<'a> {
    fn proj(&self, c: Coord) -> Coord {
        let (x, y) = self.frame.project(c.x, c.y);
        Coord { x, y }
    }

    fn offset(&self, pos: Coord, azi: f64) -> Coord {
        let (x, y) = geodesic::destination(pos.x, pos.y, azi, self.d);
        Coord { x, y }
    }

    fn push(&mut self, ring: Vec<Coord>) {
        if ring.len() >= 3 {
            let p = Polygon::new(LineString(ring), vec![]);
            self.pieces.push(p.orient(Direction::Default));
        }
    }

    /// Full circle; `start_az` aligns vertices with strip corners.
    fn circle(&mut self, c: Coord, start_az: f64) {
        let ring: Vec<Coord> = (0..self.segments)
            .map(|i| {
                let az = start_az + 360.0 * i as f64 / self.segments as f64;
                self.proj(self.offset(c, az))
            })
            .collect();
        self.push(ring);
    }

    fn seg(&self, a: Coord, b: Coord) -> Seg {
        match self.edges {
            Edges::Geodesic => {
                let inv = geodesic::inverse(a.x, a.y, b.x, b.y);
                Seg {
                    a,
                    b,
                    len: inv.s12,
                    azi1: inv.azi1,
                    azi2: inv.azi2,
                }
            }
            Edges::Planar => Seg {
                a,
                b,
                len: geodesic::distance(a.x, a.y, b.x, b.y),
                azi1: densify::planar_azimuth(a, b, a),
                azi2: densify::planar_azimuth(a, b, b),
            },
        }
    }

    /// Offset strip on both sides of an edge. Samples are placed along the
    /// edge (geodesic or lon/lat line) and offset along the local normal, which
    /// is exactly the set of points at distance d from the edge.
    fn strip(&mut self, s: &Seg) {
        let (pa, pb) = (self.proj(s.a), self.proj(s.b));
        let x_abs = pa.x.abs().max(pb.x.abs()) + self.d + s.len;
        let extra = self.d / (6_371_000.0 * 6_371_000.0)
            + match self.edges {
                Edges::Planar => densify::planar_curvature(s.a.y.abs().max(s.b.y.abs()) + self.d / 100_000.0),
                Edges::Geodesic => 0.0,
            };
        let n = densify::plane_pieces(s.len + 2.0 * self.d * (s.len / 6_371_000.0), x_abs, extra, self.tol);
        let line = match self.edges {
            Edges::Geodesic if n > 1 => Some(geodesic::Line::new(s.a.x, s.a.y, s.azi1)),
            _ => None,
        };
        let mut right = Vec::with_capacity(n + 3);
        let mut left = Vec::with_capacity(n + 1);
        for k in 0..=n {
            let (pos, azi) = if k == 0 {
                (s.a, s.azi1)
            } else if k == n {
                (s.b, s.azi2)
            } else {
                let t = k as f64 / n as f64;
                match &line {
                    Some(l) => {
                        let (lon, lat, azi) = l.position(s.len * t);
                        (Coord { x: lon, y: lat }, azi)
                    }
                    None => {
                        let p = densify::planar_lerp(s.a, s.b, t);
                        (p, densify::planar_azimuth(s.a, s.b, p))
                    }
                }
            };
            right.push(self.proj(self.offset(pos, azi + 90.0)));
            left.push(self.proj(self.offset(pos, azi - 90.0)));
        }
        // Segment end points are ring vertices so strip ends coincide exactly
        // with neighbouring wedge edges (the geodesic normal is not perfectly
        // straight in the plane).
        left.reverse();
        right.push(pb);
        right.extend(left);
        right.push(pa);
        self.push(right);
    }

    /// Fan on the convex side of `v` between the end normal of the incoming
    /// segment and the start normal of the outgoing one.
    fn wedge(&mut self, v: Coord, az_in: f64, az_out: f64) {
        let turn = normalize_deg(az_out - az_in);
        if turn.abs() < 1e-9 {
            return;
        }
        let (start, sweep) = if turn > 0.0 {
            (az_in - 90.0, turn)
        } else {
            (az_out + 90.0, -turn)
        };
        let m = ((sweep / 360.0 * self.segments as f64).ceil() as usize).max(1);
        let mut ring = Vec::with_capacity(m + 2);
        ring.push(self.proj(v));
        for j in 0..=m {
            let az = start + sweep * j as f64 / m as f64;
            ring.push(self.proj(self.offset(v, az)));
        }
        self.push(ring);
    }

    fn path(&mut self, coords: &[Coord], closed: bool) {
        let mut pts: Vec<Coord> = Vec::with_capacity(coords.len());
        for c in coords {
            if pts.last() != Some(c) {
                pts.push(*c);
            }
        }
        if closed && pts.len() > 1 && pts.first() == pts.last() {
            pts.pop();
        }
        match pts.len() {
            0 => return,
            1 => {
                self.circle(pts[0], 0.0);
                return;
            }
            _ => {}
        }
        let nseg = if closed { pts.len() } else { pts.len() - 1 };
        let mut segs: Vec<Seg> = (0..nseg).map(|i| self.seg(pts[i], pts[(i + 1) % pts.len()])).collect();

        // Joints whose offset gap is below the tolerance get a shared normal
        // (the mean azimuth) instead of a wedge: this avoids thousands of
        // degenerate sliver contours, which dominate the union cost.
        let tiny_turn = (self.tol / self.d.max(1e-9)).to_degrees();
        let mut merged = vec![false; pts.len()];
        #[allow(clippy::needless_range_loop)]
        for i in 0..merged.len() {
            let (prev, next) = match (
                if i > 0 {
                    Some(i - 1)
                } else if closed {
                    Some(nseg - 1)
                } else {
                    None
                },
                if i < nseg { Some(i) } else { None },
            ) {
                (Some(p), Some(n)) => (p, n),
                _ => continue,
            };
            let turn = normalize_deg(segs[next].azi1 - segs[prev].azi2);
            if turn.abs() < tiny_turn {
                let mean = segs[prev].azi2 + turn * 0.5;
                segs[prev].azi2 = mean;
                segs[next].azi1 = mean;
                merged[i] = true;
            }
        }

        for s in &segs {
            self.strip(s);
        }
        for (i, (&v, &is_merged)) in pts.iter().zip(merged.iter()).enumerate() {
            if is_merged {
                continue;
            }
            let incoming = if i > 0 {
                Some(segs[i - 1])
            } else if closed {
                Some(segs[nseg - 1])
            } else {
                None
            };
            let outgoing = if i < nseg { Some(segs[i]) } else { None };
            match (incoming, outgoing) {
                (Some(si), Some(so)) => self.wedge(v, si.azi2, so.azi1),
                (Some(si), None) => self.circle(v, si.azi2 + 90.0),
                (None, Some(so)) => self.circle(v, so.azi1 + 90.0),
                (None, None) => self.circle(v, 0.0),
            }
        }
    }

    fn geometry(&mut self, g: &Geometry, areal: &mut Vec<Polygon>) {
        match g {
            Geometry::Point(p) => self.circle(p.0, 0.0),
            Geometry::MultiPoint(mp) => mp.0.iter().for_each(|p| self.circle(p.0, 0.0)),
            Geometry::Line(l) => self.path(&[l.start, l.end], false),
            Geometry::LineString(ls) => self.path(&ls.0, false),
            Geometry::MultiLineString(m) => m.0.iter().for_each(|ls| self.path(&ls.0, false)),
            Geometry::Polygon(p) => self.polygon(p, areal),
            Geometry::MultiPolygon(mp) => mp.0.iter().for_each(|p| self.polygon(p, areal)),
            Geometry::Rect(r) => self.polygon(&r.to_polygon(), areal),
            Geometry::Triangle(t) => self.polygon(&t.to_polygon(), areal),
            Geometry::GeometryCollection(gc) => gc.0.iter().for_each(|g| self.geometry(g, areal)),
        }
    }

    fn polygon(&mut self, p: &Polygon, areal: &mut Vec<Polygon>) {
        self.path(&p.exterior().0, true);
        for r in p.interiors() {
            self.path(&r.0, true);
        }
        areal.push(densify::project_polygon(self.frame, p, self.edges, self.tol).orient(Direction::Default));
    }
}

/// Pieces (strips, wedges, circles, polygon interiors) in the local plane,
/// before the union. Exposed for profiling and debugging.
#[doc(hidden)]
pub fn buffer_pieces(g: &Geometry, dist_m: f64, opts: &BufferOptions) -> Result<(Vec<Polygon>, Vec<Polygon>)> {
    let frame = LocalFrame::for_geometries([g])?;
    let tol = if opts.tolerance > 0.0 {
        opts.tolerance
    } else {
        DEFAULT_TOL
    };
    let mut col = Collector {
        frame: &frame,
        d: dist_m.abs(),
        segments: opts.circle_segments.max(4).div_ceil(4) * 4,
        edges: opts.edges,
        tol,
        pieces: Vec::new(),
    };
    let mut areal = Vec::new();
    col.geometry(g, &mut areal);
    Ok((col.pieces, areal))
}

fn buffer_geodesic(
    frame: &LocalFrame,
    g: &Geometry,
    dist_m: f64,
    segments: usize,
    edges: Edges,
    tol: f64,
) -> Result<MultiPolygon> {
    let mut col = Collector {
        frame,
        d: dist_m.abs(),
        segments: segments.max(4).div_ceil(4) * 4,
        edges,
        tol,
        pieces: Vec::new(),
    };
    let mut areal = Vec::new();
    col.geometry(g, &mut areal);

    let out = if dist_m > 0.0 {
        let mut all = areal;
        all.append(&mut col.pieces);
        unary_union(all.iter())
    } else {
        if areal.is_empty() {
            return Ok(MultiPolygon(vec![]));
        }
        let subject = unary_union(areal.iter());
        let band = unary_union(col.pieces.iter());
        subject.difference(&band)
    };
    Ok(frame.unproject_geom(&out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::measure;
    use geo::{line_string, point, polygon, MultiLineString};

    fn geodesic_opts() -> BufferOptions {
        BufferOptions {
            edges: Edges::Geodesic,
            ..Default::default()
        }
    }

    fn deviations(mp: &MultiPolygon, lines: &MultiLineString, d: f64) -> Vec<f64> {
        mp.0.iter()
            .flat_map(|p| std::iter::once(p.exterior()).chain(p.interiors()))
            .flat_map(|r| r.0.iter())
            .map(|q| measure::nearest_point_on_line(lines, *q).unwrap().dist - d)
            .collect()
    }

    #[test]
    fn point_buffer_exact() {
        let c = Coord { x: 116.4, y: 39.9 };
        let mp = buffer(&Geometry::Point(point!(x: 116.4, y: 39.9)), 5000.0, &Default::default()).unwrap();
        for q in mp.0[0].exterior().0.iter() {
            assert!((measure::distance(*q, c) - 5000.0).abs() < 1e-6);
        }
    }

    #[test]
    fn line_buffer_vertices_at_distance() {
        let ls = line_string![(x: 100.0, y: 30.0), (x: 104.0, y: 31.0), (x: 106.0, y: 35.0), (x: 103.0, y: 36.0)];
        let d = 20_000.0;
        let mp = buffer(&Geometry::LineString(ls.clone()), d, &geodesic_opts()).unwrap();
        assert_eq!(mp.0.len(), 1);
        assert_eq!(mp.0[0].interiors().len(), 0);
        let devs = deviations(&mp, &MultiLineString(vec![ls]), d);
        // Vertices computed on the ellipsoid are exact; vertices created by the
        // union can sit at most one chord sagitta inside a circle.
        let sagitta = d * (1.0 - (PI / 64.0).cos());
        let (lo, hi) = devs.iter().fold((0.0f64, 0.0f64), |(l, h), e| (l.min(*e), h.max(*e)));
        assert!(hi < 0.02 && lo > -sagitta - 0.02, "min {lo} max {hi}, n={}", devs.len());
        let exact = devs.iter().filter(|e| e.abs() < 0.02).count();
        assert!(exact * 10 >= devs.len() * 9, "{exact}/{}", devs.len());
    }

    #[test]
    fn zigzag_short_segments_no_holes() {
        // Segments much shorter than the distance, sharp turns.
        let mut pts = vec![];
        for i in 0..40 {
            let x = 120.0 + i as f64 * 0.002;
            let y = 30.0 + if i % 2 == 0 { 0.0 } else { 0.003 };
            pts.push(Coord { x, y });
        }
        let ls = LineString(pts);
        let d = 1500.0;
        let g = Geometry::LineString(ls.clone());
        let mp = buffer(&g, d, &geodesic_opts()).unwrap();
        assert_eq!(mp.0.len(), 1);
        assert_eq!(mp.0[0].interiors().len(), 0, "unexpected holes");
        let devs = deviations(&mp, &MultiLineString(vec![ls]), d);
        let sagitta = d * (1.0 - (PI / 64.0).cos());
        assert!(devs.iter().all(|e| *e < 0.02 && *e > -sagitta - 0.02));
    }

    #[test]
    fn planar_edges_follow_parallel() {
        // A 3° east-west line: planar semantics keeps the buffer symmetric about the parallel.
        let ls = line_string![(x: 110.0, y: 40.0), (x: 113.0, y: 40.0)];
        let mp = buffer(&Geometry::LineString(ls), 1000.0, &Default::default()).unwrap();
        let ext = &mp.0[0].exterior().0;
        let north = ext
            .iter()
            .filter(|c| (c.x - 111.5).abs() < 0.05 && c.y > 40.0)
            .map(|c| c.y)
            .fold(f64::NAN, f64::max);
        let south = ext
            .iter()
            .filter(|c| (c.x - 111.5).abs() < 0.05 && c.y < 40.0)
            .map(|c| c.y)
            .fold(f64::NAN, f64::min);
        let dn = geodesic::distance(111.5, 40.0, 111.5, north);
        let ds = geodesic::distance(111.5, 40.0, 111.5, south);
        assert!((dn - 1000.0).abs() < 0.5 && (ds - 1000.0).abs() < 0.5, "{dn} {ds}");
    }

    #[test]
    fn planar_buffer_vertices_exact() {
        // Long diagonal lon/lat edges at high latitude: planar and geodesic differ by km.
        let ls = line_string![(x: 20.0, y: 55.0), (x: 22.0, y: 56.0), (x: 24.0, y: 55.5)];
        let d = 3000.0;
        let mp = buffer(&Geometry::LineString(ls.clone()), d, &Default::default()).unwrap();
        assert_eq!(mp.0.len(), 1);
        assert_eq!(mp.0[0].interiors().len(), 0);
        // Reference: the planar line as dense geodesic pieces (3 mm).
        let dense = densify::prepare(&Geometry::LineString(ls), Edges::Planar, 0.003);
        let Geometry::LineString(dense) = dense else {
            unreachable!()
        };
        let ext = &mp.0[0].exterior().0;
        let step = (ext.len() / 60).max(1);
        let approx = |a: Coord, b: Coord| ((a.x - b.x) * a.y.to_radians().cos()).hypot(a.y - b.y) * 111_200.0;
        let devs: Vec<f64> = ext
            .iter()
            .step_by(step)
            .map(|q| {
                dense
                    .0
                    .windows(2)
                    .filter(|w| approx(w[0], *q) < d * 1.2 + 1500.0)
                    .map(|w| measure::closest_on_segment(w[0], w[1], *q).1)
                    .fold(f64::INFINITY, f64::min)
                    - d
            })
            .collect();
        let sagitta = d * (1.0 - (PI / 64.0).cos());
        let (lo, hi) = devs.iter().fold((0.0f64, 0.0f64), |(l, h), e| (l.min(*e), h.max(*e)));
        assert!(hi < 0.02 && lo > -sagitta - 0.02, "min {lo} max {hi}");
        let exact = devs.iter().filter(|e| e.abs() < 0.02).count();
        assert!(exact * 10 >= devs.len() * 8, "{exact}/{}", devs.len());
    }

    #[test]
    fn polygon_negative_buffer() {
        let p = polygon![(x: 120.0, y: 30.0), (x: 121.0, y: 30.0), (x: 121.0, y: 31.0), (x: 120.0, y: 31.0)];
        let g = Geometry::Polygon(p.clone());
        let grown = buffer(&g, 1000.0, &Default::default()).unwrap();
        let shrunk = buffer(&g, -1000.0, &Default::default()).unwrap();
        let a0 = measure::area(&densify::prepare(&g, Edges::Planar, 0.01));
        let a1 = measure::area(&Geometry::MultiPolygon(grown));
        let a2 = measure::area(&Geometry::MultiPolygon(shrunk));
        let perim = measure::length(&g);
        let band = perim * 1000.0;
        assert!(((a1 - a0) - band).abs() / band < 0.01, "{}", (a1 - a0) / band);
        assert!(((a0 - a2) - band).abs() / band < 0.01, "{}", (a0 - a2) / band);
    }
}
