//! Polygon overlay (intersection / union / difference / xor) performed in a
//! shared local transverse Mercator plane with i_overlay's integer kernel.

use geo::bool_ops::FillRule;
use geo::orient::Direction;
use geo::{unary_union, BooleanOps, BoundingRect, Geometry, MultiPolygon, OpType, Orient};

use crate::densify::{self, Edges, DEFAULT_TOL};
use crate::local::LocalFrame;
use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayOp {
    Intersection,
    Union,
    Difference,
    Xor,
}

pub fn as_multipolygon(g: &Geometry) -> Result<MultiPolygon> {
    match g {
        Geometry::Polygon(p) => Ok(MultiPolygon(vec![p.clone()])),
        Geometry::MultiPolygon(mp) => Ok(mp.clone()),
        Geometry::Rect(r) => Ok(MultiPolygon(vec![r.to_polygon()])),
        Geometry::Triangle(t) => Ok(MultiPolygon(vec![t.to_polygon()])),
        other => Err(Error::InvalidGeometry(format!(
            "overlay expects Polygon or MultiPolygon, got {}",
            geometry_type(other)
        ))),
    }
}

/// Human-readable geometry type name.
pub fn geometry_type_name(g: &Geometry) -> &'static str {
    geometry_type(g)
}

fn geometry_type(g: &Geometry) -> &'static str {
    match g {
        Geometry::Point(_) => "Point",
        Geometry::Line(_) => "Line",
        Geometry::LineString(_) => "LineString",
        Geometry::Polygon(_) => "Polygon",
        Geometry::MultiPoint(_) => "MultiPoint",
        Geometry::MultiLineString(_) => "MultiLineString",
        Geometry::MultiPolygon(_) => "MultiPolygon",
        Geometry::GeometryCollection(_) => "GeometryCollection",
        Geometry::Rect(_) => "Rect",
        Geometry::Triangle(_) => "Triangle",
    }
}

#[derive(Debug, Clone, Copy)]
pub struct OverlayOptions {
    pub edges: Edges,
    /// Maximum deviation (m) of plane chords from the true edges.
    pub tolerance: f64,
}

impl Default for OverlayOptions {
    fn default() -> Self {
        OverlayOptions {
            edges: Edges::Planar,
            tolerance: DEFAULT_TOL,
        }
    }
}

impl OverlayOptions {
    fn tol(&self) -> f64 {
        if self.tolerance > 0.0 {
            self.tolerance
        } else {
            DEFAULT_TOL
        }
    }
}

fn prepare(frame: &LocalFrame, mp: &MultiPolygon, o: &OverlayOptions) -> MultiPolygon {
    MultiPolygon(
        mp.0.iter()
            .map(|p| densify::project_polygon(frame, p, o.edges, o.tol()))
            .collect(),
    )
    .orient(Direction::Default)
}

fn finish(frame: &LocalFrame, out: &MultiPolygon, o: &OverlayOptions) -> MultiPolygon {
    let lonlat = frame.unproject_geom(out);
    // Remove vertices that only exist because of densification. The tolerance
    // covers the densification error plus i_overlay's integer snapping.
    let granularity = out
        .bounding_rect()
        .map(|r| r.width().max(r.height()) / (1u64 << 29) as f64)
        .unwrap_or(0.0);
    densify::simplify_collinear(&lonlat, o.edges, o.tol() * 1.5 + 4.0 * granularity)
}

/// True when the (padded) bounding boxes cannot overlap.
fn clearly_disjoint(a: &MultiPolygon, b: &MultiPolygon) -> bool {
    let (Some(ra), Some(rb)) = (a.bounding_rect(), b.bounding_rect()) else {
        return true;
    };
    // Geodesic edges may bulge poleward beyond the lon/lat box; pad generously.
    let pad = |r: &geo::Rect| 0.05 * r.width().max(r.height()) + 1e-6;
    let (pa, pb) = (pad(&ra), pad(&rb));
    ra.max().x + pa < rb.min().x - pb
        || rb.max().x + pb < ra.min().x - pa
        || ra.max().y + pa < rb.min().y - pb
        || rb.max().y + pb < ra.min().y - pa
}

/// Binary overlay.
pub fn overlay(a: &Geometry, b: &Geometry, op: OverlayOp, o: &OverlayOptions) -> Result<MultiPolygon> {
    let (ma, mb) = (as_multipolygon(a)?, as_multipolygon(b)?);
    if clearly_disjoint(&ma, &mb) {
        return Ok(match op {
            OverlayOp::Intersection => MultiPolygon(vec![]),
            OverlayOp::Difference => ma,
            OverlayOp::Union | OverlayOp::Xor => MultiPolygon(ma.0.into_iter().chain(mb.0).collect()),
        });
    }
    let frame = LocalFrame::for_geometries([a, b])?;
    let (pa, pb) = (prepare(&frame, &ma, o), prepare(&frame, &mb, o));
    let op = match op {
        OverlayOp::Intersection => OpType::Intersection,
        OverlayOp::Union => OpType::Union,
        OverlayOp::Difference => OpType::Difference,
        OverlayOp::Xor => OpType::Xor,
    };
    // Non-zero keeps overlapping parts of an (invalid) multipolygon filled.
    let out = pa.boolean_op_with_fill_rule(&pb, op, FillRule::NonZero);
    Ok(finish(&frame, &out, o))
}

/// Union of many polygonal geometries in one pass.
pub fn union_all(geoms: &[Geometry], o: &OverlayOptions) -> Result<MultiPolygon> {
    if geoms.is_empty() {
        return Ok(MultiPolygon(vec![]));
    }
    let frame = LocalFrame::for_geometries(geoms.iter())?;
    let polys = geoms
        .iter()
        .map(|g| as_multipolygon(g).map(|mp| prepare(&frame, &mp, o)))
        .collect::<Result<Vec<_>>>()?;
    let out = unary_union(polys.iter());
    Ok(finish(&frame, &out, o))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::measure;
    use geo::polygon;

    #[test]
    fn intersection_area_matches() {
        let a = Geometry::Polygon(
            polygon![(x: 120.0, y: 30.0), (x: 121.0, y: 30.0), (x: 121.0, y: 31.0), (x: 120.0, y: 31.0)],
        );
        let b = Geometry::Polygon(
            polygon![(x: 120.5, y: 30.0), (x: 121.5, y: 30.0), (x: 121.5, y: 31.0), (x: 120.5, y: 31.0)],
        );
        let o = OverlayOptions::default();
        let i = overlay(&a, &b, OverlayOp::Intersection, &o).unwrap();
        let u = overlay(&a, &b, OverlayOp::Union, &o).unwrap();
        let d = overlay(&a, &b, OverlayOp::Difference, &o).unwrap();
        // Planar semantics: the intersection is exactly the lon/lat rectangle.
        assert_eq!(i.0[0].exterior().0.len(), 5, "{:?}", i.0[0].exterior());
        for c in &i.0[0].exterior().0 {
            assert!((c.x - 120.5).abs() < 1e-9 || (c.x - 121.0).abs() < 1e-9, "{c:?}");
        }
        let pa = |g: &Geometry| measure::area(&densify::prepare(g, Edges::Planar, 0.001));
        let (aa, ab) = (pa(&a), pa(&b));
        let (ai, au, ad) = (
            pa(&Geometry::MultiPolygon(i)),
            pa(&Geometry::MultiPolygon(u)),
            pa(&Geometry::MultiPolygon(d)),
        );
        assert!((au - (aa + ab - ai)).abs() / au < 1e-6, "{au} vs {}", aa + ab - ai);
        assert!((ad - (aa - ai)).abs() / ad < 1e-6);
    }
}
