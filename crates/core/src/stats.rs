//! Spatial-statistics tests: spatial weights, Moran's I and quadrat analysis.
//!
//! Distances use the WGS84 geodesic and quadrats are laid out in a local metric
//! plane, so a "5 km band" or an equal-area quadrat means the same thing at
//! every latitude.

use geo::{Coord, Geometry, Point, Polygon};

use crate::local::LocalFrame;
use crate::{measure, ops, predicates, Error, Result};

// ------------------------------------------------------------ spatial weights

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standardization {
    /// Leave the weights as computed.
    Raw,
    /// Scale each row to sum to 1 (the usual choice for Moran's I).
    Row,
}

#[derive(Debug, Clone, Copy)]
pub struct WeightOptions {
    /// Neighbourhood radius in metres. Pairs further apart get weight 0.
    pub threshold_m: f64,
    /// Distance decay exponent for the non-binary form: w = 1 / d^alpha.
    pub alpha: f64,
    /// `true` gives every neighbour weight 1 (turf's default).
    pub binary: bool,
    pub standardization: Standardization,
}

impl Default for WeightOptions {
    fn default() -> Self {
        WeightOptions {
            threshold_m: 10_000.0,
            alpha: -1.0,
            binary: true,
            standardization: Standardization::Row,
        }
    }
}

/// A spatial weight matrix over a point set (turf's `distanceWeight`).
///
/// Row `i` holds the weight of every other point as a neighbour of `i`; the
/// diagonal is always 0.
// The weight matrix is written and read by (row, column) throughout: both w[i][j]
// and w[j][i] appear, so index loops say what is happening and iterator forms do
// not.
#[allow(clippy::needless_range_loop)]
pub fn distance_weight(points: &[Coord], o: &WeightOptions) -> Result<Vec<Vec<f64>>> {
    if points.len() < 2 {
        return Err(Error::InvalidArgument("need at least 2 points".into()));
    }
    if !o.threshold_m.is_finite() || o.threshold_m <= 0.0 {
        return Err(Error::InvalidArgument("the threshold must be positive".into()));
    }
    let n = points.len();
    let mut w = vec![vec![0.0f64; n]; n];
    for i in 0..n {
        for j in (i + 1)..n {
            let d = measure::distance(points[i], points[j]);
            if d > o.threshold_m {
                continue;
            }
            // a coincident pair would divide by zero; treat it as fully adjacent
            let v = if o.binary || d <= 0.0 { 1.0 } else { d.powf(o.alpha) };
            w[i][j] = v;
            w[j][i] = v;
        }
    }
    if o.standardization == Standardization::Row {
        for row in w.iter_mut() {
            let sum: f64 = row.iter().sum();
            if sum > 0.0 {
                for v in row.iter_mut() {
                    *v /= sum;
                }
            }
        }
    }
    Ok(w)
}

// ---------------------------------------------------------------- Moran's I

#[derive(Debug, Clone, Copy)]
pub struct MoranResult {
    pub moran_index: f64,
    /// What I would be under no spatial association: −1/(n−1).
    pub expected_moran_index: f64,
    pub variance_moran_index: f64,
    pub z_norm: f64,
    /// Two-sided p-value from the normal approximation.
    pub p_norm: f64,
}

/// Moran's I for a value attached to points (turf's `moranIndex`).
///
/// Positive I means like values cluster, negative means they alternate, and
/// around the expectation means no spatial association. The variance follows
/// the normality assumption, as turf's does.
#[allow(clippy::needless_range_loop)]
pub fn moran_index(points: &[Coord], values: &[f64], o: &WeightOptions) -> Result<MoranResult> {
    if points.len() != values.len() {
        return Err(Error::InvalidArgument("one value per point".into()));
    }
    let n = points.len();
    if n < 3 {
        return Err(Error::InvalidArgument("Moran's I needs at least 3 points".into()));
    }
    let w = distance_weight(points, o)?;
    let mean = values.iter().sum::<f64>() / n as f64;
    let dev: Vec<f64> = values.iter().map(|v| v - mean).collect();
    let denom: f64 = dev.iter().map(|d| d * d).sum();
    if denom <= 0.0 {
        return Err(Error::InvalidArgument("every value is identical, so I is undefined".into()));
    }
    let mut num = 0.0;
    let mut s0 = 0.0;
    for i in 0..n {
        for j in 0..n {
            num += w[i][j] * dev[i] * dev[j];
            s0 += w[i][j];
        }
    }
    if s0 <= 0.0 {
        return Err(Error::InvalidArgument(
            "no point has a neighbour within the threshold".into(),
        ));
    }
    let i_value = (n as f64 / s0) * (num / denom);

    // moments of the weight matrix, as in Cliff & Ord
    let mut s1 = 0.0;
    for i in 0..n {
        for j in 0..n {
            s1 += (w[i][j] + w[j][i]).powi(2);
        }
    }
    s1 *= 0.5;
    let mut s2 = 0.0;
    for i in 0..n {
        let row: f64 = (0..n).map(|j| w[i][j]).sum();
        let col: f64 = (0..n).map(|j| w[j][i]).sum();
        s2 += (row + col).powi(2);
    }
    let nf = n as f64;
    let expected = -1.0 / (nf - 1.0);
    let a = nf * ((nf * nf - 3.0 * nf + 3.0) * s1 - nf * s2 + 3.0 * s0 * s0);
    let b4 = dev.iter().map(|d| d.powi(4)).sum::<f64>() / (denom / nf).powi(2) / nf;
    let c = b4 * ((nf * nf - nf) * s1 - 2.0 * nf * s2 + 6.0 * s0 * s0);
    let d = (nf - 1.0) * (nf - 2.0) * (nf - 3.0) * s0 * s0;
    let variance = (a - c) / d - expected * expected;
    let sd = variance.max(0.0).sqrt();
    let z = if sd > 0.0 { (i_value - expected) / sd } else { 0.0 };
    Ok(MoranResult {
        moran_index: i_value,
        expected_moran_index: expected,
        variance_moran_index: variance,
        z_norm: z,
        p_norm: 2.0 * (1.0 - normal_cdf(z.abs())),
    })
}

/// Φ(x) via the error function, good to ~1e-7 — plenty for a p-value.
fn normal_cdf(x: f64) -> f64 {
    0.5 * (1.0 + erf(x / std::f64::consts::SQRT_2))
}

/// Abramowitz & Stegun 7.1.26.
fn erf(x: f64) -> f64 {
    let sign = x.signum();
    let x = x.abs();
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let y = 1.0
        - (((((1.061_405_429 * t - 1.453_152_027) * t) + 1.421_413_741) * t - 0.284_496_736) * t
            + 0.254_829_592)
            * t
            * (-x * x).exp();
    sign * y
}

// ----------------------------------------------------------- quadrat analysis

#[derive(Debug, Clone)]
pub struct QuadratResult {
    pub quadrats: usize,
    pub points: usize,
    /// Observed count per quadrat, row-major from the south-west corner.
    pub counts: Vec<usize>,
    /// Mean points per quadrat.
    pub expected: f64,
    /// Variance-to-mean ratio: 1 under complete spatial randomness, above 1
    /// clustered, below 1 dispersed.
    pub variance_mean_ratio: f64,
    pub chi_squared: f64,
    pub degrees_of_freedom: usize,
    /// Critical χ² at 95% for those degrees of freedom.
    pub critical_value: f64,
    /// `true` when χ² stays under the critical value, i.e. randomness is not
    /// rejected at 95%.
    pub is_random: bool,
}

/// Quadrat count test for complete spatial randomness (turf's `quadratAnalysis`).
///
/// The quadrats are laid out in a local metric plane so they have equal area on
/// the ground; a degree-based grid would make the poleward rows smaller and bias
/// the test.
pub fn quadrat_analysis(
    points: &[Coord],
    study_area: Option<&Geometry>,
    nx: usize,
    ny: usize,
) -> Result<QuadratResult> {
    if points.len() < 2 {
        return Err(Error::InvalidArgument("need at least 2 points".into()));
    }
    if nx < 2 || ny < 2 {
        return Err(Error::InvalidArgument("need at least a 2×2 quadrat grid".into()));
    }
    let geoms: Vec<Geometry> = points.iter().map(|c| Geometry::Point(Point(*c))).collect();
    let area_geom = match study_area {
        Some(g) => g.clone(),
        None => Geometry::Polygon(ops::bbox_polygon(
            ops::bbox(&geoms).ok_or_else(|| Error::InvalidGeometry("no extent".into()))?,
        )),
    };
    let bb = ops::bbox(std::slice::from_ref(&area_geom)).ok_or_else(|| Error::InvalidGeometry("no extent".into()))?;
    let frame = LocalFrame::new(0.5 * (bb[0] + bb[2]), 0.5 * (bb[1] + bb[3]));
    let corners = [
        frame.project(bb[0], bb[1]),
        frame.project(bb[2], bb[1]),
        frame.project(bb[2], bb[3]),
        frame.project(bb[0], bb[3]),
    ];
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for (x, y) in corners {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    let (dx, dy) = ((x1 - x0) / nx as f64, (y1 - y0) / ny as f64);
    let mut counts = vec![0usize; nx * ny];
    let mut inside = 0usize;
    for c in points {
        if study_area.is_some() && !predicates::point_in_polygon(*c, &area_geom, false) {
            continue;
        }
        let (px, py) = frame.project(c.x, c.y);
        let i = (((px - x0) / dx).floor() as i64).clamp(0, nx as i64 - 1) as usize;
        let j = (((py - y0) / dy).floor() as i64).clamp(0, ny as i64 - 1) as usize;
        counts[j * nx + i] += 1;
        inside += 1;
    }
    let q = nx * ny;
    let expected = inside as f64 / q as f64;
    if expected <= 0.0 {
        return Err(Error::InvalidArgument("no points fall inside the study area".into()));
    }
    let variance = counts
        .iter()
        .map(|c| (*c as f64 - expected).powi(2))
        .sum::<f64>()
        / (q as f64 - 1.0);
    let chi = counts.iter().map(|c| (*c as f64 - expected).powi(2) / expected).sum::<f64>();
    let dof = q - 1;
    let critical = chi2_critical_95(dof);
    Ok(QuadratResult {
        quadrats: q,
        points: inside,
        counts,
        expected,
        variance_mean_ratio: variance / expected,
        chi_squared: chi,
        degrees_of_freedom: dof,
        critical_value: critical,
        is_random: chi <= critical,
    })
}

/// 95th percentile of χ² with `dof` degrees of freedom.
///
/// Wilson–Hilferty: dof·(1 − 2/(9·dof) + z·sqrt(2/(9·dof)))³ with z = 1.6449.
/// Within about 0.5% of the exact value from dof = 3 up, which is the range a
/// quadrat test uses (a 2×2 grid already gives dof = 3).
fn chi2_critical_95(dof: usize) -> f64 {
    let d = dof as f64;
    let z = 1.644_853_626_951_472_7;
    let t = 2.0 / (9.0 * d);
    d * (1.0 - t + z * t.sqrt()).powi(3)
}

// ------------------------------------------------------------- tesselation

/// Triangulate a polygon, holes included (turf's `tesselate`).
///
/// A constrained Delaunay triangulation in the local metric plane: the ring
/// edges are forced into the mesh, then the triangles outside the polygon (and
/// inside its holes) are dropped by testing their centroid.
pub fn tesselate(polygon: &Polygon) -> Result<Vec<Polygon>> {
    use spade::{ConstrainedDelaunayTriangulation, Point2, Triangulation};
    let rings: Vec<&geo::LineString> = std::iter::once(polygon.exterior()).chain(polygon.interiors()).collect();
    if polygon.exterior().0.len() < 4 {
        return Err(Error::InvalidGeometry("tesselate needs a closed ring".into()));
    }
    let frame = LocalFrame::for_geometries([&Geometry::Polygon(polygon.clone())])?;
    let mut cdt: ConstrainedDelaunayTriangulation<Point2<f64>> = ConstrainedDelaunayTriangulation::new();

    for ring in &rings {
        let pts = &ring.0;
        let n = if pts.first() == pts.last() { pts.len() - 1 } else { pts.len() };
        if n < 3 {
            continue;
        }
        let mut handles = Vec::with_capacity(n);
        for c in &pts[..n] {
            let (x, y) = frame.project(c.x, c.y);
            handles.push(
                cdt.insert(Point2::new(x, y))
                    .map_err(|e| Error::InvalidGeometry(format!("tesselate failed: {e:?}")))?,
            );
        }
        for k in 0..n {
            let (a, b) = (handles[k], handles[(k + 1) % n]);
            if a != b && cdt.can_add_constraint(a, b) {
                cdt.add_constraint(a, b);
            }
        }
    }

    let g = Geometry::Polygon(polygon.clone());
    let mut out = Vec::new();
    for face in cdt.inner_faces() {
        let vs = face.vertices();
        let mut ring: Vec<Coord> = Vec::with_capacity(4);
        let mut cx = 0.0;
        let mut cy = 0.0;
        for v in vs.iter() {
            let p = v.position();
            cx += p.x / 3.0;
            cy += p.y / 3.0;
            let (lon, lat) = frame.unproject(p.x, p.y);
            ring.push(Coord { x: lon, y: lat });
        }
        let (clon, clat) = frame.unproject(cx, cy);
        if !predicates::point_in_polygon(Coord { x: clon, y: clat }, &g, false) {
            continue;
        }
        let shoelace = ((ring[1].x - ring[0].x) * (ring[2].y - ring[0].y)
            - (ring[2].x - ring[0].x) * (ring[1].y - ring[0].y))
            .abs()
            / 2.0;
        if shoelace <= 0.0 {
            continue;
        }
        ring.push(ring[0]);
        out.push(Polygon::new(geo::LineString(ring), vec![]));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::densify::Edges;
    use geo::{coord, polygon};

    fn go(c: Coord, az: f64, d: f64) -> Coord {
        let (x, y) = crate::geodesic::destination(c.x, c.y, az, d);
        Coord { x, y }
    }

    /// A lattice of points, spaced `step` metres apart.
    fn lattice(n: usize, step: f64) -> Vec<Coord> {
        let origin = coord! {x: 120.0, y: 30.0};
        (0..n)
            .flat_map(|j| (0..n).map(move |i| (i, j)))
            .map(|(i, j)| go(go(origin, 90.0, i as f64 * step), 0.0, j as f64 * step))
            .collect()
    }

    #[test]
    #[allow(clippy::needless_range_loop)]
    fn weights_are_symmetric_and_row_standardised() {
        let pts = lattice(4, 1_000.0);
        // 1.2 km reaches the four rook neighbours but not the diagonals at 1.414 km
        let raw = distance_weight(
            &pts,
            &WeightOptions { threshold_m: 1_200.0, standardization: Standardization::Raw, ..Default::default() },
        )
        .unwrap();
        // binary weights: the four corners have 2 neighbours each
        assert_eq!(raw[0].iter().sum::<f64>(), 2.0);
        // an interior point has 4
        assert_eq!(raw[5].iter().sum::<f64>(), 4.0);
        // widen past the diagonal and the corner gains its third neighbour
        let wide = distance_weight(
            &pts,
            &WeightOptions { threshold_m: 1_500.0, standardization: Standardization::Raw, ..Default::default() },
        )
        .unwrap();
        assert_eq!(wide[0].iter().sum::<f64>(), 3.0);
        assert_eq!(wide[5].iter().sum::<f64>(), 8.0);
        assert_eq!(raw[0][0], 0.0, "no self-weight");
        for i in 0..pts.len() {
            for j in 0..pts.len() {
                assert_eq!(raw[i][j], raw[j][i]);
            }
        }

        let rowed = distance_weight(&pts, &WeightOptions { threshold_m: 1_200.0, ..Default::default() }).unwrap();
        for row in &rowed {
            let s: f64 = row.iter().sum();
            assert!((s - 1.0).abs() < 1e-12, "{s}");
        }

        // distance decay puts more weight on the nearer neighbour
        let two = vec![coord! {x: 120.0, y: 30.0}, coord! {x: 120.005, y: 30.0}, coord! {x: 120.02, y: 30.0}];
        let decay = distance_weight(
            &two,
            &WeightOptions {
                threshold_m: 5_000.0,
                binary: false,
                alpha: -1.0,
                standardization: Standardization::Raw,
            },
        )
        .unwrap();
        assert!(decay[0][1] > decay[0][2] * 2.0, "{:?}", decay[0]);
    }

    #[test]
    fn moran_separates_clustered_from_alternating_values() {
        let n = 8;
        let pts = lattice(n, 1_000.0);
        // rook neighbours only: at 1.5 km the diagonals would join in, and in a
        // checkerboard those carry the *same* value, which cancels the signal
        let opts = WeightOptions { threshold_m: 1_200.0, ..Default::default() };

        // a smooth west-to-east ramp: neighbouring values are alike
        let ramp: Vec<f64> = (0..n * n).map(|k| (k % n) as f64).collect();
        let smooth = moran_index(&pts, &ramp, &opts).unwrap();
        assert!(smooth.moran_index > 0.5, "ramp I = {}", smooth.moran_index);
        assert!(smooth.z_norm > 3.0 && smooth.p_norm < 0.01, "{smooth:?}");

        // a checkerboard: every neighbour is the opposite value
        let checker: Vec<f64> = (0..n * n).map(|k| ((k % n + k / n) % 2) as f64).collect();
        let alt = moran_index(&pts, &checker, &opts).unwrap();
        assert!(alt.moran_index < -0.8, "checker I = {}", alt.moran_index);
        assert!(alt.z_norm < -3.0, "{alt:?}");

        // the expectation is -1/(n-1) either way
        let expect = -1.0 / (n as f64 * n as f64 - 1.0);
        assert!((smooth.expected_moran_index - expect).abs() < 1e-12);
        assert!(smooth.variance_moran_index > 0.0);

        // Widening to the diagonals halves the checkerboard's neighbour signal,
        // because a diagonal neighbour shares the value: I collapses toward 0.
        let with_diagonals = moran_index(&pts, &checker, &WeightOptions { threshold_m: 1_500.0, ..Default::default() }).unwrap();
        assert!(
            with_diagonals.moran_index.abs() < 0.2,
            "diagonals should cancel the signal, got {}",
            with_diagonals.moran_index
        );

        // a constant field has no variance to explain
        assert!(moran_index(&pts, &vec![1.0; n * n], &opts).is_err());
    }

    #[test]
    fn quadrats_tell_random_from_clustered() {
        // a regular lattice puts the same count in every quadrat
        let pts = lattice(12, 1_000.0);
        let even = quadrat_analysis(&pts, None, 3, 3).unwrap();
        assert_eq!(even.points, 144);
        assert_eq!(even.quadrats, 9);
        assert_eq!(even.counts.iter().sum::<usize>(), 144);
        assert!(even.variance_mean_ratio < 0.2, "{}", even.variance_mean_ratio);
        assert!(even.is_random, "a lattice is not rejected as random: {even:?}");

        // everything in one corner is unmistakably clustered
        let origin = coord! {x: 120.0, y: 30.0};
        let mut clumped: Vec<Coord> = (0..100)
            .map(|k| go(origin, 3.6 * k as f64, 100.0))
            .collect();
        clumped.push(go(origin, 45.0, 30_000.0)); // stretch the extent
        let c = quadrat_analysis(&clumped, None, 4, 4).unwrap();
        assert!(c.variance_mean_ratio > 5.0, "{}", c.variance_mean_ratio);
        assert!(!c.is_random, "{c:?}");
        assert!(c.chi_squared > c.critical_value);
        assert_eq!(c.degrees_of_freedom, 15);

        // the 95% critical values are close to the tabulated ones
        for (dof, table) in [(3usize, 7.815), (9, 16.919), (15, 24.996), (24, 36.415)] {
            let got = chi2_critical_95(dof);
            assert!((got - table).abs() / table < 0.01, "dof {dof}: {got} vs {table}");
        }
    }

    #[test]
    fn tesselate_covers_a_polygon_with_a_hole() {
        let p = Polygon::new(
            geo::LineString(vec![
                coord! {x: 120.0, y: 30.0},
                coord! {x: 120.1, y: 30.0},
                coord! {x: 120.1, y: 30.1},
                coord! {x: 120.0, y: 30.1},
                coord! {x: 120.0, y: 30.0},
            ]),
            vec![geo::LineString(vec![
                coord! {x: 120.03, y: 30.03},
                coord! {x: 120.07, y: 30.03},
                coord! {x: 120.07, y: 30.07},
                coord! {x: 120.03, y: 30.07},
                coord! {x: 120.03, y: 30.03},
            ])],
        );
        let tris = tesselate(&p).unwrap();
        assert!(tris.len() >= 8, "{} triangles", tris.len());
        let want = measure::area_with(&Geometry::Polygon(p.clone()), Edges::Planar);
        let got: f64 = tris
            .iter()
            .map(|t| measure::area_with(&Geometry::Polygon(t.clone()), Edges::Planar))
            .sum();
        assert!((got / want - 1.0).abs() < 1e-6, "{got} vs {want}");
        // nothing lands in the hole
        for t in &tris {
            let c = ops::vertex_centroid(std::slice::from_ref(&Geometry::Polygon(t.clone()))).unwrap();
            assert!(predicates::point_in_polygon(c, &Geometry::Polygon(p.clone()), false), "{c:?}");
        }

        // a convex triangle needs exactly one triangle
        let tri = polygon![(x: 0.0, y: 0.0), (x: 1.0, y: 0.0), (x: 0.0, y: 1.0), (x: 0.0, y: 0.0)];
        assert_eq!(tesselate(&tri).unwrap().len(), 1);
    }
}
