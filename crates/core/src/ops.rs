//! Geometry processing: simplification, hulls, centres, affine-style
//! transforms and line operations.
//!
//! Anything that needs a metric (simplification tolerances, hulls, centres of
//! mass) runs in the local transverse Mercator plane; anything that needs a
//! distance or a bearing (rotate, translate, scale, slicing) is solved on the
//! ellipsoid with geodesics.

use geo::{
    Centroid, ConcaveHull, ConvexHull, Coord, Geometry, GeometryCollection, InteriorPoint, Line, LineString,
    MultiLineString, MultiPoint, MultiPolygon, Point, Polygon, Rect, Simplify, SimplifyVwPreserve,
};

use crate::densify::{self, Edges};
use crate::geodesic;
use crate::local::LocalFrame;
use crate::measure;
use crate::{Error, Result};

/// Bounding box `[min_x, min_y, max_x, max_y]`.
pub fn bbox(geoms: &[Geometry]) -> Option<[f64; 4]> {
    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    let mut any = false;
    for g in geoms {
        for c in coords_of(g) {
            any = true;
            b[0] = b[0].min(c.x);
            b[1] = b[1].min(c.y);
            b[2] = b[2].max(c.x);
            b[3] = b[3].max(c.y);
        }
    }
    any.then_some(b)
}

pub fn bbox_polygon(b: [f64; 4]) -> Polygon {
    Polygon::new(
        LineString(vec![
            Coord { x: b[0], y: b[1] },
            Coord { x: b[2], y: b[1] },
            Coord { x: b[2], y: b[3] },
            Coord { x: b[0], y: b[3] },
            Coord { x: b[0], y: b[1] },
        ]),
        vec![],
    )
}

/// Every coordinate of a geometry, in order.
pub fn coords_of(g: &Geometry) -> Vec<Coord> {
    let mut out = Vec::new();
    push_coords(g, &mut out, false);
    out
}

/// Every position, with each ring's repeated closing position dropped — the
/// `excludeWrapCoord` behaviour turf's `centroid` relies on.
pub fn coords_of_no_wrap(g: &Geometry) -> Vec<Coord> {
    let mut out = Vec::new();
    push_coords(g, &mut out, true);
    out
}

fn push_ring(r: &LineString, out: &mut Vec<Coord>, exclude_wrap: bool) {
    let cs = &r.0;
    let n = if exclude_wrap && cs.len() > 1 && cs.first() == cs.last() { cs.len() - 1 } else { cs.len() };
    out.extend(cs[..n].iter().copied());
}

fn push_coords(g: &Geometry, out: &mut Vec<Coord>, exclude_wrap: bool) {
    match g {
        Geometry::Point(p) => out.push(p.0),
        Geometry::MultiPoint(mp) => out.extend(mp.0.iter().map(|p| p.0)),
        Geometry::Line(l) => out.extend([l.start, l.end]),
        Geometry::LineString(ls) => out.extend(ls.0.iter().copied()),
        Geometry::MultiLineString(m) => out.extend(m.0.iter().flat_map(|l| l.0.iter().copied())),
        Geometry::Polygon(p) => {
            push_ring(p.exterior(), out, exclude_wrap);
            for r in p.interiors() {
                push_ring(r, out, exclude_wrap);
            }
        }
        Geometry::MultiPolygon(mp) => mp
            .0
            .iter()
            .for_each(|p| push_coords(&Geometry::Polygon(p.clone()), out, exclude_wrap)),
        Geometry::Rect(r) => push_coords(&Geometry::Polygon(r.to_polygon()), out, exclude_wrap),
        Geometry::Triangle(t) => push_coords(&Geometry::Polygon(t.to_polygon()), out, exclude_wrap),
        Geometry::GeometryCollection(gc) => gc.0.iter().for_each(|g| push_coords(g, out, exclude_wrap)),
    }
}

// ------------------------------------------------------------- simplify

/// Douglas–Peucker (or topology-preserving Visvalingam–Whyatt) simplification
/// with the tolerance given in metres.
pub fn simplify(g: &Geometry, tolerance_m: f64, preserve_topology: bool) -> Result<Geometry> {
    if tolerance_m <= 0.0 {
        return Ok(g.clone());
    }
    let frame = LocalFrame::for_geometries([g])?;
    let projected = frame.project_geom(g);
    let simplified = if preserve_topology {
        simplify_vw(&projected, tolerance_m * tolerance_m)
    } else {
        simplify_dp(&projected, tolerance_m)
    };
    Ok(frame.unproject_geom(&simplified))
}

fn simplify_dp(g: &Geometry, eps: f64) -> Geometry {
    match g {
        Geometry::LineString(ls) => Geometry::LineString(ls.simplify(eps)),
        Geometry::MultiLineString(m) => Geometry::MultiLineString(m.simplify(eps)),
        Geometry::Polygon(p) => Geometry::Polygon(p.simplify(eps)),
        Geometry::MultiPolygon(mp) => Geometry::MultiPolygon(mp.simplify(eps)),
        Geometry::GeometryCollection(gc) => {
            Geometry::GeometryCollection(GeometryCollection(gc.0.iter().map(|g| simplify_dp(g, eps)).collect()))
        }
        other => other.clone(),
    }
}

fn simplify_vw(g: &Geometry, eps_area: f64) -> Geometry {
    match g {
        Geometry::LineString(ls) => Geometry::LineString(ls.simplify_vw_preserve(eps_area)),
        Geometry::MultiLineString(m) => Geometry::MultiLineString(m.simplify_vw_preserve(eps_area)),
        Geometry::Polygon(p) => Geometry::Polygon(p.simplify_vw_preserve(eps_area)),
        Geometry::MultiPolygon(mp) => Geometry::MultiPolygon(mp.simplify_vw_preserve(eps_area)),
        Geometry::GeometryCollection(gc) => Geometry::GeometryCollection(GeometryCollection(
            gc.0.iter().map(|g| simplify_vw(g, eps_area)).collect(),
        )),
        other => other.clone(),
    }
}

// ----------------------------------------------------------------- hulls

pub fn convex_hull(geoms: &[Geometry]) -> Result<Polygon> {
    let frame = LocalFrame::for_geometries(geoms.iter())?;
    let pts: Vec<Coord> = geoms
        .iter()
        .flat_map(coords_of)
        .map(|c| {
            let (x, y) = frame.project(c.x, c.y);
            Coord { x, y }
        })
        .collect();
    if pts.len() < 3 {
        return Err(Error::InvalidGeometry("convex hull needs at least 3 points".into()));
    }
    let hull = MultiPoint(pts.into_iter().map(Point).collect()).convex_hull();
    Ok(frame.unproject_geom(&hull))
}

/// Concave hull. `max_edge_m` controls how tightly the hull wraps the points
/// (smaller = tighter); it is the concavity parameter in metres.
pub fn concave_hull(geoms: &[Geometry], max_edge_m: f64) -> Result<Polygon> {
    let frame = LocalFrame::for_geometries(geoms.iter())?;
    let pts: Vec<Point> = geoms
        .iter()
        .flat_map(coords_of)
        .map(|c| {
            let (x, y) = frame.project(c.x, c.y);
            Point::new(x, y)
        })
        .collect();
    if pts.len() < 3 {
        return Err(Error::InvalidGeometry("concave hull needs at least 3 points".into()));
    }
    let hull = MultiPoint(pts).concave_hull_with_options(geo::algorithm::concave_hull::ConcaveHullOptions {
        concavity: max_edge_m.max(1e-6),
        length_threshold: 0.0,
    });
    Ok(frame.unproject_geom(&hull))
}

// --------------------------------------------------------------- centres

/// Mean of all vertices (turf's `centroid`).
pub fn vertex_centroid(geoms: &[Geometry]) -> Option<Coord> {
    let mut n = 0.0;
    let mut sum = Coord { x: 0.0, y: 0.0 };
    for g in geoms {
        // turf's `centroid` skips the repeated closing position of each ring;
        // counting it would drag the mean toward the first corner
        for c in coords_of_no_wrap(g) {
            sum.x += c.x;
            sum.y += c.y;
            n += 1.0;
        }
    }
    (n > 0.0).then(|| Coord {
        x: sum.x / n,
        y: sum.y / n,
    })
}

/// Area-weighted centre of mass, computed in the local plane.
pub fn center_of_mass(geoms: &[Geometry]) -> Result<Coord> {
    let frame = LocalFrame::for_geometries(geoms.iter())?;
    let projected: Vec<Geometry> = geoms.iter().map(|g| frame.project_geom(g)).collect();
    let c = GeometryCollection(projected)
        .centroid()
        .ok_or_else(|| Error::InvalidGeometry("no centroid".into()))?;
    let (x, y) = frame.unproject(c.x(), c.y());
    Ok(Coord { x, y })
}

/// A point guaranteed to lie on the geometry (inside, for polygons).
pub fn point_on_feature(g: &Geometry) -> Result<Coord> {
    let frame = LocalFrame::for_geometries([g])?;
    let p = frame
        .project_geom(g)
        .interior_point()
        .ok_or_else(|| Error::InvalidGeometry("no interior point".into()))?;
    let (x, y) = frame.unproject(p.x(), p.y());
    Ok(Coord { x, y })
}

// ------------------------------------------------------------ transforms

fn map_coords(g: &Geometry, f: impl Fn(Coord) -> Coord + Copy) -> Geometry {
    use geo::MapCoords;
    g.map_coords(f)
}

/// Rotate around `pivot` by `angle` degrees clockwise (bearings increase clockwise).
pub fn transform_rotate(g: &Geometry, angle_deg: f64, pivot: Coord) -> Geometry {
    map_coords(g, |c| {
        let inv = geodesic::inverse(pivot.x, pivot.y, c.x, c.y);
        if inv.s12 == 0.0 {
            return c;
        }
        let (x, y) = geodesic::destination(pivot.x, pivot.y, inv.azi1 + angle_deg, inv.s12);
        Coord { x, y }
    })
}

/// Move every vertex `dist_m` metres along `bearing_deg`.
pub fn transform_translate(g: &Geometry, dist_m: f64, bearing_deg: f64) -> Geometry {
    map_coords(g, |c| {
        let (x, y) = geodesic::destination(c.x, c.y, bearing_deg, dist_m);
        Coord { x, y }
    })
}

/// Scale distances from `origin` by `factor` (geodesic radial scaling).
pub fn transform_scale(g: &Geometry, factor: f64, origin: Coord) -> Geometry {
    map_coords(g, |c| {
        let inv = geodesic::inverse(origin.x, origin.y, c.x, c.y);
        if inv.s12 == 0.0 {
            return c;
        }
        let (x, y) = geodesic::destination(origin.x, origin.y, inv.azi1, inv.s12 * factor);
        Coord { x, y }
    })
}

// ----------------------------------------------------------- line tools

/// Sub-line between two distances along the line (metres).
pub fn line_slice_along(ls: &LineString, start_m: f64, stop_m: f64, edges: Edges) -> Result<LineString> {
    let ls = match edges {
        Edges::Geodesic => ls.clone(),
        Edges::Planar => LineString(densify::planar_path(&ls.0, densify::DEFAULT_TOL)),
    };
    let (start_m, stop_m) = (start_m.min(stop_m).max(0.0), start_m.max(stop_m));
    let mut out: Vec<Coord> = Vec::new();
    let mut travelled = 0.0;
    for w in ls.0.windows(2) {
        let inv = geodesic::inverse(w[0].x, w[0].y, w[1].x, w[1].y);
        let seg_end = travelled + inv.s12;
        if seg_end < start_m {
            travelled = seg_end;
            continue;
        }
        if travelled > stop_m {
            break;
        }
        if out.is_empty() {
            let s = (start_m - travelled).max(0.0);
            let (x, y) = geodesic::destination(w[0].x, w[0].y, inv.azi1, s);
            out.push(Coord { x, y });
        }
        if seg_end <= stop_m {
            out.push(w[1]);
        } else {
            let s = stop_m - travelled;
            let (x, y) = geodesic::destination(w[0].x, w[0].y, inv.azi1, s);
            out.push(Coord { x, y });
            break;
        }
        travelled = seg_end;
    }
    if out.len() < 2 {
        return Err(Error::InvalidArgument("slice is empty".into()));
    }
    Ok(LineString(out))
}

/// Sub-line between the points on the line closest to `start` and `stop`.
pub fn line_slice(ls: &LineString, start: Coord, stop: Coord, edges: Edges) -> Result<LineString> {
    let dense = match edges {
        Edges::Geodesic => ls.clone(),
        Edges::Planar => LineString(densify::planar_path(&ls.0, densify::DEFAULT_TOL)),
    };
    let lines = MultiLineString(vec![dense.clone()]);
    let a = measure::nearest_point_on_line(&lines, start).ok_or_else(|| Error::InvalidGeometry("empty line".into()))?;
    let b = measure::nearest_point_on_line(&lines, stop).ok_or_else(|| Error::InvalidGeometry("empty line".into()))?;
    line_slice_along(&dense, a.location, b.location, Edges::Geodesic)
}

/// Split a line into chunks of at most `length_m`.
pub fn line_chunk(ls: &LineString, length_m: f64, edges: Edges) -> Result<Vec<LineString>> {
    if length_m <= 0.0 {
        return Err(Error::InvalidArgument("chunk length must be positive".into()));
    }
    let total = match edges {
        Edges::Geodesic => measure::line_length(ls),
        Edges::Planar => measure::planar_line_length(ls),
    };
    let n = (total / length_m).ceil().max(1.0) as usize;
    (0..n)
        .map(|i| line_slice_along(ls, i as f64 * length_m, ((i + 1) as f64 * length_m).min(total), edges))
        .collect()
}

/// Intersection points of two line-ish geometries (planar, in lon/lat).
pub fn line_intersect(a: &Geometry, b: &Geometry) -> Vec<Coord> {
    use geo::line_intersection::{line_intersection, LineIntersection};
    let mut out = Vec::new();
    for la in lines_of(a) {
        for lb in lines_of(b) {
            match line_intersection(la, lb) {
                Some(LineIntersection::SinglePoint { intersection, .. }) => out.push(intersection),
                Some(LineIntersection::Collinear { intersection }) => {
                    out.push(intersection.start);
                    out.push(intersection.end);
                }
                None => {}
            }
        }
    }
    out.sort_by(|p, q| p.x.total_cmp(&q.x).then(p.y.total_cmp(&q.y)));
    out.dedup();
    out
}

/// Every segment of a geometry as a `Line`.
/// Every LineString a geometry is made of: its own, a polygon's rings, and the
/// parts of the multi variants. Rings come back closed.
pub fn line_strings_of(g: &Geometry) -> Vec<LineString> {
    let mut out = Vec::new();
    match g {
        Geometry::Line(l) => out.push(LineString(vec![l.start, l.end])),
        Geometry::LineString(ls) => out.push(ls.clone()),
        Geometry::MultiLineString(m) => out.extend(m.0.iter().cloned()),
        Geometry::Polygon(p) => {
            out.push(p.exterior().clone());
            out.extend(p.interiors().iter().cloned());
        }
        Geometry::MultiPolygon(mp) => {
            for p in &mp.0 {
                out.extend(line_strings_of(&Geometry::Polygon(p.clone())));
            }
        }
        Geometry::Rect(r) => out.extend(line_strings_of(&Geometry::Polygon(r.to_polygon()))),
        Geometry::Triangle(t) => out.extend(line_strings_of(&Geometry::Polygon(t.to_polygon()))),
        Geometry::GeometryCollection(gc) => {
            for g in &gc.0 {
                out.extend(line_strings_of(g));
            }
        }
        Geometry::Point(_) | Geometry::MultiPoint(_) => {}
    }
    out
}

pub fn lines_of(g: &Geometry) -> Vec<Line> {
    let mut out = Vec::new();
    let mut push_ls = |ls: &LineString| out.extend(ls.lines());
    match g {
        Geometry::Line(l) => out.push(*l),
        Geometry::LineString(ls) => push_ls(ls),
        Geometry::MultiLineString(m) => m.0.iter().for_each(push_ls),
        Geometry::Polygon(p) => {
            push_ls(p.exterior());
            p.interiors().iter().for_each(push_ls);
        }
        Geometry::MultiPolygon(mp) => {
            for p in &mp.0 {
                out.extend(p.exterior().lines());
                for r in p.interiors() {
                    out.extend(r.lines());
                }
            }
        }
        Geometry::Rect(r) => out.extend(r.to_polygon().exterior().lines()),
        Geometry::Triangle(t) => out.extend(t.to_polygon().exterior().lines()),
        Geometry::GeometryCollection(gc) => {
            for g in &gc.0 {
                out.extend(lines_of(g));
            }
        }
        Geometry::Point(_) | Geometry::MultiPoint(_) => {}
    }
    out
}

/// Densified geodesic between two points (a "great circle" line).
pub fn great_circle(a: Coord, b: Coord, npoints: usize) -> LineString {
    let n = npoints.max(2);
    let inv = geodesic::inverse(a.x, a.y, b.x, b.y);
    let line = geodesic::Line::new(a.x, a.y, inv.azi1);
    LineString(
        (0..n)
            .map(|i| {
                let (lon, lat, _) = line.position(inv.s12 * i as f64 / (n - 1) as f64);
                Coord { x: lon, y: lat }
            })
            .collect(),
    )
}

/// Circular sector between two bearings.
pub fn sector(center: Coord, radius_m: f64, bearing1: f64, bearing2: f64, steps: usize) -> Polygon {
    let mut sweep = geodesic::normalize_deg(bearing2 - bearing1);
    if sweep <= 0.0 {
        sweep += 360.0;
    }
    let n = steps.max(2);
    let mut ring = vec![center];
    for i in 0..=n {
        let az = bearing1 + sweep * i as f64 / n as f64;
        let (x, y) = geodesic::destination(center.x, center.y, az, radius_m);
        ring.push(Coord { x, y });
    }
    ring.push(center);
    Polygon::new(LineString(ring), vec![])
}

/// The arc of a circle between two bearings, as a line (turf's `lineArc`).
///
/// Every vertex is exactly `radius_m` from the centre on the ellipsoid; a
/// full sweep closes the ring.
pub fn line_arc(center: Coord, radius_m: f64, bearing1: f64, bearing2: f64, steps: usize) -> LineString {
    let mut sweep = geodesic::normalize_deg(bearing2 - bearing1);
    if sweep <= 0.0 {
        sweep += 360.0;
    }
    let n = steps.max(2);
    let mut out = Vec::with_capacity(n + 2);
    for i in 0..=n {
        let az = bearing1 + sweep * i as f64 / n as f64;
        let (x, y) = geodesic::destination(center.x, center.y, az, radius_m);
        out.push(Coord { x, y });
    }
    if (sweep - 360.0).abs() < 1e-9 && out.first() != out.last() {
        out.push(out[0]);
    }
    LineString(out)
}

/// Index of the point in `points` closest to `target`, with its distance (m).
pub fn nearest_point(target: Coord, points: &[Coord]) -> Option<(usize, f64)> {
    points
        .iter()
        .enumerate()
        .map(|(i, p)| (i, measure::distance(target, *p)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

// ----------------------------------------------------- structural helpers

/// Split multi-part geometries into their parts.
pub fn flatten(g: &Geometry) -> Vec<Geometry> {
    match g {
        Geometry::MultiPoint(mp) => mp.0.iter().map(|p| Geometry::Point(*p)).collect(),
        Geometry::MultiLineString(m) => m.0.iter().map(|l| Geometry::LineString(l.clone())).collect(),
        Geometry::MultiPolygon(mp) => mp.0.iter().map(|p| Geometry::Polygon(p.clone())).collect(),
        Geometry::GeometryCollection(gc) => gc.0.iter().flat_map(flatten).collect(),
        Geometry::Rect(r) => vec![Geometry::Polygon(r.to_polygon())],
        Geometry::Triangle(t) => vec![Geometry::Polygon(t.to_polygon())],
        other => vec![other.clone()],
    }
}

/// Boundary of a polygonal geometry as lines.
pub fn polygon_to_line(g: &Geometry) -> Result<MultiLineString> {
    let rings: Vec<LineString> = match g {
        Geometry::Polygon(p) => std::iter::once(p.exterior().clone())
            .chain(p.interiors().iter().cloned())
            .collect(),
        Geometry::MultiPolygon(mp) => {
            mp.0.iter()
                .flat_map(|p| std::iter::once(p.exterior().clone()).chain(p.interiors().iter().cloned()))
                .collect()
        }
        _ => return Err(Error::InvalidGeometry("expected Polygon or MultiPolygon".into())),
    };
    Ok(MultiLineString(rings))
}

/// Closed lines to polygons.
pub fn line_to_polygon(g: &Geometry) -> Result<MultiPolygon> {
    let rings: Vec<LineString> = match g {
        Geometry::LineString(ls) => vec![ls.clone()],
        Geometry::MultiLineString(m) => m.0.clone(),
        _ => return Err(Error::InvalidGeometry("expected LineString or MultiLineString".into())),
    };
    let polys: Vec<Polygon> = rings
        .into_iter()
        .map(|mut r| {
            if r.0.first() != r.0.last() {
                if let Some(first) = r.0.first().copied() {
                    r.0.push(first);
                }
            }
            Polygon::new(r, vec![])
        })
        .filter(|p| p.exterior().0.len() >= 4)
        .collect();
    if polys.is_empty() {
        return Err(Error::InvalidGeometry("no closable ring".into()));
    }
    Ok(MultiPolygon(polys))
}

/// Round coordinates to `precision` decimal places.
pub fn truncate(g: &Geometry, precision: u32) -> Geometry {
    let f = 10f64.powi(precision as i32);
    use geo::MapCoords;
    g.map_coords(|c| Coord {
        x: (c.x * f).round() / f,
        y: (c.y * f).round() / f,
    })
}

/// Remove repeated and collinear vertices (`tolerance_m` = 0 removes only exact duplicates).
pub fn clean_coords(g: &Geometry, tolerance_m: f64) -> Geometry {
    fn clean_line(pts: &[Coord], tol: f64, closed: bool) -> Vec<Coord> {
        let mut out: Vec<Coord> = Vec::with_capacity(pts.len());
        for &p in pts {
            if out.last() == Some(&p) {
                continue;
            }
            while out.len() >= 2 && tol > 0.0 {
                let (a, b) = (out[out.len() - 2], out[out.len() - 1]);
                let (_, d, _) = measure::closest_on_segment(a, p, b);
                if d <= tol {
                    out.pop();
                } else {
                    break;
                }
            }
            out.push(p);
        }
        if closed && out.len() > 1 && out.first() != out.last() {
            let first = out[0];
            out.push(first);
        }
        out
    }
    match g {
        Geometry::LineString(ls) => Geometry::LineString(LineString(clean_line(&ls.0, tolerance_m, false))),
        Geometry::MultiLineString(m) => Geometry::MultiLineString(MultiLineString(
            m.0.iter()
                .map(|l| LineString(clean_line(&l.0, tolerance_m, false)))
                .collect(),
        )),
        Geometry::Polygon(p) => Geometry::Polygon(clean_polygon(p, tolerance_m)),
        Geometry::MultiPolygon(mp) => Geometry::MultiPolygon(MultiPolygon(
            mp.0.iter().map(|p| clean_polygon(p, tolerance_m)).collect(),
        )),
        Geometry::MultiPoint(mp) => {
            let mut pts: Vec<Point> = Vec::new();
            for p in &mp.0 {
                if !pts.iter().any(|q| q == p) {
                    pts.push(*p);
                }
            }
            Geometry::MultiPoint(MultiPoint(pts))
        }
        Geometry::GeometryCollection(gc) => Geometry::GeometryCollection(GeometryCollection(
            gc.0.iter().map(|g| clean_coords(g, tolerance_m)).collect(),
        )),
        other => other.clone(),
    }
}

fn clean_polygon(p: &Polygon, tol: f64) -> Polygon {
    fn ring(r: &LineString, tol: f64) -> LineString {
        let mut pts: Vec<Coord> = r.0.clone();
        if pts.first() == pts.last() {
            pts.pop();
        }
        let mut out: Vec<Coord> = Vec::with_capacity(pts.len() + 1);
        for &p in &pts {
            if out.last() == Some(&p) {
                continue;
            }
            while out.len() >= 2 && tol > 0.0 {
                let (a, b) = (out[out.len() - 2], out[out.len() - 1]);
                let (_, d, _) = measure::closest_on_segment(a, p, b);
                if d <= tol {
                    out.pop();
                } else {
                    break;
                }
            }
            out.push(p);
        }
        if let Some(first) = out.first().copied() {
            out.push(first);
        }
        LineString(out)
    }
    Polygon::new(
        ring(p.exterior(), tol),
        p.interiors()
            .iter()
            .map(|r| ring(r, tol))
            .filter(|r| r.0.len() >= 4)
            .collect(),
    )
}

/// Enforce RFC 7946 winding: exterior rings counter-clockwise, holes clockwise.
pub fn rewind(g: &Geometry) -> Geometry {
    use geo::orient::Direction;
    use geo::Orient;
    match g {
        Geometry::Polygon(p) => Geometry::Polygon(p.orient(Direction::Default)),
        Geometry::MultiPolygon(mp) => Geometry::MultiPolygon(mp.orient(Direction::Default)),
        Geometry::GeometryCollection(gc) => {
            Geometry::GeometryCollection(GeometryCollection(gc.0.iter().map(rewind).collect()))
        }
        other => other.clone(),
    }
}

/// Clip a geometry to a bounding box.
pub fn bbox_clip(g: &Geometry, b: [f64; 4]) -> Result<Geometry> {
    let clip = Geometry::Polygon(bbox_polygon(b));
    match g {
        Geometry::Polygon(_) | Geometry::MultiPolygon(_) | Geometry::Rect(_) | Geometry::Triangle(_) => {
            let mp = crate::overlay::overlay(
                g,
                &clip,
                crate::overlay::OverlayOp::Intersection,
                &crate::overlay::OverlayOptions::default(),
            )?;
            Ok(Geometry::MultiPolygon(mp))
        }
        Geometry::LineString(_) | Geometry::MultiLineString(_) | Geometry::Line(_) => {
            let rect = Rect::new(Coord { x: b[0], y: b[1] }, Coord { x: b[2], y: b[3] });
            Ok(Geometry::MultiLineString(clip_lines(g, rect)))
        }
        Geometry::Point(p) => {
            let inside = p.0.x >= b[0] && p.0.x <= b[2] && p.0.y >= b[1] && p.0.y <= b[3];
            if inside {
                Ok(g.clone())
            } else {
                Err(Error::InvalidGeometry("point outside the bbox".into()))
            }
        }
        Geometry::MultiPoint(mp) => Ok(Geometry::MultiPoint(MultiPoint(
            mp.0.iter()
                .filter(|p| p.0.x >= b[0] && p.0.x <= b[2] && p.0.y >= b[1] && p.0.y <= b[3])
                .copied()
                .collect(),
        ))),
        Geometry::GeometryCollection(gc) => Ok(Geometry::GeometryCollection(GeometryCollection(
            gc.0.iter().filter_map(|g| bbox_clip(g, b).ok()).collect(),
        ))),
    }
}

/// Cohen–Sutherland style clipping of every segment, keeping the pieces inside.
fn clip_lines(g: &Geometry, rect: Rect<f64>) -> MultiLineString {
    let (min, max) = (rect.min(), rect.max());
    let code = |c: Coord| -> u8 {
        let mut k = 0;
        if c.x < min.x {
            k |= 1;
        }
        if c.x > max.x {
            k |= 2;
        }
        if c.y < min.y {
            k |= 4;
        }
        if c.y > max.y {
            k |= 8;
        }
        k
    };
    let mut parts: Vec<LineString> = Vec::new();
    let mut current: Vec<Coord> = Vec::new();
    for l in lines_of(g) {
        let (mut a, mut b) = (l.start, l.end);
        let (mut ca, mut cb) = (code(a), code(b));
        let mut accepted = false;
        for _ in 0..8 {
            if ca | cb == 0 {
                accepted = true;
                break;
            }
            if ca & cb != 0 {
                break;
            }
            let out = if ca != 0 { ca } else { cb };
            let (x, y) = if out & 8 != 0 {
                (a.x + (b.x - a.x) * (max.y - a.y) / (b.y - a.y), max.y)
            } else if out & 4 != 0 {
                (a.x + (b.x - a.x) * (min.y - a.y) / (b.y - a.y), min.y)
            } else if out & 2 != 0 {
                (max.x, a.y + (b.y - a.y) * (max.x - a.x) / (b.x - a.x))
            } else {
                (min.x, a.y + (b.y - a.y) * (min.x - a.x) / (b.x - a.x))
            };
            if out == ca {
                a = Coord { x, y };
                ca = code(a);
            } else {
                b = Coord { x, y };
                cb = code(b);
            }
        }
        if accepted {
            if current.last() != Some(&a) {
                if current.len() >= 2 {
                    parts.push(LineString(std::mem::take(&mut current)));
                } else {
                    current.clear();
                }
                current.push(a);
            }
            current.push(b);
        } else if current.len() >= 2 {
            parts.push(LineString(std::mem::take(&mut current)));
        } else {
            current.clear();
        }
    }
    if current.len() >= 2 {
        parts.push(LineString(current));
    }
    MultiLineString(parts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo::{coord, line_string, polygon};

    #[test]
    fn simplify_respects_metric_tolerance() {
        let mut pts = vec![];
        for i in 0..200 {
            let x = 120.0 + i as f64 * 0.001;
            // a straight line with ±0.5 m noise
            let y = 30.0 + if i % 2 == 0 { 0.0 } else { 4.5e-6 };
            pts.push(Coord { x, y });
        }
        let g = Geometry::LineString(LineString(pts));
        let s = simplify(&g, 1.0, false).unwrap();
        let Geometry::LineString(out) = &s else { panic!() };
        assert!(out.0.len() < 10, "{} vertices left", out.0.len());
        let s2 = simplify(&g, 0.1, false).unwrap();
        let Geometry::LineString(out2) = &s2 else { panic!() };
        assert!(out2.0.len() > 100);
    }

    #[test]
    fn hull_and_centres() {
        let pts: Vec<Geometry> = [
            (120.0, 30.0),
            (120.5, 30.0),
            (120.5, 30.5),
            (120.0, 30.5),
            (120.2, 30.2),
        ]
        .iter()
        .map(|(x, y)| Geometry::Point(Point::new(*x, *y)))
        .collect();
        let hull = convex_hull(&pts).unwrap();
        assert_eq!(hull.exterior().0.len(), 5);
        let c = vertex_centroid(&pts).unwrap();
        assert!((c.x - 120.24).abs() < 1e-9);
        let poly = Geometry::Polygon(hull.clone());
        let com = center_of_mass(std::slice::from_ref(&poly)).unwrap();
        assert!((com.x - 120.25).abs() < 1e-3 && (com.y - 30.25).abs() < 1e-3);
        let pof = point_on_feature(&poly).unwrap();
        assert!(crate::predicates::point_in_polygon(pof, &poly, false));
    }

    #[test]
    fn rotate_translate_scale_are_geodesic() {
        let g = Geometry::Point(Point::new(120.0, 30.0));
        let pivot = coord! {x: 120.0, y: 30.0};
        let moved = transform_translate(&g, 1000.0, 90.0);
        let Geometry::Point(p) = moved else { panic!() };
        assert!((measure::distance(pivot, p.0) - 1000.0).abs() < 1e-6);
        let rotated = transform_rotate(&moved, 90.0, pivot);
        let Geometry::Point(r) = rotated else { panic!() };
        assert!((measure::distance(pivot, r.0) - 1000.0).abs() < 1e-6);
        assert!((measure::bearing(pivot, r.0, false) - 180.0).abs() < 1e-6);
        let scaled = transform_scale(&moved, 2.0, pivot);
        let Geometry::Point(s) = scaled else { panic!() };
        assert!((measure::distance(pivot, s.0) - 2000.0).abs() < 1e-6);
    }

    #[test]
    fn line_slicing() {
        let ls = line_string![(x: 120.0, y: 30.0), (x: 120.0, y: 31.0), (x: 121.0, y: 31.0)];
        let total = measure::line_length(&ls);
        let mid = line_slice_along(&ls, 0.0, total / 2.0, Edges::Geodesic).unwrap();
        assert!((measure::line_length(&mid) - total / 2.0).abs() < 1e-3);
        let chunks = line_chunk(&ls, 20_000.0, Edges::Geodesic).unwrap();
        let sum: f64 = chunks.iter().map(measure::line_length).sum();
        assert!((sum - total).abs() < 1e-3, "{sum} vs {total}");
        let sliced = line_slice(
            &ls,
            coord! {x: 120.1, y: 30.5},
            coord! {x: 120.5, y: 31.1},
            Edges::Geodesic,
        )
        .unwrap();
        assert!(measure::line_length(&sliced) > 0.0);
    }

    #[test]
    fn intersections_and_clipping() {
        let a = Geometry::LineString(line_string![(x: 0.0, y: 0.0), (x: 2.0, y: 2.0)]);
        let b = Geometry::LineString(line_string![(x: 0.0, y: 2.0), (x: 2.0, y: 0.0)]);
        let pts = line_intersect(&a, &b);
        assert_eq!(pts.len(), 1);
        assert!((pts[0].x - 1.0).abs() < 1e-12 && (pts[0].y - 1.0).abs() < 1e-12);

        let poly = Geometry::Polygon(polygon![(x: 0.0, y: 0.0), (x: 4.0, y: 0.0), (x: 4.0, y: 4.0), (x: 0.0, y: 4.0)]);
        let clipped = bbox_clip(&poly, [1.0, 1.0, 3.0, 3.0]).unwrap();
        let area = measure::area_with(&clipped, Edges::Planar);
        let expect = measure::area_with(&Geometry::Polygon(bbox_polygon([1.0, 1.0, 3.0, 3.0])), Edges::Planar);
        assert!((area - expect).abs() / expect < 1e-6);

        let line_clip = bbox_clip(&a, [0.5, 0.5, 1.5, 1.5]).unwrap();
        let Geometry::MultiLineString(m) = line_clip else {
            panic!()
        };
        assert_eq!(m.0.len(), 1);
    }

    #[test]
    fn structural_helpers() {
        let mp = Geometry::MultiPolygon(MultiPolygon(vec![
            polygon![(x: 0.0, y: 0.0), (x: 1.0, y: 0.0), (x: 1.0, y: 1.0)],
            polygon![(x: 3.0, y: 3.0), (x: 4.0, y: 3.0), (x: 4.0, y: 4.0)],
        ]));
        assert_eq!(flatten(&mp).len(), 2);
        assert_eq!(polygon_to_line(&mp).unwrap().0.len(), 2);
        let t = truncate(&Geometry::Point(Point::new(1.23456789, 2.3456789)), 4);
        let Geometry::Point(p) = t else { panic!() };
        assert_eq!(p.x(), 1.2346);
        let dirty = line_string![(x: 0.0, y: 0.0), (x: 0.0, y: 0.0), (x: 1.0, y: 0.0), (x: 2.0, y: 0.0)];
        let cleaned = clean_coords(&Geometry::LineString(dirty), 1.0);
        let Geometry::LineString(c) = cleaned else { panic!() };
        assert_eq!(c.0.len(), 2);
    }
}
