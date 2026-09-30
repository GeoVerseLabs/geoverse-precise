//! Ellipsoidal transverse Mercator using Krüger's series to 6th order in n
//! (Karney 2011, "Transverse Mercator with an accuracy of a few nanometers").
//! Accurate to ~5 nm within 3900 km of the central meridian.

use serde::{Deserialize, Serialize};

use crate::geodesic::normalize_deg;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Ellipsoid {
    pub a: f64,
    pub f: f64,
}

impl Ellipsoid {
    pub const WGS84: Ellipsoid = Ellipsoid {
        a: 6378137.0,
        f: 1.0 / 298.257223563,
    };
    pub const CGCS2000: Ellipsoid = Ellipsoid {
        a: 6378137.0,
        f: 1.0 / 298.257222101,
    };
    pub const GRS80: Ellipsoid = Ellipsoid {
        a: 6378137.0,
        f: 1.0 / 298.257222101,
    };

    pub fn by_name(name: &str) -> Option<Ellipsoid> {
        match name.to_ascii_uppercase().as_str() {
            "WGS84" | "WGS 84" => Some(Self::WGS84),
            "CGCS2000" => Some(Self::CGCS2000),
            "GRS80" => Some(Self::GRS80),
            _ => None,
        }
    }
}

/// Parameters of a transverse Mercator projection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TmParams {
    pub ellipsoid: Ellipsoid,
    /// Central meridian (degrees).
    pub lon0: f64,
    /// Latitude of origin (degrees).
    pub lat0: f64,
    /// Scale on the central meridian.
    pub k0: f64,
    /// False easting (m).
    pub x0: f64,
    /// False northing (m).
    pub y0: f64,
}

/// Pre-computed transverse Mercator projection.
#[derive(Debug, Clone)]
pub struct TransverseMercator {
    pub params: TmParams,
    e: f64,
    /// k0 * rectifying radius
    k0a1: f64,
    alp: [f64; 7],
    bet: [f64; 7],
    /// Northing of the latitude of origin on the central meridian (unscaled by false northing).
    y_origin: f64,
}

fn taupf(tau: f64, es: f64) -> f64 {
    let tau1 = tau.hypot(1.0);
    let sig = (es * (es * tau / tau1).atanh()).sinh();
    sig.hypot(1.0) * tau - sig * tau1
}

fn tauf(taup: f64, es: f64) -> f64 {
    let e2m = 1.0 - es * es;
    let tol = f64::EPSILON.sqrt() / 10.0;
    let mut tau = taup / e2m;
    let stol = tol * taup.abs().max(1.0);
    for _ in 0..8 {
        let taupa = taupf(tau, es);
        let dtau = (taup - taupa) * (1.0 + e2m * tau * tau) / (e2m * tau.hypot(1.0) * taupa.hypot(1.0));
        tau += dtau;
        if dtau.abs() < stol {
            break;
        }
    }
    tau
}

/// Clenshaw summation of `Σ c[j] · sin(2j·ζ)` for the complex argument
/// `ζ = ξ + iη`, returning `(Re, Im)`.
///
/// Evaluating the series term by term costs six `sin`/`cos`/`sinh`/`cosh`
/// pairs; the recurrence needs only one of each.
#[inline]
fn clenshaw_sin(c: &[f64; 7], xi: f64, eta: f64) -> (f64, f64) {
    let (s2, c2) = (2.0 * xi).sin_cos();
    let (sh2, ch2) = ((2.0 * eta).sinh(), (2.0 * eta).cosh());
    // cos(2ζ) and sin(2ζ)
    let (cr, ci) = (c2 * ch2, -s2 * sh2);
    let (sr, si) = (s2 * ch2, c2 * sh2);
    // u_j = 2·cos(2ζ)·u_{j+1} − u_{j+2} + c_j   (complex)
    let (mut u1r, mut u1i) = (0.0, 0.0);
    let (mut u2r, mut u2i) = (0.0, 0.0);
    for j in (1..=6).rev() {
        let tr = 2.0 * (cr * u1r - ci * u1i) - u2r + c[j];
        let ti = 2.0 * (cr * u1i + ci * u1r) - u2i;
        u2r = u1r;
        u2i = u1i;
        u1r = tr;
        u1i = ti;
    }
    // result = sin(2ζ) · u_1
    (sr * u1r - si * u1i, sr * u1i + si * u1r)
}

impl TransverseMercator {
    pub fn new(params: TmParams) -> Self {
        let f = params.ellipsoid.f;
        let a = params.ellipsoid.a;
        let es2 = f * (2.0 - f);
        let e = es2.sqrt();
        let n = f / (2.0 - f);
        let n2 = n * n;
        let n3 = n2 * n;
        let n4 = n3 * n;
        let n5 = n4 * n;
        let n6 = n5 * n;
        let a1 = a / (1.0 + n) * (1.0 + n2 / 4.0 + n4 / 64.0 + n6 / 256.0);
        let alp = [
            0.0,
            n / 2.0 - 2.0 * n2 / 3.0 + 5.0 * n3 / 16.0 + 41.0 * n4 / 180.0 - 127.0 * n5 / 288.0 + 7891.0 * n6 / 37800.0,
            13.0 * n2 / 48.0 - 3.0 * n3 / 5.0 + 557.0 * n4 / 1440.0 + 281.0 * n5 / 630.0 - 1983433.0 * n6 / 1935360.0,
            61.0 * n3 / 240.0 - 103.0 * n4 / 140.0 + 15061.0 * n5 / 26880.0 + 167603.0 * n6 / 181440.0,
            49561.0 * n4 / 161280.0 - 179.0 * n5 / 168.0 + 6601661.0 * n6 / 7257600.0,
            34729.0 * n5 / 80640.0 - 3418889.0 * n6 / 1995840.0,
            212378941.0 * n6 / 319334400.0,
        ];
        let bet = [
            0.0,
            n / 2.0 - 2.0 * n2 / 3.0 + 37.0 * n3 / 96.0 - n4 / 360.0 - 81.0 * n5 / 512.0 + 96199.0 * n6 / 604800.0,
            n2 / 48.0 + n3 / 15.0 - 437.0 * n4 / 1440.0 + 46.0 * n5 / 105.0 - 1118711.0 * n6 / 3870720.0,
            17.0 * n3 / 480.0 - 37.0 * n4 / 840.0 - 209.0 * n5 / 4480.0 + 5569.0 * n6 / 90720.0,
            4397.0 * n4 / 161280.0 - 11.0 * n5 / 504.0 - 830251.0 * n6 / 7257600.0,
            4583.0 * n5 / 161280.0 - 108847.0 * n6 / 3991680.0,
            20648693.0 * n6 / 638668800.0,
        ];
        let mut tm = TransverseMercator {
            params,
            e,
            k0a1: params.k0 * a1,
            alp,
            bet,
            y_origin: 0.0,
        };
        if params.lat0 != 0.0 {
            let (_, y) = tm.raw_forward(0.0, params.lat0);
            tm.y_origin = y;
        }
        tm
    }

    /// Forward without false easting/northing, `dlon` relative to the CM.
    fn raw_forward(&self, dlon: f64, lat: f64) -> (f64, f64) {
        let phi = lat.to_radians();
        let lam = dlon.to_radians();
        let tau = phi.tan();
        let taup = if lat.abs() >= 90.0 {
            f64::INFINITY.copysign(lat)
        } else {
            taupf(tau, self.e)
        };
        let (xip, etap) = if lat.abs() >= 90.0 {
            (std::f64::consts::FRAC_PI_2.copysign(lat), 0.0)
        } else {
            let xip = taup.atan2(lam.cos());
            let etap = (lam.sin() / taup.hypot(lam.cos())).asinh();
            (xip, etap)
        };
        let (dxi, deta) = clenshaw_sin(&self.alp, xip, etap);
        (self.k0a1 * (etap + deta), self.k0a1 * (xip + dxi))
    }

    /// Geographic (degrees) → projected (metres).
    pub fn forward(&self, lon: f64, lat: f64) -> (f64, f64) {
        let dlon = normalize_deg(lon - self.params.lon0);
        let (x, y) = self.raw_forward(dlon, lat);
        (x + self.params.x0, y - self.y_origin + self.params.y0)
    }

    /// Projected (metres) → geographic (degrees).
    pub fn reverse(&self, x: f64, y: f64) -> (f64, f64) {
        let xi = (y - self.params.y0 + self.y_origin) / self.k0a1;
        let eta = (x - self.params.x0) / self.k0a1;
        let (dxi, deta) = clenshaw_sin(&self.bet, xi, eta);
        let xip = xi - dxi;
        let etap = eta - deta;
        let s = etap.sinh();
        let c = xip.cos().max(0.0);
        let r = s.hypot(c);
        let (lat, lam) = if r != 0.0 {
            let taup = xip.sin() / r;
            let tau = tauf(taup, self.e);
            (tau.atan().to_degrees(), s.atan2(c).to_degrees())
        } else {
            (90.0f64.copysign(xip), 0.0)
        };
        (normalize_deg(lam + self.params.lon0), lat)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_wide() {
        let tm = TransverseMercator::new(TmParams {
            ellipsoid: Ellipsoid::CGCS2000,
            lon0: 117.0,
            lat0: 0.0,
            k0: 1.0,
            x0: 500000.0,
            y0: 0.0,
        });
        for &(lon, lat) in &[(117.0, 0.0), (120.5, 31.2), (100.0, 45.0), (135.0, 20.0), (80.0, 40.0)] {
            let (x, y) = tm.forward(lon, lat);
            let (lo, la) = tm.reverse(x, y);
            assert!(
                (lo - lon).abs() < 1e-9 && (la - lat).abs() < 1e-9,
                "{lon},{lat} -> {lo},{la}"
            );
        }
    }
}
