//! The JSON facade for the turf-parity layer: rhumb lines, line tools, grids,
//! shape constructors, interpolation, contouring and clustering.
//!
//! Same conventions as [`crate::api`]: lengths and cell sizes are in `units`
//! (kilometres unless told otherwise), areas are m², and everything is
//! converted through `crs` on the way in and out.

use geo::{Coord, Geometry, LineString, Point};
use geojson::{Feature, FeatureCollection, GeoJson, JsonObject};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::{
    feature_out, for_each_feature, geometries_in, geometry_in, multipolygon_to_geom,
    points_feature_collection, units, Io,
};
use crate::densify::Edges;
use crate::geojson_util as gju;
use crate::grids::{GridKind, GridOptions};
use crate::interp::{Grid, IdwOptions};
use crate::units::Units;
use crate::{cluster, grids, interp, lines, ops, rhumb, shapes, stats, Error, Result};

// ------------------------------------------------------------------ options

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ExtOptions {
    pub crs: Option<String>,
    pub units: Option<String>,
    pub edges: Option<String>,
    pub steps: Option<u32>,
    pub tolerance: Option<f64>,
    pub properties: Option<JsonObject>,
    /// Which feature property carries the scalar (`elevation`, then `z`, then
    /// `value` are tried when this is absent).
    pub z_property: Option<String>,
    /// Grid shape: point | square | rectangle | triangle | hex.
    pub grid_type: Option<String>,
    pub cell_width: Option<f64>,
    pub cell_height: Option<f64>,
    /// Keep only the cells touching this geometry.
    pub mask: Option<String>,
    pub bbox: Option<Vec<f64>>,
    /// IDW
    pub power: Option<f64>,
    pub search_radius: Option<f64>,
    /// DBSCAN / k-means
    pub min_points: Option<usize>,
    pub number_of_clusters: Option<usize>,
    /// nearest-neighbour analysis
    pub study_area: Option<String>,
    /// weights for the mean / median centre and the ellipse
    pub weight_property: Option<String>,
    /// routing
    pub obstacles: Option<String>,
    pub resolution: Option<f64>,
    pub padding: Option<f64>,
    /// smoothing / splines
    pub iterations: Option<usize>,
    pub sharpness: Option<f64>,
    /// bezier output spacing, in metres along the curve
    pub spacing: Option<f64>,
    /// ellipse rotation, degrees clockwise
    pub angle: Option<f64>,
    /// `angle()` reports the reflex angle instead
    pub explementary: Option<bool>,
    /// `lineOffset` arc resolution
    pub arc_steps: Option<usize>,
    /// spatial weights: neighbourhood radius in `units`
    pub threshold: Option<f64>,
    /// spatial weights: distance decay exponent when `binary` is false
    pub alpha: Option<f64>,
    /// spatial weights: every neighbour counts 1 (the default)
    pub binary: Option<bool>,
    /// `row` (default) or `raw`
    pub standardization: Option<String>,
    /// quadrat analysis grid
    pub x_quadrats: Option<usize>,
    pub y_quadrats: Option<usize>,
}

pub fn ext_opts(s: &str) -> Result<ExtOptions> {
    if s.trim().is_empty() {
        Ok(Default::default())
    } else {
        Ok(serde_json::from_str(s)?)
    }
}

fn ext_io(o: &ExtOptions) -> Result<Io> {
    Io::new(o.crs.as_deref().unwrap_or(""))
}

fn ext_units(o: &ExtOptions) -> Result<Units> {
    units(o.units.as_deref().unwrap_or(""))
}

fn ext_edges(o: &ExtOptions) -> Result<Edges> {
    Edges::parse(o.edges.as_deref().unwrap_or(""))
}

fn fc(features: Vec<Feature>) -> GeoJson {
    GeoJson::FeatureCollection(FeatureCollection { bbox: None, features, foreign_members: None })
}

fn props_of(v: Value) -> Option<JsonObject> {
    match v {
        Value::Object(m) => Some(m),
        _ => None,
    }
}

/// Merge extra entries into a feature's own properties.
fn with_props(base: Option<JsonObject>, extra: Value) -> Option<JsonObject> {
    let mut m = base.unwrap_or_default();
    if let Value::Object(e) = extra {
        for (k, v) in e {
            m.insert(k, v);
        }
    }
    Some(m)
}

/// Pull the scalar a contouring / interpolation call should use.
fn z_of(props: &Option<JsonObject>, name: Option<&str>) -> Result<f64> {
    let names: Vec<&str> = match name {
        Some(n) => vec![n],
        None => vec!["elevation", "z", "value"],
    };
    let p = props.as_ref().ok_or_else(|| Error::InvalidArgument("feature has no properties".into()))?;
    for n in &names {
        if let Some(v) = p.get(*n) {
            if let Some(f) = v.as_f64() {
                return Ok(f);
            }
        }
    }
    Err(Error::InvalidArgument(format!(
        "no numeric `{}` property on the feature",
        names.join("` / `")
    )))
}

/// Collect (position, scalar) pairs from a point FeatureCollection.
fn samples_in(io: &Io, geojson: &str, z_property: Option<&str>) -> Result<Vec<(Coord, f64)>> {
    let doc = io.doc_in(geojson)?;
    let mut out = Vec::new();
    for_each_feature(&doc, |g, props| {
        let z = z_of(&props, z_property)?;
        for c in ops::coords_of_no_wrap(g) {
            out.push((c, z));
        }
        Ok(())
    })?;
    if out.is_empty() {
        return Err(Error::InvalidArgument("no sample points".into()));
    }
    Ok(out)
}

/// Collect positions (ignoring any properties) from any GeoJSON.
fn points_in(io: &Io, geojson: &str) -> Result<Vec<Coord>> {
    let gs = geometries_in(io, geojson)?;
    let mut out = Vec::new();
    for g in &gs {
        out.extend(ops::coords_of_no_wrap(g));
    }
    if out.is_empty() {
        return Err(Error::InvalidArgument("no points".into()));
    }
    Ok(out)
}

/// Positions with their owning feature's properties, one entry per feature.
fn features_in(io: &Io, geojson: &str) -> Result<Vec<(Coord, Option<JsonObject>)>> {
    let doc = io.doc_in(geojson)?;
    let mut out = Vec::new();
    for_each_feature(&doc, |g, props| {
        for c in ops::coords_of_no_wrap(g) {
            out.push((c, props.clone()));
        }
        Ok(())
    })?;
    Ok(out)
}

fn bbox_of(o: &ExtOptions, fallback: &[Geometry]) -> Result<[f64; 4]> {
    if let Some(b) = &o.bbox {
        if b.len() < 4 {
            return Err(Error::InvalidArgument("bbox needs 4 numbers".into()));
        }
        return Ok([b[0], b[1], b[2], b[3]]);
    }
    ops::bbox(fallback).ok_or_else(|| Error::InvalidGeometry("no extent to work from".into()))
}

fn optional_geometry(io: &Io, s: &Option<String>) -> Result<Option<Geometry>> {
    match s {
        None => Ok(None),
        Some(t) if t.trim().is_empty() => Ok(None),
        Some(t) => Ok(Some(geometry_in(io, t)?)),
    }
}

// -------------------------------------------------------------- rhumb lines

pub fn rhumb_distance(x1: f64, y1: f64, x2: f64, y2: f64, units_s: &str, crs_s: &str) -> Result<f64> {
    let io = Io::new(crs_s)?;
    let d = rhumb::distance(io.pt_in(x1, y1)?, io.pt_in(x2, y2)?);
    Ok(units(units_s)?.from_meters(d))
}

pub fn rhumb_bearing(x1: f64, y1: f64, x2: f64, y2: f64, final_bearing: bool, crs_s: &str) -> Result<f64> {
    let io = Io::new(crs_s)?;
    let (a, b) = (io.pt_in(x1, y1)?, io.pt_in(x2, y2)?);
    // a rhumb line holds one bearing end to end, so the final bearing is the
    // reverse of the bearing measured the other way
    Ok(if final_bearing { crate::geodesic::normalize_deg(rhumb::bearing(b, a) + 180.0) } else { rhumb::bearing(a, b) })
}

pub fn rhumb_destination(x: f64, y: f64, dist: f64, bearing: f64, units_s: &str, crs_s: &str) -> Result<[f64; 2]> {
    let io = Io::new(crs_s)?;
    let m = units(units_s)?.to_meters(dist);
    Ok(io.pt_out(rhumb::destination(io.pt_in(x, y)?, m, bearing)))
}

// -------------------------------------------------------------- line tools

pub fn line_segment(geojson: &str, options: &str) -> Result<String> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    let doc = io.doc_in(geojson)?;
    let mut feats = Vec::new();
    for_each_feature(&doc, |g, props| {
        for seg in lines::line_segments(g) {
            // turf emits each segment as a two-position LineString
            let ls = LineString(vec![seg.start, seg.end]);
            feats.push(gju::feature(&Geometry::LineString(ls), props.clone()));
        }
        Ok(())
    })?;
    io.doc_out(fc(feats))
}

pub fn line_split(line: &str, splitter: &str, options: &str) -> Result<String> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    let l = crate::api::as_line_string(&geometry_in(&io, line)?)?;
    let s = geometry_in(&io, splitter)?;
    let feats: Vec<Feature> = lines::line_split(&l, &s)
        .into_iter()
        .map(|p| gju::feature(&Geometry::LineString(p), o.properties.clone()))
        .collect();
    io.doc_out(fc(feats))
}

pub fn line_offset(line: &str, distance: f64, options: &str) -> Result<String> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    let l = crate::api::as_line_string(&geometry_in(&io, line)?)?;
    let d = ext_units(&o)?.to_meters(distance);
    let out = lines::line_offset(&l, d, ext_edges(&o)?, o.arc_steps.unwrap_or(32))?;
    feature_out(&io, &Geometry::LineString(out), o.properties.clone())
}

pub fn line_overlap(a: &str, b: &str, options: &str) -> Result<String> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    let tol = ext_units(&o)?.to_meters(o.tolerance.unwrap_or(0.0));
    let out = lines::line_overlap(&geometry_in(&io, a)?, &geometry_in(&io, b)?, tol.max(1e-6))?;
    let feats: Vec<Feature> = out
        .0
        .into_iter()
        .map(|l| gju::feature(&Geometry::LineString(l), o.properties.clone()))
        .collect();
    io.doc_out(fc(feats))
}

/// The point of `points` nearest to `lines` (turf's `nearestPointToLine`).
pub fn nearest_point_to_line(points: &str, line: &str, options: &str) -> Result<String> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    let pts = features_in(&io, points)?;
    if pts.is_empty() {
        return Err(Error::InvalidArgument("no candidate points".into()));
    }
    let l = crate::api::as_lines(&geometry_in(&io, line)?, ext_edges(&o)?)?;
    let coords: Vec<Coord> = pts.iter().map(|(c, _)| *c).collect();
    let (idx, dist) = lines::nearest_point_to_line(&coords, &l)
        .ok_or_else(|| Error::InvalidGeometry("the line has no segments".into()))?;
    let u = ext_units(&o)?;
    feature_out(
        &io,
        &Geometry::Point(Point(coords[idx])),
        with_props(pts[idx].1.clone(), json!({ "dist": u.from_meters(dist), "index": idx })),
    )
}

pub fn point_to_polygon_distance(x: f64, y: f64, polygon: &str, options: &str) -> Result<f64> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    let poly = geometry_in(&io, polygon)?;
    let d = lines::point_to_polygon_distance(io.pt_in(x, y)?, &poly, ext_edges(&o)?)?;
    Ok(ext_units(&o)?.from_meters(d))
}

/// Interior angle at `b` between `a-b` and `b-c`, in degrees (turf's `angle`).
pub fn angle(ax: f64, ay: f64, bx: f64, by: f64, cx: f64, cy: f64, options: &str) -> Result<f64> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    Ok(lines::angle(
        io.pt_in(ax, ay)?,
        io.pt_in(bx, by)?,
        io.pt_in(cx, cy)?,
        o.explementary.unwrap_or(false),
    ))
}

// -------------------------------------------------------------------- grids

fn grid_options(o: &ExtOptions) -> Result<GridOptions> {
    let u = ext_units(o)?;
    let w = u.to_meters(o.cell_width.unwrap_or(1.0));
    let h = u.to_meters(o.cell_height.unwrap_or(o.cell_width.unwrap_or(1.0)));
    Ok(GridOptions { width_m: w, height_m: h, mask_all_touched: true })
}

pub fn grid(bbox: &[f64], options: &str) -> Result<String> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    if bbox.len() < 4 {
        return Err(Error::InvalidArgument("bbox needs 4 numbers".into()));
    }
    let a = io.pt_in(bbox[0], bbox[1])?;
    let b = io.pt_in(bbox[2], bbox[3])?;
    let kind = GridKind::parse(o.grid_type.as_deref().unwrap_or("square"))?;
    let mask = optional_geometry(&io, &o.mask)?;
    let cells = grids::grid(
        [a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y)],
        kind,
        &grid_options(&o)?,
        mask.as_ref(),
    )?;
    let feats: Vec<Feature> = cells.iter().map(|g| gju::feature(g, o.properties.clone())).collect();
    io.doc_out(fc(feats))
}

/// The bbox of a geometry, squared off (turf's `square`).
pub fn square(bbox: &[f64]) -> Result<Vec<f64>> {
    if bbox.len() < 4 {
        return Err(Error::InvalidArgument("bbox needs 4 numbers".into()));
    }
    Ok(grids::square_bbox([bbox[0], bbox[1], bbox[2], bbox[3]]).to_vec())
}

pub fn envelope(geojson: &str, options: &str) -> Result<String> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    let gs = geometries_in(&io, geojson)?;
    let poly = grids::envelope(&gs)?;
    feature_out(&io, &Geometry::Polygon(poly), o.properties.clone())
}

// ---------------------------------------------------------------- shapes

pub fn ellipse(x: f64, y: f64, x_semi: f64, y_semi: f64, options: &str) -> Result<String> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    let u = ext_units(&o)?;
    let poly = shapes::ellipse(
        io.pt_in(x, y)?,
        u.to_meters(x_semi),
        u.to_meters(y_semi),
        o.angle.unwrap_or(0.0),
        o.steps.unwrap_or(64) as usize,
    );
    feature_out(&io, &Geometry::Polygon(poly), o.properties.clone())
}

pub fn polygon_smooth(geojson: &str, options: &str) -> Result<String> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    let doc = io.doc_in(geojson)?;
    let iterations = o.iterations.unwrap_or(1);
    let mut feats = Vec::new();
    for_each_feature(&doc, |g, props| {
        feats.push(gju::feature(&shapes::polygon_smooth(g, iterations)?, props.clone()));
        Ok(())
    })?;
    io.doc_out(fc(feats))
}

pub fn polygon_tangents(x: f64, y: f64, polygon: &str, options: &str) -> Result<String> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    let poly = geometry_in(&io, polygon)?;
    let (a, b) = shapes::polygon_tangents(io.pt_in(x, y)?, &poly)?;
    points_feature_collection(&io, &[a, b])
}

pub fn mask(polygon: &str, mask_polygon: &str, options: &str) -> Result<String> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    let inner = geometry_in(&io, polygon)?;
    let outer = geometry_in(&io, mask_polygon)?;
    let out = shapes::mask(&inner, &outer)?;
    let g = multipolygon_to_geom(out).ok_or_else(|| Error::InvalidGeometry("empty mask".into()))?;
    feature_out(&io, &g, o.properties.clone())
}

pub fn boolean_concave(polygon: &str, options: &str) -> Result<bool> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    match geometry_in(&io, polygon)? {
        Geometry::Polygon(p) => Ok(shapes::boolean_concave(&p)),
        other => Err(Error::InvalidGeometry(format!(
            "booleanConcave needs a Polygon, got {}",
            crate::overlay::geometry_type_name(&other)
        ))),
    }
}

pub fn boolean_parallel(a: &str, b: &str, options: &str) -> Result<bool> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    let la = crate::api::as_line_string(&geometry_in(&io, a)?)?;
    let lb = crate::api::as_line_string(&geometry_in(&io, b)?)?;
    Ok(shapes::boolean_parallel(&la, &lb, o.tolerance.unwrap_or(0.1)))
}

/// Swap every position's x and y (turf's `flip`).
pub fn flip(geojson: &str, options: &str) -> Result<String> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    let doc = io.doc_in(geojson)?;
    let mut feats = Vec::new();
    for_each_feature(&doc, |g, props| {
        feats.push(gju::feature(&shapes::flip(g), props.clone()));
        Ok(())
    })?;
    io.doc_out(fc(feats))
}

fn weights_of(io: &Io, geojson: &str, property: Option<&str>) -> Result<(Vec<Coord>, Option<Vec<f64>>)> {
    let feats = features_in(io, geojson)?;
    if feats.is_empty() {
        return Err(Error::InvalidArgument("no points".into()));
    }
    let pts: Vec<Coord> = feats.iter().map(|(c, _)| *c).collect();
    let w = property.map(|name| {
        feats
            .iter()
            .map(|(_, p)| {
                p.as_ref()
                    .and_then(|m| m.get(name))
                    .and_then(|v| v.as_f64())
                    .unwrap_or(1.0)
            })
            .collect()
    });
    Ok((pts, w))
}

/// Weighted mean centre (turf's `centerMean`).
pub fn center_mean(geojson: &str, options: &str) -> Result<String> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    let (pts, w) = weights_of(&io, geojson, o.weight_property.as_deref())?;
    let c = shapes::center_mean(&pts, w.as_deref())
        .ok_or_else(|| Error::InvalidArgument("no points".into()))?;
    feature_out(&io, &Geometry::Point(Point(c)), o.properties.clone())
}

/// Weighted median centre, the point minimising total distance (turf's
/// `centerMedian`).
pub fn center_median(geojson: &str, options: &str) -> Result<String> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    let (pts, w) = weights_of(&io, geojson, o.weight_property.as_deref())?;
    let tol = ext_units(&o)?.to_meters(o.tolerance.unwrap_or(0.001));
    let c = shapes::center_median(&pts, w.as_deref(), tol.max(1e-6))?;
    feature_out(&io, &Geometry::Point(Point(c)), o.properties.clone())
}

pub fn bezier_spline(line: &str, options: &str) -> Result<String> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    let l = crate::api::as_line_string(&geometry_in(&io, line)?)?;
    let out = shapes::bezier_spline(&l, o.sharpness.unwrap_or(0.85), o.steps.unwrap_or(16) as usize)?;
    feature_out(&io, &Geometry::LineString(out), o.properties.clone())
}

/// Build polygons from a noded set of lines (turf's `polygonize`).
pub fn polygonize(geojson: &str, options: &str) -> Result<String> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    let gs = geometries_in(&io, geojson)?;
    let polys = shapes::polygonize(&gs)?;
    let feats: Vec<Feature> = polys
        .iter()
        .map(|p| gju::feature(&Geometry::Polygon(p.clone()), o.properties.clone()))
        .collect();
    io.doc_out(fc(feats))
}

// --------------------------------------------------------- interpolation

pub fn interpolate(points: &str, options: &str) -> Result<String> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    let samples = samples_in(&io, points, o.z_property.as_deref())?;
    let geoms: Vec<Geometry> = samples.iter().map(|(c, _)| Geometry::Point(Point(*c))).collect();
    let bbox = bbox_of(&o, &geoms)?;
    let u = ext_units(&o)?;
    let idw = IdwOptions {
        power: o.power.unwrap_or(1.0),
        search_radius_m: o.search_radius.map(|r| u.to_meters(r)),
        grid: grid_options(&o)?,
        kind: GridKind::parse(o.grid_type.as_deref().unwrap_or("square"))?,
    };
    let mask = optional_geometry(&io, &o.mask)?;
    let out = interp::interpolate(&samples, bbox, &idw, mask.as_ref())?;
    let name = o.z_property.clone().unwrap_or_else(|| "elevation".to_string());
    let feats: Vec<Feature> = out
        .iter()
        .map(|(g, z)| gju::feature(g, props_of(json!({ name.clone(): z }))))
        .collect();
    io.doc_out(fc(feats))
}

fn grid_from(io: &Io, points: &str, z_property: Option<&str>) -> Result<Grid> {
    Grid::from_points(&samples_in(io, points, z_property)?)
}

pub fn isolines(points: &str, breaks: &[f64], options: &str) -> Result<String> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    let g = grid_from(&io, points, o.z_property.as_deref())?;
    let name = o.z_property.clone().unwrap_or_else(|| "elevation".to_string());
    let feats: Vec<Feature> = interp::isolines(&g, breaks)
        .into_iter()
        .map(|(level, ml)| {
            gju::feature(
                &Geometry::MultiLineString(ml),
                with_props(o.properties.clone(), json!({ name.clone(): level })),
            )
        })
        .collect();
    io.doc_out(fc(feats))
}

pub fn isobands(points: &str, breaks: &[f64], options: &str) -> Result<String> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    let g = grid_from(&io, points, o.z_property.as_deref())?;
    let name = o.z_property.clone().unwrap_or_else(|| "elevation".to_string());
    let feats: Vec<Feature> = interp::isobands(&g, breaks)?
        .into_iter()
        .map(|((lo, hi), mp)| {
            gju::feature(
                &Geometry::MultiPolygon(mp),
                with_props(
                    o.properties.clone(),
                    json!({ name.clone(): format!("{lo}-{hi}"), "min": lo, "max": hi }),
                ),
            )
        })
        .collect();
    io.doc_out(fc(feats))
}

pub fn tin(points: &str, options: &str) -> Result<String> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    let samples = samples_in(&io, points, o.z_property.as_deref())?;
    let feats: Vec<Feature> = interp::tin(&samples)?
        .into_iter()
        .map(|(p, z)| {
            gju::feature(
                &Geometry::Polygon(p),
                props_of(json!({ "a": z[0], "b": z[1], "c": z[2] })),
            )
        })
        .collect();
    io.doc_out(fc(feats))
}

pub fn voronoi(points: &str, options: &str) -> Result<String> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    let feats_in = features_in(&io, points)?;
    let pts: Vec<Coord> = feats_in.iter().map(|(c, _)| *c).collect();
    let geoms: Vec<Geometry> = pts.iter().map(|c| Geometry::Point(Point(*c))).collect();
    let bbox = match &o.bbox {
        Some(b) if b.len() >= 4 => {
            let a = io.pt_in(b[0], b[1])?;
            let d = io.pt_in(b[2], b[3])?;
            [a.x.min(d.x), a.y.min(d.y), a.x.max(d.x), a.y.max(d.y)]
        }
        _ => {
            // turf defaults to the points' own extent; pad it so the hull sites
            // get a cell of their own instead of a degenerate sliver
            let b = bbox_of(&o, &geoms)?;
            let (dx, dy) = (((b[2] - b[0]) * 0.1).max(1e-6), ((b[3] - b[1]) * 0.1).max(1e-6));
            [b[0] - dx, b[1] - dy, b[2] + dx, b[3] + dy]
        }
    };
    let cells = interp::voronoi(&pts, bbox)?;
    let feats: Vec<Feature> = cells
        .into_iter()
        .zip(feats_in.iter())
        .filter(|(c, _)| !c.exterior().0.is_empty())
        .map(|(c, (_, props))| gju::feature(&Geometry::Polygon(c), props.clone()))
        .collect();
    io.doc_out(fc(feats))
}

pub fn planepoint(x: f64, y: f64, triangle: &str, options: &str) -> Result<f64> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    let doc = io.doc_in(triangle)?;
    let props = gju::properties(&doc);
    let g = gju::single_geometry(&doc)?;
    let Geometry::Polygon(p) = g else {
        return Err(Error::InvalidGeometry("planepoint needs a triangle Polygon".into()));
    };
    let read = |k: &str| -> Result<f64> {
        props
            .as_ref()
            .and_then(|m| m.get(k))
            .and_then(|v| v.as_f64())
            .ok_or_else(|| Error::InvalidArgument(format!("the triangle needs a numeric `{k}` property")))
    };
    let z = [read("a")?, read("b")?, read("c")?];
    interp::planepoint(io.pt_in(x, y)?, &p, z)
}

// ------------------------------------------------------------- clustering

pub fn clusters_dbscan(points: &str, max_distance: f64, options: &str) -> Result<String> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    let feats_in = features_in(&io, points)?;
    let pts: Vec<Coord> = feats_in.iter().map(|(c, _)| *c).collect();
    let d = ext_units(&o)?.to_meters(max_distance);
    let labels = cluster::clusters_dbscan(&pts, d, o.min_points.unwrap_or(3))?;
    let feats: Vec<Feature> = feats_in
        .iter()
        .zip(labels.iter())
        .map(|((c, props), l)| {
            let extra = match l.cluster {
                Some(id) => json!({ "dbscan": l.role.as_str(), "cluster": id }),
                None => json!({ "dbscan": l.role.as_str() }),
            };
            gju::feature(&Geometry::Point(Point(*c)), with_props(props.clone(), extra))
        })
        .collect();
    io.doc_out(fc(feats))
}

pub fn clusters_kmeans(points: &str, options: &str) -> Result<String> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    let feats_in = features_in(&io, points)?;
    let pts: Vec<Coord> = feats_in.iter().map(|(c, _)| *c).collect();
    let r = cluster::clusters_kmeans(&pts, o.number_of_clusters.unwrap_or(0))?;
    let feats: Vec<Feature> = feats_in
        .iter()
        .enumerate()
        .map(|(i, (c, props))| {
            let id = r.assignment[i];
            let centre = io.pt_out(r.centroids[id]);
            gju::feature(
                &Geometry::Point(Point(*c)),
                with_props(props.clone(), json!({ "cluster": id, "centroid": centre })),
            )
        })
        .collect();
    io.doc_out(fc(feats))
}

/// turf returns the study area as a Feature carrying the statistics; so do we.
pub fn nearest_neighbour_analysis(points: &str, options: &str) -> Result<String> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    let pts = points_in(&io, points)?;
    let study = optional_geometry(&io, &o.study_area)?;
    let r = cluster::nearest_neighbour_analysis(&pts, study.as_ref())?;
    let u = ext_units(&o)?;
    let area_geom = match study {
        Some(g) => g,
        None => Geometry::Polygon(ops::convex_hull(
            &pts.iter().map(|c| Geometry::Point(Point(*c))).collect::<Vec<_>>(),
        )?),
    };
    feature_out(
        &io,
        &area_geom,
        props_of(json!({
            "nearestNeighborAnalysis": {
                "units": o.units.clone().unwrap_or_else(|| "kilometers".into()),
                "arealUnits": "meters",
                "numberOfPoints": r.points,
                "observedMeanDistance": u.from_meters(r.observed_mean_m),
                "expectedMeanDistance": u.from_meters(r.expected_mean_m),
                "studyAreaSize": r.area_m2,
                "nearestNeighborIndex": r.index,
                "zScore": r.z_score,
            }
        })),
    )
}

pub fn standard_deviational_ellipse(points: &str, options: &str) -> Result<String> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    let (pts, w) = weights_of(&io, points, o.weight_property.as_deref())?;
    let r = cluster::standard_deviational_ellipse(&pts, w.as_deref(), o.steps.unwrap_or(64) as usize)?;
    let u = ext_units(&o)?;
    feature_out(
        &io,
        &Geometry::Polygon(r.polygon.clone()),
        props_of(json!({
            "standardDeviationalEllipse": {
                "numberOfFeatures": pts.len(),
                "meanCenterCoordinates": io.pt_out(r.centre),
                "semiMajorAxis": u.from_meters(r.semi_major_m),
                "semiMinorAxis": u.from_meters(r.semi_minor_m),
                "rotation": r.rotation_deg,
                "majorAxisBearing": r.major_bearing_deg,
                "numberOfFeaturesContained": r.contained,
                "percentageOfFeaturesContained": r.percentage_contained,
            }
        })),
    )
}

pub fn directional_mean(geojson: &str, options: &str) -> Result<String> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    let gs = geometries_in(&io, geojson)?;
    // whole lines, not their segments: the direction of a line is its
    // start-to-end azimuth
    let mut ls: Vec<LineString> = Vec::new();
    for g in &gs {
        ls.extend(ops::line_strings_of(g));
    }
    let r = cluster::directional_mean(&ls)?;
    let u = ext_units(&o)?;
    Ok(json!({
        "countOfLines": r.lines,
        "bearingAngle": r.bearing_deg,
        "cartesianAngle": r.cartesian_deg,
        "circularVariance": r.circular_variance,
        "averageLength": u.from_meters(r.average_length_m),
        "totalLength": u.from_meters(r.total_length_m),
    })
    .to_string())
}

pub fn shortest_path(x1: f64, y1: f64, x2: f64, y2: f64, options: &str) -> Result<String> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    let u = ext_units(&o)?;
    let obstacles = match optional_geometry(&io, &o.obstacles) {
        Ok(g) => g.map(|g| vec![g]).unwrap_or_default(),
        // a FeatureCollection of obstacles is not a single geometry
        Err(_) => match &o.obstacles {
            Some(s) if !s.trim().is_empty() => geometries_in(&io, s)?,
            _ => vec![],
        },
    };
    let opts = cluster::PathOptions {
        resolution_m: u.to_meters(o.resolution.unwrap_or(1.0)),
        padding_m: u.to_meters(o.padding.unwrap_or(0.0)),
    };
    let path = cluster::shortest_path(io.pt_in(x1, y1)?, io.pt_in(x2, y2)?, &obstacles, &opts)?;
    feature_out(&io, &Geometry::LineString(path), o.properties.clone())
}

// ----------------------------------------------------- turf-shaped aliases

/// Self-intersections of a geometry (turf's `kinks`).
pub fn kinks(geojson: &str, options: &str) -> Result<String> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    let g = geometry_in(&io, geojson)?;
    let pts = crate::validate::self_intersections(&g);
    points_feature_collection(&io, &pts)
}

/// Split a self-intersecting polygon into valid pieces (turf's `unkinkPolygon`).
pub fn unkink_polygon(geojson: &str, options: &str) -> Result<String> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    let doc = io.doc_in(geojson)?;
    let mut feats = Vec::new();
    for_each_feature(&doc, |g, props| {
        let (fixed, _) = crate::validate::make_valid(g, &Default::default())?;
        match fixed {
            Geometry::MultiPolygon(mp) => {
                for p in mp.0 {
                    feats.push(gju::feature(&Geometry::Polygon(p), props.clone()));
                }
            }
            other => feats.push(gju::feature(&other, props.clone())),
        }
        Ok(())
    })?;
    io.doc_out(fc(feats))
}

/// Web-Mercator projection of the coordinates (turf's `toMercator`).
///
/// Straight through the same CRS machinery the rest of the library uses, so the
/// result matches EPSG:3857 to the millimetre rather than approximating it.
pub fn to_mercator(geojson: &str) -> Result<String> {
    crate::api::transform(geojson, "EPSG:4326", "EPSG:3857")
}

/// Inverse of [`to_mercator`] (turf's `toWgs84`).
pub fn to_wgs84(geojson: &str) -> Result<String> {
    crate::api::transform(geojson, "EPSG:3857", "EPSG:4326")
}

/// Sum of a numeric property over the features inside each polygon (turf's
/// `collect`), returned as the polygons with an added array property.
pub fn collect(polygons: &str, points: &str, in_property: &str, out_property: &str, options: &str) -> Result<String> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    let pts = features_in(&io, points)?;
    let doc = io.doc_in(polygons)?;
    let mut feats = Vec::new();
    for_each_feature(&doc, |g, props| {
        let prepared = crate::index::Prepared::new(g.clone())?;
        let values: Vec<Value> = pts
            .iter()
            .filter(|(c, _)| prepared.contains_point(*c, false))
            .filter_map(|(_, p)| p.as_ref().and_then(|m| m.get(in_property)).cloned())
            .collect();
        feats.push(gju::feature(g, with_props(props.clone(), json!({ out_property: values }))));
        Ok(())
    })?;
    io.doc_out(fc(feats))
}

/// Copy a polygon property onto the points inside it (turf's `tag`).
pub fn tag(points: &str, polygons: &str, field: &str, out_field: &str, options: &str) -> Result<String> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    let polys = {
        let doc = io.doc_in(polygons)?;
        let mut v: Vec<(crate::index::Prepared, Option<JsonObject>)> = Vec::new();
        for_each_feature(&doc, |g, props| {
            v.push((crate::index::Prepared::new(g.clone())?, props));
            Ok(())
        })?;
        v
    };
    let pts = features_in(&io, points)?;
    let feats: Vec<Feature> = pts
        .iter()
        .map(|(c, props)| {
            let hit = polys
                .iter()
                .find(|(p, _)| p.contains_point(*c, false))
                .and_then(|(_, pp)| pp.as_ref().and_then(|m| m.get(field)).cloned());
            let extra = match hit {
                Some(v) => json!({ out_field: v }),
                None => json!({}),
            };
            gju::feature(&Geometry::Point(Point(*c)), with_props(props.clone(), extra))
        })
        .collect();
    io.doc_out(fc(feats))
}

// ------------------------------------------------- arcs and tesselation

/// The arc of a circle between two bearings (turf's `lineArc`).
pub fn line_arc(x: f64, y: f64, radius: f64, bearing1: f64, bearing2: f64, options: &str) -> Result<String> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    let r = ext_units(&o)?.to_meters(radius);
    let arc = ops::line_arc(io.pt_in(x, y)?, r, bearing1, bearing2, o.steps.unwrap_or(64) as usize);
    feature_out(&io, &Geometry::LineString(arc), o.properties.clone())
}

/// Triangulate a polygon, holes included (turf's `tesselate`).
pub fn tesselate(polygon: &str, options: &str) -> Result<String> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    let g = geometry_in(&io, polygon)?;
    let polys: Vec<geo::Polygon> = match &g {
        Geometry::Polygon(p) => stats::tesselate(p)?,
        Geometry::MultiPolygon(mp) => {
            let mut v = Vec::new();
            for p in &mp.0 {
                v.extend(stats::tesselate(p)?);
            }
            v
        }
        other => {
            return Err(Error::InvalidGeometry(format!(
                "tesselate needs a Polygon, got {}",
                crate::overlay::geometry_type_name(other)
            )))
        }
    };
    let feats: Vec<Feature> = polys
        .iter()
        .map(|p| gju::feature(&Geometry::Polygon(p.clone()), o.properties.clone()))
        .collect();
    io.doc_out(fc(feats))
}

// ------------------------------------------------------ spatial statistics

fn weight_options(o: &ExtOptions) -> Result<stats::WeightOptions> {
    let u = ext_units(o)?;
    Ok(stats::WeightOptions {
        threshold_m: u.to_meters(o.threshold.unwrap_or(10.0)),
        alpha: o.alpha.unwrap_or(-1.0),
        binary: o.binary.unwrap_or(true),
        standardization: match o.standardization.as_deref().unwrap_or("row") {
            "raw" | "none" => stats::Standardization::Raw,
            "row" | "w" => stats::Standardization::Row,
            other => {
                return Err(Error::InvalidArgument(format!(
                    "standardization must be `row` or `raw`, got `{other}`"
                )))
            }
        },
    })
}

/// The spatial weight matrix over a point set (turf's `distanceWeight`).
pub fn distance_weight(points: &str, options: &str) -> Result<String> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    let pts = points_in(&io, points)?;
    let w = stats::distance_weight(&pts, &weight_options(&o)?)?;
    Ok(serde_json::to_string(&w)?)
}

/// Moran's I for a property attached to points (turf's `moranIndex`).
pub fn moran_index(points: &str, options: &str) -> Result<String> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    let samples = samples_in(&io, points, o.z_property.as_deref())?;
    let pts: Vec<Coord> = samples.iter().map(|(c, _)| *c).collect();
    let vals: Vec<f64> = samples.iter().map(|(_, z)| *z).collect();
    let r = stats::moran_index(&pts, &vals, &weight_options(&o)?)?;
    Ok(json!({
        "moranIndex": r.moran_index,
        "expectedMoranIndex": r.expected_moran_index,
        "varianceMoranIndex": r.variance_moran_index,
        "zNorm": r.z_norm,
        "pNorm": r.p_norm,
    })
    .to_string())
}

/// Quadrat count test for complete spatial randomness (turf's `quadratAnalysis`).
pub fn quadrat_analysis(points: &str, options: &str) -> Result<String> {
    let o = ext_opts(options)?;
    let io = ext_io(&o)?;
    let pts = points_in(&io, points)?;
    let study = optional_geometry(&io, &o.study_area)?;
    let nx = o.x_quadrats.unwrap_or(4);
    let ny = o.y_quadrats.unwrap_or(nx);
    let r = stats::quadrat_analysis(&pts, study.as_ref(), nx, ny)?;
    Ok(json!({
        "quadrats": r.quadrats,
        "xQuadrats": nx,
        "yQuadrats": ny,
        "numberOfPoints": r.points,
        "counts": r.counts,
        "expected": r.expected,
        "varianceMeanRatio": r.variance_mean_ratio,
        "chiSquared": r.chi_squared,
        "degreesOfFreedom": r.degrees_of_freedom,
        "criticalValue": r.critical_value,
        "isRandom": r.is_random,
    })
    .to_string())
}
