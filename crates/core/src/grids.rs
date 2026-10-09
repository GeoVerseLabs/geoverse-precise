//! Metric grids: point, square, rectangle, triangle and hexagon tilings.
//!
//! Cell sizes are given in metres and laid out in a local transverse Mercator
//! plane, so cells keep their real size instead of stretching with latitude the
//! way a degree-based grid does. An optional mask keeps only the cells that
//! touch a polygon.

use geo::{Coord, Geometry, LineString, MultiPoint, Point, Polygon};

use crate::index::Prepared;
use crate::local::LocalFrame;
use crate::{ops, Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridKind {
    Point,
    Square,
    Rectangle,
    Triangle,
    Hex,
}

impl GridKind {
    pub fn parse(s: &str) -> Result<GridKind> {
        Ok(match s.trim().to_ascii_lowercase().as_str() {
            "point" | "points" => GridKind::Point,
            "square" => GridKind::Square,
            "rectangle" | "rect" => GridKind::Rectangle,
            "triangle" => GridKind::Triangle,
            "hex" | "hexagon" => GridKind::Hex,
            other => return Err(Error::InvalidArgument(format!("unknown grid kind `{other}`"))),
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct GridOptions {
    /// Cell width (m). For squares, triangles and hexagons this is the cell side.
    pub width_m: f64,
    /// Cell height (m); only used by the rectangle grid.
    pub height_m: f64,
    /// Keep only cells that intersect the mask (or points inside it).
    pub mask_all_touched: bool,
}

impl Default for GridOptions {
    fn default() -> Self {
        GridOptions {
            width_m: 1000.0,
            height_m: 1000.0,
            mask_all_touched: true,
        }
    }
}

/// Build a grid covering `bbox` (`[min_x, min_y, max_x, max_y]` in lon/lat).
pub fn grid(bbox: [f64; 4], kind: GridKind, opts: &GridOptions, mask: Option<&Geometry>) -> Result<Vec<Geometry>> {
    let bad = |v: f64| !v.is_finite() || v <= 0.0;
    if bad(opts.width_m) || (kind == GridKind::Rectangle && bad(opts.height_m)) {
        return Err(Error::InvalidArgument("grid cell size must be positive".into()));
    }
    let centre = Coord {
        x: 0.5 * (bbox[0] + bbox[2]),
        y: 0.5 * (bbox[1] + bbox[3]),
    };
    let frame = LocalFrame::new(centre.x, centre.y);
    // project the bbox corners and take their extent in the plane
    let corners = [
        frame.project(bbox[0], bbox[1]),
        frame.project(bbox[2], bbox[1]),
        frame.project(bbox[2], bbox[3]),
        frame.project(bbox[0], bbox[3]),
    ];
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for (x, y) in corners {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }

    let prepared = match mask {
        Some(g) => Some(Prepared::new(g.clone())?),
        None => None,
    };
    let unproject = |c: Coord| {
        let (x, y) = frame.unproject(c.x, c.y);
        Coord { x, y }
    };
    let keep_polygon = |ring: &[Coord]| -> Option<Geometry> {
        let pts: Vec<Coord> = ring.iter().map(|c| unproject(*c)).collect();
        let mut ring = pts;
        if ring.first() != ring.last() {
            ring.push(ring[0]);
        }
        let poly = Polygon::new(LineString(ring), vec![]);
        match &prepared {
            None => Some(Geometry::Polygon(poly)),
            Some(p) => {
                let touches = poly.exterior().0.iter().any(|c| p.contains_point(*c, false))
                    || crate::predicates::intersects(&Geometry::Polygon(poly.clone()), &p.geometry);
                if touches || !opts.mask_all_touched {
                    let inside_all = poly.exterior().0.iter().all(|c| p.contains_point(*c, false));
                    if opts.mask_all_touched || inside_all {
                        return Some(Geometry::Polygon(poly));
                    }
                }
                None
            }
        }
    };

    let mut out: Vec<Geometry> = Vec::new();
    match kind {
        GridKind::Point => {
            let step = opts.width_m;
            let (nx, ny) = (((x1 - x0) / step).floor() as i64, ((y1 - y0) / step).floor() as i64);
            for j in 0..=ny.max(0) {
                for i in 0..=nx.max(0) {
                    let c = unproject(Coord {
                        x: x0 + i as f64 * step,
                        y: y0 + j as f64 * step,
                    });
                    let keep = prepared.as_ref().is_none_or(|p| p.contains_point(c, false));
                    if keep {
                        out.push(Geometry::Point(Point(c)));
                    }
                }
            }
        }
        GridKind::Square | GridKind::Rectangle => {
            let (w, h) = if kind == GridKind::Square {
                (opts.width_m, opts.width_m)
            } else {
                (opts.width_m, opts.height_m)
            };
            let (nx, ny) = (((x1 - x0) / w).floor() as i64, ((y1 - y0) / h).floor() as i64);
            for j in 0..ny.max(0) {
                for i in 0..nx.max(0) {
                    let (cx, cy) = (x0 + i as f64 * w, y0 + j as f64 * h);
                    let ring = [
                        Coord { x: cx, y: cy },
                        Coord { x: cx + w, y: cy },
                        Coord { x: cx + w, y: cy + h },
                        Coord { x: cx, y: cy + h },
                    ];
                    if let Some(g) = keep_polygon(&ring) {
                        out.push(g);
                    }
                }
            }
        }
        GridKind::Triangle => {
            let s = opts.width_m;
            let (nx, ny) = (((x1 - x0) / s).floor() as i64, ((y1 - y0) / s).floor() as i64);
            for j in 0..ny.max(0) {
                for i in 0..nx.max(0) {
                    let (cx, cy) = (x0 + i as f64 * s, y0 + j as f64 * s);
                    let a = Coord { x: cx, y: cy };
                    let b = Coord { x: cx + s, y: cy };
                    let c = Coord { x: cx + s, y: cy + s };
                    let d = Coord { x: cx, y: cy + s };
                    for ring in [[a, b, c], [a, c, d]] {
                        if let Some(g) = keep_polygon(&ring) {
                            out.push(g);
                        }
                    }
                }
            }
        }
        GridKind::Hex => {
            // flat-top hexagons with side length `width_m`
            let s = opts.width_m;
            let dx = 1.5 * s;
            let dy = 3f64.sqrt() * s;
            let nx = ((x1 - x0) / dx).floor() as i64;
            let ny = ((y1 - y0) / dy).floor() as i64;
            for i in 0..=nx.max(0) {
                for j in 0..=ny.max(0) {
                    let cx = x0 + i as f64 * dx;
                    let cy = y0 + j as f64 * dy + if i % 2 == 0 { 0.0 } else { dy / 2.0 };
                    let ring: Vec<Coord> = (0..6)
                        .map(|k| {
                            let a = std::f64::consts::PI / 3.0 * k as f64;
                            Coord {
                                x: cx + s * a.cos(),
                                y: cy + s * a.sin(),
                            }
                        })
                        .collect();
                    if let Some(g) = keep_polygon(&ring) {
                        out.push(g);
                    }
                }
            }
        }
    }
    Ok(out)
}

/// Grid points as a single MultiPoint (convenience for `pointGrid`).
pub fn point_grid_multipoint(bbox: [f64; 4], opts: &GridOptions, mask: Option<&Geometry>) -> Result<MultiPoint> {
    let pts = grid(bbox, GridKind::Point, opts, mask)?;
    Ok(MultiPoint(
        pts.into_iter()
            .filter_map(|g| match g {
                Geometry::Point(p) => Some(p),
                _ => None,
            })
            .collect(),
    ))
}

/// The bounding box of a geometry, squared off (turf's `square`).
pub fn square_bbox(b: [f64; 4]) -> [f64; 4] {
    let (w, h) = (b[2] - b[0], b[3] - b[1]);
    if w >= h {
        let pad = (w - h) / 2.0;
        [b[0], b[1] - pad, b[2], b[3] + pad]
    } else {
        let pad = (h - w) / 2.0;
        [b[0] - pad, b[1], b[2] + pad, b[3]]
    }
}

/// Bounding box polygon of a geometry (turf's `envelope`).
pub fn envelope(geoms: &[Geometry]) -> Result<Polygon> {
    let b = ops::bbox(geoms).ok_or_else(|| Error::InvalidGeometry("no vertices".into()))?;
    Ok(ops::bbox_polygon(b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::densify::Edges;
    use crate::measure;
    use geo::polygon;

    #[test]
    fn square_grid_cells_have_the_requested_size() {
        let cells = grid(
            [120.0, 30.0, 120.2, 30.1],
            GridKind::Square,
            &GridOptions {
                width_m: 2000.0,
                ..Default::default()
            },
            None,
        )
        .unwrap();
        assert!(cells.len() > 20, "{}", cells.len());
        for c in cells.iter().take(20) {
            let area = measure::area_with(c, Edges::Geodesic);
            assert!((area - 4_000_000.0).abs() / 4_000_000.0 < 0.01, "{area}");
        }
    }

    #[test]
    fn point_grid_spacing_and_mask() {
        let opts = GridOptions {
            width_m: 5000.0,
            ..Default::default()
        };
        let pts = point_grid_multipoint([120.0, 30.0, 120.5, 30.3], &opts, None).unwrap();
        assert!(pts.0.len() > 40);
        let d = measure::distance(pts.0[0].0, pts.0[1].0);
        assert!((d - 5000.0).abs() < 5.0, "{d}");

        let mask = Geometry::Polygon(polygon![
            (x: 120.0, y: 30.0), (x: 120.1, y: 30.0), (x: 120.1, y: 30.1), (x: 120.0, y: 30.1), (x: 120.0, y: 30.0)
        ]);
        let masked = point_grid_multipoint([120.0, 30.0, 120.5, 30.3], &opts, Some(&mask)).unwrap();
        assert!(masked.0.len() < pts.0.len() && !masked.0.is_empty());
        for p in &masked.0 {
            assert!(crate::predicates::point_in_polygon(p.0, &mask, false));
        }
    }

    #[test]
    fn hex_and_triangle_grids() {
        let hex = grid(
            [120.0, 30.0, 120.1, 30.05],
            GridKind::Hex,
            &GridOptions {
                width_m: 1000.0,
                ..Default::default()
            },
            None,
        )
        .unwrap();
        assert!(!hex.is_empty());
        let Geometry::Polygon(p) = &hex[0] else { panic!() };
        assert_eq!(p.exterior().0.len(), 7); // 6 corners + closing
        let area = measure::area_with(&hex[0], Edges::Geodesic);
        let expect = 3.0 * 3f64.sqrt() / 2.0 * 1000.0 * 1000.0;
        assert!((area - expect).abs() / expect < 0.01, "{area} vs {expect}");

        let tri = grid(
            [120.0, 30.0, 120.05, 30.02],
            GridKind::Triangle,
            &GridOptions {
                width_m: 1000.0,
                ..Default::default()
            },
            None,
        )
        .unwrap();
        assert!(tri.len().is_multiple_of(2) && !tri.is_empty());
        let a = measure::area_with(&tri[0], Edges::Geodesic);
        assert!((a - 500_000.0).abs() / 500_000.0 < 0.02, "{a}");
    }

    #[test]
    fn square_bbox_and_envelope() {
        let b = square_bbox([0.0, 0.0, 4.0, 2.0]);
        assert!((b[3] - b[1] - 4.0).abs() < 1e-12);
        let env = envelope(&[Geometry::Polygon(
            polygon![(x: 1.0, y: 1.0), (x: 3.0, y: 1.0), (x: 2.0, y: 4.0)],
        )])
        .unwrap();
        assert_eq!(env.exterior().0.len(), 5);
    }
}
