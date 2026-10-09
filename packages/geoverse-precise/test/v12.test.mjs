import { test } from 'node:test';
import assert from 'node:assert/strict';
import * as gp from '../dist/index.js';

test('setup', async () => {
  await gp.init();
});

const pt = (x, y) => ({ type: 'Point', coordinates: [x, y] });

test('rhumb lines are consistent and longer than geodesics', () => {
  const a = [0, 50];
  const b = [60, 50];
  const rhumb = gp.rhumbDistance(a, b);
  assert.ok(rhumb > gp.distance(a, b), `${rhumb} vs ${gp.distance(a, b)}`);

  const brg = gp.rhumbBearing(a, b);
  assert.ok(Math.abs(brg - 90) < 1e-9, `${brg}`);
  // a rhumb line along a parallel stays on it
  const mid = gp.rhumbDestination(a, rhumb / 2, brg);
  assert.ok(Math.abs(mid.geometry.coordinates[1] - 50) < 1e-9);
  // round trip
  const end = gp.rhumbDestination(a, rhumb, brg);
  assert.ok(Math.abs(end.geometry.coordinates[0] - 60) < 1e-7, JSON.stringify(end.geometry.coordinates));

  // units flow through
  const m = gp.rhumbDistance(a, b, { units: 'meters' });
  assert.ok(Math.abs(m / 1000 - rhumb) < 1e-6);
});

test('line tools', () => {
  const line = { type: 'LineString', coordinates: [[0, 0], [1, 0], [1, 1]] };
  const segs = gp.lineSegment(line);
  assert.equal(segs.features.length, 2);
  assert.deepEqual(segs.features[0].geometry.coordinates, [[0, 0], [1, 0]]);

  const parts = gp.lineSplit(line, { type: 'LineString', coordinates: [[0.5, -1], [0.5, 1]] });
  assert.equal(parts.features.length, 2);

  const off = gp.lineOffset({ type: 'LineString', coordinates: [[120, 30], [120.3, 30]] }, 0.5);
  // 500 m north of a west-east line
  assert.ok(off.geometry.coordinates.every((c) => c[1] > 30));
  const d = gp.pointToLineDistance(off.geometry.coordinates[0], line0(), { units: 'meters' });
  function line0() {
    return { type: 'LineString', coordinates: [[120, 30], [120.3, 30]] };
  }
  assert.ok(Math.abs(d - 500) < 0.1, `${d}`);

  const ov = gp.lineOverlap(
    { type: 'LineString', coordinates: [[0, 0], [0.01, 0]] },
    { type: 'LineString', coordinates: [[0.002, 0], [0.006, 0]] },
    { units: 'meters', tolerance: 0.5 },
  );
  assert.equal(ov.features.length, 1);

  const near = gp.nearestPointToLine(
    { type: 'MultiPoint', coordinates: [[0.5, 0.5], [0.5, 0.01]] },
    line,
  );
  assert.deepEqual(near.geometry.coordinates, [0.5, 0.01]);
  assert.ok(near.properties.dist > 0);

  const square = {
    type: 'Polygon',
    coordinates: [[[0, 0], [1, 0], [1, 1], [0, 1], [0, 0]]],
  };
  assert.ok(gp.pointToPolygonDistance([0.5, 0.5], square) < 0, 'inside is negative');
  assert.ok(gp.pointToPolygonDistance([2, 0.5], square) > 0, 'outside is positive');

  assert.ok(Math.abs(gp.angle([1, 0], [0, 0], [0, 1]) - 90) < 0.01);
  assert.ok(Math.abs(gp.angle([1, 0], [0, 0], [0, 1], { explementary: true }) - 270) < 0.01);
});

test('grids have metric cells and honour a mask', () => {
  const b = [120, 30, 120.2, 30.1];
  const sq = gp.squareGrid(b, 2, { units: 'kilometers' });
  assert.ok(sq.features.length > 20);
  for (const f of sq.features.slice(0, 10)) {
    const a = gp.area(f, { edges: 'geodesic' });
    assert.ok(Math.abs(a - 4e6) / 4e6 < 0.01, `${a}`);
  }

  const pts = gp.pointGrid(b, 5);
  const spacing = gp.distance(pts.features[0], pts.features[1], { units: 'meters' });
  assert.ok(Math.abs(spacing - 5000) < 5, `${spacing}`);

  const maskPoly = {
    type: 'Polygon',
    coordinates: [[[120, 30], [120.05, 30], [120.05, 30.05], [120, 30.05], [120, 30]]],
  };
  const masked = gp.pointGrid(b, 2, { mask: maskPoly });
  assert.ok(masked.features.length > 0 && masked.features.length < pts.features.length * 4);
  for (const f of masked.features) {
    assert.ok(gp.booleanPointInPolygon(f, maskPoly));
  }

  const hex = gp.hexGrid(b, 1);
  assert.equal(hex.features[0].geometry.coordinates[0].length, 7);
  const tri = gp.triangleGrid(b, 2);
  assert.equal(tri.features.length % 2, 0);
  const rect = gp.rectangleGrid(b, 4, 2);
  const ra = gp.area(rect.features[0], { edges: 'geodesic' });
  assert.ok(Math.abs(ra - 8e6) / 8e6 < 0.01, `${ra}`);

  assert.deepEqual(gp.square([0, 0, 4, 2]), [0, -1, 4, 3]);
  const env = gp.envelope({ type: 'MultiPoint', coordinates: [[1, 1], [3, 4]] });
  assert.deepEqual(env.geometry.coordinates[0][0], [1, 1]);
});

test('shape constructors', () => {
  const e = gp.ellipse([120, 30], 2, 1, { steps: 128 });
  const centre = [120, 30];
  const far = e.geometry.coordinates[0].reduce(
    (m, c) => Math.max(m, gp.distance(centre, c, { units: 'meters' })),
    0,
  );
  assert.ok(Math.abs(far - 2000) < 1, `${far}`);

  const sq = { type: 'Polygon', coordinates: [[[120, 30], [120.1, 30], [120.1, 30.1], [120, 30.1], [120, 30]]] };
  const sm = gp.polygonSmooth(sq, { iterations: 1 });
  assert.equal(sm.features[0].geometry.coordinates[0].length, 9);
  assert.ok(gp.area(sm.features[0]) < gp.area(sq));

  const tangents = gp.polygonTangents([3, 0.5], {
    type: 'Polygon',
    coordinates: [[[0, 0], [1, 0], [1, 1], [0, 1], [0, 0]]],
  });
  assert.equal(tangents.features.length, 2);

  const big = { type: 'Polygon', coordinates: [[[-1, -1], [2, -1], [2, 2], [-1, 2], [-1, -1]]] };
  const m = gp.mask({ type: 'Polygon', coordinates: [[[0, 0], [1, 0], [1, 1], [0, 1], [0, 0]]] }, big);
  const rings = m.geometry.type === 'Polygon' ? m.geometry.coordinates : m.geometry.coordinates[0];
  assert.equal(rings.length, 2, 'the mask keeps a hole');

  assert.equal(gp.booleanConcave({ type: 'Polygon', coordinates: [[[0, 0], [1, 0], [1, 1], [0, 1], [0, 0]]] }), false);
  assert.equal(
    gp.booleanConcave({ type: 'Polygon', coordinates: [[[0, 0], [2, 0], [1, 1], [2, 2], [0, 2], [0, 0]]] }),
    true,
  );
  assert.equal(
    gp.booleanParallel({ type: 'LineString', coordinates: [[0, 0], [1, 0]] }, { type: 'LineString', coordinates: [[0, 1], [1, 1]] }),
    true,
  );

  const flipped = gp.flip(pt(1, 2));
  assert.deepEqual(flipped.features[0].geometry.coordinates, [2, 1]);

  const cloud = { type: 'MultiPoint', coordinates: [[0, 0], [1, 0], [0, 1], [1, 1]] };
  const mean = gp.centerMean(cloud);
  assert.ok(Math.abs(mean.geometry.coordinates[0] - 0.5) < 1e-9);
  const median = gp.centerMedian(cloud);
  assert.ok(Math.abs(median.geometry.coordinates[0] - 0.5) < 1e-3);

  const spline = gp.bezierSpline({ type: 'LineString', coordinates: [[0, 0], [1, 1], [2, 0], [3, 1]] });
  assert.ok(spline.geometry.coordinates.length > 20);

  const polys = gp.polygonize({
    type: 'GeometryCollection',
    geometries: [
      { type: 'LineString', coordinates: [[0, 0], [1, 0]] },
      { type: 'LineString', coordinates: [[1, 0], [1, 1]] },
      { type: 'LineString', coordinates: [[1, 1], [0, 1]] },
      { type: 'LineString', coordinates: [[0, 1], [0, 0]] },
    ],
  });
  assert.equal(polys.features.length, 1);
});

/** A lattice whose value is its longitude, so contours are meridians. */
function rampGrid() {
  const features = [];
  for (let j = 0; j <= 10; j++) {
    for (let i = 0; i <= 20; i++) {
      const x = 120 + i * 0.01;
      features.push({
        type: 'Feature',
        properties: { elevation: x },
        geometry: { type: 'Point', coordinates: [x, 30 + j * 0.01] },
      });
    }
  }
  return { type: 'FeatureCollection', features };
}

test('interpolation and contouring', () => {
  const g = rampGrid();

  const iso = gp.isolines(g, [120.05, 120.15]);
  assert.equal(iso.features.length, 2);
  for (const f of iso.features) {
    const level = f.properties.elevation;
    for (const part of f.geometry.coordinates) {
      for (const c of part) assert.ok(Math.abs(c[0] - level) < 1e-9, `${c} vs ${level}`);
    }
  }

  const bands = gp.isobands(g, [120, 120.1, 120.2]);
  assert.equal(bands.features.length, 2);
  const total = bands.features.reduce((s, f) => s + gp.area(f), 0);
  const domain = gp.area(gp.bboxPolygon(gp.bbox(g)));
  assert.ok(Math.abs(total / domain - 1) < 2e-3, `${total / domain}`);

  const surf = gp.interpolate(
    { type: 'FeatureCollection', features: [
      { type: 'Feature', properties: { elevation: 0 }, geometry: pt(120, 30) },
      { type: 'Feature', properties: { elevation: 100 }, geometry: pt(120.2, 30) },
    ] },
    2,
    { gridType: 'point', weight: 2, bbox: [120, 29.99, 120.2, 30.01] },
  );
  assert.ok(surf.features.length > 5);
  for (const f of surf.features) {
    assert.ok(f.properties.elevation >= -1e-9 && f.properties.elevation <= 100 + 1e-9);
  }

  const t = gp.tin(g);
  assert.ok(t.features.length > 100);
  assert.ok('a' in t.features[0].properties);
  const z = gp.planepoint(t.features[0].geometry.coordinates[0][0], t.features[0]);
  assert.ok(Math.abs(z - t.features[0].properties.a) < 1e-6, `${z}`);

  const v = gp.voronoi({ type: 'MultiPoint', coordinates: [[120, 30], [120.1, 30], [120.05, 30.1]] });
  assert.equal(v.features.length, 3);
  assert.ok(gp.booleanPointInPolygon([120, 30], v.features[0]));
});

test('clustering and point statistics', () => {
  const blob = (cx, cy, n) =>
    Array.from({ length: n }, (_, k) => {
      const d = gp.destination([cx, cy], 0.5, (360 * k) / n);
      return d.geometry.coordinates;
    });
  const coords = [...blob(120, 30, 8), ...blob(120.5, 30, 8), [121.5, 30.5]];
  const fcOf = (cs) => ({
    type: 'FeatureCollection',
    features: cs.map((c) => ({ type: 'Feature', properties: {}, geometry: pt(c[0], c[1]) })),
  });

  const db = gp.clustersDbscan(fcOf(coords), 2);
  assert.equal(db.features.length, coords.length);
  assert.equal(db.features.at(-1).properties.dbscan, 'noise');
  const first = db.features[0].properties.cluster;
  assert.ok(db.features.slice(0, 8).every((f) => f.properties.cluster === first));
  assert.notEqual(db.features[8].properties.cluster, first);

  const km = gp.clustersKmeans(fcOf(coords), { numberOfClusters: 2 });
  assert.equal(new Set(km.features.map((f) => f.properties.cluster)).size, 2);
  assert.ok(Array.isArray(km.features[0].properties.centroid));

  // The index is a mean over points, so one far outlier would dominate it —
  // measure clustering on two knots with nothing stray in the hull.
  const knots = [...blob(120, 30, 24), ...blob(120.2, 30.2, 24)];
  const nna = gp.nearestNeighborAnalysis(fcOf(knots));
  const r = nna.properties.nearestNeighborAnalysis;
  assert.equal(r.numberOfPoints, knots.length);
  assert.ok(r.studyAreaSize > 0);
  assert.ok(r.nearestNeighborIndex < 0.5, `clustered: ${r.nearestNeighborIndex}`);
  assert.ok(r.zScore < -5, `${r.zScore}`);
  // units flow through the reported distances
  const inM = gp.nearestNeighborAnalysis(fcOf(knots), { units: 'meters' }).properties.nearestNeighborAnalysis;
  assert.ok(Math.abs(inM.observedMeanDistance / 1000 - r.observedMeanDistance) < 1e-9);

  const sde = gp.standardDeviationalEllipse(fcOf([...blob(120, 30, 6), [120.2, 30], [119.8, 30]]));
  const s = sde.properties.standardDeviationalEllipse;
  assert.ok(s.semiMajorAxis >= s.semiMinorAxis);
  assert.ok(Math.abs(s.majorAxisBearing - 90) < 5, `${s.majorAxisBearing}`);

  const dm = gp.directionalMean({
    type: 'FeatureCollection',
    features: [40, 45, 50].map((az) => ({
      type: 'Feature',
      properties: {},
      geometry: { type: 'LineString', coordinates: [[120, 30], gp.destination([120, 30], 10, az).geometry.coordinates] },
    })),
  });
  assert.equal(dm.countOfLines, 3);
  assert.ok(Math.abs(dm.bearingAngle - 45) < 0.5, `${dm.bearingAngle}`);
  assert.ok(dm.circularVariance < 0.01);
});

test('shortest path goes round obstacles and straight when clear', () => {
  const start = [120, 30];
  const end = [120.4, 30];
  const clear = gp.shortestPath(start, end, { resolution: 1.5, padding: 6 });
  assert.equal(clear.geometry.coordinates.length, 2, 'nothing in the way: a straight line');

  const wall = {
    type: 'Polygon',
    coordinates: [[[120.19, 29.9], [120.21, 29.9], [120.21, 30.05], [120.19, 30.05], [120.19, 29.9]]],
  };
  const around = gp.shortestPath(start, end, { obstacles: wall, resolution: 1.5, padding: 6 });
  assert.ok(gp.length(around) > gp.length(clear) * 1.02);
  assert.ok(!gp.booleanIntersects(around, wall), 'the route must not cross the wall');
  assert.ok(around.geometry.coordinates.some((c) => c[1] > 30.04), 'it rounds the open end');
});

test('turf-shaped aliases', () => {
  const bow = {
    type: 'LineString',
    coordinates: [[0, 0], [2, 2], [2, 0], [0, 2]],
  };
  // the first and last segments cross where y = x meets y = 2 - x
  const k = gp.kinks(bow);
  assert.equal(k.features.length, 1);
  assert.deepEqual(
    k.features[0].geometry.coordinates.map((v) => Math.round(v * 1e9) / 1e9),
    [1, 1],
  );

  const bowtie = {
    type: 'Polygon',
    coordinates: [[[0, 0], [2, 2], [2, 0], [0, 2], [0, 0]]],
  };
  const fixed = gp.unkinkPolygon(bowtie);
  assert.equal(fixed.features.length, 2);

  const merc = gp.toMercator(pt(0, 0));
  // the origin can come back as -0, which deepEqual treats as distinct from 0
  assert.ok(merc.coordinates.every((v) => Math.abs(v) < 1e-6), JSON.stringify(merc.coordinates));
  const m2 = gp.toMercator(pt(90, 0));
  assert.ok(Math.abs(m2.coordinates[0] - 10018754.17) < 1, `${m2.coordinates[0]}`);
  const back = gp.toWgs84(m2);
  assert.ok(Math.abs(back.coordinates[0] - 90) < 1e-7);

  const cell = { type: 'Polygon', coordinates: [[[0, 0], [10, 0], [10, 10], [0, 10], [0, 0]]] };
  const inside = {
    type: 'FeatureCollection',
    features: [
      { type: 'Feature', properties: { pop: 3 }, geometry: pt(1, 1) },
      { type: 'Feature', properties: { pop: 4 }, geometry: pt(2, 2) },
      { type: 'Feature', properties: { pop: 9 }, geometry: pt(20, 20) },
    ],
  };
  const collected = gp.collect({ type: 'FeatureCollection', features: [{ type: 'Feature', properties: { id: 'a' }, geometry: cell }] }, inside, 'pop', 'values');
  assert.deepEqual(collected.features[0].properties.values, [3, 4]);

  const tagged = gp.tag(inside, { type: 'FeatureCollection', features: [{ type: 'Feature', properties: { zone: 'A' }, geometry: cell }] }, 'zone', 'zone');
  assert.equal(tagged.features[0].properties.zone, 'A');
  assert.equal(tagged.features[2].properties.zone, undefined);
});

test('arcs and tesselation', () => {
  const c = [120, 30];
  const arc = gp.lineArc(c, 5, 0, 90, { steps: 64 });
  assert.equal(arc.geometry.type, 'LineString');
  for (const p of arc.geometry.coordinates) {
    const d = gp.distance(c, p, { units: 'meters' });
    assert.ok(Math.abs(d - 5000) < 0.01, `${d}`);
  }
  // the arc runs from due north round to due east
  assert.ok(Math.abs(gp.bearing(c, arc.geometry.coordinates[0]) - 0) < 1e-6);
  assert.ok(Math.abs(gp.bearing(c, arc.geometry.coordinates.at(-1)) - 90) < 1e-6);
  // a full sweep closes
  const full = gp.lineArc(c, 5, 0, 360, { steps: 64 });
  assert.deepEqual(full.geometry.coordinates[0], full.geometry.coordinates.at(-1));

  const holed = {
    type: 'Polygon',
    coordinates: [
      [[120, 30], [120.1, 30], [120.1, 30.1], [120, 30.1], [120, 30]],
      [[120.03, 30.03], [120.07, 30.03], [120.07, 30.07], [120.03, 30.07], [120.03, 30.03]],
    ],
  };
  const tris = gp.tesselate(holed);
  assert.ok(tris.features.length >= 8, `${tris.features.length} triangles`);
  const want = gp.area(holed);
  const got = tris.features.reduce((s, f) => s + gp.area(f), 0);
  assert.ok(Math.abs(got / want - 1) < 1e-6, `${got} vs ${want}`);
  for (const f of tris.features) {
    assert.ok(gp.booleanPointInPolygon(gp.centroid(f), holed), 'no triangle in the hole');
  }
});

test('spatial statistics', () => {
  // a metric lattice: 8×8 points 1 km apart
  const step = 1;
  const origin = [120, 30];
  const at = (i, j) => {
    const east = gp.destination(origin, i * step, 90).geometry.coordinates;
    return gp.destination(east, j * step, 0).geometry.coordinates;
  };
  const n = 8;
  const fc = (vals) => ({
    type: 'FeatureCollection',
    features: Array.from({ length: n * n }, (_, k) => ({
      type: 'Feature',
      properties: { elevation: vals[k] },
      geometry: { type: 'Point', coordinates: at(k % n, Math.floor(k / n)) },
    })),
  });

  // rook neighbours only — 1.2 km reaches them but not the 1.414 km diagonals
  const wopts = { threshold: 1.2 };
  const w = gp.distanceWeight(fc(Array(n * n).fill(0)), { ...wopts, standardization: 'raw' });
  assert.equal(w.length, n * n);
  assert.equal(w[0].reduce((a, b) => a + b, 0), 2, 'a corner has two rook neighbours');
  assert.equal(w[9].reduce((a, b) => a + b, 0), 4, 'an interior point has four');

  const ramp = Array.from({ length: n * n }, (_, k) => k % n);
  const smooth = gp.moranIndex(fc(ramp), wopts);
  assert.ok(smooth.moranIndex > 0.5, `ramp I = ${smooth.moranIndex}`);
  assert.ok(smooth.pNorm < 0.01);
  assert.ok(Math.abs(smooth.expectedMoranIndex + 1 / (n * n - 1)) < 1e-12);

  const checker = Array.from({ length: n * n }, (_, k) => ((k % n) + Math.floor(k / n)) % 2);
  const alt = gp.moranIndex(fc(checker), wopts);
  assert.ok(alt.moranIndex < -0.8, `checker I = ${alt.moranIndex}`);

  // 4×4 quadrats divide the 8×8 lattice exactly, so every quadrat holds 4
  // points and the variance-to-mean ratio collapses to 0
  const q = gp.quadratAnalysis(fc(ramp), { xQuadrats: 4, yQuadrats: 4 });
  assert.equal(q.numberOfPoints, n * n);
  assert.equal(q.counts.reduce((a, b) => a + b, 0), n * n);
  assert.ok(q.counts.every((c) => c === 4), JSON.stringify(q.counts));
  assert.ok(q.isRandom, `a lattice is not rejected: chi2 ${q.chiSquared} vs ${q.criticalValue}`);
  assert.ok(q.varianceMeanRatio < 1e-9, `${q.varianceMeanRatio}`);
  assert.equal(q.degreesOfFreedom, 15);

  // a 3×3 grid does not divide 8 evenly, and the ratio picks that up — the
  // test measures the quadrats as much as the pattern
  const uneven = gp.quadratAnalysis(fc(ramp), { xQuadrats: 3 });
  assert.ok(uneven.varianceMeanRatio > 0.3, `${uneven.varianceMeanRatio}`);
});

test('turf naming compatibility', () => {
  const poly = { type: 'Polygon', coordinates: [[[0, 0], [4, 0], [4, 2], [0, 2], [0, 0]]] };
  assert.deepEqual(gp.center(poly).geometry.coordinates, [2, 1]);
  assert.equal(gp.convex, gp.convexHull);
  assert.equal(gp.concave, gp.concaveHull);
  assert.equal(gp.booleanValid(poly), true);
  assert.equal(gp.booleanValid({ type: 'Polygon', coordinates: [[[0, 0], [2, 2], [2, 0], [0, 2], [0, 0]]] }), false);

  // the namespace objects turf users reach for
  assert.equal(typeof gp.meta.coordEach, 'function');
  assert.equal(typeof gp.helpers.point, 'function');
  assert.equal(typeof gp.invariant.getCoord, 'function');
  assert.equal(typeof gp.projection.toMercator, 'function');
  assert.equal(typeof gp.random.randomPoint, 'function');
  assert.equal(typeof gp.clusters.clusterEach, 'function');
});
