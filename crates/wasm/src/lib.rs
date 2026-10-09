//! WebAssembly bindings for geoverse-precise-core.
//!
//! Geometries cross the boundary as JSON strings (faster than per-value
//! marshalling for large coordinate arrays); the TypeScript wrapper in
//! `packages/geoverse-precise` handles stringify / parse and argument normalisation.

use geoverse_precise_core::api;
use geoverse_precise_core::api_ext as ext;
use geoverse_precise_core::overlay::OverlayOp;
use wasm_bindgen::prelude::*;

type R<T> = Result<T, JsError>;

fn e(err: geoverse_precise_core::Error) -> JsError {
    JsError::new(&err.to_string())
}

#[wasm_bindgen]
pub fn version() -> String {
    api::VERSION.to_string()
}

#[wasm_bindgen]
pub fn distance(x1: f64, y1: f64, x2: f64, y2: f64, units: &str, crs: &str) -> R<f64> {
    api::distance(x1, y1, x2, y2, units, crs).map_err(e)
}

#[wasm_bindgen]
pub fn bearing(x1: f64, y1: f64, x2: f64, y2: f64, final_bearing: bool, crs: &str) -> R<f64> {
    api::bearing(x1, y1, x2, y2, final_bearing, crs).map_err(e)
}

#[wasm_bindgen]
pub fn destination(x: f64, y: f64, dist: f64, bearing: f64, units: &str, crs: &str) -> R<Vec<f64>> {
    api::destination(x, y, dist, bearing, units, crs)
        .map(|p| p.to_vec())
        .map_err(e)
}

#[wasm_bindgen]
pub fn midpoint(x1: f64, y1: f64, x2: f64, y2: f64, crs: &str) -> R<Vec<f64>> {
    api::midpoint(x1, y1, x2, y2, crs).map(|p| p.to_vec()).map_err(e)
}

#[wasm_bindgen]
pub fn length(geojson: &str, units: &str, crs: &str, edges: &str) -> R<f64> {
    api::length(geojson, units, crs, edges).map_err(e)
}

#[wasm_bindgen]
pub fn area(geojson: &str, crs: &str, edges: &str) -> R<f64> {
    api::area(geojson, crs, edges).map_err(e)
}

#[wasm_bindgen]
pub fn along(line: &str, dist: f64, units: &str, crs: &str, edges: &str) -> R<Vec<f64>> {
    api::along(line, dist, units, crs, edges).map(|p| p.to_vec()).map_err(e)
}

#[wasm_bindgen(js_name = nearestPointOnLine)]
pub fn nearest_point_on_line(line: &str, x: f64, y: f64, units: &str, crs: &str, edges: &str) -> R<String> {
    api::nearest_point_on_line(line, x, y, units, crs, edges).map_err(e)
}

#[wasm_bindgen(js_name = pointToLineDistance)]
pub fn point_to_line_distance(x: f64, y: f64, line: &str, units: &str, crs: &str, edges: &str) -> R<f64> {
    api::point_to_line_distance(x, y, line, units, crs, edges).map_err(e)
}

#[wasm_bindgen]
pub fn circle(x: f64, y: f64, radius: f64, units: &str, steps: u32, crs: &str) -> R<String> {
    api::circle(x, y, radius, units, steps, crs).map_err(e)
}

#[wasm_bindgen]
pub fn buffer(geojson: &str, radius: f64, options: &str) -> R<String> {
    api::buffer(geojson, radius, options).map_err(e)
}

#[wasm_bindgen]
pub fn intersect(a: &str, b: &str, options: &str) -> R<String> {
    api::overlay(a, b, OverlayOp::Intersection, options).map_err(e)
}

#[wasm_bindgen]
pub fn union(a: &str, b: &str, options: &str) -> R<String> {
    api::overlay(a, b, OverlayOp::Union, options).map_err(e)
}

#[wasm_bindgen]
pub fn difference(a: &str, b: &str, options: &str) -> R<String> {
    api::overlay(a, b, OverlayOp::Difference, options).map_err(e)
}

#[wasm_bindgen]
pub fn xor(a: &str, b: &str, options: &str) -> R<String> {
    api::overlay(a, b, OverlayOp::Xor, options).map_err(e)
}

#[wasm_bindgen(js_name = unionAll)]
pub fn union_all(fc: &str, options: &str) -> R<String> {
    api::union_all(fc, options).map_err(e)
}

#[wasm_bindgen(js_name = booleanPointInPolygon)]
pub fn boolean_point_in_polygon(x: f64, y: f64, polygon: &str, ignore_boundary: bool) -> R<bool> {
    api::boolean_point_in_polygon(x, y, polygon, ignore_boundary).map_err(e)
}

#[wasm_bindgen(js_name = booleanIntersects)]
pub fn boolean_intersects(a: &str, b: &str) -> R<bool> {
    api::boolean_intersects(a, b).map_err(e)
}

#[wasm_bindgen]
pub fn transform(geojson: &str, from: &str, to: &str) -> R<String> {
    api::transform(geojson, from, to).map_err(e)
}

/// Transforms an interleaved coordinate array in place
/// (wasm-bindgen copies the result back into the caller's Float64Array).
#[wasm_bindgen(js_name = transformCoords)]
pub fn transform_coords(coords: &mut [f64], stride: usize, from: &str, to: &str) -> R<()> {
    api::transform_coords(coords, stride, from, to).map_err(e)
}

#[wasm_bindgen(js_name = normalizeCrs)]
pub fn normalize_crs(s: &str) -> R<String> {
    api::normalize_crs(s).map_err(e)
}

#[wasm_bindgen(js_name = gaussKrugerCrs)]
pub fn gauss_kruger_crs(lon: f64, zone_width: u8, zone_prefix: bool) -> R<String> {
    api::gauss_kruger_crs(lon, zone_width, zone_prefix).map_err(e)
}

#[wasm_bindgen(js_name = utmCrs)]
pub fn utm_crs(lon: f64, lat: f64) -> String {
    api::utm_crs(lon, lat)
}

// ------------------------------------------------------------------ batch

/// Distances for interleaved `[x1, y1, x2, y2, …]` pairs.
#[wasm_bindgen(js_name = distanceBatch)]
pub fn distance_batch(pairs: &[f64], units: &str, crs: &str) -> R<Vec<f64>> {
    api::distance_batch(pairs, units, crs).map_err(e)
}

/// Distances from one origin to interleaved `[x, y, …]` points.
#[wasm_bindgen(js_name = distanceToBatch)]
pub fn distance_to_batch(x: f64, y: f64, coords: &[f64], units: &str, crs: &str) -> R<Vec<f64>> {
    api::distance_to_batch(x, y, coords, units, crs).map_err(e)
}

/// Destinations for `[x, y, distance, bearing, …]` rows.
#[wasm_bindgen(js_name = destinationBatch)]
pub fn destination_batch(rows: &[f64], units: &str, crs: &str) -> R<Vec<f64>> {
    api::destination_batch(rows, units, crs).map_err(e)
}

// ------------------------------------------------------- prepared geometry

/// A geometry parsed and indexed once, for repeated point queries.
///
/// Call `free()` when done — the geometry lives in WASM memory.
#[wasm_bindgen]
pub struct PreparedGeometry {
    inner: geoverse_precise_core::index::PreparedInCrs,
    units: f64,
}

#[wasm_bindgen]
impl PreparedGeometry {
    #[wasm_bindgen(constructor)]
    pub fn new(geojson: &str, crs: &str, units: &str) -> R<PreparedGeometry> {
        let c = api::crs(crs).map_err(e)?;
        let mut gj = geoverse_precise_core::geojson_util::parse(geojson).map_err(e)?;
        geoverse_precise_core::geojson_util::transform(&mut gj, &c, &geoverse_precise_core::crs::Crs::Wgs84);
        let geoms = geoverse_precise_core::geojson_util::geometries(&gj).map_err(e)?;
        let geometry = match geoms.len() {
            0 => return Err(JsError::new("no geometry")),
            1 => geoms.into_iter().next().unwrap(),
            _ => geoverse_precise_core::geo::Geometry::GeometryCollection(
                geoverse_precise_core::geo::GeometryCollection(geoms),
            ),
        };
        let units = geoverse_precise_core::units::Units::parse(units)
            .map_err(e)?
            .meters_per_unit();
        Ok(PreparedGeometry {
            inner: geoverse_precise_core::index::PreparedInCrs::new(geometry, &c).map_err(e)?,
            units,
        })
    }

    #[wasm_bindgen(js_name = segmentCount)]
    pub fn segment_count(&self) -> usize {
        self.inner.prepared.segment_count()
    }

    #[wasm_bindgen(js_name = isAreal)]
    pub fn is_areal(&self) -> bool {
        self.inner.prepared.is_areal()
    }

    /// `[minX, minY, maxX, maxY]` in the prepared CRS.
    pub fn bbox(&self) -> Vec<f64> {
        match self.inner.prepared.bounding_rect() {
            Some(r) => {
                let a = self.inner.point_out(r.min());
                let b = self.inner.point_out(r.max());
                vec![a[0], a[1], b[0], b[1]]
            }
            None => vec![f64::NAN; 4],
        }
    }

    #[wasm_bindgen(js_name = containsPoint)]
    pub fn contains_point(&self, x: f64, y: f64, ignore_boundary: bool) -> bool {
        self.inner
            .prepared
            .contains_point(self.inner.point_in(x, y), ignore_boundary)
    }

    /// One byte per interleaved `[x, y, …]` point: 1 = inside.
    #[wasm_bindgen(js_name = containsPoints)]
    pub fn contains_points(&self, coords: &[f64], ignore_boundary: bool) -> Vec<u8> {
        coords
            .chunks_exact(2)
            .map(|c| {
                u8::from(
                    self.inner
                        .prepared
                        .contains_point(self.inner.point_in(c[0], c[1]), ignore_boundary),
                )
            })
            .collect()
    }

    /// `[x, y, dist, location, index, part]` for the closest point.
    pub fn nearest(&self, x: f64, y: f64) -> Vec<f64> {
        match self.inner.prepared.nearest(self.inner.point_in(x, y)) {
            Some(n) => {
                let p = self.inner.point_out(n.point);
                vec![
                    p[0],
                    p[1],
                    n.dist / self.units,
                    n.location / self.units,
                    n.index as f64,
                    n.multi_index as f64,
                ]
            }
            None => vec![f64::NAN; 6],
        }
    }

    /// Six values per interleaved `[x, y, …]` query point (see `nearest`).
    #[wasm_bindgen(js_name = nearestBatch)]
    pub fn nearest_batch(&self, coords: &[f64]) -> Vec<f64> {
        let mut out = Vec::with_capacity(coords.len() * 3);
        for c in coords.chunks_exact(2) {
            match self.inner.prepared.nearest(self.inner.point_in(c[0], c[1])) {
                Some(n) => {
                    let p = self.inner.point_out(n.point);
                    out.extend_from_slice(&[
                        p[0],
                        p[1],
                        n.dist / self.units,
                        n.location / self.units,
                        n.index as f64,
                        n.multi_index as f64,
                    ]);
                }
                None => out.extend_from_slice(&[f64::NAN; 6]),
            }
        }
        out
    }

    /// Distance from each point to the geometry (0 inside an areal geometry).
    #[wasm_bindgen(js_name = distanceBatch)]
    pub fn distance_batch(&self, coords: &[f64]) -> Vec<f64> {
        coords
            .chunks_exact(2)
            .map(|c| self.inner.prepared.distance_to(self.inner.point_in(c[0], c[1])) / self.units)
            .collect()
    }

    /// Indices of the interleaved points that lie within `radius` of the geometry.
    #[wasm_bindgen(js_name = pointsWithin)]
    pub fn points_within(&self, coords: &[f64], radius: f64) -> Vec<u32> {
        let r = radius * self.units;
        coords
            .chunks_exact(2)
            .enumerate()
            .filter(|(_, c)| self.inner.prepared.distance_to(self.inner.point_in(c[0], c[1])) <= r)
            .map(|(i, _)| i as u32)
            .collect()
    }
}

// ------------------------------------------------------------ geometry ops

#[wasm_bindgen]
pub fn simplify(geojson: &str, tolerance: f64, options: &str) -> R<String> {
    api::simplify(geojson, tolerance, options).map_err(e)
}

#[wasm_bindgen(js_name = convexHull)]
pub fn convex_hull(geojson: &str, options: &str) -> R<String> {
    api::convex_hull(geojson, options).map_err(e)
}

#[wasm_bindgen(js_name = concaveHull)]
pub fn concave_hull(geojson: &str, options: &str) -> R<String> {
    api::concave_hull(geojson, options).map_err(e)
}

/// `kind`: "centroid", "centerOfMass" or "pointOnFeature".
#[wasm_bindgen]
pub fn center(geojson: &str, kind: &str, options: &str) -> R<String> {
    api::center(geojson, kind, options).map_err(e)
}

#[wasm_bindgen]
pub fn bbox(geojson: &str, options: &str) -> R<Vec<f64>> {
    api::bbox(geojson, options).map_err(e)
}

#[wasm_bindgen(js_name = bboxPolygon)]
pub fn bbox_polygon(b: &[f64], options: &str) -> R<String> {
    api::bbox_polygon(b, options).map_err(e)
}

#[wasm_bindgen(js_name = bboxClip)]
pub fn bbox_clip(geojson: &str, b: &[f64], options: &str) -> R<String> {
    api::bbox_clip(geojson, b, options).map_err(e)
}

/// `kind`: "rotate", "translate" or "scale".
#[wasm_bindgen(js_name = transformGeometry)]
pub fn transform_geometry(geojson: &str, kind: &str, a: f64, b: f64, options: &str) -> R<String> {
    api::transform_geometry(geojson, kind, a, b, options).map_err(e)
}

#[wasm_bindgen(js_name = lineSliceAlong)]
pub fn line_slice_along(line: &str, start: f64, stop: f64, options: &str) -> R<String> {
    api::line_slice_along(line, start, stop, options).map_err(e)
}

#[wasm_bindgen(js_name = lineSlice)]
pub fn line_slice(line: &str, x1: f64, y1: f64, x2: f64, y2: f64, options: &str) -> R<String> {
    api::line_slice(line, x1, y1, x2, y2, options).map_err(e)
}

#[wasm_bindgen(js_name = lineChunk)]
pub fn line_chunk(line: &str, length: f64, options: &str) -> R<String> {
    api::line_chunk(line, length, options).map_err(e)
}

#[wasm_bindgen(js_name = lineIntersect)]
pub fn line_intersect(a: &str, b: &str, options: &str) -> R<String> {
    api::line_intersect(a, b, options).map_err(e)
}

#[wasm_bindgen(js_name = greatCircle)]
pub fn great_circle(x1: f64, y1: f64, x2: f64, y2: f64, options: &str) -> R<String> {
    api::great_circle(x1, y1, x2, y2, options).map_err(e)
}

#[wasm_bindgen]
pub fn sector(x: f64, y: f64, radius: f64, bearing1: f64, bearing2: f64, options: &str) -> R<String> {
    api::sector(x, y, radius, bearing1, bearing2, options).map_err(e)
}

/// `[index, distance, x, y]` of the nearest point in a collection.
#[wasm_bindgen(js_name = nearestPoint)]
pub fn nearest_point(x: f64, y: f64, points: &str, options: &str) -> R<Vec<f64>> {
    api::nearest_point(x, y, points, options).map_err(e)
}

/// `kind`: "flatten", "explode", "polygonToLine", "lineToPolygon", "rewind",
/// "cleanCoords" or "truncate".
#[wasm_bindgen]
pub fn reshape(geojson: &str, kind: &str, options: &str) -> R<String> {
    api::reshape(geojson, kind, options).map_err(e)
}

#[wasm_bindgen]
pub fn dissolve(fc: &str, property: &str, options: &str) -> R<String> {
    api::dissolve(fc, property, options).map_err(e)
}

#[wasm_bindgen(js_name = pointsWithinPolygon)]
pub fn points_within_polygon(points: &str, polygon: &str, options: &str) -> R<String> {
    api::points_within_polygon(points, polygon, options).map_err(e)
}

// -------------------------------------------------------------- predicates

/// The nine-character DE-9IM matrix.
#[wasm_bindgen]
pub fn relate(a: &str, b: &str, options: &str) -> R<String> {
    api::relate(a, b, options).map_err(e)
}

#[wasm_bindgen(js_name = relatePattern)]
pub fn relate_pattern(a: &str, b: &str, pattern: &str, options: &str) -> R<bool> {
    api::relate_pattern(a, b, pattern, options).map_err(e)
}

/// `predicate`: contains, within, covers, coveredBy, touches, crosses,
/// overlaps, disjoint, intersects or equals.
#[wasm_bindgen(js_name = booleanRelation)]
pub fn boolean_relation(a: &str, b: &str, predicate: &str, options: &str) -> R<bool> {
    api::boolean_relation(a, b, predicate, options).map_err(e)
}

#[wasm_bindgen(js_name = booleanPointOnLine)]
pub fn boolean_point_on_line(x: f64, y: f64, line: &str, tolerance: f64, ignore_ends: bool, options: &str) -> R<bool> {
    api::boolean_point_on_line(x, y, line, tolerance, ignore_ends, options).map_err(e)
}

// ---------------------------------------------------------------- topology

#[wasm_bindgen]
pub fn validate(geojson: &str, options: &str) -> R<String> {
    api::validate(geojson, options).map_err(e)
}

#[wasm_bindgen(js_name = makeValid)]
pub fn make_valid(geojson: &str, options: &str) -> R<String> {
    api::make_valid(geojson, options).map_err(e)
}

#[wasm_bindgen(js_name = coverageIssues)]
pub fn coverage_issues(fc: &str, options: &str) -> R<String> {
    api::coverage_issues(fc, options).map_err(e)
}

#[wasm_bindgen(js_name = networkIssues)]
pub fn network_issues(fc: &str, options: &str) -> R<String> {
    api::network_issues(fc, options).map_err(e)
}

#[wasm_bindgen(js_name = snapRound)]
pub fn snap_round(geojson: &str, grid: f64, options: &str) -> R<String> {
    api::snap_round(geojson, grid, options).map_err(e)
}

#[wasm_bindgen(js_name = snapTo)]
pub fn snap_to(geojson: &str, reference: &str, tolerance: f64, options: &str) -> R<String> {
    api::snap_to(geojson, reference, tolerance, options).map_err(e)
}

// ------------------------------------------------------- turf parity layer

#[wasm_bindgen(js_name = rhumbDistance)]
pub fn rhumb_distance(x1: f64, y1: f64, x2: f64, y2: f64, units: &str, crs: &str) -> R<f64> {
    ext::rhumb_distance(x1, y1, x2, y2, units, crs).map_err(e)
}

#[wasm_bindgen(js_name = rhumbBearing)]
pub fn rhumb_bearing(x1: f64, y1: f64, x2: f64, y2: f64, final_bearing: bool, crs: &str) -> R<f64> {
    ext::rhumb_bearing(x1, y1, x2, y2, final_bearing, crs).map_err(e)
}

#[wasm_bindgen(js_name = rhumbDestination)]
pub fn rhumb_destination(x: f64, y: f64, dist: f64, bearing: f64, units: &str, crs: &str) -> R<Vec<f64>> {
    ext::rhumb_destination(x, y, dist, bearing, units, crs)
        .map(|p| p.to_vec())
        .map_err(e)
}

#[wasm_bindgen(js_name = lineSegment)]
pub fn line_segment(geojson: &str, options: &str) -> R<String> {
    ext::line_segment(geojson, options).map_err(e)
}

#[wasm_bindgen(js_name = lineSplit)]
pub fn line_split(line: &str, splitter: &str, options: &str) -> R<String> {
    ext::line_split(line, splitter, options).map_err(e)
}

#[wasm_bindgen(js_name = lineOffset)]
pub fn line_offset(line: &str, distance: f64, options: &str) -> R<String> {
    ext::line_offset(line, distance, options).map_err(e)
}

#[wasm_bindgen(js_name = lineOverlap)]
pub fn line_overlap(a: &str, b: &str, options: &str) -> R<String> {
    ext::line_overlap(a, b, options).map_err(e)
}

#[wasm_bindgen(js_name = nearestPointToLine)]
pub fn nearest_point_to_line(points: &str, line: &str, options: &str) -> R<String> {
    ext::nearest_point_to_line(points, line, options).map_err(e)
}

#[wasm_bindgen(js_name = pointToPolygonDistance)]
pub fn point_to_polygon_distance(x: f64, y: f64, polygon: &str, options: &str) -> R<f64> {
    ext::point_to_polygon_distance(x, y, polygon, options).map_err(e)
}

#[wasm_bindgen]
pub fn angle(ax: f64, ay: f64, bx: f64, by: f64, cx: f64, cy: f64, options: &str) -> R<f64> {
    ext::angle(ax, ay, bx, by, cx, cy, options).map_err(e)
}

#[wasm_bindgen]
pub fn grid(bbox: &[f64], options: &str) -> R<String> {
    ext::grid(bbox, options).map_err(e)
}

#[wasm_bindgen(js_name = squareBbox)]
pub fn square_bbox(bbox: &[f64]) -> R<Vec<f64>> {
    ext::square(bbox).map_err(e)
}

#[wasm_bindgen]
pub fn envelope(geojson: &str, options: &str) -> R<String> {
    ext::envelope(geojson, options).map_err(e)
}

#[wasm_bindgen]
pub fn ellipse(x: f64, y: f64, x_semi: f64, y_semi: f64, options: &str) -> R<String> {
    ext::ellipse(x, y, x_semi, y_semi, options).map_err(e)
}

#[wasm_bindgen(js_name = polygonSmooth)]
pub fn polygon_smooth(geojson: &str, options: &str) -> R<String> {
    ext::polygon_smooth(geojson, options).map_err(e)
}

#[wasm_bindgen(js_name = polygonTangents)]
pub fn polygon_tangents(x: f64, y: f64, polygon: &str, options: &str) -> R<String> {
    ext::polygon_tangents(x, y, polygon, options).map_err(e)
}

#[wasm_bindgen]
pub fn mask(polygon: &str, mask_polygon: &str, options: &str) -> R<String> {
    ext::mask(polygon, mask_polygon, options).map_err(e)
}

#[wasm_bindgen(js_name = booleanConcave)]
pub fn boolean_concave(polygon: &str, options: &str) -> R<bool> {
    ext::boolean_concave(polygon, options).map_err(e)
}

#[wasm_bindgen(js_name = booleanParallel)]
pub fn boolean_parallel(a: &str, b: &str, options: &str) -> R<bool> {
    ext::boolean_parallel(a, b, options).map_err(e)
}

#[wasm_bindgen]
pub fn flip(geojson: &str, options: &str) -> R<String> {
    ext::flip(geojson, options).map_err(e)
}

#[wasm_bindgen(js_name = centerMean)]
pub fn center_mean(geojson: &str, options: &str) -> R<String> {
    ext::center_mean(geojson, options).map_err(e)
}

#[wasm_bindgen(js_name = centerMedian)]
pub fn center_median(geojson: &str, options: &str) -> R<String> {
    ext::center_median(geojson, options).map_err(e)
}

#[wasm_bindgen(js_name = bezierSpline)]
pub fn bezier_spline(line: &str, options: &str) -> R<String> {
    ext::bezier_spline(line, options).map_err(e)
}

#[wasm_bindgen]
pub fn polygonize(geojson: &str, options: &str) -> R<String> {
    ext::polygonize(geojson, options).map_err(e)
}

#[wasm_bindgen]
pub fn interpolate(points: &str, options: &str) -> R<String> {
    ext::interpolate(points, options).map_err(e)
}

#[wasm_bindgen]
pub fn isolines(points: &str, breaks: &[f64], options: &str) -> R<String> {
    ext::isolines(points, breaks, options).map_err(e)
}

#[wasm_bindgen]
pub fn isobands(points: &str, breaks: &[f64], options: &str) -> R<String> {
    ext::isobands(points, breaks, options).map_err(e)
}

#[wasm_bindgen]
pub fn tin(points: &str, options: &str) -> R<String> {
    ext::tin(points, options).map_err(e)
}

#[wasm_bindgen]
pub fn voronoi(points: &str, options: &str) -> R<String> {
    ext::voronoi(points, options).map_err(e)
}

#[wasm_bindgen]
pub fn planepoint(x: f64, y: f64, triangle: &str, options: &str) -> R<f64> {
    ext::planepoint(x, y, triangle, options).map_err(e)
}

#[wasm_bindgen(js_name = clustersDbscan)]
pub fn clusters_dbscan(points: &str, max_distance: f64, options: &str) -> R<String> {
    ext::clusters_dbscan(points, max_distance, options).map_err(e)
}

#[wasm_bindgen(js_name = clustersKmeans)]
pub fn clusters_kmeans(points: &str, options: &str) -> R<String> {
    ext::clusters_kmeans(points, options).map_err(e)
}

#[wasm_bindgen(js_name = nearestNeighborAnalysis)]
pub fn nearest_neighbour_analysis(points: &str, options: &str) -> R<String> {
    ext::nearest_neighbour_analysis(points, options).map_err(e)
}

#[wasm_bindgen(js_name = standardDeviationalEllipse)]
pub fn standard_deviational_ellipse(points: &str, options: &str) -> R<String> {
    ext::standard_deviational_ellipse(points, options).map_err(e)
}

#[wasm_bindgen(js_name = directionalMean)]
pub fn directional_mean(geojson: &str, options: &str) -> R<String> {
    ext::directional_mean(geojson, options).map_err(e)
}

#[wasm_bindgen(js_name = shortestPath)]
pub fn shortest_path(x1: f64, y1: f64, x2: f64, y2: f64, options: &str) -> R<String> {
    ext::shortest_path(x1, y1, x2, y2, options).map_err(e)
}

#[wasm_bindgen]
pub fn kinks(geojson: &str, options: &str) -> R<String> {
    ext::kinks(geojson, options).map_err(e)
}

#[wasm_bindgen(js_name = unkinkPolygon)]
pub fn unkink_polygon(geojson: &str, options: &str) -> R<String> {
    ext::unkink_polygon(geojson, options).map_err(e)
}

#[wasm_bindgen(js_name = toMercator)]
pub fn to_mercator(geojson: &str) -> R<String> {
    ext::to_mercator(geojson).map_err(e)
}

#[wasm_bindgen(js_name = toWgs84)]
pub fn to_wgs84(geojson: &str) -> R<String> {
    ext::to_wgs84(geojson).map_err(e)
}

#[wasm_bindgen]
pub fn collect(polygons: &str, points: &str, in_property: &str, out_property: &str, options: &str) -> R<String> {
    ext::collect(polygons, points, in_property, out_property, options).map_err(e)
}

#[wasm_bindgen]
pub fn tag(points: &str, polygons: &str, field: &str, out_field: &str, options: &str) -> R<String> {
    ext::tag(points, polygons, field, out_field, options).map_err(e)
}

#[wasm_bindgen(js_name = lineArc)]
pub fn line_arc(x: f64, y: f64, radius: f64, bearing1: f64, bearing2: f64, options: &str) -> R<String> {
    ext::line_arc(x, y, radius, bearing1, bearing2, options).map_err(e)
}

#[wasm_bindgen]
pub fn tesselate(polygon: &str, options: &str) -> R<String> {
    ext::tesselate(polygon, options).map_err(e)
}

#[wasm_bindgen(js_name = distanceWeight)]
pub fn distance_weight(points: &str, options: &str) -> R<String> {
    ext::distance_weight(points, options).map_err(e)
}

#[wasm_bindgen(js_name = moranIndex)]
pub fn moran_index(points: &str, options: &str) -> R<String> {
    ext::moran_index(points, options).map_err(e)
}

#[wasm_bindgen(js_name = quadratAnalysis)]
pub fn quadrat_analysis(points: &str, options: &str) -> R<String> {
    ext::quadrat_analysis(points, options).map_err(e)
}
