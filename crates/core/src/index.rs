//! Prepared geometry: a parsed geometry plus a uniform grid over its segments,
//! for repeated point queries (containment, nearest point, distance).
//!
//! Without it, every call from JavaScript re-parses the GeoJSON and rescans
//! every segment, which dominates bulk workloads. Building the index once and
//! reusing it turns a 500-vertex point-in-polygon query from hundreds of
//! microseconds into about a microsecond.
//!
//! The grid lives in lon/lat, so containment keeps the planar (GeoJSON) edge
//! semantics used elsewhere; nearest-point queries still solve exactly on the
//! ellipsoid, the grid only shortlists candidates.

use geo::{BoundingRect, Coord, Geometry, LineString, MultiLineString, MultiPolygon, Polygon, Rect};

use crate::measure::{self, Nearest};
use crate::{Error, Result};

const MAX_CELLS_PER_AXIS: usize = 256;

/// Total order on f64 for the cell priority queue.
#[derive(Clone, Copy, PartialEq)]
struct OrderedFloat(f64);
impl Eq for OrderedFloat {}
impl Ord for OrderedFloat {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}
impl PartialOrd for OrderedFloat {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(feature = "index-stats")]
thread_local! {
    #[doc(hidden)]
    pub static STATS: std::cell::RefCell<(usize, usize)> = const { std::cell::RefCell::new((0, 0)) };
}

#[derive(Clone, Copy)]
struct Seg {
    a: Coord,
    b: Coord,
    part: u32,
    idx: u32,
}

/// Uniform grid over the segments of a geometry.
pub struct SegmentIndex {
    segs: Vec<Seg>,
    unit: Vec<[f64; 3]>,
    rect: Rect<f64>,
    nx: usize,
    ny: usize,
    cell_x: f64,
    cell_y: f64,
    cells: Vec<Vec<u32>>,
    rows: Vec<Vec<u32>>,
    /// Metres per degree of longitude at the least favourable latitude.
    x_scale: f64,
}

impl SegmentIndex {
    pub fn build(parts: &[Vec<Coord>]) -> Option<SegmentIndex> {
        let mut segs = Vec::new();
        for (pi, part) in parts.iter().enumerate() {
            for (i, w) in part.windows(2).enumerate() {
                if w[0] != w[1] {
                    segs.push(Seg {
                        a: w[0],
                        b: w[1],
                        part: pi as u32,
                        idx: i as u32,
                    });
                }
            }
        }
        if segs.is_empty() {
            return None;
        }
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for s in &segs {
            for c in [s.a, s.b] {
                min_x = min_x.min(c.x);
                min_y = min_y.min(c.y);
                max_x = max_x.max(c.x);
                max_y = max_y.max(c.y);
            }
        }
        let rect = Rect::new(Coord { x: min_x, y: min_y }, Coord { x: max_x, y: max_y });
        let n = (segs.len() as f64).sqrt().ceil() as usize;
        let nx = n.clamp(1, MAX_CELLS_PER_AXIS);
        let ny = n.clamp(1, MAX_CELLS_PER_AXIS);
        let cell_x = ((max_x - min_x) / nx as f64).max(1e-12);
        let cell_y = ((max_y - min_y) / ny as f64).max(1e-12);
        let mut cells = vec![Vec::new(); nx * ny];
        let mut rows = vec![Vec::new(); ny];
        for (id, s) in segs.iter().enumerate() {
            let (x0, x1) = (s.a.x.min(s.b.x), s.a.x.max(s.b.x));
            let (y0, y1) = (s.a.y.min(s.b.y), s.a.y.max(s.b.y));
            let cx0 = (((x0 - min_x) / cell_x).floor() as isize).clamp(0, nx as isize - 1) as usize;
            let cx1 = (((x1 - min_x) / cell_x).floor() as isize).clamp(0, nx as isize - 1) as usize;
            let cy0 = (((y0 - min_y) / cell_y).floor() as isize).clamp(0, ny as isize - 1) as usize;
            let cy1 = (((y1 - min_y) / cell_y).floor() as isize).clamp(0, ny as isize - 1) as usize;
            for cy in cy0..=cy1 {
                rows[cy].push(id as u32);
                for cx in cx0..=cx1 {
                    cells[cy * nx + cx].push(id as u32);
                }
            }
        }
        let worst_lat = min_y.abs().max(max_y.abs()).min(89.5);
        let unit = segs
            .iter()
            .map(|s| measure::unit_vec_for_profiling(s.a))
            .collect::<Vec<_>>();
        Some(SegmentIndex {
            segs,
            unit,
            rect,
            nx,
            ny,
            cell_x,
            cell_y,
            cells,
            rows,
            x_scale: 111_320.0 * worst_lat.to_radians().cos().max(1e-6),
        })
    }

    pub fn segment_count(&self) -> usize {
        self.segs.len()
    }

    pub fn bounding_rect(&self) -> Rect<f64> {
        self.rect
    }

    #[inline]
    fn cell_of(&self, c: Coord) -> (isize, isize) {
        (
            ((c.x - self.rect.min().x) / self.cell_x).floor() as isize,
            ((c.y - self.rect.min().y) / self.cell_y).floor() as isize,
        )
    }

    /// Ray-casting parity test along +x, using only the segments in the
    /// query point's grid row. Edges are straight in lon/lat.
    /// Returns `(inside, on_boundary)`.
    pub fn parity(&self, p: Coord) -> (bool, bool) {
        let (_, cy) = self.cell_of(p);
        if cy < 0 || cy >= self.ny as isize {
            return (false, false);
        }
        let mut crossings = 0usize;
        for &id in &self.rows[cy as usize] {
            let s = self.segs[id as usize];
            let (a, b) = (s.a, s.b);
            // on the segment?
            let cr = (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
            if cr == 0.0 && p.x >= a.x.min(b.x) && p.x <= a.x.max(b.x) && p.y >= a.y.min(b.y) && p.y <= a.y.max(b.y) {
                return (false, true);
            }
            if (a.y > p.y) != (b.y > p.y) {
                let t = (p.y - a.y) / (b.y - a.y);
                let x = a.x + t * (b.x - a.x);
                if x > p.x {
                    crossings += 1;
                }
            }
        }
        (crossings % 2 == 1, false)
    }

    /// Distance in metres from a point to the closest segment, exact on the
    /// ellipsoid. The grid is expanded ring by ring until no closer segment can
    /// exist, then the shortlist is solved exactly.
    /// Distance (m) from `p` to a grid cell's bounding box (0 inside).
    #[inline]
    fn cell_distance(&self, p: Coord, cx: usize, cy: usize) -> f64 {
        let mn = self.rect.min();
        let x0 = mn.x + cx as f64 * self.cell_x;
        let y0 = mn.y + cy as f64 * self.cell_y;
        let dx = (x0 - p.x).max(p.x - (x0 + self.cell_x)).max(0.0) * self.x_scale;
        let dy = (y0 - p.y).max(p.y - (y0 + self.cell_y)).max(0.0) * 110_574.0;
        dx.hypot(dy)
    }

    /// Closest point on the indexed segments, exact on the ellipsoid.
    ///
    /// Cells are visited nearest-first; segments are ranked with a single
    /// gnomonic iteration (already sub-millimetre for the distance) and only
    /// the winner is solved to convergence.
    pub fn nearest(&self, p: Coord) -> Option<Nearest> {
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;

        let pv = measure::unit_vec_for_profiling(p);
        let r_local = measure::local_radius_at(p.y);
        let (cx0, cy0) = self.cell_of(p);
        let start = (
            cx0.clamp(0, self.nx as isize - 1) as usize,
            cy0.clamp(0, self.ny as isize - 1) as usize,
        );

        let mut seen_seg = vec![false; self.segs.len()];
        let mut queued = vec![false; self.nx * self.ny];
        let mut heap: BinaryHeap<Reverse<(OrderedFloat, usize, usize)>> = BinaryHeap::new();
        heap.push(Reverse((
            OrderedFloat(self.cell_distance(p, start.0, start.1)),
            start.0,
            start.1,
        )));
        queued[start.1 * self.nx + start.0] = true;

        let mut best: Option<(u32, f64, Coord)> = None;
        while let Some(Reverse((OrderedFloat(cd), cx, cy))) = heap.pop() {
            if let Some((_, bd, _)) = best {
                if cd > bd * 1.004 + 0.5 {
                    break;
                }
            }
            for &id in &self.cells[cy * self.nx + cx] {
                if seen_seg[id as usize] {
                    continue;
                }
                seen_seg[id as usize] = true;
                let s = self.segs[id as usize];
                let (approx, hint) = measure::approx_seg_distance_at(
                    self.unit[id as usize],
                    measure::unit_vec_for_profiling(s.b),
                    pv,
                    r_local,
                );
                if let Some((_, bd, _)) = best {
                    if approx > bd * 1.004 + 0.5 {
                        continue;
                    }
                }
                let hint = measure::to_lonlat_for_profiling(hint);
                let d = measure::closest_on_segment_scored(s.a, s.b, p, hint);
                if best.is_none_or(|(_, bd, _)| d < bd) {
                    best = Some((id, d, hint));
                }
            }
            for (dx, dy) in [(1isize, 0isize), (-1, 0), (0, 1), (0, -1)] {
                let (nx, ny) = (cx as isize + dx, cy as isize + dy);
                if nx < 0 || ny < 0 || nx >= self.nx as isize || ny >= self.ny as isize {
                    continue;
                }
                let (nx, ny) = (nx as usize, ny as usize);
                if queued[ny * self.nx + nx] {
                    continue;
                }
                queued[ny * self.nx + nx] = true;
                heap.push(Reverse((OrderedFloat(self.cell_distance(p, nx, ny)), nx, ny)));
            }
        }

        let (id, _, hint) = best?;
        let s = self.segs[id as usize];
        let (q, d, loc) = measure::closest_on_segment_hinted(s.a, s.b, p, Some(hint));
        Some(Nearest {
            point: q,
            dist: d,
            location: loc,
            index: s.idx as usize,
            multi_index: s.part as usize,
        })
    }
}

/// A geometry parsed once, with an index, ready for repeated queries.
pub struct Prepared {
    pub geometry: Geometry<f64>,
    index: Option<SegmentIndex>,
    areal: bool,
    /// Cumulative geodesic length before each vertex, per line part.
    cumulative: Vec<Vec<f64>>,
    points: Vec<Coord>,
}

fn rings_of(p: &Polygon, out: &mut Vec<Vec<Coord>>) {
    out.push(p.exterior().0.clone());
    for r in p.interiors() {
        out.push(r.0.clone());
    }
}

fn parts_of(g: &Geometry, out: &mut Vec<Vec<Coord>>, points: &mut Vec<Coord>) {
    match g {
        Geometry::Point(p) => points.push(p.0),
        Geometry::MultiPoint(mp) => points.extend(mp.0.iter().map(|p| p.0)),
        Geometry::Line(l) => out.push(vec![l.start, l.end]),
        Geometry::LineString(ls) => out.push(ls.0.clone()),
        Geometry::MultiLineString(m) => out.extend(m.0.iter().map(|l| l.0.clone())),
        Geometry::Polygon(p) => rings_of(p, out),
        Geometry::MultiPolygon(mp) => mp.0.iter().for_each(|p| rings_of(p, out)),
        Geometry::Rect(r) => rings_of(&r.to_polygon(), out),
        Geometry::Triangle(t) => rings_of(&t.to_polygon(), out),
        Geometry::GeometryCollection(gc) => gc.0.iter().for_each(|g| parts_of(g, out, points)),
    }
}

impl Prepared {
    pub fn new(geometry: Geometry<f64>) -> Result<Prepared> {
        let mut parts = Vec::new();
        let mut points = Vec::new();
        parts_of(&geometry, &mut parts, &mut points);
        let areal = matches!(
            geometry,
            Geometry::Polygon(_) | Geometry::MultiPolygon(_) | Geometry::Rect(_) | Geometry::Triangle(_)
        ) || matches!(&geometry, Geometry::GeometryCollection(gc)
            if gc.0.iter().any(|g| matches!(g, Geometry::Polygon(_) | Geometry::MultiPolygon(_))));
        if parts.is_empty() && points.is_empty() {
            return Err(Error::InvalidGeometry("empty geometry".into()));
        }
        let cumulative = parts
            .iter()
            .map(|part| {
                let mut acc = Vec::with_capacity(part.len());
                let mut total = 0.0;
                acc.push(0.0);
                for w in part.windows(2) {
                    total += measure::distance(w[0], w[1]);
                    acc.push(total);
                }
                acc
            })
            .collect();
        let index = SegmentIndex::build(&parts);
        Ok(Prepared {
            geometry,
            index,
            areal,
            cumulative,
            points,
        })
    }

    pub fn is_areal(&self) -> bool {
        self.areal
    }

    pub fn segment_count(&self) -> usize {
        self.index.as_ref().map_or(0, |i| i.segment_count())
    }

    pub fn bounding_rect(&self) -> Option<Rect<f64>> {
        self.index
            .as_ref()
            .map(|i| i.bounding_rect())
            .or_else(|| self.geometry.bounding_rect())
    }

    /// Point in polygon (edges straight in lon/lat).
    pub fn contains_point(&self, p: Coord, ignore_boundary: bool) -> bool {
        if !self.areal {
            return false;
        }
        let Some(idx) = self.index.as_ref() else { return false };
        let r = idx.bounding_rect();
        if p.x < r.min().x || p.x > r.max().x || p.y < r.min().y || p.y > r.max().y {
            return false;
        }
        let (inside, boundary) = idx.parity(p);
        if boundary {
            !ignore_boundary
        } else {
            inside
        }
    }

    /// Closest point on the geometry's boundary (or on its lines), with the
    /// distance along the part filled in from the cached cumulative lengths.
    pub fn nearest(&self, p: Coord) -> Option<Nearest> {
        let mut n = match self.index.as_ref() {
            Some(idx) => idx.nearest(p)?,
            None => {
                // point-only geometry
                let (i, d) = self
                    .points
                    .iter()
                    .enumerate()
                    .map(|(i, c)| (i, measure::distance(*c, p)))
                    .min_by(|a, b| a.1.total_cmp(&b.1))?;
                return Some(Nearest {
                    point: self.points[i],
                    dist: d,
                    location: 0.0,
                    index: i,
                    multi_index: 0,
                });
            }
        };
        if let Some(cum) = self.cumulative.get(n.multi_index) {
            if let Some(before) = cum.get(n.index) {
                n.location += before;
            }
        }
        Some(n)
    }

    /// Distance (m) from a point to the geometry: zero inside an areal geometry.
    pub fn distance_to(&self, p: Coord) -> f64 {
        if self.areal && self.contains_point(p, false) {
            return 0.0;
        }
        self.nearest(p).map_or(f64::NAN, |n| n.dist)
    }

    /// Batch containment. `coords` is interleaved lon/lat.
    pub fn contains_points(&self, coords: &[f64], ignore_boundary: bool, out: &mut [u8]) {
        for (i, c) in coords.as_chunks::<2>().0.iter().enumerate() {
            if i >= out.len() {
                break;
            }
            out[i] = u8::from(self.contains_point(Coord { x: c[0], y: c[1] }, ignore_boundary));
        }
    }

    /// Batch nearest point: writes `[lon, lat, dist, location, index, part]` per query.
    pub fn nearest_batch(&self, coords: &[f64], out: &mut [f64]) {
        for (i, c) in coords.as_chunks::<2>().0.iter().enumerate() {
            let o = i * 6;
            if o + 6 > out.len() {
                break;
            }
            match self.nearest(Coord { x: c[0], y: c[1] }) {
                Some(n) => {
                    out[o] = n.point.x;
                    out[o + 1] = n.point.y;
                    out[o + 2] = n.dist;
                    out[o + 3] = n.location;
                    out[o + 4] = n.index as f64;
                    out[o + 5] = n.multi_index as f64;
                }
                None => out[o..o + 6].fill(f64::NAN),
            }
        }
    }

    pub fn as_multi_line_string(&self) -> MultiLineString {
        let mut parts = Vec::new();
        let mut points = Vec::new();
        parts_of(&self.geometry, &mut parts, &mut points);
        MultiLineString(parts.into_iter().map(LineString).collect())
    }

    pub fn as_multi_polygon(&self) -> Option<MultiPolygon> {
        match &self.geometry {
            Geometry::Polygon(p) => Some(MultiPolygon(vec![p.clone()])),
            Geometry::MultiPolygon(mp) => Some(mp.clone()),
            _ => None,
        }
    }
}

/// A prepared geometry plus its CRS, ready to answer queries in the caller's
/// coordinate system.
pub struct PreparedInCrs {
    pub prepared: Prepared,
    pub to_wgs84: crate::crs::Transformer,
    pub from_wgs84: crate::crs::Transformer,
}

impl PreparedInCrs {
    pub fn new(geometry: Geometry<f64>, crs: &crate::crs::Crs) -> Result<PreparedInCrs> {
        Ok(PreparedInCrs {
            prepared: Prepared::new(geometry)?,
            to_wgs84: crate::crs::Transformer::new(crs, &crate::crs::Crs::Wgs84),
            from_wgs84: crate::crs::Transformer::new(&crate::crs::Crs::Wgs84, crs),
        })
    }

    #[inline]
    pub fn point_in(&self, x: f64, y: f64) -> Coord {
        let (x, y) = self.to_wgs84.apply(x, y);
        Coord { x, y }
    }

    #[inline]
    pub fn point_out(&self, c: Coord) -> [f64; 2] {
        let (x, y) = self.from_wgs84.apply(c.x, c.y);
        [x, y]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo::{coord, polygon, MultiLineString};

    fn ring_polygon(n: usize) -> Polygon {
        let mut pts: Vec<Coord> = (0..n)
            .map(|i| {
                let a = std::f64::consts::TAU * i as f64 / n as f64;
                Coord {
                    x: 120.0 + 0.5 * a.cos(),
                    y: 30.0 + 0.5 * a.sin(),
                }
            })
            .collect();
        pts.push(pts[0]);
        Polygon::new(LineString(pts), vec![])
    }

    #[test]
    fn contains_matches_unindexed() {
        let poly = ring_polygon(400);
        let g = Geometry::Polygon(poly.clone());
        let prep = Prepared::new(g.clone()).unwrap();
        for i in 0..500 {
            let p = coord! { x: 119.4 + (i % 25) as f64 * 0.05, y: 29.4 + (i / 25) as f64 * 0.05 };
            assert_eq!(
                prep.contains_point(p, false),
                crate::predicates::point_in_polygon(p, &g, false),
                "{p:?}"
            );
        }
    }

    #[test]
    fn nearest_matches_unindexed() {
        let poly = ring_polygon(300);
        let g = Geometry::Polygon(poly.clone());
        let prep = Prepared::new(g).unwrap();
        let lines = MultiLineString(vec![poly.exterior().clone()]);
        for i in 0..200 {
            let p = coord! { x: 119.3 + (i % 20) as f64 * 0.07, y: 29.3 + (i / 20) as f64 * 0.07 };
            let a = prep.nearest(p).unwrap();
            let b = measure::nearest_point_on_line(&lines, p).unwrap();
            assert!((a.dist - b.dist).abs() < 1e-6, "{} vs {} at {p:?}", a.dist, b.dist);
            assert!(
                (a.location - b.location).abs() < 1e-3,
                "location {} vs {}",
                a.location,
                b.location
            );
        }
    }

    #[test]
    fn holes_are_outside() {
        let p = polygon!(
            exterior: [(x: 0.0, y: 0.0), (x: 2.0, y: 0.0), (x: 2.0, y: 2.0), (x: 0.0, y: 2.0), (x: 0.0, y: 0.0)],
            interiors: [[(x: 0.5, y: 0.5), (x: 1.5, y: 0.5), (x: 1.5, y: 1.5), (x: 0.5, y: 1.5), (x: 0.5, y: 0.5)]],
        );
        let prep = Prepared::new(Geometry::Polygon(p)).unwrap();
        assert!(prep.contains_point(coord! {x: 0.2, y: 1.0}, false));
        assert!(!prep.contains_point(coord! {x: 1.0, y: 1.0}, false));
        assert!(prep.contains_point(coord! {x: 0.5, y: 1.0}, false)); // on hole boundary
        assert!(!prep.contains_point(coord! {x: 0.5, y: 1.0}, true));
    }
}
