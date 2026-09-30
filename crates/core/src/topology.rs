//! Topology quality checks and snapping.
//!
//! * [`coverage_issues`] — overlaps and gaps between polygons that are meant to
//!   tile an area (cadastral parcels, administrative units, land cover).
//! * [`network_issues`] — dangles, pseudo nodes, self-intersections, crossings
//!   without a node, duplicates, undershoots and overshoots in a line network.
//! * [`snap_round`] / [`snap_to`] — precision reduction and tolerance snapping,
//!   so that layers that should line up actually do.
//!
//! All of it runs in a local transverse Mercator plane, so tolerances and areas
//! are in metres and square metres.

use std::collections::HashMap;

use geo::orient::Direction;
use geo::{unary_union, BooleanOps, BoundingRect, Coord, Geometry, LineString, MultiPolygon, Orient, Polygon, Rect};
use serde::Serialize;

use crate::densify::{self, Edges, DEFAULT_TOL};
use crate::local::LocalFrame;
use crate::measure;
use crate::ops;
use crate::{Error, Result};

// --------------------------------------------------------------- coverage

#[derive(Debug, Clone, Copy)]
pub struct CoverageOptions {
    pub edges: Edges,
    /// Ignore overlaps smaller than this (m²).
    pub min_overlap_m2: f64,
    /// Ignore gaps smaller than this (m²) — keeps numerical crumbs out of the report.
    pub min_gap_m2: f64,
    /// Report gaps up to this area (m²); larger holes are treated as genuine.
    pub max_gap_m2: f64,
    /// Also find slivers that are open at the ends (not enclosed holes) by
    /// closing the union with this width in metres. 0 = enclosed holes only.
    pub gap_tolerance_m: f64,
    pub tolerance: f64,
}

impl Default for CoverageOptions {
    fn default() -> Self {
        CoverageOptions {
            edges: Edges::Planar,
            min_overlap_m2: 1e-3,
            min_gap_m2: 1.0,
            max_gap_m2: f64::INFINITY,
            gap_tolerance_m: 0.0,
            tolerance: DEFAULT_TOL,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Overlap {
    /// Indices of the two overlapping inputs.
    pub a: usize,
    pub b: usize,
    pub area_m2: f64,
    #[serde(skip)]
    pub geometry: MultiPolygon<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Gap {
    pub area_m2: f64,
    #[serde(skip)]
    pub geometry: Polygon<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CoverageReport {
    pub overlaps: Vec<Overlap>,
    pub gaps: Vec<Gap>,
    /// Total area of the union (m²).
    pub union_area_m2: f64,
    /// Sum of the individual areas (m²); larger than the union when parts overlap.
    pub total_area_m2: f64,
}

fn rect_intersects(a: &Rect<f64>, b: &Rect<f64>, pad: f64) -> bool {
    a.min().x - pad <= b.max().x
        && b.min().x - pad <= a.max().x
        && a.min().y - pad <= b.max().y
        && b.min().y - pad <= a.max().y
}

/// Find overlaps and gaps in a set of polygons that should form a clean coverage.
pub fn coverage_issues(geoms: &[Geometry], opts: &CoverageOptions) -> Result<CoverageReport> {
    if geoms.is_empty() {
        return Err(Error::InvalidGeometry("no polygons given".into()));
    }
    let frame = LocalFrame::for_geometries(geoms.iter())?;
    let projected: Vec<MultiPolygon> = geoms
        .iter()
        .map(|g| {
            let mp = crate::overlay::as_multipolygon(g)?;
            Ok(MultiPolygon(
                mp.0.iter()
                    .map(|p| densify::project_polygon(&frame, p, opts.edges, opts.tolerance))
                    .collect(),
            )
            .orient(Direction::Default))
        })
        .collect::<Result<_>>()?;
    let rects: Vec<Option<Rect<f64>>> = projected.iter().map(|p| p.bounding_rect()).collect();

    let mut overlaps = Vec::new();
    for i in 0..projected.len() {
        for j in (i + 1)..projected.len() {
            let (Some(ri), Some(rj)) = (rects[i], rects[j]) else {
                continue;
            };
            if !rect_intersects(&ri, &rj, 0.0) {
                continue;
            }
            let inter = projected[i].intersection(&projected[j]);
            if inter.0.is_empty() {
                continue;
            }
            let area: f64 = inter.0.iter().map(planar_area_projected).sum();
            if area > opts.min_overlap_m2 {
                overlaps.push(Overlap {
                    a: i,
                    b: j,
                    area_m2: area,
                    geometry: frame.unproject_geom(&inter),
                });
            }
        }
    }

    let union = unary_union(projected.iter());
    let mut gaps = Vec::new();
    if opts.gap_tolerance_m > 0.0 {
        // Morphological closing: dilate then erode. Anything the closing adds is
        // a gap narrower than 2 × the tolerance, including slivers that are open
        // at both ends (which never show up as interior rings).
        use geo::algorithm::buffer::{Buffer, BufferStyle, LineJoin};
        let d = opts.gap_tolerance_m;
        // Mitred joins keep corners sharp, so closing restores them exactly and
        // leaves no crumbs behind at every convex corner.
        let grown = union.buffer_with_style(BufferStyle::new(d).line_join(LineJoin::Miter(0.05)));
        let closed = grown.buffer_with_style(BufferStyle::new(-d).line_join(LineJoin::Miter(0.05)));
        let found = closed.difference(&union);
        for poly in &found.0 {
            let area = planar_area_projected(poly);
            if area >= opts.min_gap_m2 && area <= opts.max_gap_m2 {
                gaps.push(Gap {
                    area_m2: area,
                    geometry: frame.unproject_geom(poly),
                });
            }
        }
    } else {
        for poly in &union.0 {
            for hole in poly.interiors() {
                let area = ring_area_projected(hole);
                if area >= opts.min_gap_m2 && area <= opts.max_gap_m2 {
                    gaps.push(Gap {
                        area_m2: area,
                        geometry: frame.unproject_geom(&Polygon::new(hole.clone(), vec![])),
                    });
                }
            }
        }
    }
    gaps.sort_by(|a, b| b.area_m2.total_cmp(&a.area_m2));

    let union_area: f64 = union.0.iter().map(planar_area_projected).sum();
    let total_area: f64 = projected
        .iter()
        .flat_map(|mp| mp.0.iter())
        .map(planar_area_projected)
        .sum();
    Ok(CoverageReport {
        overlaps,
        gaps,
        union_area_m2: union_area,
        total_area_m2: total_area,
    })
}

fn ring_area_projected(r: &LineString) -> f64 {
    use geo::Area;
    Polygon::new(r.clone(), vec![]).unsigned_area()
}

fn planar_area_projected(p: &Polygon) -> f64 {
    use geo::Area;
    p.unsigned_area()
}

// ---------------------------------------------------------------- network

#[derive(Debug, Clone, Copy)]
pub struct NetworkOptions {
    /// Distance (m) under which endpoints are considered connected.
    pub tolerance_m: f64,
    /// Report endpoints that stop within this distance (m) of another line
    /// without connecting to it. 0 disables the check.
    pub max_undershoot_m: f64,
    /// Report dangling stubs shorter than this (m) past the last junction. 0 disables.
    pub max_overshoot_m: f64,
    pub edges: Edges,
}

impl Default for NetworkOptions {
    fn default() -> Self {
        NetworkOptions {
            tolerance_m: 0.01,
            max_undershoot_m: 0.0,
            max_overshoot_m: 0.0,
            edges: Edges::Planar,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct NodeIssue {
    pub at: [f64; 2],
    /// Number of line ends meeting here.
    pub degree: usize,
    /// Indices of the lines that meet here.
    pub lines: Vec<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PointIssue {
    pub at: [f64; 2],
    pub lines: Vec<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distance_m: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct NetworkReport {
    /// Endpoints where only one line ends (potentially unconnected).
    pub dangles: Vec<NodeIssue>,
    /// Nodes where exactly two line ends meet: the lines could be merged.
    pub pseudo_nodes: Vec<NodeIssue>,
    /// Places where a line crosses itself.
    pub self_intersections: Vec<PointIssue>,
    /// Crossings between different lines that are not shared vertices.
    pub crossings_without_node: Vec<PointIssue>,
    /// Pairs of identical lines (same vertices, possibly reversed).
    pub duplicates: Vec<[usize; 2]>,
    /// Endpoints that nearly touch another line but are not connected to it.
    pub undershoots: Vec<PointIssue>,
    /// Dangling stubs shorter than `max_overshoot_m` past the last junction.
    pub overshoots: Vec<PointIssue>,
}

struct NodeGrid {
    cell: f64,
    map: HashMap<(i64, i64), Vec<usize>>,
    nodes: Vec<(Coord, Vec<usize>)>,
}

impl NodeGrid {
    fn new(cell: f64) -> NodeGrid {
        NodeGrid {
            cell: cell.max(1e-9),
            map: HashMap::new(),
            nodes: Vec::new(),
        }
    }

    fn key(&self, c: Coord) -> (i64, i64) {
        ((c.x / self.cell).floor() as i64, (c.y / self.cell).floor() as i64)
    }

    fn add(&mut self, c: Coord, line: usize, tol: f64) {
        let (kx, ky) = self.key(c);
        for dx in -1..=1 {
            for dy in -1..=1 {
                if let Some(ids) = self.map.get(&(kx + dx, ky + dy)) {
                    for &id in ids {
                        let n = &mut self.nodes[id];
                        if (n.0.x - c.x).hypot(n.0.y - c.y) <= tol {
                            n.1.push(line);
                            return;
                        }
                    }
                }
            }
        }
        self.nodes.push((c, vec![line]));
        self.map.entry((kx, ky)).or_default().push(self.nodes.len() - 1);
    }
}

/// Check a line network for the usual topology problems.
pub fn network_issues(geoms: &[Geometry], opts: &NetworkOptions) -> Result<NetworkReport> {
    use geo::line_intersection::{line_intersection, LineIntersection};

    if geoms.is_empty() {
        return Err(Error::InvalidGeometry("no lines given".into()));
    }
    let frame = LocalFrame::for_geometries(geoms.iter())?;
    let tol = opts.tolerance_m.max(0.0);

    // Project every part into the plane (metres).
    let mut parts: Vec<Vec<Coord>> = Vec::new();
    for g in geoms {
        let dense = densify::prepare(g, opts.edges, DEFAULT_TOL);
        match dense {
            Geometry::LineString(ls) => parts.push(project_path(&frame, &ls.0)),
            Geometry::MultiLineString(m) => parts.extend(m.0.iter().map(|l| project_path(&frame, &l.0))),
            Geometry::Line(l) => parts.push(project_path(&frame, &[l.start, l.end])),
            other => {
                return Err(Error::InvalidGeometry(format!(
                    "network checks expect lines, got {}",
                    crate::overlay::geometry_type_name(&other)
                )))
            }
        }
    }

    let mut report = NetworkReport::default();
    let unproject = |c: Coord| {
        let (x, y) = frame.unproject(c.x, c.y);
        [x, y]
    };

    // --- nodes -------------------------------------------------------
    let mut grid = NodeGrid::new(tol.max(1.0));
    for (i, part) in parts.iter().enumerate() {
        if let (Some(a), Some(b)) = (part.first(), part.last()) {
            grid.add(*a, i, tol);
            grid.add(*b, i, tol);
        }
    }
    for (c, lines) in &grid.nodes {
        let degree = lines.len();
        let mut unique = lines.clone();
        unique.sort_unstable();
        unique.dedup();
        if degree == 1 {
            report.dangles.push(NodeIssue {
                at: unproject(*c),
                degree,
                lines: unique,
            });
        } else if degree == 2 && unique.len() == 2 {
            report.pseudo_nodes.push(NodeIssue {
                at: unproject(*c),
                degree,
                lines: unique,
            });
        }
    }

    // --- self-intersections and crossings ---------------------------
    let segs: Vec<Vec<geo::Line<f64>>> = parts
        .iter()
        .map(|p| p.windows(2).map(|w| geo::Line::new(w[0], w[1])).collect())
        .collect();
    for (i, si) in segs.iter().enumerate() {
        for a in 0..si.len() {
            for b in (a + 2)..si.len() {
                if let Some(LineIntersection::SinglePoint {
                    intersection,
                    is_proper,
                }) = line_intersection(si[a], si[b])
                {
                    if is_proper {
                        report.self_intersections.push(PointIssue {
                            at: unproject(intersection),
                            lines: vec![i],
                            distance_m: None,
                        });
                    }
                }
            }
        }
    }
    let node_at = |c: Coord| -> bool {
        grid.nodes
            .iter()
            .any(|(n, _)| (n.x - c.x).hypot(n.y - c.y) <= tol.max(1e-9))
    };
    for i in 0..segs.len() {
        for j in (i + 1)..segs.len() {
            for a in &segs[i] {
                for b in &segs[j] {
                    if let Some(LineIntersection::SinglePoint {
                        intersection,
                        is_proper,
                    }) = line_intersection(*a, *b)
                    {
                        if is_proper && !node_at(intersection) {
                            report.crossings_without_node.push(PointIssue {
                                at: unproject(intersection),
                                lines: vec![i, j],
                                distance_m: None,
                            });
                        }
                    }
                }
            }
        }
    }

    // --- duplicates --------------------------------------------------
    let key_of = |p: &Vec<Coord>| -> Vec<(i64, i64)> {
        let q = |c: &Coord| {
            (
                (c.x / tol.max(1e-3)).round() as i64,
                (c.y / tol.max(1e-3)).round() as i64,
            )
        };
        let fwd: Vec<(i64, i64)> = p.iter().map(q).collect();
        let mut rev = fwd.clone();
        rev.reverse();
        if fwd <= rev {
            fwd
        } else {
            rev
        }
    };
    let mut seen: HashMap<Vec<(i64, i64)>, usize> = HashMap::new();
    for (i, p) in parts.iter().enumerate() {
        let k = key_of(p);
        if let Some(&first) = seen.get(&k) {
            report.duplicates.push([first, i]);
        } else {
            seen.insert(k, i);
        }
    }

    // --- undershoots and overshoots ----------------------------------
    for (i, part) in parts.iter().enumerate() {
        for end in [part.first(), part.last()].into_iter().flatten() {
            // is this endpoint a dangle?
            let is_dangle = grid
                .nodes
                .iter()
                .any(|(n, lines)| (n.x - end.x).hypot(n.y - end.y) <= tol.max(1e-9) && lines.len() == 1);
            if !is_dangle {
                continue;
            }
            let mut best: Option<(usize, f64, Coord)> = None;
            for (j, other) in segs.iter().enumerate() {
                if j == i {
                    continue;
                }
                for l in other {
                    let d = point_segment_distance(*end, l.start, l.end);
                    if best.is_none_or(|(_, bd, _)| d.0 < bd) {
                        best = Some((j, d.0, d.1));
                    }
                }
            }
            if let Some((j, d, at)) = best {
                if opts.max_undershoot_m > 0.0 && d > tol && d <= opts.max_undershoot_m {
                    report.undershoots.push(PointIssue {
                        at: unproject(*end),
                        lines: vec![i, j],
                        distance_m: Some(d),
                    });
                }
                let _ = at;
            }
            // overshoot: the stub beyond the nearest crossing on this line
            if opts.max_overshoot_m > 0.0 {
                let mut nearest_cross = f64::INFINITY;
                for (j, other) in segs.iter().enumerate() {
                    if j == i {
                        continue;
                    }
                    for a in &segs[i] {
                        for b in other {
                            if let Some(LineIntersection::SinglePoint { intersection, .. }) = line_intersection(*a, *b)
                            {
                                let d = (intersection.x - end.x).hypot(intersection.y - end.y);
                                nearest_cross = nearest_cross.min(d);
                            }
                        }
                    }
                }
                if nearest_cross.is_finite() && nearest_cross <= opts.max_overshoot_m {
                    report.overshoots.push(PointIssue {
                        at: unproject(*end),
                        lines: vec![i],
                        distance_m: Some(nearest_cross),
                    });
                }
            }
        }
    }
    Ok(report)
}

fn project_path(frame: &LocalFrame, pts: &[Coord]) -> Vec<Coord> {
    pts.iter()
        .map(|c| {
            let (x, y) = frame.project(c.x, c.y);
            Coord { x, y }
        })
        .collect()
}

/// Planar point-to-segment distance and the closest point.
fn point_segment_distance(p: Coord, a: Coord, b: Coord) -> (f64, Coord) {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len2 = dx * dx + dy * dy;
    if len2 == 0.0 {
        return ((p.x - a.x).hypot(p.y - a.y), a);
    }
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0);
    let q = Coord {
        x: a.x + t * dx,
        y: a.y + t * dy,
    };
    ((p.x - q.x).hypot(p.y - q.y), q)
}

// --------------------------------------------------------------- snapping

/// Round every coordinate to a grid of `grid_m` metres (in the local plane),
/// then drop the vertices that collapse onto each other.
pub fn snap_round(g: &Geometry, grid_m: f64) -> Result<Geometry> {
    if grid_m <= 0.0 {
        return Ok(g.clone());
    }
    let frame = LocalFrame::for_geometries([g])?;
    let snapped = {
        use geo::MapCoords;
        frame.project_geom(g).map_coords(|c| Coord {
            x: (c.x / grid_m).round() * grid_m,
            y: (c.y / grid_m).round() * grid_m,
        })
    };
    Ok(ops::clean_coords(&frame.unproject_geom(&snapped), 0.0))
}

/// Move vertices of `g` onto nearby vertices (and then edges) of `reference`,
/// so that layers that should share boundaries actually do.
///
/// Returns the snapped geometry and the number of vertices moved.
pub fn snap_to(g: &Geometry, reference: &[Geometry], tolerance_m: f64) -> Result<(Geometry, usize)> {
    if tolerance_m <= 0.0 || reference.is_empty() {
        return Ok((g.clone(), 0));
    }
    let ref_points: Vec<Coord> = reference.iter().flat_map(ops::coords_of).collect();
    let ref_lines: Vec<Vec<Coord>> = reference
        .iter()
        .map(|r| ops::lines_of(r).into_iter().flat_map(|l| [l.start, l.end]).collect())
        .collect();
    let moved = std::cell::Cell::new(0usize);

    let index = crate::index::SegmentIndex::build(
        &reference
            .iter()
            .map(|r| {
                let mut v = Vec::new();
                for l in ops::lines_of(r) {
                    if v.last() != Some(&l.start) {
                        v.push(l.start);
                    }
                    v.push(l.end);
                }
                v
            })
            .collect::<Vec<_>>(),
    );
    let _ = ref_lines;

    use geo::MapCoords;
    let out = g.map_coords(|c| {
        // vertex snap first
        let mut best: Option<(f64, Coord)> = None;
        for r in &ref_points {
            let d = measure::distance(c, *r);
            if d <= tolerance_m && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, *r));
            }
        }
        if let Some((_, target)) = best {
            if target != c {
                moved.set(moved.get() + 1);
            }
            return target;
        }
        // then edge snap
        if let Some(idx) = index.as_ref() {
            if let Some(n) = idx.nearest(c) {
                if n.dist <= tolerance_m {
                    moved.set(moved.get() + 1);
                    return n.point;
                }
            }
        }
        c
    });
    Ok((out, moved.get()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo::{line_string, polygon, Geometry};

    fn square(x0: f64, y0: f64, x1: f64, y1: f64) -> Geometry {
        Geometry::Polygon(polygon![
            (x: x0, y: y0), (x: x1, y: y0), (x: x1, y: y1), (x: x0, y: y1), (x: x0, y: y0)
        ])
    }

    #[test]
    fn finds_overlaps_and_gaps() {
        // two squares that overlap slightly, plus one that leaves a gap
        let a = square(120.0, 30.0, 120.1, 30.1);
        let b = square(120.099, 30.0, 120.2, 30.1);
        let c = square(120.0, 30.1001, 120.2, 30.2);
        // the slit between the lower squares and `c` is open at both ends
        let opts = CoverageOptions {
            gap_tolerance_m: 50.0,
            ..Default::default()
        };
        let report = coverage_issues(&[a.clone(), b.clone(), c.clone()], &opts).unwrap();
        assert_eq!(report.overlaps.len(), 1);
        assert_eq!((report.overlaps[0].a, report.overlaps[0].b), (0, 1));
        assert!(report.overlaps[0].area_m2 > 1000.0, "{}", report.overlaps[0].area_m2);
        assert_eq!(report.gaps.len(), 1, "gaps: {:?}", report.gaps.len());
        assert!(report.gaps[0].area_m2 > 10_000.0, "{}", report.gaps[0].area_m2);
        assert!(report.total_area_m2 > report.union_area_m2);

        // an enclosed hole is found without the closing step
        let ring = Geometry::Polygon(polygon!(
            exterior: [(x: 121.0, y: 30.0), (x: 121.3, y: 30.0), (x: 121.3, y: 30.3), (x: 121.0, y: 30.3), (x: 121.0, y: 30.0)],
            interiors: [[(x: 121.1, y: 30.1), (x: 121.2, y: 30.1), (x: 121.2, y: 30.2), (x: 121.1, y: 30.2), (x: 121.1, y: 30.1)]],
        ));
        let holes = coverage_issues(&[ring], &CoverageOptions::default()).unwrap();
        assert_eq!(holes.gaps.len(), 1);
        assert!(holes.gaps[0].area_m2 > 1e7);
    }

    #[test]
    fn network_dangles_pseudo_nodes_and_crossings() {
        let lines = vec![
            Geometry::LineString(line_string![(x: 0.0, y: 0.0), (x: 0.01, y: 0.0)]),
            Geometry::LineString(line_string![(x: 0.01, y: 0.0), (x: 0.02, y: 0.0)]),
            // crosses the first two without a node
            Geometry::LineString(line_string![(x: 0.005, y: -0.005), (x: 0.005, y: 0.005)]),
        ];
        let report = network_issues(&lines, &NetworkOptions::default()).unwrap();
        assert_eq!(report.pseudo_nodes.len(), 1);
        assert!(report.dangles.len() >= 4);
        assert_eq!(report.crossings_without_node.len(), 1);
        assert!(report.duplicates.is_empty());
    }

    #[test]
    fn network_duplicates_and_undershoots() {
        let lines = vec![
            Geometry::LineString(line_string![(x: 0.0, y: 0.0), (x: 0.01, y: 0.0)]),
            Geometry::LineString(line_string![(x: 0.01, y: 0.0), (x: 0.0, y: 0.0)]),
            // ends ~5 m short of the first line
            Geometry::LineString(line_string![(x: 0.005, y: -0.01), (x: 0.005, y: -0.000045)]),
        ];
        let opts = NetworkOptions {
            max_undershoot_m: 10.0,
            ..Default::default()
        };
        let report = network_issues(&lines, &opts).unwrap();
        assert_eq!(report.duplicates.len(), 1);
        assert!(report.undershoots.iter().any(|u| u.distance_m.unwrap_or(0.0) < 10.0));
    }

    #[test]
    fn snapping() {
        let g = Geometry::LineString(line_string![(x: 120.000001, y: 30.0), (x: 120.01, y: 30.000002)]);
        let snapped = snap_round(&g, 1.0).unwrap();
        let Geometry::LineString(ls) = &snapped else { panic!() };
        assert_eq!(ls.0.len(), 2);

        let reference = vec![Geometry::LineString(
            line_string![(x: 120.0, y: 30.0), (x: 120.01, y: 30.0)],
        )];
        let (moved, n) = snap_to(&g, &reference, 5.0).unwrap();
        assert!(n >= 1);
        let Geometry::LineString(ls) = &moved else { panic!() };
        assert_eq!(ls.0[0], Coord { x: 120.0, y: 30.0 });
    }
}
