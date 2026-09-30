// v1.1 surface: prepared geometry, batch APIs, geometry processing,
// DE-9IM predicates, topology validation and quality checks.
import { test, before } from 'node:test';
import assert from 'node:assert/strict';
import * as gp from '../dist/index.js';

before(async () => {
  await gp.init();
});

const square = (x0, y0, x1, y1) => ({
  type: 'Polygon',
  coordinates: [[[x0, y0], [x1, y0], [x1, y1], [x0, y1], [x0, y0]]],
});
const feature = (geometry, properties = {}) => ({ type: 'Feature', properties, geometry });
const fc = (...features) => ({ type: 'FeatureCollection', features });

// ------------------------------------------------------------------ batch

test('batch measurement matches the single-call API', () => {
  const pairs = new Float64Array([116, 39, 121, 31, 100, 20, 110, 25]);
  const d = gp.distanceBatch(pairs, { units: 'meters' });
  assert.equal(d.length, 2);
  assert.ok(Math.abs(d[0] - gp.distance([116, 39], [121, 31], { units: 'meters' })) < 1e-9);

  const pts = [[116, 39], [117, 40]];
  const to = gp.distanceToBatch([121, 31], pts, { units: 'kilometers' });
  assert.ok(Math.abs(to[1] - gp.distance([121, 31], [117, 40])) < 1e-9);

  const dest = gp.destinationBatch(new Float64Array([116, 39, 10, 90]), { units: 'kilometers' });
  const single = gp.destination([116, 39], 10, 90).geometry.coordinates;
  assert.ok(Math.abs(dest[0] - single[0]) < 1e-12 && Math.abs(dest[1] - single[1]) < 1e-12);
});

test('prepared geometry answers the same as the one-shot calls', () => {
  const poly = square(120, 30, 121, 31);
  // distances and radii use `units` (default kilometers, like the rest of the API)
  const prep = gp.prepare(poly, { units: 'meters' });
  try {
    assert.ok(prep.segmentCount >= 4);
    assert.ok(prep.isAreal);
    const b = prep.bbox();
    assert.deepEqual(b.map((v) => Math.round(v)), [120, 30, 121, 31]);

    const pts = [[120.5, 30.5], [119, 30], [121.0, 30.5]];
    for (const p of pts) {
      assert.equal(prep.contains(p), gp.booleanPointInPolygon(p, poly), `${p}`);
    }
    const mask = prep.containsMany(pts);
    assert.deepEqual(Array.from(mask), [1, 0, 1]);
    assert.deepEqual(Array.from(prep.containsMany(pts, { ignoreBoundary: true })), [1, 0, 0]);

    // nearest point on the boundary
    const n = prep.nearest([120.5, 30.5]);
    assert.equal(n.geometry.type, 'Point');
    assert.ok(n.properties.dist > 0);
    const flat = prep.nearestMany([[120.5, 30.5], [120.1, 30.1]]);
    assert.equal(flat.length, 12);
    assert.ok(Math.abs(flat[2] - n.properties.dist) < 1e-9);

    // distance is zero inside an areal geometry
    assert.equal(prep.distance([120.5, 30.5]), 0);
    assert.ok(prep.distance([119.9, 30.5]) > 0);
    const within = prep.within([[120.5, 30.5], [119.0, 30.5]], 1000);
    assert.deepEqual(Array.from(within), [0]);
    const wide = gp.prepare(poly);
    try {
      // same query in kilometers reaches both points
      assert.deepEqual(Array.from(wide.within([[120.5, 30.5], [119.0, 30.5]], 1000)), [0, 1]);
    } finally {
      wide.free();
    }
  } finally {
    prep.free();
  }
});

test('prepared geometry throws after free()', () => {
  const p = gp.prepare(square(0, 0, 1, 1));
  p.free();
  assert.throws(() => p.contains([0.5, 0.5]), /freed/);
});

// -------------------------------------------------------------- geometry

test('simplify uses a metric tolerance', () => {
  const coords = [];
  for (let i = 0; i < 200; i++) coords.push([120 + i * 0.001, 30 + (i % 2) * 4.5e-6]);
  const line = { type: 'LineString', coordinates: coords };
  const coarse = gp.simplify(line, 5, { units: 'meters' });
  const fine = gp.simplify(line, 0.05, { units: 'meters' });
  assert.ok(coarse.geometry.coordinates.length < 10);
  assert.ok(fine.geometry.coordinates.length > 100);
  const vw = gp.simplify(line, 5, { units: 'meters', preserveTopology: true });
  assert.ok(vw.geometry.coordinates.length <= coords.length);
});

test('hulls, centres and bounding boxes', () => {
  const pts = fc(
    feature({ type: 'Point', coordinates: [120, 30] }),
    feature({ type: 'Point', coordinates: [120.5, 30] }),
    feature({ type: 'Point', coordinates: [120.5, 30.5] }),
    feature({ type: 'Point', coordinates: [120, 30.5] }),
    feature({ type: 'Point', coordinates: [120.25, 30.25] }),
  );
  const hull = gp.convexHull(pts);
  assert.equal(hull.geometry.type, 'Polygon');
  assert.equal(hull.geometry.coordinates[0].length, 5);
  const concave = gp.concaveHull(pts, { maxEdge: 100, units: 'kilometers' });
  assert.equal(concave.geometry.type, 'Polygon');

  const c = gp.centroid(pts).geometry.coordinates;
  assert.ok(Math.abs(c[0] - 120.25) < 1e-9);
  const com = gp.centerOfMass(hull).geometry.coordinates;
  assert.ok(Math.abs(com[0] - 120.25) < 1e-3);
  const onFeature = gp.pointOnFeature(hull).geometry.coordinates;
  assert.ok(gp.booleanPointInPolygon(onFeature, hull));

  const b = gp.bbox(pts);
  assert.deepEqual(b, [120, 30, 120.5, 30.5]);
  const bp = gp.bboxPolygon(b);
  assert.equal(bp.geometry.coordinates[0].length, 5);
  const clipped = gp.bboxClip(hull, [120.1, 30.1, 120.4, 30.4]);
  assert.ok(gp.area(clipped) < gp.area(hull));
});

test('transforms are geodesic', () => {
  const pt = { type: 'Point', coordinates: [120, 30] };
  const moved = gp.transformTranslate(pt, 1, 90, { units: 'kilometers' });
  assert.ok(Math.abs(gp.distance(pt, moved, { units: 'meters' }) - 1000) < 1e-6);
  const rotated = gp.transformRotate(moved, 90, { pivot: [120, 30] });
  assert.ok(Math.abs(gp.bearing([120, 30], rotated) - 180) < 1e-6);
  const scaled = gp.transformScale(moved, 2, { origin: [120, 30] });
  assert.ok(Math.abs(gp.distance([120, 30], scaled, { units: 'meters' }) - 2000) < 1e-6);
});

test('line tools', () => {
  const line = { type: 'LineString', coordinates: [[120, 30], [120, 31], [121, 31]] };
  const total = gp.length(line, { units: 'kilometers' });
  const half = gp.lineSliceAlong(line, 0, total / 2);
  assert.ok(Math.abs(gp.length(half) - total / 2) < 1e-6);
  const chunks = gp.lineChunk(line, 20);
  assert.ok(chunks.features.length >= 8);
  const sum = chunks.features.reduce((s, f) => s + gp.length(f), 0);
  assert.ok(Math.abs(sum - total) < 1e-6);
  const sliced = gp.lineSlice([120, 30.4], [120.6, 31], line);
  assert.ok(gp.length(sliced) > 0 && gp.length(sliced) < total);

  const a = { type: 'LineString', coordinates: [[0, 0], [2, 2]] };
  const b = { type: 'LineString', coordinates: [[0, 2], [2, 0]] };
  const x = gp.lineIntersect(a, b);
  assert.equal(x.features.length, 1);
  assert.deepEqual(x.features[0].geometry.coordinates, [1, 1]);

  const gc = gp.greatCircle([116, 39], [121, 31], { steps: 50 });
  assert.equal(gc.geometry.coordinates.length, 50);
  const sec = gp.sector([120, 30], 10, 0, 90, { units: 'kilometers' });
  assert.equal(sec.geometry.type, 'Polygon');
  const near = gp.nearestPoint([120, 30], fc(
    feature({ type: 'Point', coordinates: [121, 31] }),
    feature({ type: 'Point', coordinates: [120.1, 30.1] }),
  ));
  assert.equal(near.properties.index, 1);
});

test('structural helpers and dissolve', () => {
  const multi = {
    type: 'MultiPolygon',
    coordinates: [square(0, 0, 1, 1).coordinates, square(2, 2, 3, 3).coordinates],
  };
  assert.equal(gp.flatten(multi).features.length, 2);
  assert.equal(gp.explode(square(0, 0, 1, 1)).features.length, 5);
  assert.equal(gp.polygonToLine(square(0, 0, 1, 1)).geometry.type, 'MultiLineString');
  const ring = { type: 'LineString', coordinates: [[0, 0], [1, 0], [1, 1], [0, 0]] };
  assert.equal(gp.lineToPolygon(ring).geometry.type, 'MultiPolygon');
  const cw = { type: 'Polygon', coordinates: [[[0, 0], [0, 1], [1, 1], [1, 0], [0, 0]]] };
  const wound = gp.rewind(cw);
  assert.notDeepEqual(wound.geometry.coordinates[0], cw.coordinates[0]);
  const t = gp.truncate({ type: 'Point', coordinates: [1.23456789, 2.3456789] }, { precision: 3 });
  assert.deepEqual(t.geometry.coordinates, [1.235, 2.346]);

  const parcels = fc(
    feature(square(120, 30, 120.1, 30.1), { zone: 'a' }),
    feature(square(120.1, 30, 120.2, 30.1), { zone: 'a' }),
    feature(square(120.3, 30, 120.4, 30.1), { zone: 'b' }),
  );
  const dissolved = gp.dissolve(parcels, { propertyName: 'zone' });
  assert.equal(dissolved.features.length, 2);
  assert.equal(dissolved.features[0].properties.zone, 'a');

  const inside = gp.pointsWithinPolygon(
    fc(feature({ type: 'Point', coordinates: [120.05, 30.05] }), feature({ type: 'Point', coordinates: [130, 30] })),
    square(120, 30, 120.1, 30.1),
  );
  assert.equal(inside.features.length, 1);
});

// ------------------------------------------------------------ predicates

test('DE-9IM matrix and named relations', () => {
  const a = square(0, 0, 2, 2);
  const b = square(0.5, 0.5, 1.5, 1.5);
  const c = square(2, 0, 4, 2);
  const m = gp.relate(a, b);
  assert.equal(m.length, 9);
  assert.ok(gp.booleanContains(a, b));
  assert.ok(gp.booleanWithin(b, a));
  assert.ok(gp.booleanTouches(a, c));
  assert.ok(gp.booleanDisjoint(b, c));
  assert.ok(gp.booleanEqual(a, a));
  assert.ok(!gp.booleanOverlap(a, c));
  assert.ok(gp.booleanOverlap(a, square(1, 1, 3, 3)));
  assert.ok(gp.booleanCrosses({ type: 'LineString', coordinates: [[-1, 1], [3, 1]] }, a));
  assert.ok(gp.relatePattern(a, b, 'T*****FF*'));
  assert.equal(gp.booleanRelation(a, b, 'coveredBy'), false);
  assert.throws(() => gp.booleanRelation(a, b, 'nonsense'), /unknown predicate/);

  const line = { type: 'LineString', coordinates: [[120, 30], [121, 30]] };
  assert.ok(gp.booleanPointOnLine([120.5, 30], line, { tolerance: 0.01 }));
  assert.ok(!gp.booleanPointOnLine([120.5, 30], line, { tolerance: 1, edges: 'geodesic' }));
});

// -------------------------------------------------------------- topology

test('validate finds the usual problems', () => {
  const bowtie = { type: 'Polygon', coordinates: [[[0, 0], [2, 2], [2, 0], [0, 2], [0, 0]]] };
  const report = gp.validate(bowtie);
  assert.equal(report.valid, false);
  assert.ok(report.issues.some((i) => i.code === 'self-intersection'));

  const clockwise = { type: 'Polygon', coordinates: [[[0, 0], [0, 1], [1, 1], [1, 0], [0, 0]]] };
  const warnings = gp.validate(clockwise);
  assert.equal(warnings.valid, true);
  assert.ok(warnings.issues.some((i) => i.code === 'wrong-winding' && i.severity === 'warning'));

  const good = gp.validate(square(0, 0, 1, 1));
  assert.equal(good.valid, true);
  assert.equal(good.issues.length, 0);
});

test('makeValid repairs and reports what it did', () => {
  const bowtie = feature({ type: 'Polygon', coordinates: [[[0, 0], [2, 2], [2, 0], [0, 2], [0, 0]]] }, { id: 7 });
  const fixed = gp.makeValid(bowtie);
  assert.ok(fixed['geoprecise:fixes'].length > 0);
  assert.equal(fixed.properties.id, 7);
  assert.equal(gp.validate(fixed).valid, true);
  assert.equal(fixed.geometry.type, 'MultiPolygon');
  assert.equal(fixed.geometry.coordinates.length, 2);
});

test('coverage issues: overlaps and gaps', () => {
  const parcels = fc(
    feature(square(120, 30, 120.1, 30.1)),
    feature(square(120.099, 30, 120.2, 30.1)),
    feature(square(120, 30.1001, 120.2, 30.2)),
  );
  const report = gp.coverageIssues(parcels, { gapTolerance: 50 });
  assert.equal(report.overlaps.length, 1);
  assert.deepEqual([report.overlaps[0].a, report.overlaps[0].b], [0, 1]);
  assert.ok(report.overlaps[0].areaM2 > 1000);
  assert.equal(report.overlaps[0].geometry.type, 'MultiPolygon');
  assert.equal(report.gaps.length, 1);
  assert.ok(report.gaps[0].areaM2 > 10000);
  assert.ok(report.totalAreaM2 > report.unionAreaM2);
});

test('network issues: dangles, pseudo nodes, crossings, duplicates', () => {
  const lines = fc(
    feature({ type: 'LineString', coordinates: [[0, 0], [0.01, 0]] }),
    feature({ type: 'LineString', coordinates: [[0.01, 0], [0.02, 0]] }),
    feature({ type: 'LineString', coordinates: [[0.005, -0.005], [0.005, 0.005]] }),
  );
  const report = gp.networkIssues(lines);
  assert.equal(report.pseudo_nodes.length, 1);
  assert.ok(report.dangles.length >= 4);
  assert.equal(report.crossings_without_node.length, 1);
  assert.ok(Math.abs(report.crossings_without_node[0].at[0] - 0.005) < 1e-6);
});

test('snapping', () => {
  const line = { type: 'LineString', coordinates: [[120.000001, 30], [120.01, 30.000002]] };
  const snapped = gp.snapRound(line, 1);
  assert.equal(snapped.geometry.coordinates.length, 2);

  const reference = { type: 'LineString', coordinates: [[120, 30], [120.01, 30]] };
  const moved = gp.snapTo(line, reference, 5);
  assert.ok(moved['geoprecise:moved'] >= 1);
  assert.deepEqual(moved.geometry.coordinates[0], [120, 30]);
});

test('CRS options work across the new API', () => {
  const polyGcj = gp.transform(square(120, 30, 120.1, 30.1), 'WGS84', 'GCJ02');
  const prep = gp.prepare(polyGcj, { crs: 'GCJ02' });
  try {
    const centreGcj = gp.centroid(polyGcj, { crs: 'GCJ02' }).geometry.coordinates;
    assert.ok(prep.contains(centreGcj));
    const areaGcj = gp.area(polyGcj, { crs: 'GCJ02' });
    const areaWgs = gp.area(square(120, 30, 120.1, 30.1));
    assert.ok(Math.abs(areaGcj - areaWgs) / areaWgs < 1e-3);
  } finally {
    prep.free();
  }
});
