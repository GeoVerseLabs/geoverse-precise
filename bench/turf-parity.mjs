// Behavioural comparison against turf.js, function by function.
//
//   cd packages/geoprecise && npm run build && cd ../../bench && node turf-parity.mjs
//
// For each function it reports whether geoprecise agrees with turf, and where it
// deliberately does not, why. Writes bench/out/parity.json.
import { writeFileSync, mkdirSync } from 'node:fs';
import * as turf from '@turf/turf';
import * as gp from '../packages/geoprecise/dist/index.js';

await gp.init();
mkdirSync(new URL('out/', import.meta.url), { recursive: true });

const rows = [];
/**
 * @param name   turf function name
 * @param verdict 'same' | 'better' | 'differs' | 'new'
 */
function row(name, verdict, detail) {
  rows.push({ name, verdict, detail });
  const mark = { same: '  =', better: '  +', differs: '  ~', new: '  *' }[verdict];
  console.log(`${mark} ${name.padEnd(30)} ${detail}`);
}

const pt = (x, y) => turf.point([x, y]);
const rel = (a, b) => (b === 0 ? Math.abs(a) : Math.abs(a - b) / Math.abs(b));
const fmtPct = (v) => `${(v * 100).toFixed(4)}%`;

// ------------------------------------------------------------- measurement

{
  const a = [116.4, 39.9];
  const b = [121.47, 31.23];
  const t = turf.distance(a, b, { units: 'meters' });
  const g = gp.distance(a, b, { units: 'meters' });
  // Karney's geodesic is the reference; turf uses a sphere.
  row('distance', 'better', `turf ${t.toFixed(1)} m vs geodesic ${g.toFixed(1)} m — turf off by ${fmtPct(rel(t, g))}`);

  const tb = turf.bearing(a, b);
  const gb = gp.bearing(a, b);
  row('bearing', 'better', `turf ${tb.toFixed(4)}° vs ellipsoidal ${gb.toFixed(4)}° (${Math.abs(tb - gb).toFixed(4)}° apart)`);

  const td = turf.destination(a, 500, 45, { units: 'kilometers' });
  const gd = gp.destination(a, 500, 45);
  const off = gp.distance(td, gd, { units: 'meters' });
  row('destination', 'better', `${off.toFixed(0)} m apart after 500 km`);

  const line = turf.lineString([[116.4, 39.9], [117, 40], [118, 40.5]]);
  row(
    'length',
    'better',
    `turf ${turf.length(line, { units: 'meters' }).toFixed(1)} m vs ${gp.length(line, { units: 'meters' }).toFixed(1)} m`,
  );

  const poly = turf.polygon([[[116, 39], [117, 39], [117, 40], [116, 40], [116, 39]]]);
  row('area', 'better', `turf ${turf.area(poly).toFixed(0)} m² vs ${gp.area(poly).toFixed(0)} m² (${fmtPct(rel(turf.area(poly), gp.area(poly)))})`);

  const tm = turf.midpoint(a, b);
  row('midpoint', 'better', `${gp.distance(tm, gp.midpoint(a, b), { units: 'meters' }).toFixed(0)} m apart`);

  const al = turf.along(line, 50, { units: 'kilometers' });
  row('along', 'better', `${gp.distance(al, gp.along(line, 50), { units: 'meters' }).toFixed(1)} m apart at 50 km`);

  const np = turf.nearestPointOnLine(line, pt(117.2, 40.2));
  const gnp = gp.nearestPointOnLine(line, pt(117.2, 40.2));
  row('nearestPointOnLine', 'same', `${gp.distance(np, gnp, { units: 'meters' }).toFixed(2)} m apart`);

  const d1 = turf.pointToLineDistance(pt(117.2, 40.2), line, { units: 'meters' });
  const d2 = gp.pointToLineDistance(pt(117.2, 40.2), line, { units: 'meters' });
  row('pointToLineDistance', 'better', `turf ${d1.toFixed(1)} m vs ${d2.toFixed(1)} m`);

  const rd = turf.rhumbDistance(a, b, { units: 'meters' });
  const grd = gp.rhumbDistance(a, b, { units: 'meters' });
  row('rhumbDistance', 'better', `turf ${rd.toFixed(1)} m vs ellipsoidal ${grd.toFixed(1)} m (${fmtPct(rel(rd, grd))})`);
  row('rhumbBearing', 'same', `turf ${turf.rhumbBearing(a, b).toFixed(5)}° vs ${gp.rhumbBearing(a, b).toFixed(5)}°`);
  const trd = turf.rhumbDestination(a, 500, 45);
  row('rhumbDestination', 'better', `${gp.distance(trd, gp.rhumbDestination(a, 500, 45), { units: 'meters' }).toFixed(0)} m apart`);
}

// ----------------------------------------------------------- constructive

{
  const c = [116.4, 39.9];
  const tc = turf.circle(c, 10, { steps: 256, units: 'kilometers' });
  const gc = gp.circle(c, 10, { steps: 256 });
  const radii = (f) => turf.getCoords(f)[0].map((p) => gp.distance(c, p, { units: 'meters' }));
  const spread = (rs) => Math.max(...rs) - Math.min(...rs);
  row('circle', 'better', `turf radius spread ${spread(radii(tc)).toFixed(1)} m vs ${spread(radii(gc)).toFixed(3)} m`);

  const tb = turf.buffer(turf.point(c), 50, { units: 'kilometers', steps: 64 });
  const gb = gp.buffer(turf.point(c), 50, { steps: 64 });
  row(
    'buffer',
    'better',
    `turf edge distance spread ${spread(radii(tb)).toFixed(0)} m vs ${spread(radii(gb)).toFixed(2)} m at 50 km`,
  );

  const line = turf.lineString([[116.4, 39.9], [116.6, 39.9]]);
  const to = turf.lineOffset(line, 1, { units: 'kilometers' });
  const go = gp.lineOffset(line, 1);
  const dist = (f) => turf.getCoords(f).map((p) => gp.pointToLineDistance(p, line, { units: 'meters' }));
  row('lineOffset', 'better', `turf ${dist(to).map((v) => v.toFixed(0)).join('/')} m vs ${dist(go).map((v) => v.toFixed(0)).join('/')} m for a 1 km offset`);

  const te = turf.ellipse(c, 20, 10, { steps: 180 });
  const ge = gp.ellipse(c, 20, 10, { steps: 180 });
  const semi = (f) => Math.max(...turf.getCoords(f)[0].map((p) => gp.distance(c, p, { units: 'meters' })));
  row('ellipse', 'better', `turf semi-major ${semi(te).toFixed(0)} m vs ${semi(ge).toFixed(0)} m for 20 km`);
}

// --------------------------------------------------------------- overlay

{
  const a = turf.polygon([[[0, 0], [2, 0], [2, 2], [0, 2], [0, 0]]]);
  const b = turf.polygon([[[1, 1], [3, 1], [3, 3], [1, 3], [1, 1]]]);
  const ti = turf.intersect(turf.featureCollection([a, b]));
  const gi = gp.intersect(a, b);
  row('intersect', 'same', `areas ${turf.area(ti).toFixed(0)} vs ${gp.area(gi).toFixed(0)} m² (turf's is spherical)`);
  row('union', 'same', `areas ${turf.area(turf.union(turf.featureCollection([a, b]))).toFixed(0)} vs ${gp.area(gp.union(a, b)).toFixed(0)} m²`);
  row('difference', 'same', `areas ${turf.area(turf.difference(turf.featureCollection([a, b]))).toFixed(0)} vs ${gp.area(gp.difference(a, b)).toFixed(0)} m²`);
}

// -------------------------------------------------------------- topology

{
  const bow = turf.polygon([[[0, 0], [2, 2], [2, 0], [0, 2], [0, 0]]]);
  const tk = turf.kinks(bow);
  const gk = gp.kinks(bow);
  row('kinks', 'same', `${tk.features.length} vs ${gk.features.length} self-intersections`);
  row('unkinkPolygon', 'same', `${turf.unkinkPolygon(bow).features.length} vs ${gp.unkinkPolygon(bow).features.length} pieces`);

  const line = turf.lineString([[0, 0], [1, 0], [1, 1]]);
  row('lineSegment', 'same', `${turf.lineSegment(line).features.length} vs ${gp.lineSegment(line).features.length} segments`);
  const sp = turf.lineSplit(line, turf.lineString([[0.5, -1], [0.5, 1]]));
  row('lineSplit', 'same', `${sp.features.length} vs ${gp.lineSplit(line, turf.lineString([[0.5, -1], [0.5, 1]])).features.length} parts`);

  const l1 = turf.lineString([[0, 0], [0.01, 0]]);
  const l2 = turf.lineString([[0.002, 0], [0.006, 0]]);
  row('lineOverlap', 'same', `${turf.lineOverlap(l1, l2).features.length} vs ${gp.lineOverlap(l1, l2, { units: 'meters', tolerance: 0.5 }).features.length} shared runs`);
}

// ------------------------------------------------------------------ grids

{
  const b = [116, 39, 116.4, 39.2];
  const ts = turf.squareGrid(b, 5, { units: 'kilometers' });
  const gs = gp.squareGrid(b, 5);
  const cellAreas = (fc) => fc.features.map((f) => gp.area(f, { edges: 'geodesic' }));
  const spread = (v) => (Math.max(...v) - Math.min(...v)) / 25e6;
  const median = (v) => [...v].sort((a, c) => a - c)[Math.floor(v.length / 2)];
  row(
    'squareGrid',
    'better',
    `turf cell areas vary by ${fmtPct(spread(cellAreas(ts)))} of nominal, geoprecise by ${fmtPct(spread(cellAreas(gs)))}`,
  );
  const th = turf.hexGrid(b, 5, { units: 'kilometers' });
  const gh = gp.hexGrid(b, 5);
  const hexNominal = ((3 * Math.sqrt(3)) / 2) * 25e6;
  row(
    'hexGrid',
    'better',
    `median cell ${(median(cellAreas(th)) / hexNominal).toFixed(3)}× nominal (turf) vs ${(median(cellAreas(gh)) / hexNominal).toFixed(3)}×; ${th.features.length} vs ${gh.features.length} cells`,
  );
  const tpg = turf.pointGrid(b, 5, { units: 'kilometers' });
  const gpg = gp.pointGrid(b, 5);
  const gap = (fc) => gp.distance(fc.features[0], fc.features[1], { units: 'meters' });
  row('pointGrid', 'better', `neighbour spacing ${gap(tpg).toFixed(0)} m (turf) vs ${gap(gpg).toFixed(0)} m for a 5 km grid`);
  const tt = turf.triangleGrid(b, 5, { units: 'kilometers' });
  const gtri = gp.triangleGrid(b, 5);
  const nominalTri = 12.5e6;
  row(
    'triangleGrid',
    'better',
    `${tt.features.length} vs ${gtri.features.length} cells; median area ${(median(cellAreas(tt)) / nominalTri).toFixed(3)}× nominal (turf) vs ${(median(cellAreas(gtri)) / nominalTri).toFixed(3)}×`,
  );
  const tr = turf.rectangleGrid(b, 5, 3, { units: 'kilometers' });
  const grect = gp.rectangleGrid(b, 5, 3);
  row(
    'rectangleGrid',
    'better',
    `${tr.features.length} vs ${grect.features.length} cells; median area ${(median(cellAreas(tr)) / 15e6).toFixed(3)}× nominal (turf) vs ${(median(cellAreas(grect)) / 15e6).toFixed(3)}×`,
  );
  row('square', 'same', `${JSON.stringify(turf.square([0, 0, 4, 2]))} vs ${JSON.stringify(gp.square([0, 0, 4, 2]))}`);
  const poly = turf.polygon([[[1, 1], [3, 1], [2, 4], [1, 1]]]);
  row('envelope', 'same', `${JSON.stringify(turf.bbox(turf.envelope(poly)))} vs ${JSON.stringify(turf.bbox(gp.envelope(poly)))}`);
}

// ------------------------------------------------------- interpolation

{
  const features = [];
  for (let j = 0; j <= 10; j++) {
    for (let i = 0; i <= 20; i++) {
      const x = 116 + i * 0.01;
      features.push(turf.point([x, 39 + j * 0.01], { elevation: x }));
    }
  }
  const grid = turf.featureCollection(features);
  const breaks = [116.05, 116.15];
  const ti = turf.isolines(grid, breaks, { zProperty: 'elevation' });
  const gi = gp.isolines(grid, breaks);
  const maxDev = (fc) =>
    Math.max(
      ...fc.features.flatMap((f) => {
        const level = Number(f.properties.elevation);
        const parts = f.geometry.type === 'MultiLineString' ? f.geometry.coordinates : [f.geometry.coordinates];
        return parts.flatMap((p) => p.map((c) => Math.abs(c[0] - level)));
      }),
    );
  row('isolines', 'same', `contour x deviates from the level by ${maxDev(ti).toExponential(1)}° (turf) vs ${maxDev(gi).toExponential(1)}°`);

  const tb = turf.isobands(grid, [116, 116.1, 116.2], { zProperty: 'elevation' });
  const gb = gp.isobands(grid, [116, 116.1, 116.2]);
  const domain = gp.area(gp.bboxPolygon(gp.bbox(grid)));
  const cover = (fc) => fc.features.reduce((s, f) => s + gp.area(f), 0) / domain;
  row('isobands', 'better', `bands cover ${cover(tb).toFixed(4)} of the domain (turf) vs ${cover(gb).toFixed(4)}`);

  const tt = turf.tin(grid, 'elevation');
  const gt = gp.tin(grid);
  const hull = gp.area(gp.convexHull(grid));
  const tri = (fc) => fc.features.reduce((s, f) => s + gp.area(f), 0) / hull;
  row('tin', 'same', `${tt.features.length} vs ${gt.features.length} triangles, covering ${tri(tt).toFixed(6)} vs ${tri(gt).toFixed(6)} of the hull`);

  const sites = turf.featureCollection([turf.point([116, 39]), turf.point([116.1, 39]), turf.point([116.05, 39.1])]);
  const bb = [115.95, 38.95, 116.15, 39.15];
  const tv = turf.voronoi(sites, { bbox: bb });
  const gv = gp.voronoi(sites, { bbox: bb });
  const bbArea = gp.area(gp.bboxPolygon(bb));
  const vcov = (fc) => fc.features.filter(Boolean).reduce((s, f) => s + gp.area(f), 0) / bbArea;
  row(
    'voronoi',
    'differs',
    `cells cover ${vcov(tv).toFixed(5)} of the bbox (turf) vs ${vcov(gv).toFixed(5)}; bisectors are straight in the metric plane, so writing them as lon/lat chords leaves a hairline along each shared edge`,
  );

  const samples = turf.featureCollection([
    turf.point([116, 39], { elevation: 0 }),
    turf.point([116.2, 39], { elevation: 100 }),
  ]);
  const gsurf = gp.interpolate(samples, 2, { gridType: 'point', weight: 2, bbox: [116, 38.99, 116.2, 39.01] });
  const tsurf = turf.interpolate(samples, 2, { gridType: 'point', property: 'elevation', weight: 2, units: 'kilometers' });
  row(
    'interpolate',
    'better',
    `${tsurf.features.length} vs ${gsurf.features.length} cells over a 0.02°-tall bbox — turf's degree-converted cells are taller than the box, so it returns nothing`,
  );

  const t0 = gt.features[0];
  const corner = t0.geometry.coordinates[0][0];
  row('planepoint', 'same', `${turf.planepoint(corner, t0).toFixed(6)} vs ${gp.planepoint(corner, t0).toFixed(6)} at a corner`);
}

// -------------------------------------------------------------- clustering

{
  const blob = (cx, cy, n) => Array.from({ length: n }, (_, k) => gp.destination([cx, cy], 0.5, (360 * k) / n).geometry.coordinates);
  const coords = [...blob(116, 39, 8), ...blob(116.5, 39, 8), [117.5, 39.5]];
  const fc = turf.featureCollection(coords.map((c) => turf.point(c)));

  const td = turf.clustersDbscan(turf.clone(fc), 2, { units: 'kilometers', minPoints: 3 });
  const gd = gp.clustersDbscan(fc, 2, { minPoints: 3 });
  const count = (f) => new Set(f.features.map((x) => x.properties.cluster).filter((v) => v !== undefined)).size;
  const noise = (f) => f.features.filter((x) => x.properties.dbscan === 'noise').length;
  row('clustersDbscan', 'same', `${count(td)}/${count(gd)} clusters, ${noise(td)}/${noise(gd)} noise points`);

  const tk = turf.clustersKmeans(turf.clone(fc), { numberOfClusters: 2 });
  const gk = gp.clustersKmeans(fc, { numberOfClusters: 2 });
  row('clustersKmeans', 'better', `${count(tk)}/${count(gk)} clusters; geoprecise seeds deterministically, so the result repeats`);

  const knots = turf.featureCollection([...blob(116, 39, 24), ...blob(116.2, 39.2, 24)].map((c) => turf.point(c)));
  const tn = turf.nearestNeighborAnalysis(knots).properties.nearestNeighborAnalysis;
  const gn = gp.nearestNeighborAnalysis(knots).properties.nearestNeighborAnalysis;
  // Closed form for this pattern: 24 points on each 500 m ring, so every
  // nearest neighbour is 2·500·sin(π/24) apart, and the hull is two discs
  // joined by their common tangents.
  const obs = 2 * 500 * Math.sin(Math.PI / 24);
  const sep = gp.distance([116, 39], [116.2, 39.2], { units: 'meters' });
  const hullArea = 2 * 500 * sep + Math.PI * 500 * 500;
  const exp = 0.5 / Math.sqrt(48 / hullArea);
  row(
    'nearestNeighborAnalysis',
    'better',
    `index ${tn.nearestNeighborIndex.toFixed(4)} (turf) vs ${gn.nearestNeighborIndex.toFixed(4)}; closed form for this pattern is ${(obs / exp).toFixed(4)}`,
  );

  const ts = turf.standardDeviationalEllipse(knots).properties.standardDeviationalEllipse;
  const gs = gp.standardDeviationalEllipse(knots).properties.standardDeviationalEllipse;
  row('standardDeviationalEllipse', 'better', `turf axes ${ts.semiMajorAxis.toFixed(4)}/${ts.semiMinorAxis.toFixed(4)} (degrees, skewed by latitude) vs ${gs.semiMajorAxis.toFixed(4)}/${gs.semiMinorAxis.toFixed(4)} km`);

  const lines = turf.featureCollection(
    [40, 45, 50].map((az) => turf.lineString([[116, 39], gp.destination([116, 39], 10, az).geometry.coordinates])),
  );
  const tdm = turf.directionalMean(lines).properties.cartesianAngle;
  const gdm = gp.directionalMean(lines).cartesianAngle;
  row('directionalMean', 'same', `cartesian angle ${tdm.toFixed(3)}° (turf) vs ${gdm.toFixed(3)}°`);
}

// ------------------------------------------------------------------ routing

{
  const start = [116, 39];
  const end = [116.4, 39];
  const wall = turf.polygon([[[116.19, 38.9], [116.21, 38.9], [116.21, 39.05], [116.19, 39.05], [116.19, 38.9]]]);
  const tp = turf.shortestPath(start, end, { obstacles: turf.featureCollection([wall]), resolution: 1.5 });
  const gpp = gp.shortestPath(start, end, { obstacles: wall, resolution: 1.5, padding: 6 });
  const direct = gp.distance(start, end);
  row(
    'shortestPath',
    'better',
    `turf ${gp.length(tp).toFixed(2)} km vs ${gp.length(gpp).toFixed(2)} km (straight line ${direct.toFixed(2)} km); geoprecise pulls the staircase taut`,
  );
  const clear = gp.shortestPath(start, end, { resolution: 1.5, padding: 6 });
  row('shortestPath (clear)', 'better', `${gp.length(clear).toFixed(4)} km vs the straight line ${direct.toFixed(4)} km`);
}

// ------------------------------------------------------------- helper layer

{
  row('convertLength', 'same', `${turf.convertLength(1, 'miles', 'meters').toFixed(6)} vs ${gp.convertLength(1, 'miles', 'meters').toFixed(6)}`);
  row('convertArea', 'same', `${turf.convertArea(1, 'kilometers', 'meters')} vs ${gp.convertArea(1, 'kilometers', 'meters')}`);
  row('bearingToAzimuth', 'same', `${turf.bearingToAzimuth(-45)} vs ${gp.bearingToAzimuth(-45)}`);
  row('radiansToLength', 'same', `${turf.radiansToLength(1, 'meters')} vs ${gp.radiansToLength(1, 'meters')}`);
  const poly = turf.polygon([[[0, 0], [2, 0], [2, 2], [0, 2], [0, 0]], [[0.5, 0.5], [1.5, 0.5], [1.5, 1.5], [0.5, 1.5], [0.5, 0.5]]]);
  row('coordAll', 'same', `${turf.coordAll(poly).length} vs ${gp.coordAll(poly).length} positions`);
  let ts = 0;
  let gs = 0;
  turf.segmentEach(poly, () => ts++);
  gp.segmentEach(poly, () => gs++);
  row('segmentEach', 'same', `${ts} vs ${gs} segments`);
  row('centroid', 'same', `${JSON.stringify(turf.centroid(poly).geometry.coordinates)} vs ${JSON.stringify(gp.centroid(poly).geometry.coordinates)}`);
  const cloud = turf.featureCollection([turf.point([0, 0]), turf.point([1, 0]), turf.point([0, 1]), turf.point([1, 1])]);
  row('centerMean', 'same', `${JSON.stringify(turf.centerMean(cloud).geometry.coordinates)} vs ${JSON.stringify(gp.centerMean(cloud).geometry.coordinates.map((v) => gp.round(v, 9)))}`);
  // A symmetric set has a very flat basin, so use one with an outlier: the
  // median should stay with the cluster while the mean is dragged away.
  const skewed = turf.featureCollection([
    turf.point([116, 39]),
    turf.point([116.01, 39]),
    turf.point([116, 39.01]),
    turf.point([116.01, 39.01]),
    turf.point([117, 40]),
  ]);
  const tmed = turf.centerMedian(skewed).geometry.coordinates;
  const gmed = gp.centerMedian(skewed).geometry.coordinates;
  const tmean = turf.centerMean(skewed).geometry.coordinates;
  const pull = (c) => gp.distance([116.005, 39.005], c, { units: 'meters' });
  row(
    'centerMedian',
    'same',
    `${pull(tmed).toFixed(0)} m from the cluster centre (turf) vs ${pull(gmed).toFixed(0)} m, against ${pull(tmean).toFixed(0)} m for the mean`,
  );
  row('booleanClockwise', 'same', `${turf.booleanClockwise([[0, 0], [0, 1], [1, 1], [1, 0], [0, 0]])} vs ${gp.booleanClockwise([[0, 0], [0, 1], [1, 1], [1, 0], [0, 0]])}`);
  row('combine', 'same', `${turf.combine(cloud).features[0].geometry.coordinates.length} vs ${gp.combine(cloud).features[0].geometry.coordinates.length} positions merged`);
}


// ------------------------------------------- arcs, tesselation, statistics

{
  const c = [116.4, 39.9];
  const ta = turf.lineArc(c, 5, 0, 90, { steps: 64 });
  const ga = gp.lineArc(c, 5, 0, 90, { steps: 64 });
  const spread = (f) => {
    const rs = turf.getCoords(f).map((p) => gp.distance(c, p, { units: 'meters' }));
    return Math.max(...rs) - Math.min(...rs);
  };
  row('lineArc', 'better', `turf radius spread ${spread(ta).toFixed(1)} m vs ${spread(ga).toFixed(4)} m`);

  const holed = turf.polygon([
    [[116, 39], [116.1, 39], [116.1, 39.1], [116, 39.1], [116, 39]],
    [[116.03, 39.03], [116.07, 39.03], [116.07, 39.07], [116.03, 39.07], [116.03, 39.03]],
  ]);
  const tt = turf.tesselate(holed);
  const gt = gp.tesselate(holed);
  const want = gp.area(holed);
  const cover = (fc) => fc.features.reduce((s, f) => s + gp.area(f), 0) / want;
  row('tesselate', 'same', `${tt.features.length} vs ${gt.features.length} triangles, covering ${cover(tt).toFixed(6)} vs ${cover(gt).toFixed(6)} of the polygon`);

  // an 8x8 metric lattice, 1 km apart, valued as a west-to-east ramp
  const at = (i, j) => {
    const east = gp.destination([116, 39], i, 90).geometry.coordinates;
    return gp.destination(east, j, 0).geometry.coordinates;
  };
  const n = 8;
  const lattice = turf.featureCollection(
    Array.from({ length: n * n }, (_, k) => turf.point(at(k % n, Math.floor(k / n)), { elevation: k % n })),
  );
  // turf thresholds a Minkowski distance on raw degrees, so there is no single
  // value that captures the four 1 km neighbours: at 39°N one degree of
  // longitude is 0.0117° per km against 0.0090° for latitude. Try turf's own
  // best case — a threshold just past the latitude step.
  const gw = gp.distanceWeight(lattice, { threshold: 1.2, standardization: 'raw' });
  const rook = (m) => m[9].reduce((a, b) => a + b, 0);
  const twWide = turf.distanceWeight(lattice, { threshold: 1.2, standardization: false, binary: true });
  const twTuned = turf.distanceWeight(lattice, { threshold: 0.0095, standardization: false, binary: true });
  row(
    'distanceWeight',
    'better',
    `interior point has ${rook(twWide)} neighbours at turf threshold 1.2 and ${rook(twTuned)} at 0.0095 (degrees), against ${rook(gw)} for a 1.2 km geodesic radius`,
  );

  const tmWide = turf.moranIndex(lattice, { inputField: 'elevation', threshold: 1.2, binary: true });
  const tmTuned = turf.moranIndex(lattice, { inputField: 'elevation', threshold: 0.0095, binary: true });
  const gm = gp.moranIndex(lattice, { threshold: 1.2 });
  row(
    'moranIndex',
    'better',
    `ramp I = ${tmWide.moranIndex.toFixed(4)} / ${tmTuned.moranIndex.toFixed(4)} (turf, at those two thresholds) vs ${gm.moranIndex.toFixed(4)}; expectation ${gm.expectedMoranIndex.toFixed(4)}`,
  );

  const gq = gp.quadratAnalysis(lattice, { xQuadrats: 4, yQuadrats: 4 });
  const tq = turf.quadratAnalysis(lattice, { studyBbox: turf.bbox(lattice) });
  row(
    'quadratAnalysis',
    'better',
    `on a perfect lattice turf's degree quadrats give uneven counts and reject randomness (${tq.isRandom}); equal-area quadrats give ${JSON.stringify(gq.counts.slice(0, 4))}… so chi² is ${gq.chiSquared.toFixed(1)} and the variance-to-mean ratio ${gq.varianceMeanRatio.toFixed(2)} is what reports the regularity`,
  );
}

// ---------------------------------------------------------- naming and index

{
  const poly = turf.polygon([[[0, 0], [4, 0], [4, 2], [0, 2], [0, 0]]]);
  row('center', 'same', `${JSON.stringify(turf.center(poly).geometry.coordinates)} vs ${JSON.stringify(gp.center(poly).geometry.coordinates)}`);
  const pts = turf.featureCollection([turf.point([0, 0]), turf.point([2, 0]), turf.point([1, 2]), turf.point([1, 1])]);
  row('convex', 'same', `hull areas ${turf.area(turf.convex(pts)).toFixed(0)} vs ${gp.area(gp.convex(pts)).toFixed(0)} m²`);
  row('booleanValid', 'same', `${turf.booleanValid(poly)} / ${gp.booleanValid(poly)} for a square, ${turf.booleanValid(turf.polygon([[[0, 0], [2, 2], [2, 0], [0, 2], [0, 0]]]))} / ${gp.booleanValid(turf.polygon([[[0, 0], [2, 2], [2, 0], [0, 2], [0, 0]]]))} for a bowtie`);

  const boxes = [];
  for (let i = 0; i < 40; i++) {
    for (let j = 0; j < 5; j++) {
      boxes.push(turf.polygon([[[i, j], [i + 0.5, j], [i + 0.5, j + 0.5], [i, j + 0.5], [i, j]]], { i, j }));
    }
  }
  const tTree = turf.geojsonRbush();
  tTree.load(turf.featureCollection(boxes));
  const gTree = gp.geojsonRbush();
  gTree.load(boxes);
  const q = [2, 2, 3, 3];
  row('geojsonRbush', 'same', `${tTree.search(q).features.length} vs ${gTree.search(q).features.length} hits; collides ${tTree.collides(q)} / ${gTree.collides(q)}`);
}

const summary = rows.reduce((m, r) => ({ ...m, [r.verdict]: (m[r.verdict] ?? 0) + 1 }), {});
console.log('\nsummary:', summary);
writeFileSync(new URL('out/parity.json', import.meta.url), JSON.stringify({ summary, rows }, null, 2));
