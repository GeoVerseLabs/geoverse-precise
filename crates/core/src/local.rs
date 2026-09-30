//! A local conformal working plane (transverse Mercator centred on the data).
//!
//! Used as a *topology workspace* by buffer and overlay: vertex positions are
//! computed on the ellipsoid, then projected here only so that planar
//! robust boolean operations can resolve overlaps.

use geo::{BoundingRect, Coord, Geometry, MapCoords, Rect};

use crate::crs::{Ellipsoid, TmParams, TransverseMercator};
use crate::{Error, Result};

/// Maximum distance (km, approx.) between the centre and any vertex.
const MAX_EXTENT_KM: f64 = 3900.0;

pub struct LocalFrame {
    tm: TransverseMercator,
    pub lon0: f64,
    pub lat0: f64,
}

impl LocalFrame {
    pub fn new(lon0: f64, lat0: f64) -> Self {
        let tm = TransverseMercator::new(TmParams {
            ellipsoid: Ellipsoid::WGS84,
            lon0,
            lat0,
            k0: 1.0,
            x0: 0.0,
            y0: 0.0,
        });
        LocalFrame { tm, lon0, lat0 }
    }

    /// Frame centred on the bounding box of one or more geometries.
    pub fn for_geometries<'a>(geoms: impl IntoIterator<Item = &'a Geometry<f64>>) -> Result<Self> {
        let mut rect: Option<Rect<f64>> = None;
        for g in geoms {
            if let Some(r) = g.bounding_rect() {
                rect = Some(match rect {
                    None => r,
                    Some(acc) => Rect::new(
                        Coord {
                            x: acc.min().x.min(r.min().x),
                            y: acc.min().y.min(r.min().y),
                        },
                        Coord {
                            x: acc.max().x.max(r.max().x),
                            y: acc.max().y.max(r.max().y),
                        },
                    ),
                });
            }
        }
        let rect = rect.ok_or_else(|| Error::InvalidGeometry("empty geometry".into()))?;
        let c = rect.center();
        let frame = LocalFrame::new(c.x, c.y);
        frame.check_extent(&rect, 0.0)?;
        Ok(frame)
    }

    /// Guard against extents where the conformal plane is no longer usable.
    pub fn check_extent(&self, rect: &Rect<f64>, pad_m: f64) -> Result<()> {
        let half_lon = (rect.max().x - self.lon0).abs().max((self.lon0 - rect.min().x).abs());
        let min_lat_abs = if rect.min().y <= 0.0 && rect.max().y >= 0.0 {
            0.0
        } else {
            rect.min().y.abs().min(rect.max().y.abs())
        };
        // Worst case east-west distance happens at the latitude closest to the equator.
        let ew_km = half_lon.to_radians() * 6378.137 * min_lat_abs.to_radians().cos();
        let km = ew_km + pad_m / 1000.0;
        if km > MAX_EXTENT_KM || half_lon > 60.0 {
            return Err(Error::ExtentTooLarge(km));
        }
        Ok(())
    }

    #[inline]
    pub fn project(&self, lon: f64, lat: f64) -> (f64, f64) {
        self.tm.forward(lon, lat)
    }

    #[inline]
    pub fn unproject(&self, x: f64, y: f64) -> (f64, f64) {
        self.tm.reverse(x, y)
    }

    pub fn project_geom<G: MapCoords<f64, f64>>(&self, g: &G) -> G::Output {
        g.map_coords(|c| {
            let (x, y) = self.project(c.x, c.y);
            Coord { x, y }
        })
    }

    pub fn unproject_geom<G: MapCoords<f64, f64>>(&self, g: &G) -> G::Output {
        g.map_coords(|c| {
            let (x, y) = self.unproject(c.x, c.y);
            Coord { x, y }
        })
    }
}
