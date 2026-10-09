//! Geometry validity checks and automatic repair.
//!
//! Two layers:
//! * OGC simple-feature validity (self-intersections, rings that do not close,
//!   holes outside their shell, overlapping parts) — checked with `geo`'s
//!   validation, which returns the offending ring or coordinate.
//! * GeoJSON conventions on top of it (RFC 7946 winding, coordinate ranges,
//!   repeated vertices, spikes, antimeridian crossings).
//!
//! [`make_valid`] applies the repairs that can be made safely and reports what
//! it changed.

use geo::algorithm::validation::Validation;
use geo::orient::Direction;
use geo::{BooleanOps, Coord, Geometry, GeometryCollection, LineString, MultiPolygon, Orient, Polygon};
use serde::{Deserialize, Serialize};

use crate::measure;
use crate::ops;
use crate::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// The geometry is invalid under the OGC simple-feature rules.
    Error,
    /// Usable, but likely a data problem (or a convention violation).
    Warning,
}

#[derive(Debug, Clone, Serialize)]
pub struct Issue {
    /// Machine-readable identifier, e.g. `self-intersection`.
    pub code: &'static str,
    pub message: String,
    pub severity: Severity,
    /// Index of the part inside a multi-geometry or collection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<usize>,
    /// Index of the ring inside a polygon (0 = exterior).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ring: Option<usize>,
    /// Index of the vertex, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<usize>,
    /// Where the problem is, when a single position describes it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<[f64; 2]>,
}

impl Issue {
    fn new(code: &'static str, message: impl Into<String>, severity: Severity) -> Issue {
        Issue {
            code,
            message: message.into(),
            severity,
            part: None,
            ring: None,
            index: None,
            at: None,
        }
    }

    fn at(mut self, c: Coord) -> Issue {
        self.at = Some([c.x, c.y]);
        self
    }

    fn part(mut self, p: usize) -> Issue {
        self.part = Some(p);
        self
    }

    fn ring(mut self, r: usize) -> Issue {
        self.ring = Some(r);
        self
    }

    fn index(mut self, i: usize) -> Issue {
        self.index = Some(i);
        self
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ValidateOptions {
    /// Flag vertices whose turn leaves a zero-area spike.
    pub check_spikes: bool,
    /// Warn when a ring is not wound per RFC 7946 (exterior CCW, holes CW).
    pub check_winding: bool,
    /// Warn about edges crossing the antimeridian (unsupported in v1).
    pub check_antimeridian: bool,
    /// Distance (m) under which two consecutive vertices count as repeated.
    pub duplicate_tolerance: f64,
}

impl Default for ValidateOptions {
    fn default() -> Self {
        ValidateOptions {
            check_spikes: true,
            check_winding: true,
            check_antimeridian: true,
            duplicate_tolerance: 0.0,
        }
    }
}

/// Check one geometry, returning every problem found.
pub fn validate(g: &Geometry, opts: &ValidateOptions) -> Vec<Issue> {
    let mut issues = Vec::new();
    coordinate_checks(g, opts, &mut issues);
    ogc_checks(g, &mut issues);
    structure_checks(g, opts, &mut issues, None);
    issues
}

fn coordinate_checks(g: &Geometry, opts: &ValidateOptions, out: &mut Vec<Issue>) {
    for (i, c) in ops::coords_of(g).into_iter().enumerate() {
        if !c.x.is_finite() || !c.y.is_finite() {
            out.push(Issue::new("non-finite", "coordinate is not a finite number", Severity::Error).index(i));
            continue;
        }
        if !(-180.0..=180.0).contains(&c.x) || !(-90.0..=90.0).contains(&c.y) {
            out.push(
                Issue::new(
                    "out-of-range",
                    format!("coordinate {c:?} is outside lon/lat range"),
                    Severity::Error,
                )
                .index(i)
                .at(c),
            );
        }
    }
    if opts.check_antimeridian {
        for l in ops::lines_of(g) {
            if (l.end.x - l.start.x).abs() > 180.0 {
                out.push(
                    Issue::new(
                        "antimeridian-crossing",
                        "edge crosses the antimeridian; split the geometry first",
                        Severity::Warning,
                    )
                    .at(l.start),
                );
                break;
            }
        }
    }
}

fn ogc_checks(g: &Geometry, out: &mut Vec<Issue>) {
    for e in g.validation_errors() {
        let text = e.to_string();
        let code = if text.contains("self-intersection") {
            "self-intersection"
        } else if text.contains("at least") {
            "too-few-points"
        } else if text.contains("contained") {
            "hole-outside-shell"
        } else if text.contains("non-finite") {
            "non-finite"
        } else if text.contains("intersect") {
            "rings-intersect"
        } else {
            "invalid-geometry"
        };
        out.push(Issue::new(code, text, Severity::Error));
    }
}

fn ring_checks(r: &LineString, role: usize, part: Option<usize>, opts: &ValidateOptions, out: &mut Vec<Issue>) {
    let pts = &r.0;
    if pts.len() < 4 {
        let mut i = Issue::new("too-few-points", "a ring needs at least 4 positions", Severity::Error).ring(role);
        if let Some(p) = part {
            i = i.part(p);
        }
        out.push(i);
        return;
    }
    if pts.first() != pts.last() {
        let mut i = Issue::new("ring-not-closed", "first and last position differ", Severity::Error).ring(role);
        if let Some(p) = part {
            i = i.part(p);
        }
        out.push(i);
    }
    duplicate_and_spike_checks(pts, role, part, opts, out, true);
    if opts.check_winding {
        let signed = measure::planar_ring_signed_area(r);
        let ccw = signed > 0.0;
        let expected_ccw = role == 0;
        if signed != 0.0 && ccw != expected_ccw {
            let mut i = Issue::new(
                "wrong-winding",
                if expected_ccw {
                    "exterior ring should be counter-clockwise (RFC 7946)"
                } else {
                    "interior ring should be clockwise (RFC 7946)"
                },
                Severity::Warning,
            )
            .ring(role);
            if let Some(p) = part {
                i = i.part(p);
            }
            out.push(i);
        }
        if signed == 0.0 {
            let mut i = Issue::new("zero-area-ring", "ring encloses no area", Severity::Error).ring(role);
            if let Some(p) = part {
                i = i.part(p);
            }
            out.push(i);
        }
    }
}

fn duplicate_and_spike_checks(
    pts: &[Coord],
    role: usize,
    part: Option<usize>,
    opts: &ValidateOptions,
    out: &mut Vec<Issue>,
    closed: bool,
) {
    for (i, w) in pts.windows(2).enumerate() {
        let repeated = if opts.duplicate_tolerance > 0.0 {
            measure::distance(w[0], w[1]) <= opts.duplicate_tolerance
        } else {
            w[0] == w[1]
        };
        if repeated {
            let mut issue = Issue::new(
                "repeated-point",
                "consecutive positions are identical",
                Severity::Warning,
            )
            .index(i)
            .at(w[0]);
            if let Some(p) = part {
                issue = issue.part(p);
            }
            out.push(issue.ring(role));
        }
    }
    if opts.check_spikes && pts.len() >= 3 {
        let n = if closed && pts.first() == pts.last() {
            pts.len() - 1
        } else {
            pts.len()
        };
        for i in 0..n {
            let (prev, cur, next) = if closed {
                (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n])
            } else {
                if i == 0 || i + 1 >= n {
                    continue;
                }
                (pts[i - 1], pts[i], pts[i + 1])
            };
            if prev == cur || cur == next {
                continue;
            }
            let b1 = measure::bearing(cur, prev, false);
            let b2 = measure::bearing(cur, next, false);
            let turn = crate::geodesic::normalize_deg(b2 - b1).abs();
            if turn < 1e-6 {
                let mut issue = Issue::new(
                    "spike",
                    "vertex doubles back on itself (zero-width spike)",
                    Severity::Warning,
                )
                .index(i)
                .at(cur)
                .ring(role);
                if let Some(p) = part {
                    issue = issue.part(p);
                }
                out.push(issue);
            }
        }
    }
}

fn structure_checks(g: &Geometry, opts: &ValidateOptions, out: &mut Vec<Issue>, part: Option<usize>) {
    match g {
        Geometry::LineString(ls) => {
            if ls.0.len() < 2 {
                out.push(Issue::new(
                    "too-few-points",
                    "a line needs at least 2 positions",
                    Severity::Error,
                ));
            }
            duplicate_and_spike_checks(&ls.0, 0, part, opts, out, false);
            if self_intersects(ls) {
                out.push(Issue::new(
                    "line-self-intersection",
                    "line crosses itself",
                    Severity::Warning,
                ));
            }
        }
        Geometry::MultiLineString(m) => {
            for (i, ls) in m.0.iter().enumerate() {
                structure_checks(&Geometry::LineString(ls.clone()), opts, out, Some(i));
            }
        }
        Geometry::Polygon(p) => {
            ring_checks(p.exterior(), 0, part, opts, out);
            for (i, r) in p.interiors().iter().enumerate() {
                ring_checks(r, i + 1, part, opts, out);
            }
        }
        Geometry::MultiPolygon(mp) => {
            for (i, p) in mp.0.iter().enumerate() {
                structure_checks(&Geometry::Polygon(p.clone()), opts, out, Some(i));
            }
        }
        Geometry::GeometryCollection(gc) => {
            for (i, g) in gc.0.iter().enumerate() {
                structure_checks(g, opts, out, Some(i));
            }
        }
        _ => {}
    }
}

/// Does a line cross itself (ignoring shared endpoints of adjacent segments)?
pub fn self_intersects(ls: &LineString) -> bool {
    use geo::line_intersection::{line_intersection, LineIntersection};
    let lines: Vec<_> = ls.lines().collect();
    for i in 0..lines.len() {
        for j in (i + 2)..lines.len() {
            // adjacent segments share an endpoint by construction
            if i == 0 && j == lines.len() - 1 && lines[i].start == lines[j].end {
                continue;
            }
            match line_intersection(lines[i], lines[j]) {
                Some(LineIntersection::SinglePoint { is_proper: true, .. }) => return true,
                Some(LineIntersection::SinglePoint { .. }) => {}
                Some(LineIntersection::Collinear { .. }) => return true,
                None => {}
            }
        }
    }
    false
}

/// Every point where a line or ring crosses itself (turf's `kinks`).
///
/// Collinear overlaps report their two shared endpoints, which is what lets a
/// doubled-back segment show up at all.
pub fn self_intersections(g: &Geometry) -> Vec<Coord> {
    use geo::line_intersection::{line_intersection, LineIntersection};
    let mut out: Vec<Coord> = Vec::new();
    let mut push = |c: Coord| {
        const Q: f64 = 1e9;
        let k = |v: f64| (v * Q).round();
        if !out.iter().any(|o| k(o.x) == k(c.x) && k(o.y) == k(c.y)) {
            out.push(c);
        }
    };
    let mut scan = |ls: &LineString| {
        let lines: Vec<_> = ls.lines().collect();
        for i in 0..lines.len() {
            for j in (i + 2)..lines.len() {
                if i == 0 && j == lines.len() - 1 && lines[i].start == lines[j].end {
                    continue;
                }
                match line_intersection(lines[i], lines[j]) {
                    Some(LineIntersection::SinglePoint {
                        intersection,
                        is_proper: true,
                    }) => push(intersection),
                    Some(LineIntersection::Collinear { intersection }) => {
                        push(intersection.start);
                        push(intersection.end);
                    }
                    _ => {}
                }
            }
        }
    };
    for ls in crate::ops::line_strings_of(g) {
        scan(&ls);
    }
    out
}

// ------------------------------------------------------------------ repair

#[derive(Debug, Clone, Copy)]
pub struct MakeValidOptions {
    /// Snap coordinates to this grid (metres) before repairing. 0 disables.
    pub snap_grid_m: f64,
    /// Drop rings / parts smaller than this area (m²).
    pub min_area_m2: f64,
    /// Remove vertices that deviate less than this (metres) from their neighbours.
    pub clean_tolerance_m: f64,
}

impl Default for MakeValidOptions {
    fn default() -> Self {
        MakeValidOptions {
            snap_grid_m: 0.0,
            min_area_m2: 0.0,
            clean_tolerance_m: 0.0,
        }
    }
}

/// Repair a geometry, returning the result and a list of the fixes applied.
pub fn make_valid(g: &Geometry, opts: &MakeValidOptions) -> Result<(Geometry, Vec<String>)> {
    let mut log: Vec<String> = Vec::new();
    let mut current = g.clone();

    if opts.snap_grid_m > 0.0 {
        current = crate::topology::snap_round(&current, opts.snap_grid_m)?;
        log.push(format!("snapped coordinates to a {} m grid", opts.snap_grid_m));
    }

    let cleaned = ops::clean_coords(&current, opts.clean_tolerance_m);
    if count_coords(&cleaned) != count_coords(&current) {
        log.push(format!(
            "removed {} repeated or collinear vertices",
            count_coords(&current) - count_coords(&cleaned)
        ));
    }
    current = cleaned;

    current = close_rings(&current, &mut log);

    let repaired = match &current {
        Geometry::Polygon(_) | Geometry::MultiPolygon(_) => {
            let before = validate(&current, &ValidateOptions::default())
                .into_iter()
                .filter(|i| i.severity == Severity::Error)
                .count();
            if before > 0 {
                let fixed = repair_areal(&current)?;
                log.push(format!("resolved {before} validity error(s) in the polygon topology"));
                fixed
            } else {
                current.clone()
            }
        }
        other => other.clone(),
    };
    current = repaired;

    if opts.min_area_m2 > 0.0 {
        if let Geometry::MultiPolygon(mp) = &current {
            let kept: Vec<Polygon> =
                mp.0.iter()
                    .filter(|p| measure::planar_ring_area(p.exterior()) >= opts.min_area_m2)
                    .cloned()
                    .collect();
            if kept.len() != mp.0.len() {
                log.push(format!(
                    "dropped {} part(s) below the minimum area",
                    mp.0.len() - kept.len()
                ));
            }
            current = Geometry::MultiPolygon(MultiPolygon(kept));
        }
    }

    let oriented = match &current {
        Geometry::Polygon(p) => Geometry::Polygon(p.orient(Direction::Default)),
        Geometry::MultiPolygon(mp) => Geometry::MultiPolygon(mp.orient(Direction::Default)),
        other => other.clone(),
    };
    if !same_coords(&oriented, &current) {
        log.push("re-wound rings to the RFC 7946 orientation".into());
    }
    Ok((oriented, log))
}

fn repair_areal(g: &Geometry) -> Result<Geometry> {
    // A self-union under the non-zero rule resolves self-intersections,
    // overlapping parts and holes that poke outside their shell.
    let mp = match g {
        Geometry::Polygon(p) => MultiPolygon(vec![p.clone()]),
        Geometry::MultiPolygon(mp) => mp.clone(),
        _ => return Ok(g.clone()),
    };
    let oriented = mp.orient(Direction::Default);
    let unioned = oriented.union(&MultiPolygon(vec![]));
    Ok(Geometry::MultiPolygon(unioned))
}

fn close_rings(g: &Geometry, log: &mut Vec<String>) -> Geometry {
    fn close(r: &LineString, log: &mut Vec<String>) -> LineString {
        let mut pts = r.0.clone();
        if pts.len() >= 3 && pts.first() != pts.last() {
            if let Some(first) = pts.first().copied() {
                pts.push(first);
                log.push("closed a ring".into());
            }
        }
        LineString(pts)
    }
    fn poly(p: &Polygon, log: &mut Vec<String>) -> Polygon {
        Polygon::new(
            close(p.exterior(), log),
            p.interiors().iter().map(|r| close(r, log)).collect(),
        )
    }
    match g {
        Geometry::Polygon(p) => Geometry::Polygon(poly(p, log)),
        Geometry::MultiPolygon(mp) => Geometry::MultiPolygon(MultiPolygon(mp.0.iter().map(|p| poly(p, log)).collect())),
        Geometry::GeometryCollection(gc) => {
            Geometry::GeometryCollection(GeometryCollection(gc.0.iter().map(|g| close_rings(g, log)).collect()))
        }
        other => other.clone(),
    }
}

fn count_coords(g: &Geometry) -> usize {
    ops::coords_of(g).len()
}

fn same_coords(a: &Geometry, b: &Geometry) -> bool {
    ops::coords_of(a) == ops::coords_of(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo::{line_string, polygon};

    #[test]
    fn detects_self_intersection_and_repairs_it() {
        // bow-tie
        let p = polygon![(x: 0.0, y: 0.0), (x: 2.0, y: 2.0), (x: 2.0, y: 0.0), (x: 0.0, y: 2.0), (x: 0.0, y: 0.0)];
        let g = Geometry::Polygon(p);
        let issues = validate(&g, &ValidateOptions::default());
        assert!(issues.iter().any(|i| i.code == "self-intersection"), "{issues:?}");
        let (fixed, log) = make_valid(&g, &MakeValidOptions::default()).unwrap();
        assert!(!log.is_empty());
        let after = validate(&fixed, &ValidateOptions::default());
        assert!(
            after.iter().all(|i| i.severity != Severity::Error),
            "still invalid: {after:?}"
        );
        // the bow-tie becomes two triangles
        let Geometry::MultiPolygon(mp) = &fixed else {
            panic!("{fixed:?}")
        };
        assert_eq!(mp.0.len(), 2);
    }

    #[test]
    fn winding_and_duplicates_are_warnings() {
        let p = polygon![(x: 0.0, y: 0.0), (x: 0.0, y: 1.0), (x: 1.0, y: 1.0), (x: 1.0, y: 1.0), (x: 1.0, y: 0.0), (x: 0.0, y: 0.0)];
        let g = Geometry::Polygon(p);
        let issues = validate(&g, &ValidateOptions::default());
        assert!(issues.iter().any(|i| i.code == "wrong-winding"));
        assert!(issues.iter().any(|i| i.code == "repeated-point"));
        assert!(issues.iter().all(|i| i.severity == Severity::Warning));
        let (fixed, log) = make_valid(&g, &MakeValidOptions::default()).unwrap();
        assert!(log.iter().any(|l| l.contains("repeated")));
        assert!(
            validate(&fixed, &ValidateOptions::default()).is_empty(),
            "{:?}",
            validate(&fixed, &ValidateOptions::default())
        );
    }

    #[test]
    fn hole_outside_shell_is_an_error() {
        let p = polygon!(
            exterior: [(x: 0.0, y: 0.0), (x: 1.0, y: 0.0), (x: 1.0, y: 1.0), (x: 0.0, y: 1.0), (x: 0.0, y: 0.0)],
            interiors: [[(x: 2.0, y: 2.0), (x: 3.0, y: 2.0), (x: 3.0, y: 3.0), (x: 2.0, y: 2.0)]],
        );
        let issues = validate(&Geometry::Polygon(p), &ValidateOptions::default());
        assert!(issues.iter().any(|i| i.severity == Severity::Error), "{issues:?}");
    }

    #[test]
    fn line_checks() {
        let ls = line_string![(x: 0.0, y: 0.0), (x: 2.0, y: 2.0), (x: 2.0, y: 0.0), (x: 0.0, y: 2.0)];
        let issues = validate(&Geometry::LineString(ls), &ValidateOptions::default());
        assert!(issues.iter().any(|i| i.code == "line-self-intersection"));
        let short = line_string![(x: 0.0, y: 0.0)];
        let issues = validate(&Geometry::LineString(short), &ValidateOptions::default());
        assert!(issues.iter().any(|i| i.code == "too-few-points"));
    }

    #[test]
    fn out_of_range_and_antimeridian() {
        let ls = line_string![(x: 179.0, y: 10.0), (x: -179.0, y: 10.0)];
        let issues = validate(&Geometry::LineString(ls), &ValidateOptions::default());
        assert!(issues.iter().any(|i| i.code == "antimeridian-crossing"));
        let ls = line_string![(x: 200.0, y: 10.0), (x: 10.0, y: 100.0)];
        let issues = validate(&Geometry::LineString(ls), &ValidateOptions::default());
        assert_eq!(issues.iter().filter(|i| i.code == "out-of-range").count(), 2);
    }
}
