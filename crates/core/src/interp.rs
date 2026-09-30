//! Interpolation, contouring and triangulation.
//!
//! Every routine works on a lattice or a point set expressed in lon/lat but
//! does its geometry in a local transverse Mercator plane, so triangles,
//! Voronoi cells and inverse-distance weights use real metres instead of
//! degrees. Contours are derived from triangles rather than from a
//! marching-squares case table: a linear interpolant over a triangle has no
//! saddle ambiguity, so the ambiguous cases that make raster contouring
//! delicate simply do not arise.

use std::collections::HashMap;

use geo::{Coord, Geometry, LineString, MultiLineString, MultiPolygon, Polygon};

use crate::grids::{self, GridKind, GridOptions};
use crate::local::LocalFrame;
use crate::densify::Edges;
use crate::overlay::{self, OverlayOp, OverlayOptions};
use crate::{measure, Error, Result};

/// A regular lon/lat lattice of scalar samples.
///
/// Sample `(i, j)` sits at `(x0 + i·dx, y0 + j·dy)` and lives at
/// `values[j * nx + i]`. A `NaN` marks a hole in the data.
#[derive(Debug, Clone)]
pub struct Grid {
    pub x0: f64,
    pub y0: f64,
    pub dx: f64,
    pub dy: f64,
    pub nx: usize,
    pub ny: usize,
    pub values: Vec<f64>,
}

impl Grid {
    pub fn new(x0: f64, y0: f64, dx: f64, dy: f64, nx: usize, ny: usize, values: Vec<f64>) -> Result<Grid> {
        if nx < 2 || ny < 2 {
            return Err(Error::InvalidArgument("a grid needs at least 2×2 samples".into()));
        }
        if values.len() != nx * ny {
            return Err(Error::InvalidArgument(format!(
                "expected {} values for a {nx}×{ny} grid, got {}",
                nx * ny,
                values.len()
            )));
        }
        if !dx.is_finite() || !dy.is_finite() || dx == 0.0 || dy == 0.0 {
            return Err(Error::InvalidArgument("grid spacing must be non-zero".into()));
        }
        Ok(Grid { x0, y0, dx, dy, nx, ny, values })
    }

    /// Recover a lattice from scattered sample points (turf feeds contouring a
    /// point FeatureCollection; this is the equivalent of its `gridToMatrix`).
    ///
    /// Positions are snapped to the distinct coordinate values found in the
    /// input, so a grid written out with rounded coordinates still reads back.
    pub fn from_points(points: &[(Coord, f64)]) -> Result<Grid> {
        if points.len() < 4 {
            return Err(Error::InvalidArgument("need at least 4 points to infer a lattice".into()));
        }
        let axis = |vals: &mut Vec<f64>| -> Result<(f64, f64, usize)> {
            vals.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let span = vals[vals.len() - 1] - vals[0];
            // the smallest non-zero gap defines the step; treat anything under
            // a thousandth of it as the same coordinate
            let mut step = f64::MAX;
            for w in vals.windows(2) {
                let g = w[1] - w[0];
                if g > span * 1e-9 && g < step {
                    step = g;
                }
            }
            if !step.is_finite() || step <= 0.0 {
                return Err(Error::InvalidArgument("points do not form a lattice".into()));
            }
            let n = (span / step).round() as usize + 1;
            Ok((vals[0], step, n))
        };
        let (x0, dx, nx) = axis(&mut points.iter().map(|(c, _)| c.x).collect())?;
        let (y0, dy, ny) = axis(&mut points.iter().map(|(c, _)| c.y).collect())?;
        if nx < 2 || ny < 2 || nx * ny > 40_000_000 {
            return Err(Error::InvalidArgument(format!("inferred an implausible {nx}×{ny} lattice")));
        }
        let mut values = vec![f64::NAN; nx * ny];
        for (c, z) in points {
            let i = ((c.x - x0) / dx).round();
            let j = ((c.y - y0) / dy).round();
            if i < 0.0 || j < 0.0 || i as usize >= nx || j as usize >= ny {
                continue;
            }
            values[j as usize * nx + i as usize] = *z;
        }
        Grid::new(x0, y0, dx, dy, nx, ny, values)
    }

    #[inline]
    pub fn position(&self, i: usize, j: usize) -> Coord {
        Coord { x: self.x0 + i as f64 * self.dx, y: self.y0 + j as f64 * self.dy }
    }

    #[inline]
    pub fn value(&self, i: usize, j: usize) -> f64 {
        self.values[j * self.nx + i]
    }

    pub fn bbox(&self) -> [f64; 4] {
        let a = self.position(0, 0);
        let b = self.position(self.nx - 1, self.ny - 1);
        [a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y)]
    }

    /// The two triangles of cell `(i, j)`, as position/value triples. Cells
    /// with any missing corner are skipped, so holes in the data leave holes in
    /// the output rather than fabricating values.
    fn triangles(&self) -> impl Iterator<Item = [(Coord, f64); 3]> + '_ {
        (0..self.ny - 1).flat_map(move |j| {
            (0..self.nx - 1).flat_map(move |i| {
                let c = [
                    (self.position(i, j), self.value(i, j)),
                    (self.position(i + 1, j), self.value(i + 1, j)),
                    (self.position(i + 1, j + 1), self.value(i + 1, j + 1)),
                    (self.position(i, j + 1), self.value(i, j + 1)),
                ];
                let ok = c.iter().all(|(_, z)| z.is_finite());
                let tris = [[c[0], c[1], c[2]], [c[0], c[2], c[3]]];
                tris.into_iter().filter(move |_| ok)
            })
        })
    }
}

/// Where the plane `z = level` crosses a triangle with linear `z`.
fn tri_level_segment(t: &[(Coord, f64); 3], level: f64) -> Option<(Coord, Coord)> {
    let mut hits: Vec<Coord> = Vec::with_capacity(2);
    for k in 0..3 {
        let (p, zp) = t[k];
        let (q, zq) = t[(k + 1) % 3];
        // count a crossing on [p, q) so a vertex on the level is used once
        let (a, b) = (zp - level, zq - level);
        if (a < 0.0 && b >= 0.0) || (a >= 0.0 && b < 0.0) {
            let f = a / (a - b);
            hits.push(Coord { x: p.x + (q.x - p.x) * f, y: p.y + (q.y - p.y) * f });
        }
    }
    if hits.len() == 2 && (hits[0].x != hits[1].x || hits[0].y != hits[1].y) {
        Some((hits[0], hits[1]))
    } else {
        None
    }
}

/// The part of a triangle where `z >= level`, as a convex ring (3 or 4 corners).
fn tri_clip_above(t: &[(Coord, f64); 3], level: f64) -> Option<Vec<Coord>> {
    let mut out: Vec<Coord> = Vec::with_capacity(4);
    for k in 0..3 {
        let (p, zp) = t[k];
        let (q, zq) = t[(k + 1) % 3];
        let (a, b) = (zp - level, zq - level);
        if a >= 0.0 {
            out.push(p);
        }
        if (a < 0.0) != (b < 0.0) {
            let f = a / (a - b);
            out.push(Coord { x: p.x + (q.x - p.x) * f, y: p.y + (q.y - p.y) * f });
        }
    }
    if out.len() < 3 {
        return None;
    }
    out.dedup();
    if out.len() >= 3 {
        out.push(out[0]);
        Some(out)
    } else {
        None
    }
}

const STITCH_QUANT: f64 = 1e10;

#[inline]
fn key_of(c: Coord) -> (i64, i64) {
    ((c.x * STITCH_QUANT).round() as i64, (c.y * STITCH_QUANT).round() as i64)
}

/// Join loose segments end to end into the longest chains they support.
fn stitch(segments: Vec<(Coord, Coord)>) -> MultiLineString {
    let n = segments.len();
    let mut ends: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, (a, b)) in segments.iter().enumerate() {
        ends.entry(key_of(*a)).or_default().push(i);
        ends.entry(key_of(*b)).or_default().push(i);
    }
    let mut used = vec![false; n];
    let mut out: Vec<LineString> = Vec::new();
    for start in 0..n {
        if used[start] {
            continue;
        }
        used[start] = true;
        let mut chain = vec![segments[start].0, segments[start].1];
        // extend from both ends
        for front in [false, true] {
            loop {
                let tip = if front { chain[0] } else { *chain.last().unwrap() };
                let Some(cands) = ends.get(&key_of(tip)) else { break };
                let Some(&next) = cands.iter().find(|&&i| !used[i]) else { break };
                used[next] = true;
                let (a, b) = segments[next];
                let other = if key_of(a) == key_of(tip) { b } else { a };
                if front {
                    chain.insert(0, other);
                } else {
                    chain.push(other);
                }
                if key_of(other) == key_of(if front { *chain.last().unwrap() } else { chain[0] }) {
                    break; // closed ring
                }
            }
        }
        if chain.len() >= 2 {
            out.push(LineString(chain));
        }
    }
    MultiLineString(out)
}

/// Contour lines at each break (turf's `isolines`).
pub fn isolines(grid: &Grid, breaks: &[f64]) -> Vec<(f64, MultiLineString)> {
    let tris: Vec<[(Coord, f64); 3]> = grid.triangles().collect();
    breaks
        .iter()
        .map(|&level| {
            let segs: Vec<(Coord, Coord)> = tris.iter().filter_map(|t| tri_level_segment(t, level)).collect();
            (level, stitch(segs))
        })
        .collect()
}

/// Contour geometry is built from lattice triangles whose edges are straight in
/// lon/lat by construction, which is exactly what `Edges::Planar` means — the
/// interpolation defines the edges, so there is nothing to densify.
const OPTS: OverlayOptions = OverlayOptions { edges: Edges::Planar, tolerance: 0.001 };

/// Filled bands between consecutive breaks (turf's `isobands`).
///
/// Band `k` is `{ z >= breaks[k] } \ { z >= breaks[k+1] }`, each side built by
/// clipping triangles and unioning the pieces, so bands tile the domain
/// without slivers or overlaps.
pub fn isobands(grid: &Grid, breaks: &[f64]) -> Result<Vec<((f64, f64), MultiPolygon)>> {
    if breaks.len() < 2 {
        return Err(Error::InvalidArgument("isobands needs at least 2 breaks".into()));
    }
    let mut sorted = breaks.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let tris: Vec<[(Coord, f64); 3]> = grid.triangles().collect();

    let region = |level: f64| -> Result<MultiPolygon> {
        let mut parts: Vec<Geometry> = Vec::new();
        for t in &tris {
            // a wholly-above triangle needs no clipping
            if t.iter().all(|(_, z)| *z >= level) {
                let ring: Vec<Coord> = vec![t[0].0, t[1].0, t[2].0, t[0].0];
                parts.push(Geometry::Polygon(Polygon::new(LineString(ring), vec![])));
            } else if let Some(ring) = tri_clip_above(t, level) {
                parts.push(Geometry::Polygon(Polygon::new(LineString(ring), vec![])));
            }
        }
        if parts.is_empty() {
            return Ok(MultiPolygon(vec![]));
        }
        overlay::union_all(&parts, &OPTS)
    };

    let mut regions: Vec<MultiPolygon> = Vec::with_capacity(sorted.len());
    for &b in &sorted {
        regions.push(region(b)?);
    }
    let mut out = Vec::with_capacity(sorted.len() - 1);
    for k in 0..sorted.len() - 1 {
        let band = overlay::overlay(
            &Geometry::MultiPolygon(regions[k].clone()),
            &Geometry::MultiPolygon(regions[k + 1].clone()),
            OverlayOp::Difference,
            &OPTS,
        )?;
        out.push(((sorted[k], sorted[k + 1]), band));
    }
    Ok(out)
}

/// Inverse-distance weighting options.
#[derive(Debug, Clone, Copy)]
pub struct IdwOptions {
    /// Exponent on 1/distance. turf's default is 1.
    pub power: f64,
    /// Ignore samples further away than this (metres). `None` uses them all.
    pub search_radius_m: Option<f64>,
    /// Cell size and shape of the output grid.
    pub grid: GridOptions,
    pub kind: GridKind,
}

impl Default for IdwOptions {
    fn default() -> Self {
        IdwOptions { power: 1.0, search_radius_m: None, grid: GridOptions::default(), kind: GridKind::Square }
    }
}

/// Interpolate scattered samples onto a grid (turf's `interpolate`).
///
/// Distances are geodesic, so a search radius means the same thing at every
/// latitude. Returns each cell (or point) with its interpolated value; cells
/// with no sample inside the radius are omitted.
pub fn interpolate(
    points: &[(Coord, f64)],
    bbox: [f64; 4],
    opts: &IdwOptions,
    mask: Option<&Geometry>,
) -> Result<Vec<(Geometry, f64)>> {
    if points.is_empty() {
        return Err(Error::InvalidArgument("interpolate needs at least one sample".into()));
    }
    if !opts.power.is_finite() || opts.power <= 0.0 {
        return Err(Error::InvalidArgument("IDW power must be positive".into()));
    }
    let cells = grids::grid(bbox, opts.kind, &opts.grid, mask)?;
    let mut out = Vec::with_capacity(cells.len());
    for cell in cells {
        let centre = match &cell {
            Geometry::Point(p) => p.0,
            other => crate::ops::vertex_centroid(std::slice::from_ref(other))
                .ok_or_else(|| Error::InvalidGeometry("empty grid cell".into()))?,
        };
        let mut num = 0.0;
        let mut den = 0.0;
        let mut exact: Option<f64> = None;
        for (c, z) in points {
            let d = measure::distance(centre, *c);
            if let Some(r) = opts.search_radius_m {
                if d > r {
                    continue;
                }
            }
            if d < 1e-9 {
                exact = Some(*z);
                break;
            }
            let w = d.powf(-opts.power);
            num += w * z;
            den += w;
        }
        if let Some(z) = exact {
            out.push((cell, z));
        } else if den > 0.0 {
            out.push((cell, num / den));
        }
    }
    Ok(out)
}

/// Interpolate a value inside a triangle whose corners carry `z` (turf's
/// `planepoint`), using barycentric coordinates in the local plane.
pub fn planepoint(p: Coord, triangle: &Polygon, z: [f64; 3]) -> Result<f64> {
    let ring = &triangle.exterior().0;
    if ring.len() < 4 {
        return Err(Error::InvalidGeometry("planepoint needs a triangle".into()));
    }
    let frame = LocalFrame::for_geometries([&Geometry::Polygon(triangle.clone())])?;
    let pr = frame.project(p.x, p.y);
    let a = frame.project(ring[0].x, ring[0].y);
    let b = frame.project(ring[1].x, ring[1].y);
    let c = frame.project(ring[2].x, ring[2].y);
    let det = (b.0 - a.0) * (c.1 - a.1) - (c.0 - a.0) * (b.1 - a.1);
    if det.abs() < 1e-12 {
        return Err(Error::InvalidGeometry("degenerate triangle".into()));
    }
    let l1 = ((pr.0 - a.0) * (c.1 - a.1) - (c.0 - a.0) * (pr.1 - a.1)) / det;
    let l2 = ((b.0 - a.0) * (pr.1 - a.1) - (pr.0 - a.0) * (b.1 - a.1)) / det;
    let l0 = 1.0 - l1 - l2;
    Ok(l0 * z[0] + l1 * z[1] + l2 * z[2])
}

/// Delaunay triangulation of a point set (turf's `tin`).
///
/// The triangulation runs in the local metric plane, so the "well shaped
/// triangles" property holds on the ground rather than in degree space.
/// Returns each triangle with the `z` of its three corners in ring order.
pub fn tin(points: &[(Coord, f64)]) -> Result<Vec<(Polygon, [f64; 3])>> {
    use spade::{DelaunayTriangulation, Point2, Triangulation};
    if points.len() < 3 {
        return Err(Error::InvalidArgument("tin needs at least 3 points".into()));
    }
    let frame = LocalFrame::new(
        points.iter().map(|(c, _)| c.x).sum::<f64>() / points.len() as f64,
        points.iter().map(|(c, _)| c.y).sum::<f64>() / points.len() as f64,
    );
    let mut tri: DelaunayTriangulation<Point2<f64>> = DelaunayTriangulation::new();
    let mut zs: Vec<f64> = Vec::with_capacity(points.len());
    for (c, z) in points {
        let (x, y) = frame.project(c.x, c.y);
        tri.insert(Point2::new(x, y))
            .map_err(|e| Error::InvalidGeometry(format!("triangulation failed: {e:?}")))?;
        zs.push(*z);
    }
    // A lattice whose edge is collinear in lon/lat is very slightly convex in
    // the metric plane, and Delaunay answers that with a fan of slivers. In the
    // plane they are ordinary triangles (tens of m² for a kilometre lattice),
    // but back in lon/lat their three corners are exactly collinear, so as
    // GeoJSON they are zero-area polygons that no interpolation can use. Judge
    // degeneracy in the output space, against the extent of the input.
    let (mut lx0, mut ly0, mut lx1, mut ly1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for (c, _) in points {
        lx0 = lx0.min(c.x);
        ly0 = ly0.min(c.y);
        lx1 = lx1.max(c.x);
        ly1 = ly1.max(c.y);
    }
    let floor = 1e-9 * (lx1 - lx0).max(f64::MIN_POSITIVE) * (ly1 - ly0).max(f64::MIN_POSITIVE);

    let mut out = Vec::new();
    for face in tri.inner_faces() {
        let vs = face.vertices();
        let mut ring: Vec<Coord> = Vec::with_capacity(4);
        let mut z = [0.0f64; 3];
        for (k, v) in vs.iter().enumerate() {
            let p = v.position();
            let (lon, lat) = frame.unproject(p.x, p.y);
            ring.push(Coord { x: lon, y: lat });
            z[k] = zs.get(v.index()).copied().unwrap_or(f64::NAN);
        }
        let shoelace = ((ring[1].x - ring[0].x) * (ring[2].y - ring[0].y)
            - (ring[2].x - ring[0].x) * (ring[1].y - ring[0].y))
            .abs()
            / 2.0;
        if shoelace < floor {
            continue;
        }
        ring.push(ring[0]);
        out.push((Polygon::new(LineString(ring), vec![]), z));
    }
    Ok(out)
}

/// Voronoi cells clipped to `bbox` (turf's `voronoi`).
///
/// Each cell is built by clipping the bbox rectangle with the perpendicular
/// bisectors against the site's Delaunay neighbours — exact, and it needs no
/// special case for the unbounded cells at the hull. The bisectors are straight
/// in the metric plane, so the emitted lon/lat rings are chords of the true
/// boundaries: neighbouring cells meet to within ~1e-4 of their area over a
/// 30 km domain rather than exactly.
pub fn voronoi(points: &[Coord], bbox: [f64; 4]) -> Result<Vec<Polygon>> {
    use spade::{DelaunayTriangulation, Point2, Triangulation};
    if points.is_empty() {
        return Err(Error::InvalidArgument("voronoi needs at least one point".into()));
    }
    let frame = LocalFrame::new(0.5 * (bbox[0] + bbox[2]), 0.5 * (bbox[1] + bbox[3]));
    let corners: Vec<(f64, f64)> = [
        (bbox[0], bbox[1]),
        (bbox[2], bbox[1]),
        (bbox[2], bbox[3]),
        (bbox[0], bbox[3]),
    ]
    .iter()
    .map(|(x, y)| frame.project(*x, *y))
    .collect();
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for (x, y) in &corners {
        x0 = x0.min(*x);
        y0 = y0.min(*y);
        x1 = x1.max(*x);
        y1 = y1.max(*y);
    }
    let rect = vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)];

    let projected: Vec<(f64, f64)> = points.iter().map(|c| frame.project(c.x, c.y)).collect();
    let mut tri: DelaunayTriangulation<Point2<f64>> = DelaunayTriangulation::new();
    let mut index_of: Vec<Option<usize>> = Vec::with_capacity(points.len());
    let mut site_of: HashMap<usize, usize> = HashMap::new();
    for (k, (x, y)) in projected.iter().enumerate() {
        match tri.insert(Point2::new(*x, *y)) {
            Ok(h) => {
                let idx = h.index();
                index_of.push(Some(idx));
                site_of.insert(idx, k);
            }
            Err(_) => index_of.push(None),
        }
    }

    let mut out = Vec::with_capacity(points.len());
    for (k, site) in projected.iter().enumerate() {
        let mut cell = rect.clone();
        if let Some(idx) = index_of[k] {
            let handle = tri.vertex(spade::handles::FixedVertexHandle::from_index(idx));
            for edge in handle.out_edges() {
                let n = edge.to().position();
                cell = clip_halfplane(&cell, *site, (n.x, n.y));
                if cell.len() < 3 {
                    break;
                }
            }
        } else {
            // a duplicate site: its cell is empty
            cell.clear();
        }
        if cell.len() < 3 {
            out.push(Polygon::new(LineString(vec![]), vec![]));
            continue;
        }
        let mut ring: Vec<Coord> = cell
            .iter()
            .map(|(x, y)| {
                let (lon, lat) = frame.unproject(*x, *y);
                Coord { x: lon, y: lat }
            })
            .collect();
        ring.push(ring[0]);
        let poly = Polygon::new(LineString(ring), vec![]);
        // The plane rectangle is the extent of the projected bbox corners, so
        // it bulges a little past the lon/lat bbox (the parallels curve in a
        // transverse Mercator frame). Trim anything that escaped, and skip the
        // boolean entirely for the interior cells, which is nearly all of them.
        let cb = crate::ops::bbox(std::slice::from_ref(&Geometry::Polygon(poly.clone())));
        let inside = cb.is_some_and(|b| b[0] >= bbox[0] && b[1] >= bbox[1] && b[2] <= bbox[2] && b[3] <= bbox[3]);
        if inside {
            out.push(poly);
        } else {
            let clipped = overlay::overlay(
                &Geometry::Polygon(poly),
                &Geometry::Polygon(crate::ops::bbox_polygon(bbox)),
                OverlayOp::Intersection,
                &OPTS,
            )?;
            // a cell stays one piece under a convex clip
            out.push(clipped.0.into_iter().next().unwrap_or_else(|| Polygon::new(LineString(vec![]), vec![])));
        }
    }
    Ok(out)
}

/// Keep the part of a convex ring closer to `site` than to `other`.
fn clip_halfplane(ring: &[(f64, f64)], site: (f64, f64), other: (f64, f64)) -> Vec<(f64, f64)> {
    // points p with (p - m) · d <= 0, where m is the midpoint and d the offset
    let d = (other.0 - site.0, other.1 - site.1);
    let m = (0.5 * (site.0 + other.0), 0.5 * (site.1 + other.1));
    let side = |p: (f64, f64)| (p.0 - m.0) * d.0 + (p.1 - m.1) * d.1;
    let n = ring.len();
    let mut out: Vec<(f64, f64)> = Vec::with_capacity(n + 2);
    for i in 0..n {
        let (p, q) = (ring[i], ring[(i + 1) % n]);
        let (sp, sq) = (side(p), side(q));
        if sp <= 0.0 {
            out.push(p);
        }
        if (sp <= 0.0) != (sq <= 0.0) {
            let t = sp / (sp - sq);
            out.push((p.0 + (q.0 - p.0) * t, p.1 + (q.1 - p.1) * t));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ops, predicates};
    use geo::{coord, polygon};

    /// Deterministic pseudo-random points, so failures are reproducible.
    fn lcg(seed: u64) -> impl FnMut() -> f64 {
        let mut s = seed;
        move || {
            s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            ((s >> 11) as f64) / ((1u64 << 53) as f64)
        }
    }

    fn ramp_grid() -> Grid {
        // z = x, so the contour at level v is the meridian x = v
        let (nx, ny) = (21usize, 11usize);
        let (x0, y0, dx, dy) = (120.0, 30.0, 0.01, 0.01);
        let mut v = Vec::with_capacity(nx * ny);
        for j in 0..ny {
            for i in 0..nx {
                let _ = j;
                v.push(x0 + i as f64 * dx);
            }
        }
        Grid::new(x0, y0, dx, dy, nx, ny, v).unwrap()
    }

    #[test]
    fn lattice_is_recovered_from_points() {
        let g = ramp_grid();
        let pts: Vec<(Coord, f64)> = (0..g.ny)
            .flat_map(|j| (0..g.nx).map(move |i| (i, j)))
            .map(|(i, j)| (g.position(i, j), g.value(i, j)))
            .collect();
        let back = Grid::from_points(&pts).unwrap();
        assert_eq!((back.nx, back.ny), (g.nx, g.ny));
        assert!((back.dx - g.dx).abs() < 1e-12 && (back.dy - g.dy).abs() < 1e-12);
        for k in 0..g.values.len() {
            assert!((back.values[k] - g.values[k]).abs() < 1e-12);
        }
    }

    #[test]
    fn isolines_follow_the_level() {
        let g = ramp_grid();
        for level in [120.03, 120.105, 120.17] {
            let out = isolines(&g, &[level]);
            let ml = &out[0].1;
            assert!(!ml.0.is_empty(), "no contour at {level}");
            let mut n = 0;
            for ls in &ml.0 {
                for c in &ls.0 {
                    assert!((c.x - level).abs() < 1e-9, "{c:?} not on x = {level}");
                    n += 1;
                }
            }
            // the contour spans the full height of the lattice
            let total: f64 = ml.0.iter().map(measure::line_length).sum();
            let height = measure::distance(g.position(0, 0), g.position(0, g.ny - 1));
            assert!((total - height).abs() < 1.0, "{total} vs {height}");
            assert!(n >= g.ny, "{n} vertices");
        }
        // a level outside the data has no contour
        assert!(isolines(&g, &[119.0])[0].1.0.is_empty());
    }

    #[test]
    fn isolines_close_a_ring_around_a_peak() {
        // a cone: z falls off with distance from the centre
        let (nx, ny) = (41usize, 41usize);
        let (x0, y0, d) = (119.8, 29.8, 0.01);
        let centre = coord! {x: 120.0, y: 30.0};
        let mut v = Vec::with_capacity(nx * ny);
        for j in 0..ny {
            for i in 0..nx {
                let p = Coord { x: x0 + i as f64 * d, y: y0 + j as f64 * d };
                v.push(-measure::distance(p, centre));
            }
        }
        let g = Grid::new(x0, y0, d, d, nx, ny, v).unwrap();
        let out = isolines(&g, &[-10_000.0]);
        let ml = &out[0].1;
        assert_eq!(ml.0.len(), 1, "one closed ring expected");
        let ring = &ml.0[0];
        assert_eq!(ring.0.first(), ring.0.last(), "the ring must close");
        for c in &ring.0 {
            let r = measure::distance(*c, centre);
            // the lattice resolves ~1 km cells, so allow a cell of wobble
            assert!((r - 10_000.0).abs() < 60.0, "{r}");
        }
    }

    #[test]
    fn isobands_tile_the_domain_without_overlap() {
        let g = ramp_grid();
        let breaks = [120.0, 120.05, 120.1, 120.2];
        let bands = isobands(&g, &breaks).unwrap();
        assert_eq!(bands.len(), 3);
        let domain = ops::bbox_polygon(g.bbox());
        let full = measure::area_with(&Geometry::Polygon(domain), Edges::Planar);
        let mut sum = 0.0;
        for ((lo, hi), mp) in &bands {
            let a = measure::area_with(&Geometry::MultiPolygon(mp.clone()), Edges::Planar);
            assert!(a > 0.0, "band [{lo}, {hi}) is empty");
            // the band's width in x is (hi - lo) out of the 0.2° domain
            let share = (hi - lo) / 0.2;
            assert!((a / full - share).abs() < 1e-3, "band [{lo}, {hi}): {} vs {share}", a / full);
            sum += a;
        }
        assert!((sum / full - 1.0).abs() < 1e-3, "bands cover {}", sum / full);

        // consecutive bands must not overlap
        for k in 0..bands.len() - 1 {
            let i = overlay::overlay(
                &Geometry::MultiPolygon(bands[k].1.clone()),
                &Geometry::MultiPolygon(bands[k + 1].1.clone()),
                OverlayOp::Intersection,
                &OPTS,
            )
            .unwrap();
            let a = measure::area_with(&Geometry::MultiPolygon(i), Edges::Planar);
            assert!(a < 1.0, "bands {k} and {} overlap by {a} m²", k + 1);
        }
    }

    #[test]
    fn idw_reproduces_samples_and_averages() {
        let a = coord! {x: 120.0, y: 30.0};
        let b = coord! {x: 120.2, y: 30.0};
        let pts = vec![(a, 0.0), (b, 100.0)];
        let opts = IdwOptions {
            power: 2.0,
            grid: GridOptions { width_m: 2_000.0, ..Default::default() },
            kind: GridKind::Point,
            ..Default::default()
        };
        let out = interpolate(&pts, [120.0, 29.98, 120.2, 30.02], &opts, None).unwrap();
        assert!(out.len() > 20, "{} cells", out.len());
        for (g, z) in &out {
            let Geometry::Point(p) = g else { panic!() };
            assert!(*z >= -1e-9 && *z <= 100.0 + 1e-9, "{z} outside the sample range");
            // closer to a ⇒ nearer a's value
            let (da, db) = (measure::distance(p.0, a), measure::distance(p.0, b));
            if da < db * 0.5 {
                assert!(*z < 30.0, "{z} at {da} m from a / {db} m from b");
            }
            if db < da * 0.5 {
                assert!(*z > 70.0, "{z}");
            }
        }
        // a lone sample makes a flat surface
        let flat = interpolate(&[(a, 42.0)], [120.0, 29.99, 120.05, 30.01], &opts, None).unwrap();
        assert!(flat.iter().all(|(_, z)| (z - 42.0).abs() < 1e-9));
        // and the search radius drops far cells entirely
        let near = interpolate(
            &[(a, 42.0)],
            [120.0, 29.99, 120.2, 30.01],
            &IdwOptions { search_radius_m: Some(5_000.0), ..opts },
            None,
        )
        .unwrap();
        assert!(near.len() < flat.len() + 200);
        for (g, _) in &near {
            let Geometry::Point(p) = g else { panic!() };
            assert!(measure::distance(p.0, a) <= 5_000.0);
        }
    }

    #[test]
    fn planepoint_is_linear() {
        let tri = polygon![(x: 120.0, y: 30.0), (x: 120.1, y: 30.0), (x: 120.0, y: 30.1)];
        let z = [0.0, 10.0, 20.0];
        for (k, c) in tri.exterior().0.iter().take(3).enumerate() {
            let got = planepoint(*c, &tri, z).unwrap();
            assert!((got - z[k]).abs() < 1e-6, "corner {k}: {got} vs {}", z[k]);
        }
        let centroid = ops::vertex_centroid(&[Geometry::Polygon(tri.clone())]).unwrap();
        let mid = planepoint(centroid, &tri, z).unwrap();
        assert!((mid - 10.0).abs() < 0.2, "{mid}");
    }

    #[test]
    fn tin_is_a_delaunay_cover_of_the_hull() {
        let mut rnd = lcg(7);
        let pts: Vec<(Coord, f64)> = (0..60)
            .map(|_| (coord! {x: 120.0 + rnd() * 0.2, y: 30.0 + rnd() * 0.2}, rnd() * 100.0))
            .collect();
        let tris = tin(&pts).unwrap();
        assert!(!tris.is_empty());

        // the triangles tile the convex hull exactly
        let hull = ops::convex_hull(&pts.iter().map(|(c, _)| Geometry::Point(geo::Point(*c))).collect::<Vec<_>>())
            .unwrap();
        let hull_area = measure::area_with(&Geometry::Polygon(hull.clone()), Edges::Planar);
        let sum: f64 = tris
            .iter()
            .map(|(p, _)| measure::area_with(&Geometry::Polygon(p.clone()), Edges::Planar))
            .sum();
        assert!((sum / hull_area - 1.0).abs() < 1e-6, "{sum} vs {hull_area}");

        // Euler's formula for a triangulation of n points with h on the hull
        let h = hull.exterior().0.len() - 1;
        assert_eq!(tris.len(), 2 * pts.len() - 2 - h, "hull has {h} vertices");

        // z travels with the corners
        for (p, z) in &tris {
            for (k, c) in p.exterior().0.iter().take(3).enumerate() {
                let want = pts
                    .iter()
                    .min_by(|a, b| {
                        measure::distance(a.0, *c)
                            .partial_cmp(&measure::distance(b.0, *c))
                            .unwrap()
                    })
                    .unwrap()
                    .1;
                assert!((z[k] - want).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn tin_of_a_lattice_has_no_sliver_triangles() {
        // A lon/lat lattice is collinear along its edges but very slightly
        // convex in the metric plane, which is where the slivers come from.
        let g = ramp_grid();
        let pts: Vec<(Coord, f64)> = (0..g.ny)
            .flat_map(|j| (0..g.nx).map(move |i| (i, j)))
            .map(|(i, j)| (g.position(i, j), g.value(i, j)))
            .collect();
        let tris = tin(&pts).unwrap();

        // Euler: a triangulation of n points with h on the hull has 2n-2-h
        // triangles. Every boundary point of the lattice is on the hull.
        let boundary = 2 * (g.nx + g.ny) - 4;
        assert_eq!(tris.len(), 2 * pts.len() - 2 - boundary, "{} triangles", tris.len());

        let areas: Vec<f64> = tris
            .iter()
            .map(|(p, _)| measure::area_with(&Geometry::Polygon(p.clone()), Edges::Planar))
            .collect();
        let smallest = areas.iter().cloned().fold(f64::MAX, f64::min);
        let cell = measure::distance(g.position(0, 0), g.position(1, 0))
            * measure::distance(g.position(0, 0), g.position(0, 1));
        assert!(smallest > 0.4 * cell, "sliver of {smallest} m² against a {cell} m² cell");

        // and the triangles still tile the hull
        let hull = ops::convex_hull(&pts.iter().map(|(c, _)| Geometry::Point(geo::Point(*c))).collect::<Vec<_>>()).unwrap();
        let hull_area = measure::area_with(&Geometry::Polygon(hull), Edges::Planar);
        let sum: f64 = areas.iter().sum();
        assert!((sum / hull_area - 1.0).abs() < 1e-6, "{sum} vs {hull_area}");
    }

    #[test]
    fn voronoi_cells_are_nearest_site_regions() {
        let mut rnd = lcg(11);
        let sites: Vec<Coord> = (0..25)
            .map(|_| coord! {x: 120.0 + rnd() * 0.2, y: 30.0 + rnd() * 0.2})
            .collect();
        let bbox = [119.95, 29.95, 120.25, 30.25];
        let cells = voronoi(&sites, bbox).unwrap();
        assert_eq!(cells.len(), sites.len());

        let domain = measure::area_with(&Geometry::Polygon(ops::bbox_polygon(bbox)), Edges::Planar);
        let mut sum = 0.0;
        for (k, cell) in cells.iter().enumerate() {
            let g = Geometry::Polygon(cell.clone());
            sum += measure::area_with(&g, Edges::Planar);
            // a site lies in its own cell
            assert!(predicates::point_in_polygon(sites[k], &g, false), "site {k} outside its cell");
            // and every corner of the cell is no closer to another site
            for c in cell.exterior().0.iter() {
                let own = measure::distance(*c, sites[k]);
                for (m, s) in sites.iter().enumerate() {
                    if m != k {
                        assert!(measure::distance(*c, *s) > own - 1.0, "cell {k} corner nearer site {m}");
                    }
                }
            }
        }
        // The bisectors are straight in the metric plane; writing them out as
        // lon/lat vertices renders them as chords, which leaves a hairline
        // (~1e-4 of the area over a 30 km domain) along each shared edge.
        assert!((sum / domain - 1.0).abs() < 1e-3, "cells cover {}", sum / domain);
    }
}
