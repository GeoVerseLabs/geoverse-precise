//! Shape construction and polygon-level helpers: ellipses, smoothing,
//! tangents, masks, splines, centres and polygonisation.

use std::collections::HashMap;

use geo::{Coord, Geometry, LineString, MultiPolygon, Point, Polygon};

use crate::local::LocalFrame;
use crate::{geodesic, measure, ops, overlay, Error, Result};

/// Geodesic ellipse: semi-axes in metres, `rotation` degrees clockwise from north.
pub fn ellipse(center: Coord, x_semi_m: f64, y_semi_m: f64, rotation_deg: f64, steps: usize) -> Polygon {
    let n = steps.max(8);
    let mut ring: Vec<Coord> = (0..n)
        .map(|i| {
            // parametric angle measured from the semi-major (x) axis
            let t = std::f64::consts::TAU * i as f64 / n as f64;
            let (st, ct) = t.sin_cos();
            let (x, y) = (x_semi_m * ct, y_semi_m * st);
            let r = x.hypot(y);
            // convert the local direction into a bearing (x = east, y = north)
            let bearing = x.atan2(y).to_degrees() + rotation_deg;
            let (lon, lat) = geodesic::destination(center.x, center.y, bearing, r);
            Coord { x: lon, y: lat }
        })
        .collect();
    ring.push(ring[0]);
    Polygon::new(LineString(ring), vec![])
}

/// Chaikin corner cutting, run `iterations` times in the local plane.
pub fn polygon_smooth(g: &Geometry, iterations: usize) -> Result<Geometry> {
    let frame = LocalFrame::for_geometries([g])?;
    let mut current = frame.project_geom(g);
    for _ in 0..iterations.max(1) {
        current = smooth_once(&current);
    }
    Ok(frame.unproject_geom(&current))
}

fn chaikin_ring(ring: &LineString) -> LineString {
    let pts = &ring.0;
    let n = if pts.first() == pts.last() {
        pts.len() - 1
    } else {
        pts.len()
    };
    if n < 3 {
        return ring.clone();
    }
    let mut out = Vec::with_capacity(n * 2 + 1);
    for i in 0..n {
        let a = pts[i];
        let b = pts[(i + 1) % n];
        out.push(Coord {
            x: 0.75 * a.x + 0.25 * b.x,
            y: 0.75 * a.y + 0.25 * b.y,
        });
        out.push(Coord {
            x: 0.25 * a.x + 0.75 * b.x,
            y: 0.25 * a.y + 0.75 * b.y,
        });
    }
    out.push(out[0]);
    LineString(out)
}

fn smooth_once(g: &Geometry) -> Geometry {
    match g {
        Geometry::Polygon(p) => Geometry::Polygon(Polygon::new(
            chaikin_ring(p.exterior()),
            p.interiors().iter().map(chaikin_ring).collect(),
        )),
        Geometry::MultiPolygon(mp) => Geometry::MultiPolygon(MultiPolygon(
            mp.0.iter()
                .map(|p| {
                    Polygon::new(
                        chaikin_ring(p.exterior()),
                        p.interiors().iter().map(chaikin_ring).collect(),
                    )
                })
                .collect(),
        )),
        Geometry::LineString(ls) => {
            let pts = &ls.0;
            if pts.len() < 3 {
                return g.clone();
            }
            let mut out = vec![pts[0]];
            for w in pts.windows(2) {
                out.push(Coord {
                    x: 0.75 * w[0].x + 0.25 * w[1].x,
                    y: 0.75 * w[0].y + 0.25 * w[1].y,
                });
                out.push(Coord {
                    x: 0.25 * w[0].x + 0.75 * w[1].x,
                    y: 0.25 * w[0].y + 0.75 * w[1].y,
                });
            }
            out.push(*pts.last().unwrap());
            Geometry::LineString(LineString(out))
        }
        other => other.clone(),
    }
}

/// The two vertices of a polygon that a viewer at `from` sees at the extreme
/// bearings — the tangent points.
pub fn polygon_tangents(from: Coord, polygon: &Geometry) -> Result<(Coord, Coord)> {
    let pts = ops::coords_of(polygon);
    if pts.len() < 3 {
        return Err(Error::InvalidGeometry("polygon needs at least 3 vertices".into()));
    }
    let base = measure::bearing(from, pts[0], false);
    let mut min = (0.0f64, pts[0]);
    let mut max = (0.0f64, pts[0]);
    for p in &pts {
        let d = geodesic::normalize_deg(measure::bearing(from, *p, false) - base);
        if d < min.0 {
            min = (d, *p);
        }
        if d > max.0 {
            max = (d, *p);
        }
    }
    Ok((min.1, max.1))
}

/// Everything inside `mask` except `polygon` (turf's `mask`).
pub fn mask(polygon: &Geometry, mask_polygon: &Geometry) -> Result<MultiPolygon> {
    overlay::overlay(
        mask_polygon,
        polygon,
        overlay::OverlayOp::Difference,
        &overlay::OverlayOptions::default(),
    )
}

/// Does the exterior ring turn both ways (i.e. is the polygon concave)?
pub fn boolean_concave(polygon: &Polygon) -> bool {
    let pts = &polygon.exterior().0;
    let n = if pts.first() == pts.last() {
        pts.len() - 1
    } else {
        pts.len()
    };
    if n < 4 {
        return false;
    }
    let (mut pos, mut neg) = (false, false);
    for i in 0..n {
        let a = pts[i];
        let b = pts[(i + 1) % n];
        let c = pts[(i + 2) % n];
        let cross = (b.x - a.x) * (c.y - b.y) - (b.y - a.y) * (c.x - b.x);
        if cross > 0.0 {
            pos = true;
        } else if cross < 0.0 {
            neg = true;
        }
    }
    pos && neg
}

/// Do two lines run parallel segment by segment (within `tolerance_deg`)?
pub fn boolean_parallel(a: &LineString, b: &LineString, tolerance_deg: f64) -> bool {
    let (sa, sb): (Vec<_>, Vec<_>) = (a.lines().collect(), b.lines().collect());
    if sa.is_empty() || sa.len() != sb.len() {
        return false;
    }
    sa.iter().zip(sb.iter()).all(|(x, y)| {
        let bx = measure::bearing(x.start, x.end, false);
        let by = measure::bearing(y.start, y.end, false);
        let d = geodesic::normalize_deg(bx - by).abs();
        d <= tolerance_deg || (180.0 - d).abs() <= tolerance_deg
    })
}

/// Swap longitude and latitude (turf's `flip`).
pub fn flip(g: &Geometry) -> Geometry {
    use geo::MapCoords;
    g.map_coords(|c| Coord { x: c.y, y: c.x })
}

/// Weighted mean centre.
pub fn center_mean(points: &[Coord], weights: Option<&[f64]>) -> Option<Coord> {
    if points.is_empty() {
        return None;
    }
    let mut sum = Coord { x: 0.0, y: 0.0 };
    let mut total = 0.0;
    for (i, p) in points.iter().enumerate() {
        let w = weights.and_then(|w| w.get(i).copied()).unwrap_or(1.0);
        sum.x += p.x * w;
        sum.y += p.y * w;
        total += w;
    }
    (total != 0.0).then(|| Coord {
        x: sum.x / total,
        y: sum.y / total,
    })
}

/// Geometric median (the point minimising total distance), solved with
/// Weiszfeld's algorithm in the local plane.
pub fn center_median(points: &[Coord], weights: Option<&[f64]>, tolerance_m: f64) -> Result<Coord> {
    if points.is_empty() {
        return Err(Error::InvalidGeometry("no points".into()));
    }
    let geoms: Vec<Geometry> = points.iter().map(|c| Geometry::Point(Point(*c))).collect();
    let frame = LocalFrame::for_geometries(geoms.iter())?;
    let proj: Vec<Coord> = points
        .iter()
        .map(|c| {
            let (x, y) = frame.project(c.x, c.y);
            Coord { x, y }
        })
        .collect();
    let w: Vec<f64> = (0..proj.len())
        .map(|i| weights.and_then(|w| w.get(i).copied()).unwrap_or(1.0))
        .collect();
    let mut cur = center_mean(&proj, Some(&w)).unwrap();
    for _ in 0..200 {
        let (mut nx, mut ny, mut den) = (0.0, 0.0, 0.0);
        for (p, wi) in proj.iter().zip(w.iter()) {
            let d = (p.x - cur.x).hypot(p.y - cur.y).max(1e-9);
            nx += wi * p.x / d;
            ny += wi * p.y / d;
            den += wi / d;
        }
        if den == 0.0 {
            break;
        }
        let next = Coord {
            x: nx / den,
            y: ny / den,
        };
        let step = (next.x - cur.x).hypot(next.y - cur.y);
        cur = next;
        if step < tolerance_m.max(1e-6) {
            break;
        }
    }
    let (x, y) = frame.unproject(cur.x, cur.y);
    Ok(Coord { x, y })
}

/// Cubic spline through the vertices of a line (turf's `bezierSpline`).
///
/// `sharpness` (0–1) controls how tightly the curve hugs the control polygon;
/// `resolution` is the number of samples per input segment.
pub fn bezier_spline(line: &LineString, sharpness: f64, resolution: usize) -> Result<LineString> {
    let pts = &line.0;
    if pts.len() < 3 {
        return Ok(line.clone());
    }
    let frame = LocalFrame::for_geometries([&Geometry::LineString(line.clone())])?;
    let p: Vec<Coord> = pts
        .iter()
        .map(|c| {
            let (x, y) = frame.project(c.x, c.y);
            Coord { x, y }
        })
        .collect();
    let s = sharpness.clamp(0.0, 1.0);
    let res = resolution.max(2);
    let n = p.len();
    let mut out: Vec<Coord> = Vec::with_capacity(n * res);
    for i in 0..n - 1 {
        let p0 = if i == 0 { p[0] } else { p[i - 1] };
        let (p1, p2) = (p[i], p[i + 1]);
        let p3 = if i + 2 < n { p[i + 2] } else { p[n - 1] };
        // Catmull–Rom control points scaled by the sharpness
        let c1 = Coord {
            x: p1.x + (p2.x - p0.x) * s / 6.0,
            y: p1.y + (p2.y - p0.y) * s / 6.0,
        };
        let c2 = Coord {
            x: p2.x - (p3.x - p1.x) * s / 6.0,
            y: p2.y - (p3.y - p1.y) * s / 6.0,
        };
        for k in 0..res {
            let t = k as f64 / res as f64;
            let mt = 1.0 - t;
            let x = mt * mt * mt * p1.x + 3.0 * mt * mt * t * c1.x + 3.0 * mt * t * t * c2.x + t * t * t * p2.x;
            let y = mt * mt * mt * p1.y + 3.0 * mt * mt * t * c1.y + 3.0 * mt * t * t * c2.y + t * t * t * p2.y;
            out.push(Coord { x, y });
        }
    }
    out.push(*p.last().unwrap());
    Ok(LineString(
        out.into_iter()
            .map(|c| {
                let (x, y) = frame.unproject(c.x, c.y);
                Coord { x, y }
            })
            .collect(),
    ))
}

// ------------------------------------------------------------ polygonize

const QUANT: f64 = 1e9;

#[inline]
fn key_of(c: Coord) -> (i64, i64) {
    ((c.x * QUANT).round() as i64, (c.y * QUANT).round() as i64)
}

/// Build polygons from a noded line network (turf's `polygonize`).
///
/// Input lines are split at every crossing first, so a road network or a set of
/// boundary lines can be turned into the faces they enclose.
pub fn polygonize(geoms: &[Geometry]) -> Result<MultiPolygon> {
    use geo::line_intersection::{line_intersection, LineIntersection};

    // 1. collect and node every segment
    let raw: Vec<geo::Line<f64>> = geoms.iter().flat_map(ops::lines_of).collect();
    if raw.is_empty() {
        return Err(Error::InvalidGeometry("no lines to polygonize".into()));
    }
    let mut segments: Vec<(Coord, Coord)> = Vec::new();
    for (i, a) in raw.iter().enumerate() {
        let mut cuts: Vec<f64> = vec![0.0, 1.0];
        for (j, b) in raw.iter().enumerate() {
            if i == j {
                continue;
            }
            let t_of = |c: Coord| {
                let (dx, dy) = (a.end.x - a.start.x, a.end.y - a.start.y);
                let len2 = dx * dx + dy * dy;
                if len2 == 0.0 {
                    0.0
                } else {
                    ((c.x - a.start.x) * dx + (c.y - a.start.y) * dy) / len2
                }
            };
            match line_intersection(*a, *b) {
                Some(LineIntersection::SinglePoint { intersection, .. }) => cuts.push(t_of(intersection)),
                Some(LineIntersection::Collinear { intersection }) => {
                    cuts.push(t_of(intersection.start));
                    cuts.push(t_of(intersection.end));
                }
                None => {}
            }
        }
        cuts.retain(|t| (-1e-12..=1.0 + 1e-12).contains(t));
        cuts.sort_by(f64::total_cmp);
        cuts.dedup_by(|x, y| (*x - *y).abs() < 1e-12);
        for w in cuts.windows(2) {
            let at = |t: f64| Coord {
                x: a.start.x + (a.end.x - a.start.x) * t,
                y: a.start.y + (a.end.y - a.start.y) * t,
            };
            let (p, q) = (at(w[0]), at(w[1]));
            if key_of(p) != key_of(q) {
                segments.push((p, q));
            }
        }
    }

    // 2. build the planar graph
    let mut node_of: HashMap<(i64, i64), usize> = HashMap::new();
    let mut nodes: Vec<Coord> = Vec::new();
    let id = |c: Coord, nodes: &mut Vec<Coord>, node_of: &mut HashMap<(i64, i64), usize>| -> usize {
        *node_of.entry(key_of(c)).or_insert_with(|| {
            nodes.push(c);
            nodes.len() - 1
        })
    };
    let mut edges: Vec<(usize, usize)> = Vec::new();
    let mut seen_edge: HashMap<(usize, usize), ()> = HashMap::new();
    for (p, q) in segments {
        let (a, b) = (id(p, &mut nodes, &mut node_of), id(q, &mut nodes, &mut node_of));
        if a == b {
            continue;
        }
        let key = (a.min(b), a.max(b));
        if seen_edge.insert(key, ()).is_none() {
            edges.push((a, b));
        }
    }
    if edges.is_empty() {
        return Ok(MultiPolygon(vec![]));
    }

    // directed half-edges, sorted around each node by angle
    let mut out_edges: Vec<Vec<(usize, usize)>> = vec![Vec::new(); nodes.len()]; // (to, half-edge id)
    let mut halves: Vec<(usize, usize)> = Vec::new(); // (from, to)
    for &(a, b) in &edges {
        halves.push((a, b));
        out_edges[a].push((b, halves.len() - 1));
        halves.push((b, a));
        out_edges[b].push((a, halves.len() - 1));
    }
    let angle = |from: usize, to: usize| -> f64 {
        let (a, b) = (nodes[from], nodes[to]);
        (b.y - a.y).atan2(b.x - a.x)
    };
    for (n, list) in out_edges.iter_mut().enumerate() {
        list.sort_by(|x, y| angle(n, x.0).total_cmp(&angle(n, y.0)));
    }

    // 3. walk minimal faces: always take the next edge clockwise from the reverse
    let mut used = vec![false; halves.len()];
    let mut rings: Vec<Vec<Coord>> = Vec::new();
    for start in 0..halves.len() {
        if used[start] {
            continue;
        }
        let mut ring_nodes: Vec<usize> = Vec::new();
        let mut h = start;
        loop {
            if used[h] {
                break;
            }
            used[h] = true;
            let (from, to) = halves[h];
            ring_nodes.push(from);
            // at `to`, pick the edge just before the reverse direction
            let list = &out_edges[to];
            let rev_angle = angle(to, from);
            let mut next = None;
            let mut best = f64::MAX;
            for &(cand, hid) in list {
                if cand == from && list.len() > 1 {
                    continue;
                }
                let mut d = rev_angle - angle(to, cand);
                while d <= 0.0 {
                    d += std::f64::consts::TAU;
                }
                if d < best {
                    best = d;
                    next = Some(hid);
                }
            }
            match next {
                Some(nh) if nh != start => h = nh,
                _ => {
                    ring_nodes.push(to);
                    break;
                }
            }
        }
        if ring_nodes.len() >= 3 {
            let mut ring: Vec<Coord> = ring_nodes.iter().map(|i| nodes[*i]).collect();
            if ring.first() != ring.last() {
                ring.push(ring[0]);
            }
            rings.push(ring);
        }
    }

    // 4. keep the counter-clockwise faces (the outer walk comes out clockwise)
    let mut polys: Vec<Polygon> = Vec::new();
    for ring in rings {
        let ls = LineString(ring);
        let signed = shoelace(&ls);
        if signed > 0.0 {
            let p = Polygon::new(ls, vec![]);
            if p.exterior().0.len() >= 4 {
                polys.push(p);
            }
        }
    }
    Ok(MultiPolygon(polys))
}

fn shoelace(ring: &LineString) -> f64 {
    let pts = &ring.0;
    let n = if pts.first() == pts.last() {
        pts.len() - 1
    } else {
        pts.len()
    };
    let mut sum = 0.0;
    for i in 0..n {
        let a = pts[i];
        let b = pts[(i + 1) % n];
        sum += a.x * b.y - b.x * a.y;
    }
    sum / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::densify::Edges;
    use geo::{coord, line_string, polygon};

    #[test]
    fn ellipse_axes_are_metric() {
        let c = coord! {x: 120.0, y: 30.0};
        let e = ellipse(c, 2000.0, 1000.0, 0.0, 128);
        let pts = &e.exterior().0;
        let east = pts.iter().map(|p| measure::distance(c, *p)).fold(0.0, f64::max);
        assert!((east - 2000.0).abs() < 1.0, "{east}");
        // rotation by 90° swaps which direction is long
        let r = ellipse(c, 2000.0, 1000.0, 90.0, 128);
        let north_r = r.exterior().0.iter().map(|p| (p.y - 30.0).abs()).fold(0.0, f64::max);
        let north_0 = e.exterior().0.iter().map(|p| (p.y - 30.0).abs()).fold(0.0, f64::max);
        assert!(north_r > north_0 * 1.5, "{north_r} vs {north_0}");
    }

    #[test]
    fn smoothing_rounds_corners_and_keeps_area() {
        // Chaikin corner-cutting: each iteration replaces every vertex with the
        // 1/4 and 3/4 points of its two edges. On a square that removes four
        // corner triangles of leg 1/4, i.e. exactly 1/8 of the area, and the
        // second pass removes a further 1/32 — 27/32 in all. Checking that
        // ratio algebraically pins the subdivision down far better than a
        // fuzzy band would. The cutting runs in the local metric plane (so
        // cells do not skew with latitude), and that plane is conformal rather
        // than affine, which leaves a ~1e-7 relative wobble in the lon/lat
        // ratio over an 11 km square.
        let ring = |g: &Geometry| -> Vec<Coord> {
            let Geometry::Polygon(p) = g else {
                panic!("not a polygon")
            };
            p.exterior().0.clone()
        };
        // shoelace about the first vertex: at lon 120 / lat 30 the raw form
        // loses ten digits to cancellation, which would swamp the ratio below
        let shoelace = |cs: &[Coord]| -> f64 {
            let o = cs[0];
            let mut a = 0.0;
            for w in cs.windows(2) {
                let (p, q) = (w[0] - o, w[1] - o);
                a += p.x * q.y - q.x * p.y;
            }
            0.5 * a.abs()
        };
        let p = Geometry::Polygon(polygon![
            (x: 120.0, y: 30.0), (x: 120.1, y: 30.0), (x: 120.1, y: 30.1), (x: 120.0, y: 30.1), (x: 120.0, y: 30.0)
        ]);
        let a0 = shoelace(&ring(&p));

        let s1 = polygon_smooth(&p, 1).unwrap();
        assert_eq!(ring(&s1).len(), 9, "one pass doubles the 4 corners");
        assert!(
            (shoelace(&ring(&s1)) / a0 - 7.0 / 8.0).abs() < 1e-6,
            "{}",
            shoelace(&ring(&s1)) / a0
        );

        let s2 = polygon_smooth(&p, 2).unwrap();
        assert_eq!(ring(&s2).len(), 17);
        assert!(
            (shoelace(&ring(&s2)) / a0 - 27.0 / 32.0).abs() < 1e-6,
            "{}",
            shoelace(&ring(&s2)) / a0
        );

        // corner cutting keeps the result inside the original square, up to the
        // sub-metre bulge between a lon/lat-straight edge and the plane chord
        // the cutting runs on
        for c in ring(&s2) {
            let d = crate::lines::point_to_polygon_distance(c, &p, Edges::Planar).unwrap();
            assert!(d < 1.0, "{c:?} is {d} m outside the square");
        }

        // and the same holds for the measured (ellipsoidal) area
        let g0 = measure::area_with(&p, Edges::Planar);
        let g2 = measure::area_with(&s2, Edges::Planar);
        assert!(g2 < g0 && (g2 / g0 - 27.0 / 32.0).abs() < 0.01, "{g2} vs {g0}");

        // holes are smoothed too, and a line is returned untouched
        let holed = Geometry::Polygon(Polygon::new(
            LineString(ring(&p)),
            vec![
                line_string![(x: 120.02, y: 30.02), (x: 120.05, y: 30.02), (x: 120.05, y: 30.05), (x: 120.02, y: 30.05), (x: 120.02, y: 30.02)],
            ],
        ));
        let sh = polygon_smooth(&holed, 1).unwrap();
        let Geometry::Polygon(shp) = &sh else { panic!() };
        assert_eq!(shp.interiors()[0].0.len(), 9);
    }

    #[test]
    fn tangents_mask_and_flags() {
        let square = polygon![(x: 0.0, y: 0.0), (x: 1.0, y: 0.0), (x: 1.0, y: 1.0), (x: 0.0, y: 1.0), (x: 0.0, y: 0.0)];
        let (t1, t2) = polygon_tangents(coord! {x: 3.0, y: 0.5}, &Geometry::Polygon(square.clone())).unwrap();
        assert!((t1.y - t2.y).abs() > 0.5, "{t1:?} {t2:?}");

        let big = Geometry::Polygon(
            polygon![(x: -1.0, y: -1.0), (x: 2.0, y: -1.0), (x: 2.0, y: 2.0), (x: -1.0, y: 2.0), (x: -1.0, y: -1.0)],
        );
        let m = mask(&Geometry::Polygon(square.clone()), &big).unwrap();
        assert_eq!(m.0.len(), 1);
        assert_eq!(m.0[0].interiors().len(), 1);

        assert!(!boolean_concave(&square));
        let c = polygon![(x: 0.0, y: 0.0), (x: 2.0, y: 0.0), (x: 1.0, y: 1.0), (x: 2.0, y: 2.0), (x: 0.0, y: 2.0), (x: 0.0, y: 0.0)];
        assert!(boolean_concave(&c));

        let l1 = line_string![(x: 0.0, y: 0.0), (x: 1.0, y: 0.0)];
        let l2 = line_string![(x: 0.0, y: 1.0), (x: 1.0, y: 1.0)];
        assert!(boolean_parallel(&l1, &l2, 0.1));
        let l3 = line_string![(x: 0.0, y: 1.0), (x: 1.0, y: 2.0)];
        assert!(!boolean_parallel(&l1, &l3, 0.1));
    }

    #[test]
    fn centres_and_spline() {
        let pts = [
            coord! {x: 0.0, y: 0.0},
            coord! {x: 0.1, y: 0.0},
            coord! {x: 0.0, y: 0.1},
            coord! {x: 5.0, y: 5.0},
        ];
        let mean = center_mean(&pts, None).unwrap();
        let median = center_median(&pts, None, 0.01).unwrap();
        // the median resists the outlier, the mean does not
        assert!(median.x < mean.x && median.x < 0.2, "{median:?} vs {mean:?}");

        let line = line_string![(x: 0.0, y: 0.0), (x: 0.01, y: 0.02), (x: 0.02, y: 0.0), (x: 0.03, y: 0.02)];
        let sp = bezier_spline(&line, 0.85, 10).unwrap();
        assert!(sp.0.len() > line.0.len() * 5);
        assert!((sp.0[0].x - line.0[0].x).abs() < 1e-9);
    }

    #[test]
    fn polygonize_builds_faces() {
        // a 2×1 grid of squares drawn as separate lines
        let lines = vec![
            Geometry::LineString(line_string![(x: 0.0, y: 0.0), (x: 2.0, y: 0.0)]),
            Geometry::LineString(line_string![(x: 0.0, y: 1.0), (x: 2.0, y: 1.0)]),
            Geometry::LineString(line_string![(x: 0.0, y: 0.0), (x: 0.0, y: 1.0)]),
            Geometry::LineString(line_string![(x: 1.0, y: 0.0), (x: 1.0, y: 1.0)]),
            Geometry::LineString(line_string![(x: 2.0, y: 0.0), (x: 2.0, y: 1.0)]),
        ];
        let faces = polygonize(&lines).unwrap();
        assert_eq!(faces.0.len(), 2, "{:?}", faces.0.len());
        for f in &faces.0 {
            let a = measure::area_with(&Geometry::Polygon(f.clone()), Edges::Planar);
            assert!(a > 1e10, "{a}");
        }
    }
}
