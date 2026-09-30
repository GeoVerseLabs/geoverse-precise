//! GeoJSON helpers: parsing, structure-preserving coordinate mapping and
//! conversion to/from `geo` geometries.

use geo::Geometry as GeoGeometry;
use geojson::{Feature, FeatureCollection, GeoJson, Geometry, GeometryValue, JsonObject, Position};

use crate::crs::{Crs, Transformer};
use crate::{Error, Result};

pub fn parse(s: &str) -> Result<GeoJson> {
    Ok(s.parse::<GeoJson>()?)
}

fn map_positions_value(v: &mut GeometryValue, f: &mut impl FnMut(&mut Position)) {
    match v {
        GeometryValue::Point { coordinates } => f(coordinates),
        GeometryValue::MultiPoint { coordinates } | GeometryValue::LineString { coordinates } => {
            coordinates.iter_mut().for_each(f)
        }
        GeometryValue::MultiLineString { coordinates } | GeometryValue::Polygon { coordinates } => {
            coordinates.iter_mut().flatten().for_each(f)
        }
        GeometryValue::MultiPolygon { coordinates } => coordinates.iter_mut().flatten().flatten().for_each(f),
        GeometryValue::GeometryCollection { geometries } => {
            for g in geometries {
                map_positions_value(&mut g.value, f)
            }
        }
    }
}

/// Visit every position in place (features, properties and extra dimensions preserved).
pub fn map_positions(gj: &mut GeoJson, mut f: impl FnMut(&mut Position)) {
    match gj {
        GeoJson::Geometry(g) => map_positions_value(&mut g.value, &mut f),
        GeoJson::Feature(feat) => {
            if let Some(g) = feat.geometry.as_mut() {
                map_positions_value(&mut g.value, &mut f)
            }
        }
        GeoJson::FeatureCollection(fc) => {
            for feat in fc.features.iter_mut() {
                if let Some(g) = feat.geometry.as_mut() {
                    map_positions_value(&mut g.value, &mut f)
                }
            }
        }
    }
    // Any bbox is now stale.
    match gj {
        GeoJson::Geometry(g) => g.bbox = None,
        GeoJson::Feature(feat) => {
            feat.bbox = None;
            if let Some(g) = feat.geometry.as_mut() {
                g.bbox = None
            }
        }
        GeoJson::FeatureCollection(fc) => {
            fc.bbox = None;
            for feat in fc.features.iter_mut() {
                feat.bbox = None;
            }
        }
    }
}

/// Transform all coordinates between two CRSs.
pub fn transform(gj: &mut GeoJson, from: &Crs, to: &Crs) {
    let t = Transformer::new(from, to);
    if t.is_identity() {
        return;
    }
    map_positions(gj, |p| {
        let s = p.as_slice_mut();
        if s.len() >= 2 {
            let (x, y) = t.apply(s[0], s[1]);
            s[0] = x;
            s[1] = y;
        }
    });
}

pub fn value_to_geo(v: &GeometryValue) -> Result<GeoGeometry> {
    GeoGeometry::try_from(v).map_err(Error::from)
}

pub fn geo_to_geometry(g: &GeoGeometry) -> Geometry {
    Geometry::new(GeometryValue::from(g))
}

/// All geometries contained in a GeoJSON document, flattened.
pub fn geometries(gj: &GeoJson) -> Result<Vec<GeoGeometry>> {
    match gj {
        GeoJson::Geometry(g) => Ok(vec![value_to_geo(&g.value)?]),
        GeoJson::Feature(f) => match &f.geometry {
            Some(g) => Ok(vec![value_to_geo(&g.value)?]),
            None => Ok(vec![]),
        },
        GeoJson::FeatureCollection(fc) => fc
            .features
            .iter()
            .filter_map(|f| f.geometry.as_ref())
            .map(|g| value_to_geo(&g.value))
            .collect(),
    }
}

/// The single geometry of a Geometry or Feature (error for collections / empty).
pub fn single_geometry(gj: &GeoJson) -> Result<GeoGeometry> {
    match gj {
        GeoJson::Geometry(g) => value_to_geo(&g.value),
        GeoJson::Feature(f) => f
            .geometry
            .as_ref()
            .ok_or_else(|| Error::InvalidGeometry("feature has no geometry".into()))
            .and_then(|g| value_to_geo(&g.value)),
        GeoJson::FeatureCollection(_) => Err(Error::InvalidGeometry(
            "expected a Geometry or Feature, got FeatureCollection".into(),
        )),
    }
}

pub fn properties(gj: &GeoJson) -> Option<JsonObject> {
    match gj {
        GeoJson::Feature(f) => f.properties.clone(),
        _ => None,
    }
}

pub fn feature(g: &GeoGeometry, properties: Option<JsonObject>) -> Feature {
    Feature {
        bbox: None,
        geometry: Some(geo_to_geometry(g)),
        id: None,
        properties: Some(properties.unwrap_or_default()),
        foreign_members: None,
    }
}

/// Apply `f` to each feature's geometry, keeping ids and properties.
/// A bare Geometry input becomes a Feature (turf convention).
pub fn map_features(gj: &GeoJson, mut f: impl FnMut(&GeoGeometry) -> Result<Option<GeoGeometry>>) -> Result<GeoJson> {
    let mut one = |feat: &Feature| -> Result<Feature> {
        let geometry = match &feat.geometry {
            Some(g) => f(&value_to_geo(&g.value)?)?.map(|r| geo_to_geometry(&r)),
            None => None,
        };
        Ok(Feature {
            bbox: None,
            geometry,
            id: feat.id.clone(),
            properties: Some(feat.properties.clone().unwrap_or_default()),
            foreign_members: feat.foreign_members.clone(),
        })
    };
    Ok(match gj {
        GeoJson::Geometry(g) => GeoJson::Feature(one(&Feature::from(g.clone()))?),
        GeoJson::Feature(feat) => GeoJson::Feature(one(feat)?),
        GeoJson::FeatureCollection(fc) => GeoJson::FeatureCollection(FeatureCollection {
            bbox: None,
            features: fc.features.iter().map(&mut one).collect::<Result<Vec<_>>>()?,
            foreign_members: fc.foreign_members.clone(),
        }),
    })
}
