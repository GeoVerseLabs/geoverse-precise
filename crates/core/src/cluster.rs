//! Clustering, point-pattern statistics and grid routing.
//!
//! Distances are geodesic and the fitting is done in a local metric plane, so a
//! "5 km" neighbourhood, a k-means centroid and an ellipse axis all mean the
//! same thing at 60°N as at the equator — which is not true of the same
//! algorithms run on raw degrees.

use std::collections::{BinaryHeap, HashMap};

use geo::{Coord, Geometry, LineString, Point, Polygon};

use crate::index::Prepared;
use crate::local::LocalFrame;
use crate::{geodesic, measure, ops, predicates, shapes, Error, Result};

// ------------------------------------------------------------------ DBSCAN

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbscanRole {
    Core,
    Edge,
    Noise,
}

impl DbscanRole {
    pub fn as_str(self) -> &'static str {
        match self {
            DbscanRole::Core => "core",
            DbscanRole::Edge => "edge",
            DbscanRole::Noise => "noise",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DbscanLabel {
    pub role: DbscanRole,
    /// `None` for noise.
    pub cluster: Option<usize>,
}

/// A uniform bucket index for metric radius queries in the local plane.
struct Buckets {
    cell: f64,
    map: HashMap<(i64, i64), Vec<usize>>,
    pts: Vec<(f64, f64)>,
}

impl Buckets {
    fn new(pts: Vec<(f64, f64)>, cell: f64) -> Buckets {
        let cell = cell.max(1e-6);
        let mut map: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
        for (i, (x, y)) in pts.iter().enumerate() {
            map.entry(((x / cell).floor() as i64, (y / cell).floor() as i64))
                .or_default()
                .push(i);
        }
        Buckets { cell, map, pts }
    }

    fn within(&self, i: usize, r: f64, out: &mut Vec<usize>) {
        out.clear();
        let (x, y) = self.pts[i];
        let (cx, cy) = ((x / self.cell).floor() as i64, (y / self.cell).floor() as i64);
        let span = (r / self.cell).ceil() as i64;
        let r2 = r * r;
        for gy in cy - span..=cy + span {
            for gx in cx - span..=cx + span {
                if let Some(ids) = self.map.get(&(gx, gy)) {
                    for &j in ids {
                        let (jx, jy) = self.pts[j];
                        let (dx, dy) = (jx - x, jy - y);
                        if dx * dx + dy * dy <= r2 {
                            out.push(j);
                        }
                    }
                }
            }
        }
    }
}

/// Density clustering (turf's `clustersDbscan`).
///
/// A point is *core* when at least `min_points` points (counting itself) lie
/// within `max_distance_m`; a non-core point adjacent to a core point is an
/// *edge* point, and the rest is noise. Cluster ids are assigned in input
/// order, so the output is stable run to run.
pub fn clusters_dbscan(points: &[Coord], max_distance_m: f64, min_points: usize) -> Result<Vec<DbscanLabel>> {
    if !max_distance_m.is_finite() || max_distance_m <= 0.0 {
        return Err(Error::InvalidArgument("maxDistance must be positive".into()));
    }
    let n = points.len();
    if n == 0 {
        return Ok(vec![]);
    }
    let frame = LocalFrame::for_geometries(
        points
            .iter()
            .map(|c| Geometry::Point(Point(*c)))
            .collect::<Vec<_>>()
            .iter(),
    )?;
    let projected: Vec<(f64, f64)> = points.iter().map(|c| frame.project(c.x, c.y)).collect();
    let buckets = Buckets::new(projected, max_distance_m);

    let mut labels = vec![
        DbscanLabel {
            role: DbscanRole::Noise,
            cluster: None
        };
        n
    ];
    let mut visited = vec![false; n];
    let mut neigh: Vec<usize> = Vec::new();
    let mut more: Vec<usize> = Vec::new();
    let mut next_cluster = 0usize;

    for i in 0..n {
        if visited[i] {
            continue;
        }
        visited[i] = true;
        buckets.within(i, max_distance_m, &mut neigh);
        if neigh.len() < min_points.max(1) {
            continue; // noise for now; it may still be claimed as an edge point
        }
        let id = next_cluster;
        next_cluster += 1;
        labels[i] = DbscanLabel {
            role: DbscanRole::Core,
            cluster: Some(id),
        };
        let mut queue: Vec<usize> = neigh.clone();
        let mut k = 0;
        while k < queue.len() {
            let j = queue[k];
            k += 1;
            if labels[j].cluster.is_none() {
                labels[j] = DbscanLabel {
                    role: DbscanRole::Edge,
                    cluster: Some(id),
                };
            }
            if visited[j] {
                continue;
            }
            visited[j] = true;
            buckets.within(j, max_distance_m, &mut more);
            if more.len() >= min_points.max(1) {
                labels[j] = DbscanLabel {
                    role: DbscanRole::Core,
                    cluster: Some(id),
                };
                for &m in &more {
                    if !queue.contains(&m) {
                        queue.push(m);
                    }
                }
            }
        }
    }
    Ok(labels)
}

// ----------------------------------------------------------------- k-means

#[derive(Debug, Clone)]
pub struct KmeansResult {
    /// Cluster index per input point.
    pub assignment: Vec<usize>,
    /// Cluster centroids, back in lon/lat.
    pub centroids: Vec<Coord>,
    pub iterations: usize,
}

/// k-means clustering (turf's `clustersKmeans`).
///
/// Seeded with k-means++ from a fixed generator, so repeated runs on the same
/// input give the same clusters — turf's `skmeans` picks its seeds at random.
// Lloyd's iteration walks points and centres by index in step, and the squared
// distances are carried in a parallel array; index loops keep those aligned.
#[allow(clippy::needless_range_loop)]
pub fn clusters_kmeans(points: &[Coord], k: usize) -> Result<KmeansResult> {
    let n = points.len();
    if n == 0 {
        return Err(Error::InvalidArgument("kmeans needs at least one point".into()));
    }
    // turf's default: sqrt(n / 2), clamped to the input size
    let k = if k == 0 {
        ((n as f64 / 2.0).sqrt().round() as usize).max(1)
    } else {
        k
    }
    .min(n);
    let frame = LocalFrame::for_geometries(
        points
            .iter()
            .map(|c| Geometry::Point(Point(*c)))
            .collect::<Vec<_>>()
            .iter(),
    )?;
    let p: Vec<(f64, f64)> = points.iter().map(|c| frame.project(c.x, c.y)).collect();

    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };

    // k-means++ seeding
    let mut centres: Vec<(f64, f64)> = vec![p[(rnd() * n as f64) as usize % n]];
    let mut d2 = vec![f64::MAX; n];
    while centres.len() < k {
        let last = *centres.last().unwrap();
        let mut total = 0.0;
        for i in 0..n {
            let (dx, dy) = (p[i].0 - last.0, p[i].1 - last.1);
            d2[i] = d2[i].min(dx * dx + dy * dy);
            total += d2[i];
        }
        if total <= 0.0 {
            // every remaining point coincides with a centre
            centres.push(p[centres.len() % n]);
            continue;
        }
        let mut target = rnd() * total;
        let mut pick = n - 1;
        for i in 0..n {
            target -= d2[i];
            if target <= 0.0 {
                pick = i;
                break;
            }
        }
        centres.push(p[pick]);
    }

    let mut assignment = vec![0usize; n];
    let mut iterations = 0;
    for _ in 0..200 {
        iterations += 1;
        let mut moved = false;
        for i in 0..n {
            let mut best = 0;
            let mut best_d = f64::MAX;
            for (c, centre) in centres.iter().enumerate() {
                let (dx, dy) = (p[i].0 - centre.0, p[i].1 - centre.1);
                let d = dx * dx + dy * dy;
                if d < best_d {
                    best_d = d;
                    best = c;
                }
            }
            if assignment[i] != best {
                assignment[i] = best;
                moved = true;
            }
        }
        let mut sums = vec![(0.0, 0.0, 0usize); centres.len()];
        for i in 0..n {
            let s = &mut sums[assignment[i]];
            s.0 += p[i].0;
            s.1 += p[i].1;
            s.2 += 1;
        }
        for (c, s) in sums.iter().enumerate() {
            if s.2 > 0 {
                centres[c] = (s.0 / s.2 as f64, s.1 / s.2 as f64);
            }
        }
        if !moved {
            break;
        }
    }
    Ok(KmeansResult {
        assignment,
        centroids: centres
            .iter()
            .map(|(x, y)| {
                let (lon, lat) = frame.unproject(*x, *y);
                Coord { x: lon, y: lat }
            })
            .collect(),
        iterations,
    })
}

// ------------------------------------------------- nearest-neighbour analysis

#[derive(Debug, Clone, Copy)]
pub struct NearestNeighbour {
    pub points: usize,
    /// Mean distance (m) from each point to its nearest neighbour.
    pub observed_mean_m: f64,
    /// What that mean would be for the same density under complete spatial
    /// randomness: `0.5 / sqrt(n / A)`.
    pub expected_mean_m: f64,
    pub area_m2: f64,
    /// `observed / expected`: below 1 is clustered, above 1 is dispersed.
    pub index: f64,
    pub z_score: f64,
}

/// Nearest-neighbour index for a point pattern (turf's `nearestNeighborAnalysis`).
///
/// `study_area` defaults to the convex hull of the points when `None`, matching
/// turf. Areas and distances are ellipsoidal, so the index is not distorted by
/// latitude the way a degree-space computation is.
pub fn nearest_neighbour_analysis(points: &[Coord], study_area: Option<&Geometry>) -> Result<NearestNeighbour> {
    let n = points.len();
    if n < 2 {
        return Err(Error::InvalidArgument(
            "nearest-neighbour analysis needs at least 2 points".into(),
        ));
    }
    let geoms: Vec<Geometry> = points.iter().map(|c| Geometry::Point(Point(*c))).collect();
    let area_geom = match study_area {
        Some(g) => g.clone(),
        None => Geometry::Polygon(ops::convex_hull(&geoms)?),
    };
    let area = measure::area_with(&area_geom, crate::densify::Edges::Planar);
    if !area.is_finite() || area <= 0.0 {
        return Err(Error::InvalidGeometry("study area has no area".into()));
    }
    let frame = LocalFrame::for_geometries(geoms.iter())?;
    let p: Vec<(f64, f64)> = points.iter().map(|c| frame.project(c.x, c.y)).collect();
    // a bucket side of sqrt(A/n) puts a handful of points in each cell
    let buckets = Buckets::new(p.clone(), (area / n as f64).sqrt());
    let mut sum = 0.0;
    let mut hits: Vec<usize> = Vec::new();
    for i in 0..n {
        let mut r = buckets.cell;
        let mut best = f64::MAX;
        // grow the search radius until the nearest neighbour is inside it
        for _ in 0..32 {
            buckets.within(i, r, &mut hits);
            best = hits
                .iter()
                .filter(|&&j| j != i)
                .map(|&j| {
                    let (dx, dy) = (p[j].0 - p[i].0, p[j].1 - p[i].1);
                    (dx * dx + dy * dy).sqrt()
                })
                .fold(f64::MAX, f64::min);
            if best <= r {
                break;
            }
            r *= 2.0;
        }
        if !best.is_finite() {
            // fall back to a scan; happens only for pathological inputs
            best = (0..n)
                .filter(|&j| j != i)
                .map(|j| measure::distance(points[i], points[j]))
                .fold(f64::MAX, f64::min);
        }
        sum += best;
    }
    let observed = sum / n as f64;
    let expected = 0.5 / (n as f64 / area).sqrt();
    let se = 0.26136 / ((n as f64 * n as f64 / area).sqrt());
    Ok(NearestNeighbour {
        points: n,
        observed_mean_m: observed,
        expected_mean_m: expected,
        area_m2: area,
        index: observed / expected,
        z_score: (observed - expected) / se,
    })
}

// ---------------------------------------------- standard deviational ellipse

#[derive(Debug, Clone)]
pub struct SdeResult {
    pub polygon: Polygon,
    pub centre: Coord,
    pub semi_major_m: f64,
    pub semi_minor_m: f64,
    /// Semi-axis along the ellipse's own x direction, and along its y — which
    /// of the two is the major one depends on θ (see `rotation_deg`).
    pub sigma_x_m: f64,
    pub sigma_y_m: f64,
    /// The θ of the Yuill/ArcGIS formulation, in (-90°, 90°], reported as turf
    /// reports it: the clockwise rotation applied to the ellipse, whose x
    /// semi-axis starts out pointing east. θ alone does not tell you where the
    /// long axis points, because σx and σy swap roles across the range — use
    /// `major_bearing_deg` for that.
    pub rotation_deg: f64,
    /// Bearing of the major axis, degrees clockwise from north in [0°, 180°).
    pub major_bearing_deg: f64,
    pub contained: usize,
    pub percentage_contained: f64,
}

/// Standard deviational ellipse (turf's `standardDeviationalEllipse`).
///
/// Fitted in the local metric plane, so the axes are real distances; turf fits
/// in degrees, which stretches the ellipse east-west away from the equator.
pub fn standard_deviational_ellipse(points: &[Coord], weights: Option<&[f64]>, steps: usize) -> Result<SdeResult> {
    let n = points.len();
    if n < 3 {
        return Err(Error::InvalidArgument("the ellipse needs at least 3 points".into()));
    }
    if let Some(w) = weights {
        if w.len() != n {
            return Err(Error::InvalidArgument("one weight per point".into()));
        }
    }
    let geoms: Vec<Geometry> = points.iter().map(|c| Geometry::Point(Point(*c))).collect();
    let frame = LocalFrame::for_geometries(geoms.iter())?;
    let p: Vec<(f64, f64)> = points.iter().map(|c| frame.project(c.x, c.y)).collect();
    let w: Vec<f64> = weights.map(|w| w.to_vec()).unwrap_or_else(|| vec![1.0; n]);
    let wt: f64 = w.iter().sum();
    if !wt.is_finite() || wt <= 0.0 {
        return Err(Error::InvalidArgument("weights must sum to more than zero".into()));
    }
    let mx: f64 = p.iter().zip(&w).map(|(q, w)| q.0 * w).sum::<f64>() / wt;
    let my: f64 = p.iter().zip(&w).map(|(q, w)| q.1 * w).sum::<f64>() / wt;

    let (mut sxx, mut syy, mut sxy) = (0.0, 0.0, 0.0);
    for (q, w) in p.iter().zip(&w) {
        let (dx, dy) = (q.0 - mx, q.1 - my);
        sxx += w * dx * dx;
        syy += w * dy * dy;
        sxy += w * dx * dy;
    }
    // orientation of the major axis (the classic Yuill formulation)
    let theta = if sxy.abs() < 1e-12 {
        if sxx >= syy {
            0.0
        } else {
            std::f64::consts::FRAC_PI_2
        }
    } else {
        ((sxx - syy + ((sxx - syy).powi(2) + 4.0 * sxy * sxy).sqrt()) / (2.0 * sxy)).atan()
    };
    let (st, ct) = theta.sin_cos();
    let (mut ax, mut ay) = (0.0, 0.0);
    for (q, w) in p.iter().zip(&w) {
        let (dx, dy) = (q.0 - mx, q.1 - my);
        ax += w * (dx * ct - dy * st).powi(2);
        ay += w * (dx * st + dy * ct).powi(2);
    }
    let denom = (wt - 2.0).max(1.0);
    let sigma_x = std::f64::consts::SQRT_2 * (ax / denom).sqrt();
    let sigma_y = std::f64::consts::SQRT_2 * (ay / denom).sqrt();

    let centre = {
        let (lon, lat) = frame.unproject(mx, my);
        Coord { x: lon, y: lat }
    };
    let rotation = theta.to_degrees();
    let polygon = shapes::ellipse(centre, sigma_x, sigma_y, rotation, steps.max(16));
    let g = Geometry::Polygon(polygon.clone());
    let inside = points
        .iter()
        .filter(|c| predicates::point_in_polygon(**c, &g, false))
        .count();
    let major_bearing = if sigma_x >= sigma_y {
        (90.0 + rotation).rem_euclid(180.0)
    } else {
        rotation.rem_euclid(180.0)
    };
    Ok(SdeResult {
        polygon,
        centre,
        semi_major_m: sigma_x.max(sigma_y),
        semi_minor_m: sigma_x.min(sigma_y),
        sigma_x_m: sigma_x,
        sigma_y_m: sigma_y,
        rotation_deg: rotation,
        major_bearing_deg: major_bearing,
        contained: inside,
        percentage_contained: 100.0 * inside as f64 / n as f64,
    })
}

// -------------------------------------------------------- directional mean

#[derive(Debug, Clone, Copy)]
pub struct DirectionalMean {
    pub lines: usize,
    /// Mean bearing, degrees clockwise from north in [0, 360).
    pub bearing_deg: f64,
    /// The same direction as a maths angle: degrees counter-clockwise from east.
    pub cartesian_deg: f64,
    /// 0 when every line points the same way, 1 when they cancel out.
    pub circular_variance: f64,
    pub average_length_m: f64,
    pub total_length_m: f64,
}

/// Mean direction of a set of lines (turf's `directionalMean`).
///
/// Bearings are geodesic start-to-end azimuths. Lines are treated as directed;
/// reverse a line and you reverse its contribution.
pub fn directional_mean(lines: &[LineString]) -> Result<DirectionalMean> {
    let mut n = 0usize;
    let (mut sx, mut sy) = (0.0, 0.0);
    let mut total = 0.0;
    for l in lines {
        if l.0.len() < 2 {
            continue;
        }
        let (a, b) = (l.0[0], *l.0.last().unwrap());
        if a == b {
            continue;
        }
        let inv = geodesic::inverse(a.x, a.y, b.x, b.y);
        let az = inv.azi1.to_radians();
        sx += az.sin();
        sy += az.cos();
        total += measure::line_length(l);
        n += 1;
    }
    if n == 0 {
        return Err(Error::InvalidArgument("no lines with two distinct ends".into()));
    }
    let r = (sx * sx + sy * sy).sqrt();
    let bearing = geodesic::normalize_deg(sx.atan2(sy).to_degrees()).rem_euclid(360.0);
    Ok(DirectionalMean {
        lines: n,
        bearing_deg: bearing,
        cartesian_deg: (90.0 - bearing).rem_euclid(360.0),
        circular_variance: 1.0 - r / n as f64,
        average_length_m: total / n as f64,
        total_length_m: total,
    })
}

// --------------------------------------------------------- shortest path

#[derive(Debug, Clone, Copy)]
pub struct PathOptions {
    /// Grid spacing in metres; smaller hugs the obstacles more closely.
    pub resolution_m: f64,
    /// Extra room around the start/end/obstacle extent, in metres.
    pub padding_m: f64,
}

impl Default for PathOptions {
    fn default() -> Self {
        PathOptions {
            resolution_m: 1_000.0,
            padding_m: 0.0,
        }
    }
}

#[derive(PartialEq)]
struct Node(f64, usize);
impl Eq for Node {}
impl Ord for Node {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // a min-heap over the f-score
        other.0.partial_cmp(&self.0).unwrap_or(std::cmp::Ordering::Equal)
    }
}
impl PartialOrd for Node {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Shortest path around obstacles (turf's `shortestPath`).
///
/// A* over a metric lattice with 8-way moves. The lattice is laid out in the
/// local plane, so `resolution_m` is a real spacing and diagonal steps really
/// are √2 cells long.
pub fn shortest_path(start: Coord, end: Coord, obstacles: &[Geometry], opts: &PathOptions) -> Result<LineString> {
    if !opts.resolution_m.is_finite() || opts.resolution_m <= 0.0 {
        return Err(Error::InvalidArgument("resolution must be positive".into()));
    }
    let mut all: Vec<Geometry> = vec![Geometry::Point(Point(start)), Geometry::Point(Point(end))];
    all.extend(obstacles.iter().cloned());
    let frame = LocalFrame::for_geometries(all.iter())?;
    let bb = ops::bbox(&all).ok_or_else(|| Error::InvalidGeometry("no extent".into()))?;

    let corners = [
        frame.project(bb[0], bb[1]),
        frame.project(bb[2], bb[1]),
        frame.project(bb[2], bb[3]),
        frame.project(bb[0], bb[3]),
    ];
    let pad = opts.padding_m + opts.resolution_m;
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for (x, y) in corners {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    let (x0, y0, x1, y1) = (x0 - pad, y0 - pad, x1 + pad, y1 + pad);
    let step = opts.resolution_m;
    // Anchor the lattice on the start point: the route then leaves it without a
    // half-cell jog, which on an unobstructed run is the whole of the error.
    let (sx, sy) = frame.project(start.x, start.y);
    let (x0, y0) = (
        sx - ((sx - x0) / step).ceil() * step,
        sy - ((sy - y0) / step).ceil() * step,
    );
    let nx = (((x1 - x0) / step).ceil() as usize + 1).max(2);
    let ny = (((y1 - y0) / step).ceil() as usize + 1).max(2);
    if nx * ny > 12_000_000 {
        return Err(Error::InvalidArgument(format!(
            "a {nx}×{ny} lattice is too large; raise the resolution"
        )));
    }

    let prepared: Vec<Prepared> = obstacles
        .iter()
        .filter(|g| matches!(g, Geometry::Polygon(_) | Geometry::MultiPolygon(_)))
        .map(|g| Prepared::new(g.clone()))
        .collect::<Result<_>>()?;
    let at = |i: usize, j: usize| -> Coord {
        let (lon, lat) = frame.unproject(x0 + i as f64 * step, y0 + j as f64 * step);
        Coord { x: lon, y: lat }
    };
    let mut blocked = vec![false; nx * ny];
    for j in 0..ny {
        for i in 0..nx {
            let c = at(i, j);
            blocked[j * nx + i] = prepared.iter().any(|p| p.contains_point(c, false));
        }
    }
    let nearest_free = |c: Coord| -> usize {
        let (px, py) = frame.project(c.x, c.y);
        let i = (((px - x0) / step).round() as i64).clamp(0, nx as i64 - 1) as usize;
        let j = (((py - y0) / step).round() as i64).clamp(0, ny as i64 - 1) as usize;
        if !blocked[j * nx + i] {
            return j * nx + i;
        }
        // walk outwards until a free cell turns up
        for r in 1..nx.max(ny) as i64 {
            for dj in -r..=r {
                for di in -r..=r {
                    if di.abs() != r && dj.abs() != r {
                        continue;
                    }
                    let (ii, jj) = (i as i64 + di, j as i64 + dj);
                    if ii >= 0
                        && jj >= 0
                        && (ii as usize) < nx
                        && (jj as usize) < ny
                        && !blocked[jj as usize * nx + ii as usize]
                    {
                        return jj as usize * nx + ii as usize;
                    }
                }
            }
        }
        j * nx + i
    };
    let s = nearest_free(start);
    let t = nearest_free(end);

    let heuristic = |k: usize| -> f64 {
        let (i, j) = (k % nx, k / nx);
        let (ti, tj) = (t % nx, t / nx);
        let (dx, dy) = ((i as f64 - ti as f64).abs(), (j as f64 - tj as f64).abs());
        // octile distance: exact for 8-way movement, so A* stays admissible
        step * (dx.max(dy) + (std::f64::consts::SQRT_2 - 1.0) * dx.min(dy))
    };
    let mut g = vec![f64::MAX; nx * ny];
    let mut from = vec![usize::MAX; nx * ny];
    let mut heap = BinaryHeap::new();
    g[s] = 0.0;
    heap.push(Node(heuristic(s), s));
    let diag = std::f64::consts::SQRT_2 * step;
    while let Some(Node(f, k)) = heap.pop() {
        if k == t {
            break;
        }
        if f > g[k] + heuristic(k) + 1e-9 {
            continue; // a stale entry
        }
        let (i, j) = (k % nx, k / nx);
        for (di, dj) in [
            (-1i64, 0i64),
            (1, 0),
            (0, -1),
            (0, 1),
            (-1, -1),
            (-1, 1),
            (1, -1),
            (1, 1),
        ] {
            let (ii, jj) = (i as i64 + di, j as i64 + dj);
            if ii < 0 || jj < 0 || ii as usize >= nx || jj as usize >= ny {
                continue;
            }
            let m = jj as usize * nx + ii as usize;
            if blocked[m] {
                continue;
            }
            // no cutting a corner between two blocked cells
            if di != 0 && dj != 0 {
                let a = j * nx + (i as i64 + di) as usize;
                let b = (j as i64 + dj) as usize * nx + i;
                if blocked[a] || blocked[b] {
                    continue;
                }
            }
            let cost = if di != 0 && dj != 0 { diag } else { step };
            let ng = g[k] + cost;
            if ng < g[m] - 1e-9 {
                g[m] = ng;
                from[m] = k;
                heap.push(Node(ng + heuristic(m), m));
            }
        }
    }
    if g[t] == f64::MAX {
        return Err(Error::InvalidGeometry("the obstacles leave no route".into()));
    }
    let mut cells = vec![t];
    let mut k = t;
    while from[k] != usize::MAX {
        k = from[k];
        cells.push(k);
    }
    cells.reverse();
    let mut path: Vec<Coord> = Vec::with_capacity(cells.len() + 2);
    path.push(start);
    path.extend(cells.iter().map(|&k| at(k % nx, k / nx)));
    path.push(end);
    path.dedup();

    // The lattice route is a staircase, and its snapped end nodes can sit past
    // the real endpoints — which is how a clear run ends up doubling back over
    // half a cell. Pull the string taut: keep only the vertices that a straight
    // segment cannot skip without touching an obstacle. That removes the
    // staircase and the end jogs in one pass.
    let visible = |a: Coord, b: Coord| -> bool {
        if obstacles.is_empty() {
            return true;
        }
        let seg = Geometry::LineString(LineString(vec![a, b]));
        !obstacles.iter().any(|o| predicates::intersects(&seg, o))
    };
    const LOOKAHEAD: usize = 512;
    let mut taut: Vec<Coord> = vec![path[0]];
    let mut i = 0usize;
    while i + 1 < path.len() {
        let limit = (i + LOOKAHEAD).min(path.len() - 1);
        let mut next = i + 1;
        for k in (i + 2..=limit).rev() {
            if visible(path[i], path[k]) {
                next = k;
                break;
            }
        }
        taut.push(path[next]);
        i = next;
    }
    taut.dedup();
    Ok(LineString(taut))
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo::{coord, line_string, polygon};

    /// Move `dist` metres from `c` on a geodesic azimuth.
    fn go(c: Coord, az: f64, dist: f64) -> Coord {
        let (x, y) = geodesic::destination(c.x, c.y, az, dist);
        Coord { x, y }
    }

    fn blob(centre: Coord, radius: f64, n: usize) -> Vec<Coord> {
        (0..n)
            .map(|k| {
                go(
                    centre,
                    360.0 * k as f64 / n as f64,
                    radius * (0.3 + 0.7 * (k % 3) as f64 / 3.0),
                )
            })
            .collect()
    }

    #[test]
    fn dbscan_separates_blobs_and_flags_noise() {
        let mut pts = blob(coord! {x: 120.0, y: 30.0}, 800.0, 9);
        pts.extend(blob(coord! {x: 120.5, y: 30.0}, 800.0, 9));
        pts.push(coord! {x: 121.5, y: 30.5}); // far from everything
        let labels = clusters_dbscan(&pts, 2_000.0, 3).unwrap();
        assert_eq!(labels.len(), pts.len());

        let ids: Vec<Option<usize>> = labels.iter().map(|l| l.cluster).collect();
        let outlier = labels.last().unwrap();
        assert_eq!(outlier.role, DbscanRole::Noise);
        assert!(outlier.cluster.is_none());

        // the two blobs come out as exactly two clusters, each whole
        let a: Vec<Option<usize>> = ids[..9].to_vec();
        let b: Vec<Option<usize>> = ids[9..18].to_vec();
        assert!(a.iter().all(|c| *c == a[0]) && a[0].is_some(), "{a:?}");
        assert!(b.iter().all(|c| *c == b[0]) && b[0].is_some(), "{b:?}");
        assert_ne!(a[0], b[0]);
        assert!(labels[..18].iter().all(|l| l.role != DbscanRole::Noise));

        // a radius that spans the gap merges them
        let merged = clusters_dbscan(&pts, 60_000.0, 3).unwrap();
        assert_eq!(merged[0].cluster, merged[17].cluster);
        // and a min_points above the blob size leaves only noise
        let strict = clusters_dbscan(&pts, 2_000.0, 40).unwrap();
        assert!(strict.iter().all(|l| l.role == DbscanRole::Noise));
    }

    #[test]
    #[allow(clippy::needless_range_loop)]
    fn kmeans_recovers_separated_blobs() {
        let centres = [
            coord! {x: 120.0, y: 30.0},
            coord! {x: 120.6, y: 30.0},
            coord! {x: 120.3, y: 30.5},
        ];
        let mut pts = Vec::new();
        for c in centres {
            pts.extend(blob(c, 1_500.0, 8));
        }
        let r = clusters_kmeans(&pts, 3).unwrap();
        assert_eq!(r.centroids.len(), 3);
        for b in 0..3 {
            let group = &r.assignment[b * 8..(b + 1) * 8];
            assert!(group.iter().all(|a| *a == group[0]), "blob {b} split: {group:?}");
            // its centroid sits at the blob centre
            let c = r.centroids[group[0]];
            assert!(
                measure::distance(c, centres[b]) < 800.0,
                "{:?}",
                measure::distance(c, centres[b])
            );
        }
        // distinct clusters, and the same answer on a re-run
        let mut seen: Vec<usize> = r.assignment.clone();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), 3);
        assert_eq!(clusters_kmeans(&pts, 3).unwrap().assignment, r.assignment);

        // k = 0 asks for turf's sqrt(n / 2) default
        let auto = clusters_kmeans(&pts, 0).unwrap();
        assert_eq!(auto.centroids.len(), ((24.0f64 / 2.0).sqrt().round()) as usize);
    }

    #[test]
    fn nearest_neighbour_tells_clustered_from_dispersed() {
        // a regular lattice is as dispersed as a pattern gets
        let mut lattice = Vec::new();
        for j in 0..8 {
            for i in 0..8 {
                let p = go(
                    go(coord! {x: 120.0, y: 30.0}, 90.0, i as f64 * 3_000.0),
                    0.0,
                    j as f64 * 3_000.0,
                );
                lattice.push(p);
            }
        }
        let d = nearest_neighbour_analysis(&lattice, None).unwrap();
        assert_eq!(d.points, 64);
        assert!((d.observed_mean_m - 3_000.0).abs() < 60.0, "{}", d.observed_mean_m);
        assert!(d.index > 1.6, "index {}", d.index);
        assert!(d.z_score > 5.0, "z {}", d.z_score);

        // two tight knots inside the same hull are clustered
        let mut knots = blob(coord! {x: 120.0, y: 30.0}, 300.0, 32);
        knots.extend(blob(coord! {x: 120.2, y: 30.2}, 300.0, 32));
        let c = nearest_neighbour_analysis(&knots, None).unwrap();
        assert!(c.index < 0.5, "index {}", c.index);
        assert!(c.z_score < -5.0, "z {}", c.z_score);
    }

    #[test]
    fn sde_follows_the_spread() {
        let centre = coord! {x: 120.0, y: 30.0};
        // an elongated cloud pointing along `az`, 10 km by 1 km
        let cloud = |az: f64| -> Vec<Coord> {
            let mut v = Vec::new();
            for u in [-10_000.0, -6_000.0, -2_000.0, 2_000.0, 6_000.0, 10_000.0] {
                for w in [-1_000.0, 0.0, 1_000.0] {
                    v.push(go(go(centre, az, u), az + 90.0, w));
                }
            }
            v
        };
        let e = standard_deviational_ellipse(&cloud(90.0), None, 64).unwrap();
        assert!(measure::distance(e.centre, centre) < 50.0);
        assert!(
            e.semi_major_m / e.semi_minor_m > 5.0,
            "{:?}",
            (e.semi_major_m, e.semi_minor_m)
        );
        // east-west spread ⇒ the major axis lies along east
        assert!(
            (e.major_bearing_deg - 90.0).abs() < 2.0,
            "bearing {}",
            e.major_bearing_deg
        );
        assert!(e.contained > 0 && e.percentage_contained > 50.0, "{e:?}");

        // the fit tracks the cloud whatever direction it points, and the axes
        // are unchanged by the rotation. The furthest vertex of the drawn
        // ellipse must agree with the reported major-axis bearing — that is
        // what catches an axis mixed up with its perpendicular.
        for az in [0.0, 30.0, 60.0, 120.0, 150.0] {
            let r = standard_deviational_ellipse(&cloud(az), None, 180).unwrap();
            assert!(
                (r.major_bearing_deg - az.rem_euclid(180.0)).abs() < 2.0,
                "cloud {az}: bearing {}",
                r.major_bearing_deg
            );
            assert!((r.semi_major_m / e.semi_major_m - 1.0).abs() < 0.05);
            let far = r
                .polygon
                .exterior()
                .0
                .iter()
                .max_by(|a, b| {
                    measure::distance(r.centre, **a)
                        .partial_cmp(&measure::distance(r.centre, **b))
                        .unwrap()
                })
                .copied()
                .unwrap();
            let drawn = geodesic::inverse(r.centre.x, r.centre.y, far.x, far.y)
                .azi1
                .rem_euclid(180.0);
            assert!(
                (drawn - r.major_bearing_deg).abs() < 2.0,
                "cloud {az}: drawn {drawn} vs {}",
                r.major_bearing_deg
            );
        }

        // weights pull the centre
        let w: Vec<f64> = (0..18).map(|k| if k < 3 { 50.0 } else { 1.0 }).collect();
        let heavy = standard_deviational_ellipse(&cloud(90.0), Some(&w), 64).unwrap();
        assert!(heavy.centre.x < e.centre.x, "{:?} vs {:?}", heavy.centre, e.centre);
    }

    #[test]
    fn directional_mean_averages_bearings() {
        let a = coord! {x: 120.0, y: 30.0};
        let lines: Vec<LineString> = [40.0, 45.0, 50.0]
            .iter()
            .map(|az| LineString(vec![a, go(a, *az, 10_000.0)]))
            .collect();
        let d = directional_mean(&lines).unwrap();
        assert_eq!(d.lines, 3);
        assert!((d.bearing_deg - 45.0).abs() < 0.5, "{}", d.bearing_deg);
        assert!((d.cartesian_deg - 45.0).abs() < 0.5, "{}", d.cartesian_deg);
        assert!(d.circular_variance < 0.01, "{}", d.circular_variance);
        assert!((d.average_length_m - 10_000.0).abs() < 1.0);
        assert!((d.total_length_m - 30_000.0).abs() < 1.0);

        // opposing lines cancel: maximum circular variance
        let opposed = vec![
            LineString(vec![a, go(a, 0.0, 5_000.0)]),
            LineString(vec![a, go(a, 180.0, 5_000.0)]),
        ];
        let o = directional_mean(&opposed).unwrap();
        assert!(o.circular_variance > 0.99, "{}", o.circular_variance);

        // degenerate lines are skipped, not counted
        let mut mixed = lines.clone();
        mixed.push(line_string![(x: 120.0, y: 30.0), (x: 120.0, y: 30.0)]);
        assert_eq!(directional_mean(&mixed).unwrap().lines, 3);
    }

    #[test]
    fn shortest_path_goes_around_obstacles() {
        let start = coord! {x: 120.0, y: 30.0};
        let end = coord! {x: 120.4, y: 30.0};
        let direct = measure::distance(start, end);
        let opts = PathOptions {
            resolution_m: 1_500.0,
            padding_m: 6_000.0,
        };

        // with nothing in the way the route is essentially the straight line
        let clear = shortest_path(start, end, &[], &opts).unwrap();
        let clear_len = measure::line_length(&clear);
        // nothing in the way, so the taut route is the straight line itself
        assert_eq!(clear.0.len(), 2);
        assert!((clear_len - direct).abs() < 1e-6, "{clear_len} vs {direct}");

        // a wall across the middle, open at the top
        let wall = Geometry::Polygon(polygon![
            (x: 120.19, y: 29.9), (x: 120.21, y: 29.9), (x: 120.21, y: 30.05), (x: 120.19, y: 30.05), (x: 120.19, y: 29.9)
        ]);
        let around = shortest_path(start, end, std::slice::from_ref(&wall), &opts).unwrap();
        let len = measure::line_length(&around);
        assert!(
            len > clear_len * 1.02,
            "the route ignored the wall: {len} vs {clear_len}"
        );
        // and the detour is not wildly longer than going round the wall's end
        assert!(len < clear_len * 1.5, "{len} vs {clear_len}");
        for c in &around.0 {
            assert!(
                !predicates::point_in_polygon(*c, &wall, false),
                "{c:?} is inside the wall"
            );
        }
        assert!(
            !predicates::intersects(&Geometry::LineString(around.clone()), &wall),
            "the route crosses the wall"
        );
        // it goes round the open end
        assert!(around.0.iter().any(|c| c.y > 30.04), "{:?}", around.0);
        assert_eq!(around.0.first(), Some(&start));
        assert_eq!(around.0.last(), Some(&end));

        // a ring of obstacle around the start encloses it: no route out. (A
        // wall across the line is never enough — the lattice always extends
        // past its ends, which is the whole point of routing round it.)
        let ring = Geometry::Polygon(geo::Polygon::new(
            line_string![(x: 119.9, y: 29.9), (x: 120.1, y: 29.9), (x: 120.1, y: 30.1), (x: 119.9, y: 30.1), (x: 119.9, y: 29.9)],
            vec![
                line_string![(x: 119.96, y: 29.96), (x: 120.04, y: 29.96), (x: 120.04, y: 30.04), (x: 119.96, y: 30.04), (x: 119.96, y: 29.96)],
            ],
        ));
        assert!(
            !predicates::point_in_polygon(start, &ring, false),
            "the start must sit in the courtyard"
        );
        assert!(shortest_path(start, end, &[ring], &opts).is_err());
    }
}
