//! JSON-in / JSON-out facade used by language bindings (WASM, N-API, …).
//!
//! Conventions (turf-compatible):
//! * lengths use `units` (default kilometres), areas are m²;
//! * every function takes a `crs` (default WGS84). Inputs are converted to
//!   WGS84 for computation and results are converted back to `crs`.

use geo::{Coord, Geometry, LineString, MultiLineString, MultiPolygon};
use geojson::{Feature, FeatureCollection, GeoJson, JsonObject};
use serde::Deserialize;

use crate::buffer::{self, BufferMethod, BufferOptions};
use crate::crs::{Crs, Transformer};
use crate::densify::{self, Edges, DEFAULT_TOL};
use crate::geojson_util as gju;
use crate::ops;
use crate::overlay::{self, OverlayOp, OverlayOptions};
use crate::topology;
use crate::units::Units;
use crate::validate;
use crate::{measure, predicates, Error, Result};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

trait PipeOk: Sized {
    fn pipe_ok(self) -> Result<Self> {
        Ok(self)
    }
}
impl PipeOk for serde_json::Value {}

/// Parse an optional CRS string (empty → WGS84).
pub fn crs(s: &str) -> Result<Crs> {
    if s.trim().is_empty() {
        Ok(Crs::Wgs84)
    } else {
        Crs::parse(s)
    }
}

pub(crate) struct Io {
    crs: Crs,
    to_wgs: Transformer,
    from_wgs: Transformer,
}

impl Io {
    pub(crate) fn new(crs_str: &str) -> Result<Io> {
        let c = crs(crs_str)?;
        Ok(Io {
            to_wgs: Transformer::new(&c, &Crs::Wgs84),
            from_wgs: Transformer::new(&Crs::Wgs84, &c),
            crs: c,
        })
    }

    pub(crate) fn pt_in(&self, x: f64, y: f64) -> Result<Coord> {
        if !x.is_finite() || !y.is_finite() {
            return Err(Error::InvalidArgument("coordinates must be finite numbers".into()));
        }
        let (x, y) = self.to_wgs.apply(x, y);
        Ok(Coord { x, y })
    }

    pub(crate) fn pt_out(&self, c: Coord) -> [f64; 2] {
        let (x, y) = self.from_wgs.apply(c.x, c.y);
        [x, y]
    }

    pub(crate) fn doc_in(&self, s: &str) -> Result<GeoJson> {
        let mut gj = gju::parse(s)?;
        gju::transform(&mut gj, &self.crs, &Crs::Wgs84);
        Ok(gj)
    }

    pub(crate) fn doc_out(&self, mut gj: GeoJson) -> Result<String> {
        gju::transform(&mut gj, &Crs::Wgs84, &self.crs);
        Ok(gj.to_string())
    }
}

pub(crate) fn units(s: &str) -> Result<Units> {
    Units::parse(s)
}

// ----------------------------------------------------------------- measure

pub fn distance(x1: f64, y1: f64, x2: f64, y2: f64, units_s: &str, crs_s: &str) -> Result<f64> {
    let io = Io::new(crs_s)?;
    let d = measure::distance(io.pt_in(x1, y1)?, io.pt_in(x2, y2)?);
    Ok(units(units_s)?.from_meters(d))
}

pub fn bearing(x1: f64, y1: f64, x2: f64, y2: f64, final_bearing: bool, crs_s: &str) -> Result<f64> {
    let io = Io::new(crs_s)?;
    Ok(measure::bearing(io.pt_in(x1, y1)?, io.pt_in(x2, y2)?, final_bearing))
}

pub fn destination(x: f64, y: f64, dist: f64, bearing: f64, units_s: &str, crs_s: &str) -> Result<[f64; 2]> {
    let io = Io::new(crs_s)?;
    let m = units(units_s)?.to_meters(dist);
    Ok(io.pt_out(measure::destination(io.pt_in(x, y)?, m, bearing)))
}

pub fn midpoint(x1: f64, y1: f64, x2: f64, y2: f64, crs_s: &str) -> Result<[f64; 2]> {
    let io = Io::new(crs_s)?;
    Ok(io.pt_out(measure::midpoint(io.pt_in(x1, y1)?, io.pt_in(x2, y2)?)))
}

/// `edges`: "planar" (default, GeoJSON semantics) or "geodesic".
pub fn length(geojson: &str, units_s: &str, crs_s: &str, edges: &str) -> Result<f64> {
    let io = Io::new(crs_s)?;
    let e = Edges::parse(edges)?;
    // Planar edges are integrated in closed form — no densification needed.
    let total: f64 = gju::geometries(&io.doc_in(geojson)?)?
        .iter()
        .map(|g| measure::length_with(g, e))
        .sum();
    Ok(units(units_s)?.from_meters(total))
}

pub fn area(geojson: &str, crs_s: &str, edges: &str) -> Result<f64> {
    let io = Io::new(crs_s)?;
    let e = Edges::parse(edges)?;
    Ok(gju::geometries(&io.doc_in(geojson)?)?
        .iter()
        .map(|g| measure::area_with(g, e))
        .sum())
}

pub(crate) fn as_lines(g: &Geometry, edges: Edges) -> Result<MultiLineString> {
    match densify::prepare(g, edges, DEFAULT_TOL) {
        Geometry::LineString(ls) => Ok(MultiLineString(vec![ls])),
        Geometry::MultiLineString(m) => Ok(m),
        Geometry::Line(l) => Ok(MultiLineString(vec![LineString(vec![l.start, l.end])])),
        _ => Err(Error::InvalidGeometry("expected LineString or MultiLineString".into())),
    }
}

pub fn along(line: &str, dist: f64, units_s: &str, crs_s: &str, edges: &str) -> Result<[f64; 2]> {
    let io = Io::new(crs_s)?;
    let g = gju::single_geometry(&io.doc_in(line)?)?;
    let Geometry::LineString(ls) = densify::prepare(&g, Edges::parse(edges)?, DEFAULT_TOL) else {
        return Err(Error::InvalidGeometry("along expects a LineString".into()));
    };
    let c = measure::along(&ls, units(units_s)?.to_meters(dist))
        .ok_or_else(|| Error::InvalidGeometry("empty LineString".into()))?;
    Ok(io.pt_out(c))
}

/// Returns a GeoJSON Point Feature with `dist`, `location` (in `units`),
/// `index` (segment) and `multiFeatureIndex` properties.
pub fn nearest_point_on_line(line: &str, x: f64, y: f64, units_s: &str, crs_s: &str, edges: &str) -> Result<String> {
    let io = Io::new(crs_s)?;
    let u = units(units_s)?;
    let src = gju::single_geometry(&io.doc_in(line)?)?;
    let e = Edges::parse(edges)?;
    let lines = as_lines(&src, e)?;
    let mut n = measure::nearest_point_on_line(&lines, io.pt_in(x, y)?)
        .ok_or_else(|| Error::InvalidGeometry("empty line".into()))?;
    if e == Edges::Planar {
        // Map the segment index back to the caller's (undensified) vertices.
        n.index = original_segment_index(&src, &lines, n.multi_index, n.index);
    }
    let mut props = JsonObject::new();
    props.insert("dist".into(), u.from_meters(n.dist).into());
    props.insert("location".into(), u.from_meters(n.location).into());
    props.insert("index".into(), n.index.into());
    props.insert("multiFeatureIndex".into(), n.multi_index.into());
    let feat = gju::feature(&Geometry::Point(n.point.into()), Some(props));
    io.doc_out(GeoJson::Feature(feat))
}

fn original_segment_index(src: &Geometry, dense: &MultiLineString, part: usize, dense_index: usize) -> usize {
    let orig: &[Coord] = match src {
        Geometry::LineString(ls) => &ls.0,
        Geometry::MultiLineString(m) => match m.0.get(part) {
            Some(l) => &l.0,
            None => return dense_index,
        },
        _ => return dense_index,
    };
    let Some(dl) = dense.0.get(part) else {
        return dense_index;
    };
    // Dense vertices are a superset of the originals, in order.
    let mut oi = 0usize;
    for (di, c) in dl.0.iter().enumerate().take(dense_index + 1) {
        if oi + 1 < orig.len() && *c == orig[oi + 1] && di > 0 {
            oi += 1;
        }
    }
    oi.min(orig.len().saturating_sub(2))
}

pub fn point_to_line_distance(x: f64, y: f64, line: &str, units_s: &str, crs_s: &str, edges: &str) -> Result<f64> {
    let io = Io::new(crs_s)?;
    let lines = as_lines(&gju::single_geometry(&io.doc_in(line)?)?, Edges::parse(edges)?)?;
    let n = measure::nearest_point_on_line_opts(&lines, io.pt_in(x, y)?, false)
        .ok_or_else(|| Error::InvalidGeometry("empty line".into()))?;
    Ok(units(units_s)?.from_meters(n.dist))
}

// ------------------------------------------------------------------ buffer

pub fn circle(x: f64, y: f64, radius: f64, units_s: &str, steps: u32, crs_s: &str) -> Result<String> {
    let io = Io::new(crs_s)?;
    let r = units(units_s)?.to_meters(radius);
    let p = buffer::circle(io.pt_in(x, y)?, r, steps.max(3) as usize);
    io.doc_out(GeoJson::Feature(gju::feature(&Geometry::Polygon(p), None)))
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct BufferJsonOptions {
    pub units: Option<String>,
    /// Segments per quarter circle (turf semantics, default 16).
    pub steps: Option<u32>,
    /// "geodesic" (default) or "projected".
    pub method: Option<String>,
    /// "planar" (default) or "geodesic".
    pub edges: Option<String>,
    /// Maximum chord deviation in metres (default 0.01).
    pub tolerance: Option<f64>,
    pub crs: Option<String>,
}

pub(crate) fn multipolygon_to_geom(mp: MultiPolygon) -> Option<Geometry> {
    match mp.0.len() {
        0 => None,
        1 => Some(Geometry::Polygon(mp.0.into_iter().next().unwrap())),
        _ => Some(Geometry::MultiPolygon(mp)),
    }
}

pub fn buffer(geojson: &str, radius: f64, options: &str) -> Result<String> {
    let o: BufferJsonOptions = if options.trim().is_empty() {
        Default::default()
    } else {
        serde_json::from_str(options)?
    };
    let io = Io::new(o.crs.as_deref().unwrap_or(""))?;
    let d = units(o.units.as_deref().unwrap_or(""))?.to_meters(radius);
    let method = match o.method.as_deref().unwrap_or("geodesic") {
        "geodesic" => BufferMethod::Geodesic,
        "projected" => BufferMethod::Projected,
        m => return Err(Error::InvalidArgument(format!("unknown buffer method {m}"))),
    };
    let opts = BufferOptions {
        circle_segments: (o.steps.unwrap_or(16).max(1) * 4) as usize,
        method,
        edges: Edges::parse(o.edges.as_deref().unwrap_or(""))?,
        tolerance: o.tolerance.unwrap_or(DEFAULT_TOL),
    };
    let out = gju::map_features(&io.doc_in(geojson)?, |g| {
        Ok(multipolygon_to_geom(buffer::buffer(g, d, &opts)?))
    })?;
    io.doc_out(out)
}

// ----------------------------------------------------------------- overlay

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct OverlayJsonOptions {
    /// "planar" (default) or "geodesic".
    pub edges: Option<String>,
    /// Maximum chord deviation in metres (default 0.01).
    pub tolerance: Option<f64>,
    pub crs: Option<String>,
    /// Properties for the output feature.
    pub properties: Option<JsonObject>,
}

fn overlay_opts(s: &str) -> Result<(OverlayJsonOptions, OverlayOptions)> {
    let j: OverlayJsonOptions = if s.trim().is_empty() {
        Default::default()
    } else {
        serde_json::from_str(s)?
    };
    let o = OverlayOptions {
        edges: Edges::parse(j.edges.as_deref().unwrap_or(""))?,
        tolerance: j.tolerance.unwrap_or(DEFAULT_TOL),
    };
    Ok((j, o))
}

/// Returns a Feature JSON, or `"null"` for an empty result.
pub fn overlay(a: &str, b: &str, op: OverlayOp, options: &str) -> Result<String> {
    let (j, o) = overlay_opts(options)?;
    let io = Io::new(j.crs.as_deref().unwrap_or(""))?;
    let ga = gju::single_geometry(&io.doc_in(a)?)?;
    let gb = gju::single_geometry(&io.doc_in(b)?)?;
    let mp = overlay::overlay(&ga, &gb, op, &o)?;
    match multipolygon_to_geom(mp) {
        None => Ok("null".into()),
        Some(g) => io.doc_out(GeoJson::Feature(gju::feature(&g, j.properties))),
    }
}

/// Union of every polygon in a FeatureCollection (or geometry list).
pub fn union_all(fc: &str, options: &str) -> Result<String> {
    let (j, o) = overlay_opts(options)?;
    let io = Io::new(j.crs.as_deref().unwrap_or(""))?;
    let geoms = gju::geometries(&io.doc_in(fc)?)?;
    let mp = overlay::union_all(&geoms, &o)?;
    match multipolygon_to_geom(mp) {
        None => Ok("null".into()),
        Some(g) => io.doc_out(GeoJson::Feature(gju::feature(&g, j.properties))),
    }
}

// ------------------------------------------------------------------ batch

/// Distances (m) for interleaved `[x1, y1, x2, y2, …]` pairs.
pub fn distance_batch(pairs: &[f64], units_s: &str, crs_s: &str) -> Result<Vec<f64>> {
    let io = Io::new(crs_s)?;
    let u = units(units_s)?;
    if !pairs.len().is_multiple_of(4) {
        return Err(Error::InvalidArgument("expected [x1, y1, x2, y2, …]".into()));
    }
    pairs
        .chunks_exact(4)
        .map(|c| {
            let a = io.pt_in(c[0], c[1])?;
            let b = io.pt_in(c[2], c[3])?;
            Ok(u.from_meters(measure::distance(a, b)))
        })
        .collect()
}

/// Distance (m) from one origin to many interleaved `[x, y, …]` points.
pub fn distance_to_batch(x: f64, y: f64, coords: &[f64], units_s: &str, crs_s: &str) -> Result<Vec<f64>> {
    let io = Io::new(crs_s)?;
    let u = units(units_s)?;
    let origin = io.pt_in(x, y)?;
    coords
        .chunks_exact(2)
        .map(|c| {
            let p = io.pt_in(c[0], c[1])?;
            Ok(u.from_meters(measure::distance(origin, p)))
        })
        .collect()
}

/// Destination points for many `[x, y, distance, bearing]` rows.
pub fn destination_batch(rows: &[f64], units_s: &str, crs_s: &str) -> Result<Vec<f64>> {
    let io = Io::new(crs_s)?;
    let u = units(units_s)?;
    if !rows.len().is_multiple_of(4) {
        return Err(Error::InvalidArgument("expected [x, y, distance, bearing, …]".into()));
    }
    let mut out = Vec::with_capacity(rows.len() / 2);
    for c in rows.chunks_exact(4) {
        let p = io.pt_in(c[0], c[1])?;
        let d = measure::destination(p, u.to_meters(c[2]), c[3]);
        let r = io.pt_out(d);
        out.push(r[0]);
        out.push(r[1]);
    }
    Ok(out)
}

// -------------------------------------------------------------- predicates

/// Evaluated directly in the input coordinates (no CRS conversion needed).
pub fn boolean_point_in_polygon(x: f64, y: f64, polygon: &str, ignore_boundary: bool) -> Result<bool> {
    let gj = gju::parse(polygon)?;
    let g = gju::single_geometry(&gj)?;
    if !matches!(g, Geometry::Polygon(_) | Geometry::MultiPolygon(_)) {
        return Err(Error::InvalidGeometry("expected Polygon or MultiPolygon".into()));
    }
    Ok(predicates::point_in_polygon(Coord { x, y }, &g, ignore_boundary))
}

pub fn boolean_intersects(a: &str, b: &str) -> Result<bool> {
    let ga = gju::single_geometry(&gju::parse(a)?)?;
    let gb = gju::single_geometry(&gju::parse(b)?)?;
    Ok(predicates::intersects(&ga, &gb))
}

// -------------------------------------------------------------------- CRS

pub fn transform(geojson: &str, from: &str, to: &str) -> Result<String> {
    let (f, t) = (crs(from)?, crs(to)?);
    let mut gj = gju::parse(geojson)?;
    gju::transform(&mut gj, &f, &t);
    Ok(gj.to_string())
}

pub fn transform_coords(coords: &mut [f64], stride: usize, from: &str, to: &str) -> Result<()> {
    if stride < 2 {
        return Err(Error::InvalidArgument("stride must be >= 2".into()));
    }
    if !coords.len().is_multiple_of(stride) {
        return Err(Error::InvalidArgument(
            "coordinate array length is not a multiple of stride".into(),
        ));
    }
    Transformer::new(&crs(from)?, &crs(to)?).apply_slice(coords, stride);
    Ok(())
}

pub fn normalize_crs(s: &str) -> Result<String> {
    Ok(crs(s)?.id())
}

pub fn gauss_kruger_crs(lon: f64, zone_width: u8, zone_prefix: bool) -> Result<String> {
    Ok(Crs::gauss_kruger(lon, zone_width, zone_prefix)?.id())
}

pub fn utm_crs(lon: f64, lat: f64) -> String {
    Crs::utm(lon, lat).id()
}

/// Wrap a list of Features into a FeatureCollection (helper for bindings).
pub fn feature_collection(features: Vec<Feature>) -> String {
    GeoJson::FeatureCollection(FeatureCollection {
        bbox: None,
        features,
        foreign_members: None,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_roundtrip_gcj() {
        let pt = r#"{"type":"Feature","properties":{"name":"a"},"geometry":{"type":"Point","coordinates":[116.404,39.915]}}"#;
        let out = buffer(pt, 500.0, r#"{"units":"meters","crs":"GCJ02"}"#).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["properties"]["name"], "a");
        assert_eq!(v["geometry"]["type"], "Polygon");
        // Radius measured in GCJ02 input space, after converting back to WGS84, must be 500 m.
        let c0 = v["geometry"]["coordinates"][0][0].as_array().unwrap();
        let d = distance(
            116.404,
            39.915,
            c0[0].as_f64().unwrap(),
            c0[1].as_f64().unwrap(),
            "meters",
            "GCJ02",
        )
        .unwrap();
        assert!((d - 500.0).abs() < 1e-6, "{d}");
    }

    #[test]
    fn nearest_index_refers_to_original_vertices() {
        // Long east-west segments get densified under planar semantics.
        let line = r#"{"type":"LineString","coordinates":[[100,40],[105,40],[110,40],[110,45]]}"#;
        let out = nearest_point_on_line(line, 107.0, 40.5, "meters", "", "").unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["properties"]["index"], 1);
        let out = nearest_point_on_line(line, 110.5, 44.0, "meters", "", "").unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["properties"]["index"], 2);
        // Planar: the closest point lies on the parallel 40°N.
        let out = nearest_point_on_line(line, 102.5, 39.0, "meters", "", "").unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert!((v["geometry"]["coordinates"][1].as_f64().unwrap() - 40.0).abs() < 1e-6);
        let d_planar = point_to_line_distance(102.5, 39.0, line, "meters", "", "planar").unwrap();
        let d_geo = point_to_line_distance(102.5, 39.0, line, "meters", "", "geodesic").unwrap();
        assert!(d_geo > d_planar + 1000.0, "{d_geo} vs {d_planar}");
    }

    #[test]
    fn overlay_null_on_disjoint() {
        let a = r#"{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,1],[0,0]]]}"#;
        let b = r#"{"type":"Polygon","coordinates":[[[2,2],[3,2],[3,3],[2,3],[2,2]]]}"#;
        assert_eq!(overlay(a, b, OverlayOp::Intersection, "").unwrap(), "null");
    }

    #[test]
    fn transform_keeps_z_and_props() {
        let s = r#"{"type":"Feature","id":7,"properties":{"k":1},"geometry":{"type":"LineString","coordinates":[[120,30,12.5],[121,31,13]]}}"#;
        let out = transform(s, "WGS84", "EPSG:4549").unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["id"], 7);
        assert_eq!(v["properties"]["k"], 1);
        assert_eq!(v["geometry"]["coordinates"][0][2], 12.5);
        let back = transform(&out, "EPSG:4549", "WGS84").unwrap();
        let v: serde_json::Value = serde_json::from_str(&back).unwrap();
        assert!((v["geometry"]["coordinates"][1][0].as_f64().unwrap() - 121.0).abs() < 1e-9);
    }
}

// ------------------------------------------------------------ geometry ops

pub(crate) fn geometry_in(io: &Io, geojson: &str) -> Result<Geometry> {
    gju::single_geometry(&io.doc_in(geojson)?)
}

pub(crate) fn geometries_in(io: &Io, geojson: &str) -> Result<Vec<Geometry>> {
    gju::geometries(&io.doc_in(geojson)?)
}

pub(crate) fn feature_out(io: &Io, g: &Geometry, props: Option<JsonObject>) -> Result<String> {
    io.doc_out(GeoJson::Feature(gju::feature(g, props)))
}

pub(crate) fn points_feature_collection(io: &Io, pts: &[Coord]) -> Result<String> {
    let feats: Vec<Feature> = pts
        .iter()
        .map(|c| gju::feature(&Geometry::Point((*c).into()), None))
        .collect();
    io.doc_out(GeoJson::FeatureCollection(FeatureCollection {
        bbox: None,
        features: feats,
        foreign_members: None,
    }))
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct OpOptions {
    pub crs: Option<String>,
    pub units: Option<String>,
    pub edges: Option<String>,
    /// simplify
    pub tolerance: Option<f64>,
    pub preserve_topology: Option<bool>,
    /// concave hull
    pub max_edge: Option<f64>,
    /// truncate
    pub precision: Option<u32>,
    /// transforms
    pub pivot: Option<[f64; 2]>,
    pub origin: Option<[f64; 2]>,
    /// sector / great circle
    pub steps: Option<u32>,
    pub properties: Option<JsonObject>,
}

pub(crate) fn op_opts(s: &str) -> Result<OpOptions> {
    if s.trim().is_empty() {
        Ok(Default::default())
    } else {
        Ok(serde_json::from_str(s)?)
    }
}

pub(crate) fn io_of(o: &OpOptions) -> Result<Io> {
    Io::new(o.crs.as_deref().unwrap_or(""))
}

pub(crate) fn edges_of(o: &OpOptions) -> Result<Edges> {
    Edges::parse(o.edges.as_deref().unwrap_or(""))
}

pub(crate) fn units_of(o: &OpOptions) -> Result<Units> {
    units(o.units.as_deref().unwrap_or(""))
}

/// Douglas–Peucker / topology-preserving simplification, tolerance in `units`.
pub fn simplify(geojson: &str, tolerance: f64, options: &str) -> Result<String> {
    let o = op_opts(options)?;
    let io = io_of(&o)?;
    let tol = units_of(&o)?.to_meters(tolerance);
    let preserve = o.preserve_topology.unwrap_or(false);
    let out = gju::map_features(&io.doc_in(geojson)?, |g| Ok(Some(ops::simplify(g, tol, preserve)?)))?;
    io.doc_out(out)
}

pub fn convex_hull(geojson: &str, options: &str) -> Result<String> {
    let o = op_opts(options)?;
    let io = io_of(&o)?;
    let hull = ops::convex_hull(&geometries_in(&io, geojson)?)?;
    feature_out(&io, &Geometry::Polygon(hull), o.properties)
}

pub fn concave_hull(geojson: &str, options: &str) -> Result<String> {
    let o = op_opts(options)?;
    let io = io_of(&o)?;
    let max_edge = units_of(&o)?.to_meters(o.max_edge.unwrap_or(1.0));
    let hull = ops::concave_hull(&geometries_in(&io, geojson)?, max_edge)?;
    feature_out(&io, &Geometry::Polygon(hull), o.properties)
}

/// `kind`: "centroid" (mean of vertices), "centerOfMass" or "pointOnFeature".
pub fn center(geojson: &str, kind: &str, options: &str) -> Result<String> {
    let o = op_opts(options)?;
    let io = io_of(&o)?;
    let geoms = geometries_in(&io, geojson)?;
    let c = match kind {
        "" | "centroid" => ops::vertex_centroid(&geoms).ok_or_else(|| Error::InvalidGeometry("no vertices".into()))?,
        "centerOfMass" => ops::center_of_mass(&geoms)?,
        "pointOnFeature" => {
            let g = geoms
                .first()
                .ok_or_else(|| Error::InvalidGeometry("no geometry".into()))?;
            ops::point_on_feature(g)?
        }
        other => return Err(Error::InvalidArgument(format!("unknown centre type `{other}`"))),
    };
    feature_out(&io, &Geometry::Point(c.into()), o.properties)
}

pub fn bbox(geojson: &str, options: &str) -> Result<Vec<f64>> {
    let o = op_opts(options)?;
    let io = io_of(&o)?;
    let b = ops::bbox(&geometries_in(&io, geojson)?).ok_or_else(|| Error::InvalidGeometry("no vertices".into()))?;
    // convert the corners back to the caller's CRS
    let min = io.pt_out(Coord { x: b[0], y: b[1] });
    let max = io.pt_out(Coord { x: b[2], y: b[3] });
    Ok(vec![min[0], min[1], max[0], max[1]])
}

pub fn bbox_polygon(b: &[f64], options: &str) -> Result<String> {
    if b.len() < 4 {
        return Err(Error::InvalidArgument("bbox needs 4 numbers".into()));
    }
    let o = op_opts(options)?;
    let io = io_of(&o)?;
    let min = io.pt_in(b[0], b[1])?;
    let max = io.pt_in(b[2], b[3])?;
    let poly = ops::bbox_polygon([min.x, min.y, max.x, max.y]);
    feature_out(&io, &Geometry::Polygon(poly), o.properties)
}

pub fn bbox_clip(geojson: &str, b: &[f64], options: &str) -> Result<String> {
    if b.len() < 4 {
        return Err(Error::InvalidArgument("bbox needs 4 numbers".into()));
    }
    let o = op_opts(options)?;
    let io = io_of(&o)?;
    let min = io.pt_in(b[0], b[1])?;
    let max = io.pt_in(b[2], b[3])?;
    let rect = [min.x, min.y, max.x, max.y];
    let out = gju::map_features(&io.doc_in(geojson)?, |g| Ok(ops::bbox_clip(g, rect).ok()))?;
    io.doc_out(out)
}

/// `kind`: "rotate" (angle in degrees), "translate" (distance + bearing) or "scale" (factor).
pub fn transform_geometry(geojson: &str, kind: &str, a: f64, b: f64, options: &str) -> Result<String> {
    let o = op_opts(options)?;
    let io = io_of(&o)?;
    let doc = io.doc_in(geojson)?;
    let geoms = gju::geometries(&doc)?;
    let anchor = |given: Option<[f64; 2]>| -> Result<Coord> {
        match given {
            Some(p) => io.pt_in(p[0], p[1]),
            None => ops::vertex_centroid(&geoms).ok_or_else(|| Error::InvalidGeometry("no vertices".into())),
        }
    };
    let out = match kind {
        "rotate" => {
            let pivot = anchor(o.pivot)?;
            gju::map_features(&doc, |g| Ok(Some(ops::transform_rotate(g, a, pivot))))?
        }
        "translate" => {
            let dist = units_of(&o)?.to_meters(a);
            gju::map_features(&doc, |g| Ok(Some(ops::transform_translate(g, dist, b))))?
        }
        "scale" => {
            let origin = anchor(o.origin)?;
            gju::map_features(&doc, |g| Ok(Some(ops::transform_scale(g, a, origin))))?
        }
        other => return Err(Error::InvalidArgument(format!("unknown transform `{other}`"))),
    };
    io.doc_out(out)
}

pub(crate) fn as_line_string(g: &Geometry) -> Result<geo::LineString> {
    match g {
        Geometry::LineString(ls) => Ok(ls.clone()),
        Geometry::Line(l) => Ok(geo::LineString(vec![l.start, l.end])),
        other => Err(Error::InvalidGeometry(format!(
            "expected a LineString, got {}",
            crate::overlay::geometry_type_name(other)
        ))),
    }
}

pub fn line_slice_along(line: &str, start: f64, stop: f64, options: &str) -> Result<String> {
    let o = op_opts(options)?;
    let io = io_of(&o)?;
    let u = units_of(&o)?;
    let ls = as_line_string(&geometry_in(&io, line)?)?;
    let out = ops::line_slice_along(&ls, u.to_meters(start), u.to_meters(stop), edges_of(&o)?)?;
    feature_out(&io, &Geometry::LineString(out), o.properties)
}

pub fn line_slice(line: &str, x1: f64, y1: f64, x2: f64, y2: f64, options: &str) -> Result<String> {
    let o = op_opts(options)?;
    let io = io_of(&o)?;
    let ls = as_line_string(&geometry_in(&io, line)?)?;
    let out = ops::line_slice(&ls, io.pt_in(x1, y1)?, io.pt_in(x2, y2)?, edges_of(&o)?)?;
    feature_out(&io, &Geometry::LineString(out), o.properties)
}

pub fn line_chunk(line: &str, length: f64, options: &str) -> Result<String> {
    let o = op_opts(options)?;
    let io = io_of(&o)?;
    let ls = as_line_string(&geometry_in(&io, line)?)?;
    let chunks = ops::line_chunk(&ls, units_of(&o)?.to_meters(length), edges_of(&o)?)?;
    let feats: Vec<Feature> = chunks
        .into_iter()
        .map(|c| gju::feature(&Geometry::LineString(c), None))
        .collect();
    io.doc_out(GeoJson::FeatureCollection(FeatureCollection {
        bbox: None,
        features: feats,
        foreign_members: None,
    }))
}

pub fn line_intersect(a: &str, b: &str, options: &str) -> Result<String> {
    let o = op_opts(options)?;
    let io = io_of(&o)?;
    let (ga, gb) = (geometry_in(&io, a)?, geometry_in(&io, b)?);
    let pts = ops::line_intersect(&ga, &gb);
    points_feature_collection(&io, &pts)
}

pub fn great_circle(x1: f64, y1: f64, x2: f64, y2: f64, options: &str) -> Result<String> {
    let o = op_opts(options)?;
    let io = io_of(&o)?;
    let ls = ops::great_circle(io.pt_in(x1, y1)?, io.pt_in(x2, y2)?, o.steps.unwrap_or(100) as usize);
    feature_out(&io, &Geometry::LineString(ls), o.properties)
}

pub fn sector(x: f64, y: f64, radius: f64, bearing1: f64, bearing2: f64, options: &str) -> Result<String> {
    let o = op_opts(options)?;
    let io = io_of(&o)?;
    let r = units_of(&o)?.to_meters(radius);
    let poly = ops::sector(io.pt_in(x, y)?, r, bearing1, bearing2, o.steps.unwrap_or(64) as usize);
    feature_out(&io, &Geometry::Polygon(poly), o.properties)
}

/// Index of the closest point in a collection, and its distance.
pub fn nearest_point(x: f64, y: f64, points: &str, options: &str) -> Result<Vec<f64>> {
    let o = op_opts(options)?;
    let io = io_of(&o)?;
    let pts: Vec<Coord> = geometries_in(&io, points)?.iter().flat_map(ops::coords_of).collect();
    let (i, d) = ops::nearest_point(io.pt_in(x, y)?, &pts).ok_or_else(|| Error::InvalidGeometry("no points".into()))?;
    let out = io.pt_out(pts[i]);
    Ok(vec![i as f64, units_of(&o)?.from_meters(d), out[0], out[1]])
}

/// Structural helpers: "flatten", "explode", "polygonToLine", "lineToPolygon",
/// "rewind", "cleanCoords", "truncate".
pub fn reshape(geojson: &str, kind: &str, options: &str) -> Result<String> {
    let o = op_opts(options)?;
    let io = io_of(&o)?;
    let doc = io.doc_in(geojson)?;
    match kind {
        "flatten" => {
            let mut feats = Vec::new();
            for_each_feature(&doc, |g, props| {
                for part in ops::flatten(g) {
                    feats.push(gju::feature(&part, props.clone()));
                }
                Ok(())
            })?;
            io.doc_out(GeoJson::FeatureCollection(FeatureCollection {
                bbox: None,
                features: feats,
                foreign_members: None,
            }))
        }
        "explode" => {
            let mut feats = Vec::new();
            for_each_feature(&doc, |g, props| {
                for c in ops::coords_of(g) {
                    feats.push(gju::feature(&Geometry::Point(c.into()), props.clone()));
                }
                Ok(())
            })?;
            io.doc_out(GeoJson::FeatureCollection(FeatureCollection {
                bbox: None,
                features: feats,
                foreign_members: None,
            }))
        }
        "polygonToLine" => {
            let out = gju::map_features(&doc, |g| Ok(Some(Geometry::MultiLineString(ops::polygon_to_line(g)?))))?;
            io.doc_out(out)
        }
        "lineToPolygon" => {
            let out = gju::map_features(&doc, |g| Ok(Some(Geometry::MultiPolygon(ops::line_to_polygon(g)?))))?;
            io.doc_out(out)
        }
        "rewind" => {
            let out = gju::map_features(&doc, |g| Ok(Some(ops::rewind(g))))?;
            io.doc_out(out)
        }
        "cleanCoords" => {
            let tol = units_of(&o)?.to_meters(o.tolerance.unwrap_or(0.0));
            let out = gju::map_features(&doc, |g| Ok(Some(ops::clean_coords(g, tol))))?;
            io.doc_out(out)
        }
        "truncate" => {
            let p = o.precision.unwrap_or(6);
            let out = gju::map_features(&doc, |g| Ok(Some(ops::truncate(g, p))))?;
            io.doc_out(out)
        }
        other => Err(Error::InvalidArgument(format!("unknown reshape `{other}`"))),
    }
}

pub(crate) fn for_each_feature(
    gj: &GeoJson,
    mut f: impl FnMut(&Geometry, Option<JsonObject>) -> Result<()>,
) -> Result<()> {
    match gj {
        GeoJson::Geometry(g) => f(&gju::value_to_geo(&g.value)?, None),
        GeoJson::Feature(feat) => match &feat.geometry {
            Some(g) => f(&gju::value_to_geo(&g.value)?, feat.properties.clone()),
            None => Ok(()),
        },
        GeoJson::FeatureCollection(fc) => {
            for feat in &fc.features {
                if let Some(g) = &feat.geometry {
                    f(&gju::value_to_geo(&g.value)?, feat.properties.clone())?;
                }
            }
            Ok(())
        }
    }
}

/// Union polygons that share a property value (turf's `dissolve`).
pub fn dissolve(fc: &str, property: &str, options: &str) -> Result<String> {
    let o = op_opts(options)?;
    let io = io_of(&o)?;
    let doc = io.doc_in(fc)?;
    let mut groups: Vec<(Option<serde_json::Value>, Vec<Geometry>, Option<JsonObject>)> = Vec::new();
    for_each_feature(&doc, |g, props| {
        let key = if property.is_empty() {
            None
        } else {
            props.as_ref().and_then(|p| p.get(property).cloned())
        };
        match groups.iter_mut().find(|(k, _, _)| *k == key) {
            Some((_, list, _)) => list.push(g.clone()),
            None => groups.push((key, vec![g.clone()], props)),
        }
        Ok(())
    })?;
    let ov = OverlayOptions {
        edges: edges_of(&o)?,
        tolerance: o.tolerance.unwrap_or(DEFAULT_TOL),
    };
    let mut feats = Vec::new();
    for (_, geoms, props) in groups {
        let mp = overlay::union_all(&geoms, &ov)?;
        if let Some(g) = multipolygon_to_geom(mp) {
            feats.push(gju::feature(&g, props));
        }
    }
    io.doc_out(GeoJson::FeatureCollection(FeatureCollection {
        bbox: None,
        features: feats,
        foreign_members: None,
    }))
}

/// Points of `points` that fall inside `polygon`.
pub fn points_within_polygon(points: &str, polygon: &str, options: &str) -> Result<String> {
    let o = op_opts(options)?;
    let io = io_of(&o)?;
    let poly = geometry_in(&io, polygon)?;
    let prepared = crate::index::Prepared::new(poly)?;
    let doc = io.doc_in(points)?;
    let mut feats = Vec::new();
    for_each_feature(&doc, |g, props| {
        for c in ops::coords_of(g) {
            if prepared.contains_point(c, false) {
                feats.push(gju::feature(&Geometry::Point(c.into()), props.clone()));
            }
        }
        Ok(())
    })?;
    io.doc_out(GeoJson::FeatureCollection(FeatureCollection {
        bbox: None,
        features: feats,
        foreign_members: None,
    }))
}

// ------------------------------------------------------------- predicates

pub fn relate(a: &str, b: &str, options: &str) -> Result<String> {
    let o = op_opts(options)?;
    let io = io_of(&o)?;
    Ok(predicates::relate_matrix(
        &geometry_in(&io, a)?,
        &geometry_in(&io, b)?,
        edges_of(&o)?,
    ))
}

pub fn relate_pattern(a: &str, b: &str, pattern: &str, options: &str) -> Result<bool> {
    let o = op_opts(options)?;
    let io = io_of(&o)?;
    predicates::matches_pattern(&geometry_in(&io, a)?, &geometry_in(&io, b)?, pattern, edges_of(&o)?)
}

pub fn boolean_relation(a: &str, b: &str, predicate: &str, options: &str) -> Result<bool> {
    let o = op_opts(options)?;
    let io = io_of(&o)?;
    Ok(predicates::evaluate(
        &geometry_in(&io, a)?,
        &geometry_in(&io, b)?,
        predicates::Predicate::parse(predicate)?,
        edges_of(&o)?,
    ))
}

pub fn boolean_point_on_line(
    x: f64,
    y: f64,
    line: &str,
    tolerance: f64,
    ignore_ends: bool,
    options: &str,
) -> Result<bool> {
    let o = op_opts(options)?;
    let io = io_of(&o)?;
    let lines = as_lines(&geometry_in(&io, line)?, Edges::Geodesic)?;
    Ok(predicates::point_on_line(
        io.pt_in(x, y)?,
        &lines,
        units_of(&o)?.to_meters(tolerance),
        ignore_ends,
        edges_of(&o)?,
    ))
}

// --------------------------------------------------------------- topology

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct TopologyOptions {
    pub crs: Option<String>,
    pub edges: Option<String>,
    pub units: Option<String>,
    // validate
    pub check_spikes: Option<bool>,
    pub check_winding: Option<bool>,
    pub check_antimeridian: Option<bool>,
    pub duplicate_tolerance: Option<f64>,
    // make valid
    pub snap_grid: Option<f64>,
    pub min_area: Option<f64>,
    pub clean_tolerance: Option<f64>,
    // coverage
    pub min_overlap: Option<f64>,
    pub min_gap: Option<f64>,
    pub max_gap: Option<f64>,
    pub gap_tolerance: Option<f64>,
    pub tolerance: Option<f64>,
    // network
    pub max_undershoot: Option<f64>,
    pub max_overshoot: Option<f64>,
}

fn topo_opts(s: &str) -> Result<TopologyOptions> {
    if s.trim().is_empty() {
        Ok(Default::default())
    } else {
        Ok(serde_json::from_str(s)?)
    }
}

/// Validity report as JSON: `{"valid":bool,"issues":[…]}` per feature.
pub fn validate(geojson: &str, options: &str) -> Result<String> {
    let o = topo_opts(options)?;
    let io = Io::new(o.crs.as_deref().unwrap_or(""))?;
    let vopts = validate::ValidateOptions {
        check_spikes: o.check_spikes.unwrap_or(true),
        check_winding: o.check_winding.unwrap_or(true),
        check_antimeridian: o.check_antimeridian.unwrap_or(true),
        duplicate_tolerance: o.duplicate_tolerance.unwrap_or(0.0),
    };
    let geoms = geometries_in(&io, geojson)?;
    let mut all = Vec::new();
    for (i, g) in geoms.iter().enumerate() {
        for issue in validate::validate(g, &vopts) {
            let mut v = serde_json::to_value(&issue)?;
            v.as_object_mut().unwrap().insert("feature".into(), i.into());
            all.push(v);
        }
    }
    let valid = all
        .iter()
        .all(|v| v.get("severity").and_then(|s| s.as_str()) != Some("error"));
    Ok(serde_json::json!({ "valid": valid, "issues": all }).to_string())
}

/// Repair: `{"geometry":<GeoJSON Feature>,"fixes":[…]}`.
pub fn make_valid(geojson: &str, options: &str) -> Result<String> {
    let o = topo_opts(options)?;
    let io = Io::new(o.crs.as_deref().unwrap_or(""))?;
    let mopts = validate::MakeValidOptions {
        snap_grid_m: o.snap_grid.unwrap_or(0.0),
        min_area_m2: o.min_area.unwrap_or(0.0),
        clean_tolerance_m: o.clean_tolerance.unwrap_or(0.0),
    };
    let mut fixes: Vec<String> = Vec::new();
    let out = gju::map_features(&io.doc_in(geojson)?, |g| {
        let (fixed, log) = validate::make_valid(g, &mopts)?;
        fixes.extend(log);
        Ok(Some(fixed))
    })?;
    let mut doc: serde_json::Value = serde_json::from_str(&io.doc_out(out)?)?;
    if let Some(obj) = doc.as_object_mut() {
        obj.insert("geoverse-precise:fixes".into(), serde_json::to_value(&fixes)?);
    }
    Ok(doc.to_string())
}

/// Overlaps and gaps in a polygon coverage.
pub fn coverage_issues(fc: &str, options: &str) -> Result<String> {
    let o = topo_opts(options)?;
    let io = Io::new(o.crs.as_deref().unwrap_or(""))?;
    let copts = topology::CoverageOptions {
        edges: Edges::parse(o.edges.as_deref().unwrap_or(""))?,
        min_overlap_m2: o.min_overlap.unwrap_or(1e-3),
        min_gap_m2: o.min_gap.unwrap_or(1.0),
        max_gap_m2: o.max_gap.unwrap_or(f64::INFINITY),
        gap_tolerance_m: o.gap_tolerance.unwrap_or(0.0),
        tolerance: o.tolerance.unwrap_or(DEFAULT_TOL),
    };
    let report = topology::coverage_issues(&geometries_in(&io, fc)?, &copts)?;
    let overlaps: Vec<serde_json::Value> = report
        .overlaps
        .iter()
        .map(|ov| {
            let g = Geometry::MultiPolygon(ov.geometry.clone());
            serde_json::json!({
                "a": ov.a,
                "b": ov.b,
                "areaM2": ov.area_m2,
                "geometry": serde_json::from_str::<serde_json::Value>(&io.doc_out(GeoJson::Geometry(gju::geo_to_geometry(&g)))?)?,
            })
            .pipe_ok()
        })
        .collect::<Result<_>>()?;
    let gaps: Vec<serde_json::Value> = report
        .gaps
        .iter()
        .map(|gap| {
            let g = Geometry::Polygon(gap.geometry.clone());
            serde_json::json!({
                "areaM2": gap.area_m2,
                "geometry": serde_json::from_str::<serde_json::Value>(&io.doc_out(GeoJson::Geometry(gju::geo_to_geometry(&g)))?)?,
            })
            .pipe_ok()
        })
        .collect::<Result<_>>()?;
    Ok(serde_json::json!({
        "overlaps": overlaps,
        "gaps": gaps,
        "unionAreaM2": report.union_area_m2,
        "totalAreaM2": report.total_area_m2,
    })
    .to_string())
}

/// Line-network topology report.
pub fn network_issues(fc: &str, options: &str) -> Result<String> {
    let o = topo_opts(options)?;
    let io = Io::new(o.crs.as_deref().unwrap_or(""))?;
    let u = units(o.units.as_deref().unwrap_or(""))?;
    let nopts = topology::NetworkOptions {
        tolerance_m: u.to_meters(o.tolerance.unwrap_or(0.01 / u.meters_per_unit())),
        max_undershoot_m: u.to_meters(o.max_undershoot.unwrap_or(0.0)),
        max_overshoot_m: u.to_meters(o.max_overshoot.unwrap_or(0.0)),
        edges: Edges::parse(o.edges.as_deref().unwrap_or(""))?,
    };
    let report = topology::network_issues(&geometries_in(&io, fc)?, &nopts)?;
    let mut v = serde_json::to_value(&report)?;
    // convert coordinates back into the caller's CRS
    if let Some(obj) = v.as_object_mut() {
        for (_, list) in obj.iter_mut() {
            if let Some(arr) = list.as_array_mut() {
                for item in arr.iter_mut() {
                    if let Some(at) = item.get_mut("at").and_then(|a| a.as_array_mut()) {
                        if at.len() == 2 {
                            let p = io.pt_out(Coord {
                                x: at[0].as_f64().unwrap_or(f64::NAN),
                                y: at[1].as_f64().unwrap_or(f64::NAN),
                            });
                            at[0] = serde_json::json!(p[0]);
                            at[1] = serde_json::json!(p[1]);
                        }
                    }
                }
            }
        }
    }
    Ok(v.to_string())
}

pub fn snap_round(geojson: &str, grid: f64, options: &str) -> Result<String> {
    let o = op_opts(options)?;
    let io = io_of(&o)?;
    let g = units_of(&o)?.to_meters(grid);
    let out = gju::map_features(&io.doc_in(geojson)?, |geom| Ok(Some(topology::snap_round(geom, g)?)))?;
    io.doc_out(out)
}

/// Snap vertices onto a reference layer; returns `{"geometry":…,"moved":n}`.
pub fn snap_to(geojson: &str, reference: &str, tolerance: f64, options: &str) -> Result<String> {
    let o = op_opts(options)?;
    let io = io_of(&o)?;
    let tol = units_of(&o)?.to_meters(tolerance);
    let refs = geometries_in(&io, reference)?;
    let mut moved = 0usize;
    let out = gju::map_features(&io.doc_in(geojson)?, |g| {
        let (snapped, n) = topology::snap_to(g, &refs, tol)?;
        moved += n;
        Ok(Some(snapped))
    })?;
    let mut doc: serde_json::Value = serde_json::from_str(&io.doc_out(out)?)?;
    if let Some(obj) = doc.as_object_mut() {
        obj.insert("geoverse-precise:moved".into(), moved.into());
    }
    Ok(doc.to_string())
}
