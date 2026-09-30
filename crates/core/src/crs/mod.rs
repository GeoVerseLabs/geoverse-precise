//! Coordinate reference systems and transforms.
//!
//! Every transform pivots through WGS84 geographic coordinates:
//! `source → WGS84 (lon, lat) → target`.

pub mod china;
pub mod tmerc;
pub mod webmerc;

use serde::Deserialize;

pub use tmerc::{Ellipsoid, TmParams, TransverseMercator};

use crate::{Error, Result};

/// A supported coordinate reference system.
#[derive(Debug, Clone, PartialEq)]
pub enum Crs {
    /// EPSG:4326
    Wgs84,
    /// EPSG:4490 – treated as the same frame as WGS84 (difference < 1 dm).
    Cgcs2000,
    /// GCJ-02 (AutoNavi / Tencent / Google China).
    Gcj02,
    /// BD-09 (Baidu) geographic.
    Bd09,
    /// EPSG:3857
    WebMercator,
    /// Transverse Mercator: UTM, CGCS2000 Gauss-Krüger or custom.
    Tmerc { params: TmParams, epsg: Option<u32> },
}

#[derive(Deserialize)]
struct CustomTm {
    proj: String,
    lon0: f64,
    #[serde(default)]
    lat0: f64,
    #[serde(default = "one")]
    k0: f64,
    #[serde(default)]
    x0: f64,
    #[serde(default)]
    y0: f64,
    #[serde(default)]
    ellps: Option<String>,
}

fn one() -> f64 {
    1.0
}

impl Crs {
    /// Parse a CRS identifier.
    ///
    /// Accepts `WGS84`, `EPSG:4326`, `CGCS2000`, `EPSG:4490`, `GCJ02`, `BD09`,
    /// `EPSG:3857`, `EPSG:326xx/327xx` (UTM), `EPSG:4491–4554` (CGCS2000
    /// Gauss-Krüger) and a JSON object
    /// `{"proj":"tmerc","lon0":120,"k0":1,"x0":500000,"y0":0,"ellps":"CGCS2000"}`.
    pub fn parse(s: &str) -> Result<Crs> {
        let t = s.trim();
        if t.starts_with('{') {
            let c: CustomTm = serde_json::from_str(t)?;
            if !c.proj.eq_ignore_ascii_case("tmerc") {
                return Err(Error::UnsupportedCrs(c.proj));
            }
            let ellipsoid = match c.ellps.as_deref() {
                None => Ellipsoid::WGS84,
                Some(n) => Ellipsoid::by_name(n).ok_or_else(|| Error::UnsupportedCrs(n.into()))?,
            };
            return Ok(Crs::Tmerc {
                params: TmParams {
                    ellipsoid,
                    lon0: c.lon0,
                    lat0: c.lat0,
                    k0: c.k0,
                    x0: c.x0,
                    y0: c.y0,
                },
                epsg: None,
            });
        }
        let u = t.to_ascii_uppercase().replace(['-', '_', ' '], "");
        match u.as_str() {
            "WGS84" | "EPSG:4326" | "4326" | "CRS84" | "OGC:CRS84" | "URN:OGC:DEF:CRS:OGC:1.3:CRS84" => {
                return Ok(Crs::Wgs84)
            }
            "CGCS2000" | "EPSG:4490" | "4490" => return Ok(Crs::Cgcs2000),
            "GCJ02" | "GCJ" | "AMAP" | "MARS" => return Ok(Crs::Gcj02),
            "BD09" | "BD09LL" | "BAIDU" => return Ok(Crs::Bd09),
            "EPSG:3857" | "3857" | "EPSG:900913" | "EPSG:3785" | "WEBMERCATOR" => return Ok(Crs::WebMercator),
            _ => {}
        }
        let code = u
            .strip_prefix("EPSG:")
            .unwrap_or(&u)
            .parse::<u32>()
            .map_err(|_| Error::UnsupportedCrs(s.to_string()))?;
        Crs::from_epsg(code).ok_or_else(|| Error::UnsupportedCrs(s.to_string()))
    }

    pub fn from_epsg(code: u32) -> Option<Crs> {
        let tm = |ellipsoid, lon0: f64, k0, x0, y0| Crs::Tmerc {
            params: TmParams {
                ellipsoid,
                lon0,
                lat0: 0.0,
                k0,
                x0,
                y0,
            },
            epsg: Some(code),
        };
        match code {
            4326 => Some(Crs::Wgs84),
            4490 => Some(Crs::Cgcs2000),
            3857 | 900913 | 3785 => Some(Crs::WebMercator),
            32601..=32660 => {
                let z = (code - 32600) as f64;
                Some(tm(Ellipsoid::WGS84, 6.0 * z - 183.0, 0.9996, 500000.0, 0.0))
            }
            32701..=32760 => {
                let z = (code - 32700) as f64;
                Some(tm(Ellipsoid::WGS84, 6.0 * z - 183.0, 0.9996, 500000.0, 10_000_000.0))
            }
            // CGCS2000 / Gauss-Kruger zone 13..23 (6°, zone-prefixed easting)
            4491..=4501 => {
                let z = (code - 4491 + 13) as f64;
                Some(tm(Ellipsoid::CGCS2000, 6.0 * z - 3.0, 1.0, z * 1e6 + 500000.0, 0.0))
            }
            // CGCS2000 / Gauss-Kruger CM 75E..135E (6°)
            4502..=4512 => {
                let z = (code - 4502 + 13) as f64;
                Some(tm(Ellipsoid::CGCS2000, 6.0 * z - 3.0, 1.0, 500000.0, 0.0))
            }
            // CGCS2000 / 3-degree Gauss-Kruger zone 25..45 (zone-prefixed)
            4513..=4533 => {
                let z = (code - 4513 + 25) as f64;
                Some(tm(Ellipsoid::CGCS2000, 3.0 * z, 1.0, z * 1e6 + 500000.0, 0.0))
            }
            // CGCS2000 / 3-degree Gauss-Kruger CM 75E..135E
            4534..=4554 => {
                let z = (code - 4534 + 25) as f64;
                Some(tm(Ellipsoid::CGCS2000, 3.0 * z, 1.0, 500000.0, 0.0))
            }
            _ => None,
        }
    }

    /// CGCS2000 Gauss-Krüger CRS for a longitude.
    ///
    /// * `zone_width` – 3 or 6 degrees
    /// * `zone_prefix` – whether the false easting carries the zone number
    ///   (e.g. 40 500 000 m) or is plain 500 000 m.
    pub fn gauss_kruger(lon: f64, zone_width: u8, zone_prefix: bool) -> Result<Crs> {
        let (zone, lon0) = match zone_width {
            3 => {
                let z = (lon / 3.0).round();
                (z, 3.0 * z)
            }
            6 => {
                let z = (lon / 6.0).floor() + 1.0;
                (z, 6.0 * z - 3.0)
            }
            w => return Err(Error::InvalidArgument(format!("zone width must be 3 or 6, got {w}"))),
        };
        let epsg = match (zone_width, zone_prefix) {
            (3, true) if (25.0..=45.0).contains(&zone) => Some(4513 + (zone as u32 - 25)),
            (3, false) if (25.0..=45.0).contains(&zone) => Some(4534 + (zone as u32 - 25)),
            (6, true) if (13.0..=23.0).contains(&zone) => Some(4491 + (zone as u32 - 13)),
            (6, false) if (13.0..=23.0).contains(&zone) => Some(4502 + (zone as u32 - 13)),
            _ => None,
        };
        let x0 = if zone_prefix { zone * 1e6 + 500000.0 } else { 500000.0 };
        Ok(Crs::Tmerc {
            params: TmParams {
                ellipsoid: Ellipsoid::CGCS2000,
                lon0,
                lat0: 0.0,
                k0: 1.0,
                x0,
                y0: 0.0,
            },
            epsg,
        })
    }

    /// WGS84 UTM zone containing a position.
    pub fn utm(lon: f64, lat: f64) -> Crs {
        let zone = (((lon + 180.0) / 6.0).floor() as i32).clamp(0, 59) + 1;
        let code = if lat >= 0.0 { 32600 } else { 32700 } + zone as u32;
        Crs::from_epsg(code).expect("valid UTM code")
    }

    pub fn is_geographic(&self) -> bool {
        matches!(self, Crs::Wgs84 | Crs::Cgcs2000 | Crs::Gcj02 | Crs::Bd09)
    }

    /// True when coordinates are WGS84-equivalent lon/lat (no transform needed
    /// for measurement).
    pub fn is_wgs84_equivalent(&self) -> bool {
        matches!(self, Crs::Wgs84 | Crs::Cgcs2000)
    }

    /// Canonical identifier.
    pub fn id(&self) -> String {
        match self {
            Crs::Wgs84 => "EPSG:4326".into(),
            Crs::Cgcs2000 => "EPSG:4490".into(),
            Crs::Gcj02 => "GCJ02".into(),
            Crs::Bd09 => "BD09".into(),
            Crs::WebMercator => "EPSG:3857".into(),
            Crs::Tmerc { epsg: Some(c), .. } => format!("EPSG:{c}"),
            Crs::Tmerc { params: p, epsg: None } => format!(
                "{{\"proj\":\"tmerc\",\"lon0\":{},\"lat0\":{},\"k0\":{},\"x0\":{},\"y0\":{},\"ellps\":\"{}\"}}",
                p.lon0,
                p.lat0,
                p.k0,
                p.x0,
                p.y0,
                if p.ellipsoid == Ellipsoid::WGS84 {
                    "WGS84"
                } else {
                    "CGCS2000"
                }
            ),
        }
    }
}

enum Step {
    Identity,
    Gcj02,
    Bd09,
    WebMercator,
    Tm(Box<TransverseMercator>),
}

impl Step {
    fn new(crs: &Crs) -> Step {
        match crs {
            Crs::Wgs84 | Crs::Cgcs2000 => Step::Identity,
            Crs::Gcj02 => Step::Gcj02,
            Crs::Bd09 => Step::Bd09,
            Crs::WebMercator => Step::WebMercator,
            Crs::Tmerc { params, .. } => Step::Tm(Box::new(TransverseMercator::new(*params))),
        }
    }

    #[inline]
    fn unproject_to_wgs84(&self, x: f64, y: f64) -> (f64, f64) {
        match self {
            Step::Identity => (x, y),
            Step::Gcj02 => china::gcj02_to_wgs84(x, y),
            Step::Bd09 => china::bd09_to_wgs84(x, y),
            Step::WebMercator => webmerc::reverse(x, y),
            Step::Tm(tm) => tm.reverse(x, y),
        }
    }

    #[inline]
    fn project_from_wgs84(&self, lon: f64, lat: f64) -> (f64, f64) {
        match self {
            Step::Identity => (lon, lat),
            Step::Gcj02 => china::wgs84_to_gcj02(lon, lat),
            Step::Bd09 => china::wgs84_to_bd09(lon, lat),
            Step::WebMercator => webmerc::forward(lon, lat),
            Step::Tm(tm) => tm.forward(lon, lat),
        }
    }
}

/// A prepared transform between two CRSs.
pub struct Transformer {
    src: Step,
    dst: Step,
    shortcut: Shortcut,
}

enum Shortcut {
    None,
    Identity,
    GcjToBd,
    BdToGcj,
}

impl Transformer {
    pub fn new(from: &Crs, to: &Crs) -> Transformer {
        let shortcut = match (from, to) {
            (a, b) if a == b => Shortcut::Identity,
            (a, b) if a.is_wgs84_equivalent() && b.is_wgs84_equivalent() => Shortcut::Identity,
            (Crs::Gcj02, Crs::Bd09) => Shortcut::GcjToBd,
            (Crs::Bd09, Crs::Gcj02) => Shortcut::BdToGcj,
            _ => Shortcut::None,
        };
        Transformer {
            src: Step::new(from),
            dst: Step::new(to),
            shortcut,
        }
    }

    pub fn is_identity(&self) -> bool {
        matches!(self.shortcut, Shortcut::Identity)
    }

    #[inline]
    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        match self.shortcut {
            Shortcut::Identity => (x, y),
            Shortcut::GcjToBd => china::gcj02_to_bd09(x, y),
            Shortcut::BdToGcj => china::bd09_to_gcj02(x, y),
            Shortcut::None => {
                let (lon, lat) = self.src.unproject_to_wgs84(x, y);
                self.dst.project_from_wgs84(lon, lat)
            }
        }
    }

    /// Transform an interleaved buffer in place (`stride` = 2 for xy, 3 for xyz …).
    pub fn apply_slice(&self, coords: &mut [f64], stride: usize) {
        if self.is_identity() || stride < 2 {
            return;
        }
        for c in coords.chunks_exact_mut(stride) {
            let (x, y) = self.apply(c[0], c[1]);
            c[0] = x;
            c[1] = y;
        }
    }
}

/// One-shot convenience.
pub fn transform_point(from: &Crs, to: &Crs, x: f64, y: f64) -> (f64, f64) {
    Transformer::new(from, to).apply(x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_variants() {
        assert_eq!(Crs::parse("wgs84").unwrap(), Crs::Wgs84);
        assert_eq!(Crs::parse("GCJ-02").unwrap(), Crs::Gcj02);
        assert_eq!(Crs::parse("bd09").unwrap(), Crs::Bd09);
        assert_eq!(Crs::parse("EPSG:3857").unwrap(), Crs::WebMercator);
        assert!(matches!(Crs::parse("EPSG:4549").unwrap(), Crs::Tmerc { .. }));
        assert!(Crs::parse("EPSG:9999").is_err());
        let c = Crs::parse(r#"{"proj":"tmerc","lon0":114,"x0":500000,"ellps":"CGCS2000"}"#).unwrap();
        assert!(matches!(c, Crs::Tmerc { .. }));
    }

    #[test]
    fn gk_zone_selection() {
        assert_eq!(Crs::gauss_kruger(120.3, 3, false).unwrap().id(), "EPSG:4549");
        assert_eq!(Crs::gauss_kruger(120.3, 3, true).unwrap().id(), "EPSG:4528");
        assert_eq!(Crs::gauss_kruger(116.4, 6, true).unwrap().id(), "EPSG:4498");
        assert_eq!(Crs::gauss_kruger(116.4, 6, false).unwrap().id(), "EPSG:4509");
    }
}
