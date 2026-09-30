//! Stage-level profiling of the core algorithms (native, release).
//!
//!     cargo run --release -p geoprecise-core --example profile

use std::time::Instant;

use geo::{unary_union, BooleanOps, Coord, Geometry, LineString, MultiPolygon, Polygon};
use geoprecise_core::buffer::{buffer, buffer_pieces, BufferMethod, BufferOptions};
use geoprecise_core::crs::{Crs, Transformer};
use geoprecise_core::densify::Edges;
use geoprecise_core::overlay::{overlay, OverlayOp, OverlayOptions};
use geoprecise_core::{geodesic, measure};

fn bench<T>(name: &str, reps: usize, mut f: impl FnMut() -> T) -> T {
    let mut out = f();
    let t0 = Instant::now();
    for _ in 0..reps {
        out = f();
    }
    let per = t0.elapsed().as_secs_f64() * 1000.0 / reps as f64;
    println!("{name:<52} {per:>10.3} ms");
    out
}

fn route(a: Coord, b: Coord, step_km: f64) -> Vec<Coord> {
    let inv = geodesic::inverse(a.x, a.y, b.x, b.y);
    let n = (inv.s12 / (step_km * 1000.0)).ceil() as usize;
    (0..=n)
        .map(|i| {
            let (x, y) = geodesic::destination(a.x, a.y, inv.azi1, inv.s12 * i as f64 / n as f64);
            Coord { x, y }
        })
        .collect()
}

fn main() {
    println!("--- primitives (per call, µs) ---");
    let reps = 200_000;
    let t0 = Instant::now();
    let mut acc = 0.0;
    for i in 0..reps {
        let d = i as f64 * 1e-5;
        acc += geodesic::distance(116.0 + d, 39.0, 121.0, 31.0 + d);
    }
    println!(
        "{:<52} {:>10.3} µs  ({acc:.0})",
        "geodesic inverse (distance)",
        t0.elapsed().as_secs_f64() * 1e6 / reps as f64
    );
    let t0 = Instant::now();
    let mut acc = 0.0;
    for i in 0..reps {
        let (x, _) = geodesic::destination(116.0, 39.0, i as f64 % 360.0, 1000.0);
        acc += x;
    }
    println!(
        "{:<52} {:>10.3} µs  ({acc:.0})",
        "geodesic direct (destination)",
        t0.elapsed().as_secs_f64() * 1e6 / reps as f64
    );
    let t0 = Instant::now();
    let tr = Transformer::new(&Crs::Wgs84, &Crs::parse("EPSG:4549").unwrap());
    let mut acc = 0.0;
    for i in 0..reps {
        let (x, _) = tr.apply(120.0 + i as f64 * 1e-6, 30.0);
        acc += x;
    }
    println!(
        "{:<52} {:>10.3} µs  ({acc:.0})",
        "tmerc forward (prepared)",
        t0.elapsed().as_secs_f64() * 1e6 / reps as f64
    );
    let t0 = Instant::now();
    for _ in 0..20_000 {
        let _ = Transformer::new(&Crs::Wgs84, &Crs::parse("EPSG:4549").unwrap());
    }
    println!(
        "{:<52} {:>10.3} µs",
        "Transformer::new + Crs::parse",
        t0.elapsed().as_secs_f64() * 1e6 / 20_000.0
    );

    println!("\n--- buffer stages ---");
    let bj = Coord { x: 116.397, y: 39.909 };
    let sh = Coord { x: 121.474, y: 31.230 };
    let mut line = route(bj, Coord { x: 118.8, y: 32.06 }, 4.0);
    line.extend(route(Coord { x: 118.8, y: 32.06 }, sh, 4.0).into_iter().skip(1));
    let line_geom = Geometry::LineString(LineString(line.clone()));
    let ring: Vec<Coord> = route(Coord { x: 112.0, y: 30.0 }, Coord { x: 118.0, y: 30.5 }, 4.0)
        .into_iter()
        .chain(
            route(Coord { x: 118.0, y: 30.5 }, Coord { x: 117.5, y: 35.0 }, 4.0)
                .into_iter()
                .skip(1),
        )
        .chain(
            route(Coord { x: 117.5, y: 35.0 }, Coord { x: 111.5, y: 34.5 }, 4.0)
                .into_iter()
                .skip(1),
        )
        .chain(
            route(Coord { x: 111.5, y: 34.5 }, Coord { x: 112.0, y: 30.0 }, 4.0)
                .into_iter()
                .skip(1),
        )
        .collect();
    let poly_geom = Geometry::Polygon(Polygon::new(LineString(ring), vec![]));

    for (name, g, d) in [("line 2 km", &line_geom, 2000.0), ("polygon 5 km", &poly_geom, 5000.0)] {
        for edges in [Edges::Planar, Edges::Geodesic] {
            let opts = BufferOptions {
                edges,
                ..Default::default()
            };
            let tag = format!(
                "{name} [{}]",
                if edges == Edges::Planar { "planar" } else { "geodesic" }
            );
            bench(&format!("{tag} total"), 10, || buffer(g, d, &opts).unwrap());
            let (pieces, areal) = bench(&format!("{tag} pieces only"), 10, || {
                buffer_pieces(g, d, &opts).unwrap()
            });
            let n: usize = pieces.iter().chain(areal.iter()).map(|p| p.exterior().0.len()).sum();
            println!("    pieces = {} (+{} areal), vertices = {n}", pieces.len(), areal.len());
            let all: Vec<Polygon> = pieces.iter().chain(areal.iter()).cloned().collect();
            bench(&format!("{tag} union only"), 10, || unary_union(all.iter()));
        }
        let opts = BufferOptions {
            method: BufferMethod::Projected,
            ..Default::default()
        };
        bench(&format!("{name} projected total"), 10, || buffer(g, d, &opts).unwrap());
    }

    println!("\n--- overlay ---");
    let a = Geometry::Polygon(Polygon::new(
        LineString(
            route(Coord { x: 120.0, y: 30.0 }, Coord { x: 121.0, y: 30.0 }, 2.0)
                .into_iter()
                .chain(
                    route(Coord { x: 121.0, y: 30.0 }, Coord { x: 121.0, y: 31.0 }, 2.0)
                        .into_iter()
                        .skip(1),
                )
                .chain(
                    route(Coord { x: 121.0, y: 31.0 }, Coord { x: 120.0, y: 31.0 }, 2.0)
                        .into_iter()
                        .skip(1),
                )
                .chain(
                    route(Coord { x: 120.0, y: 31.0 }, Coord { x: 120.0, y: 30.0 }, 2.0)
                        .into_iter()
                        .skip(1),
                )
                .collect(),
        ),
        vec![],
    ));
    let b = Geometry::Polygon(Polygon::new(
        LineString(
            route(Coord { x: 120.5, y: 30.5 }, Coord { x: 121.5, y: 30.5 }, 2.0)
                .into_iter()
                .chain(
                    route(Coord { x: 121.5, y: 30.5 }, Coord { x: 121.5, y: 31.5 }, 2.0)
                        .into_iter()
                        .skip(1),
                )
                .chain(
                    route(Coord { x: 121.5, y: 31.5 }, Coord { x: 120.5, y: 31.5 }, 2.0)
                        .into_iter()
                        .skip(1),
                )
                .chain(
                    route(Coord { x: 120.5, y: 31.5 }, Coord { x: 120.5, y: 30.5 }, 2.0)
                        .into_iter()
                        .skip(1),
                )
                .collect(),
        ),
        vec![],
    ));
    let o = OverlayOptions::default();
    bench("intersection (2 × ~220 vertices)", 50, || {
        overlay(&a, &b, OverlayOp::Intersection, &o).unwrap()
    });
    let (pa, pb) = (
        MultiPolygon(vec![match &a {
            Geometry::Polygon(p) => p.clone(),
            _ => unreachable!(),
        }]),
        MultiPolygon(vec![match &b {
            Geometry::Polygon(p) => p.clone(),
            _ => unreachable!(),
        }]),
    );
    bench("  i_overlay alone (lon/lat, no projection)", 50, || {
        pa.intersection(&pb)
    });

    println!("\n--- measurement ---");
    let ls = LineString(line.clone());
    bench("length of 280-vertex line", 200, || measure::line_length(&ls));
    bench("area of 900-vertex polygon", 200, || measure::area(&poly_geom));
    let pts: Vec<Coord> = (0..1000)
        .map(|i| Coord {
            x: 117.0 + i as f64 * 1e-3,
            y: 34.0,
        })
        .collect();
    let mls = geo::MultiLineString(vec![ls.clone()]);
    bench("1000 × nearest_point_on_line (280 segs)", 3, || {
        pts.iter()
            .map(|p| measure::nearest_point_on_line(&mls, *p).unwrap().dist)
            .sum::<f64>()
    });
    bench("1000 × nearest without location", 3, || {
        pts.iter()
            .map(|p| measure::nearest_point_on_line_opts(&mls, *p, false).unwrap().dist)
            .sum::<f64>()
    });
    {
        use geoprecise_core::measure::approx_seg_distance_for_profiling as approx;
        let verts: Vec<[f64; 3]> =
            ls.0.iter()
                .map(|c| geoprecise_core::measure::unit_vec_for_profiling(*c))
                .collect();
        bench("1000 × spherical filter pass only", 20, || {
            let mut acc = 0.0;
            for p in &pts {
                let pv = geoprecise_core::measure::unit_vec_for_profiling(*p);
                let mut m = f64::INFINITY;
                for w in verts.windows(2) {
                    m = m.min(approx(w[0], w[1], pv).0);
                }
                acc += m;
            }
            acc
        });
        bench("1000 × exact closest_on_segment (hinted)", 20, || {
            let mut acc = 0.0;
            for (k, p) in pts.iter().enumerate() {
                let i = k % (ls.0.len() - 1);
                acc += measure::closest_on_segment_hinted(ls.0[i], ls.0[i + 1], *p, None).1;
            }
            acc
        });
    }
    let inside_pts: Vec<Coord> = (0..1000)
        .map(|i| Coord {
            x: 111.0 + (i % 40) as f64 * 0.2,
            y: 30.0 + (i / 40) as f64 * 0.2,
        })
        .collect();
    bench("1000 × point-in-polygon, no index (900 vertices)", 5, || {
        inside_pts
            .iter()
            .filter(|p| geoprecise_core::predicates::point_in_polygon(**p, &poly_geom, false))
            .count()
    });
    let prep = geoprecise_core::index::Prepared::new(poly_geom.clone()).unwrap();
    bench("  Prepared::new (build index)", 20, || {
        geoprecise_core::index::Prepared::new(poly_geom.clone()).unwrap()
    });
    bench("1000 × point-in-polygon, indexed", 50, || {
        inside_pts.iter().filter(|p| prep.contains_point(**p, false)).count()
    });
    let line_prep = geoprecise_core::index::Prepared::new(line_geom.clone()).unwrap();
    // points scattered close to the route (map-matching style workload)
    let near_pts: Vec<Coord> = (0..1000)
        .map(|i| {
            let base = line[i % line.len()];
            Coord {
                x: base.x + ((i % 7) as f64 - 3.0) * 0.01,
                y: base.y + ((i % 5) as f64 - 2.0) * 0.01,
            }
        })
        .collect();
    bench("1000 × nearest, near the line, no index", 5, || {
        near_pts
            .iter()
            .map(|p| measure::nearest_point_on_line_opts(&mls, *p, false).unwrap().dist)
            .sum::<f64>()
    });
    bench("1000 × nearest, near the line, indexed", 20, || {
        near_pts
            .iter()
            .map(|p| line_prep.nearest(*p).unwrap().dist)
            .sum::<f64>()
    });
    bench("1000 × nearest on line, no index (same points)", 5, || {
        inside_pts
            .iter()
            .map(|p| measure::nearest_point_on_line_opts(&mls, *p, false).unwrap().dist)
            .sum::<f64>()
    });
    bench("1000 × nearest on line, indexed (same points)", 5, || {
        inside_pts
            .iter()
            .map(|p| line_prep.nearest(*p).unwrap().dist)
            .sum::<f64>()
    });
    #[cfg(feature = "index-stats")]
    {
        geoprecise_core::index::STATS.with(|s| *s.borrow_mut() = (0, 0));
        for p in &inside_pts {
            let _ = line_prep.nearest(*p);
        }
        geoprecise_core::index::STATS.with(|s| {
            let (c, r) = *s.borrow();
            println!(
                "    candidates/query = {:.1}, refinements/query = {:.1}",
                c as f64 / 1000.0,
                r as f64 / 1000.0
            );
        });
    }
}
