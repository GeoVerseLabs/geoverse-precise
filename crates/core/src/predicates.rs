//! Spatial predicates.
//!
//! Relations are topological, so they are evaluated in lon/lat with edges
//! interpreted as straight lines — the GeoJSON (RFC 7946) convention used
//! throughout this crate. Pass `Edges::Geodesic` to have long edges densified
//! along geodesics first, which matters only when edges span degrees.

use geo::algorithm::dimensions::Dimensions;
use geo::coordinate_position::CoordPos;
use geo::relate::IntersectionMatrix;
use geo::{Contains, Coord, CoordinatePosition, Geometry, Intersects, Relate};

use crate::densify::{self, Edges, DEFAULT_TOL};
use crate::{Error, Result};

pub fn point_in_polygon(p: Coord, g: &Geometry, ignore_boundary: bool) -> bool {
    match g.coordinate_position(&p) {
        CoordPos::Inside => true,
        CoordPos::OnBoundary => !ignore_boundary,
        CoordPos::Outside => false,
    }
}

pub fn intersects(a: &Geometry, b: &Geometry) -> bool {
    a.intersects(b)
}

pub fn contains(a: &Geometry, b: &Geometry) -> bool {
    a.contains(b)
}

/// The nine-character DE-9IM matrix of two geometries, e.g. `"212101212"`.
pub fn relate_matrix(a: &Geometry, b: &Geometry, edges: Edges) -> String {
    let (a, b) = prepare_pair(a, b, edges);
    matrix_string(&a.relate(&b))
}

fn prepare_pair(a: &Geometry, b: &Geometry, edges: Edges) -> (Geometry, Geometry) {
    match edges {
        Edges::Planar => (a.clone(), b.clone()),
        Edges::Geodesic => (
            densify::prepare(a, Edges::Geodesic, DEFAULT_TOL),
            densify::prepare(b, Edges::Geodesic, DEFAULT_TOL),
        ),
    }
}

fn dim_char(d: Dimensions) -> char {
    match d {
        Dimensions::Empty => 'F',
        Dimensions::ZeroDimensional => '0',
        Dimensions::OneDimensional => '1',
        Dimensions::TwoDimensional => '2',
    }
}

fn matrix_string(m: &IntersectionMatrix) -> String {
    let pos = [CoordPos::Inside, CoordPos::OnBoundary, CoordPos::Outside];
    let mut s = String::with_capacity(9);
    for a in pos {
        for b in pos {
            s.push(dim_char(m.get(a, b)));
        }
    }
    s
}

/// The named relations of the OGC simple-feature model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Predicate {
    Intersects,
    Disjoint,
    Contains,
    Within,
    Covers,
    CoveredBy,
    Touches,
    Crosses,
    Overlaps,
    Equals,
}

impl Predicate {
    pub fn parse(s: &str) -> Result<Predicate> {
        Ok(match s.trim().to_ascii_lowercase().replace('-', "").as_str() {
            "intersects" => Predicate::Intersects,
            "disjoint" => Predicate::Disjoint,
            "contains" => Predicate::Contains,
            "within" => Predicate::Within,
            "covers" => Predicate::Covers,
            "coveredby" => Predicate::CoveredBy,
            "touches" => Predicate::Touches,
            "crosses" => Predicate::Crosses,
            "overlaps" => Predicate::Overlaps,
            "equals" | "equal" => Predicate::Equals,
            other => return Err(Error::InvalidArgument(format!("unknown predicate `{other}`"))),
        })
    }
}

/// Evaluate a named relation.
pub fn evaluate(a: &Geometry, b: &Geometry, pred: Predicate, edges: Edges) -> bool {
    let (a, b) = prepare_pair(a, b, edges);
    let m = a.relate(&b);
    match pred {
        Predicate::Intersects => m.is_intersects(),
        Predicate::Disjoint => m.is_disjoint(),
        Predicate::Contains => m.is_contains(),
        Predicate::Within => m.is_within(),
        Predicate::Covers => m.is_covers(),
        Predicate::CoveredBy => b.relate(&a).is_covers(),
        Predicate::Touches => m.is_touches(),
        Predicate::Crosses => m.is_crosses(),
        Predicate::Overlaps => m.is_overlaps(),
        Predicate::Equals => m.is_equal_topo(),
    }
}

/// Test a DE-9IM pattern such as `"T*F**F***"`.
///
/// `0`, `1`, `2` require that exact dimension, `T` any intersection, `F` none,
/// `*` anything.
pub fn matches_pattern(a: &Geometry, b: &Geometry, pattern: &str, edges: Edges) -> Result<bool> {
    let pattern: Vec<char> = pattern.trim().chars().collect();
    if pattern.len() != 9 {
        return Err(Error::InvalidArgument("a DE-9IM pattern has 9 characters".into()));
    }
    let actual: Vec<char> = relate_matrix(a, b, edges).chars().collect();
    for (p, a) in pattern.iter().zip(actual.iter()) {
        let ok = match p.to_ascii_uppercase() {
            '*' => true,
            'F' => *a == 'F',
            'T' => *a != 'F',
            '0' | '1' | '2' => *a == *p,
            other => return Err(Error::InvalidArgument(format!("bad pattern character `{other}`"))),
        };
        if !ok {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Is the point on the line (within `tolerance_m`)?
pub fn point_on_line(
    p: Coord,
    lines: &geo::MultiLineString,
    tolerance_m: f64,
    ignore_ends: bool,
    edges: Edges,
) -> bool {
    let dense;
    let lines = match edges {
        Edges::Geodesic => lines,
        Edges::Planar => {
            dense = geo::MultiLineString(
                lines
                    .0
                    .iter()
                    .map(|l| geo::LineString(densify::planar_path(&l.0, tolerance_m.max(DEFAULT_TOL) * 0.5)))
                    .collect(),
            );
            &dense
        }
    };
    let Some(n) = crate::measure::nearest_point_on_line_opts(lines, p, false) else {
        return false;
    };
    if n.dist > tolerance_m {
        return false;
    }
    if ignore_ends {
        for ls in &lines.0 {
            if let (Some(a), Some(b)) = (ls.0.first(), ls.0.last()) {
                if crate::measure::distance(*a, p) <= tolerance_m || crate::measure::distance(*b, p) <= tolerance_m {
                    return false;
                }
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo::{line_string, point, polygon, Geometry};

    fn poly(x0: f64, y0: f64, x1: f64, y1: f64) -> Geometry {
        Geometry::Polygon(polygon![(x: x0, y: y0), (x: x1, y: y0), (x: x1, y: y1), (x: x0, y: y1), (x: x0, y: y0)])
    }

    #[test]
    fn named_relations() {
        let a = poly(0.0, 0.0, 2.0, 2.0);
        let b = poly(0.5, 0.5, 1.5, 1.5);
        let c = poly(2.0, 0.0, 4.0, 2.0);
        let d = poly(1.0, 1.0, 3.0, 3.0);
        for (x, y, pred, expect) in [
            (&a, &b, Predicate::Contains, true),
            (&b, &a, Predicate::Within, true),
            (&a, &c, Predicate::Touches, true),
            (&a, &c, Predicate::Intersects, true),
            (&a, &c, Predicate::Overlaps, false),
            (&a, &d, Predicate::Overlaps, true),
            (&a, &a, Predicate::Equals, true),
            (&b, &c, Predicate::Disjoint, true),
        ] {
            assert_eq!(evaluate(x, y, pred, Edges::Planar), expect, "{pred:?}");
        }
        let line = Geometry::LineString(line_string![(x: -1.0, y: 1.0), (x: 3.0, y: 1.0)]);
        assert!(evaluate(&line, &a, Predicate::Crosses, Edges::Planar));
    }

    #[test]
    fn de9im_matrix_and_patterns() {
        let a = poly(0.0, 0.0, 2.0, 2.0);
        let b = poly(0.5, 0.5, 1.5, 1.5);
        let m = relate_matrix(&a, &b, Edges::Planar);
        assert_eq!(m.len(), 9);
        assert!(matches_pattern(&a, &b, "T*****FF*", Edges::Planar).unwrap(), "{m}");
        assert!(!matches_pattern(&a, &b, "FF*FF****", Edges::Planar).unwrap());
        assert!(matches_pattern(&a, &b, "*********", Edges::Planar).unwrap());
        assert!(matches_pattern(&a, &b, "T********", Edges::Planar).unwrap());
        assert!(matches_pattern(&a, &b, "bad", Edges::Planar).is_err());
    }

    #[test]
    fn point_on_line_tolerance() {
        let lines = geo::MultiLineString(vec![line_string![(x: 120.0, y: 30.0), (x: 121.0, y: 30.0)]]);
        // planar edges: the point lies on the parallel, so it is on the line
        assert!(point_on_line(
            point!(x: 120.5, y: 30.0).0,
            &lines,
            0.01,
            false,
            Edges::Planar
        ));
        // geodesic edges: the geodesic bulges ~130 m north of the parallel here
        assert!(!point_on_line(
            point!(x: 120.5, y: 30.0).0,
            &lines,
            1.0,
            false,
            Edges::Geodesic
        ));
        assert!(point_on_line(
            point!(x: 120.5, y: 30.0).0,
            &lines,
            200.0,
            false,
            Edges::Geodesic
        ));
        assert!(!point_on_line(
            point!(x: 120.5, y: 30.1).0,
            &lines,
            1.0,
            false,
            Edges::Planar
        ));
        assert!(!point_on_line(
            point!(x: 120.0, y: 30.0).0,
            &lines,
            1.0,
            true,
            Edges::Planar
        ));
    }
}
