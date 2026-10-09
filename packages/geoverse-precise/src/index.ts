/**
 * geoverse-precise — ellipsoid-accurate spatial analysis (Rust → WebAssembly).
 *
 * API mirrors turf.js where possible:
 *  - lengths default to kilometers, areas are m²
 *  - points can be given as [lng, lat], Point geometries or Point features
 *
 * Every measuring / constructive function accepts `crs`. Inputs in that CRS are
 * converted to WGS84, computed on the WGS84 ellipsoid, and results are
 * converted back to the same CRS.
 */
import initWasm, * as wasm from '../wasm/geoverse_precise_wasm.js';
import type {
  Feature,
  FeatureCollection,
  GeoJSON,
  GeoJsonProperties,
  Geometry,
  LineString,
  MultiLineString,
  MultiPolygon,
  Point,
  Polygon,
  Position,
} from 'geojson';

// ------------------------------------------------------------------ types

export type Units =
  | 'meters'
  | 'metres'
  | 'millimeters'
  | 'millimetres'
  | 'centimeters'
  | 'centimetres'
  | 'kilometers'
  | 'kilometres'
  | 'miles'
  | 'nauticalmiles'
  | 'inches'
  | 'yards'
  | 'feet';

/** Custom transverse Mercator definition. */
export interface TmercCrs {
  proj: 'tmerc';
  /** Central meridian, degrees. */
  lon0: number;
  lat0?: number;
  k0?: number;
  /** False easting (m). */
  x0?: number;
  /** False northing (m). */
  y0?: number;
  ellps?: 'WGS84' | 'CGCS2000' | 'GRS80';
}

/**
 * `WGS84` / `EPSG:4326`, `CGCS2000` / `EPSG:4490`, `GCJ02`, `BD09`, `EPSG:3857`,
 * `EPSG:326xx` / `EPSG:327xx` (UTM), `EPSG:4491`–`EPSG:4554` (CGCS2000 Gauss-Krüger),
 * or a {@link TmercCrs} object.
 */
export type Crs =
  | 'WGS84'
  | 'EPSG:4326'
  | 'CGCS2000'
  | 'EPSG:4490'
  | 'GCJ02'
  | 'BD09'
  | 'EPSG:3857'
  | `EPSG:${number}`
  | TmercCrs
  | (string & {});

export type PointLike = Position | Point | Feature<Point>;
export type LineLike = LineString | MultiLineString | Feature<LineString | MultiLineString>;
export type PolygonLike = Polygon | MultiPolygon | Feature<Polygon | MultiPolygon>;

export interface CrsOption {
  /** CRS of inputs and outputs. Default WGS84. */
  crs?: Crs;
}
export interface UnitsOption {
  /** Default `kilometers` (same as turf). */
  units?: Units;
}

/**
 * How an edge between two vertices is interpreted.
 * - `planar` (default): straight line in lon/lat — GeoJSON (RFC 7946) and turf semantics,
 *   and what a web map draws for east-west / north-south edges.
 * - `geodesic`: shortest path on the ellipsoid.
 * Only matters for long edges (differences reach ~100 m on a 1° east-west edge at 30°N).
 */
export type EdgeMode = 'planar' | 'geodesic';
export interface EdgesOption {
  edges?: EdgeMode;
}

// ------------------------------------------------------------------- init

let ready: Promise<void> | null = null;
let initialized = false;

export type InitInput = Parameters<typeof initWasm>[0];

const isNode = !!(globalThis as { process?: { versions?: { node?: string } } }).process?.versions?.node;

/**
 * Load the WebAssembly module. Call once before anything else.
 *
 * - Browsers / bundlers: `await init()` (the .wasm is resolved next to the JS glue).
 * - Node: `await init()` reads the file from disk automatically.
 * - Custom: pass a URL, Response, BufferSource or WebAssembly.Module.
 */
export function init(input?: InitInput): Promise<void> {
  if (!ready) {
    ready = (async () => {
      let source = input;
      if (source === undefined && isNode) {
        const fsName = 'node:fs/promises';
        const fs = (await import(/* @vite-ignore */ /* webpackIgnore: true */ fsName)) as {
          readFile(p: URL): Promise<Uint8Array>;
        };
        source = { module_or_path: await fs.readFile(new URL('../wasm/geoverse_precise_wasm_bg.wasm', import.meta.url)) };
      }
      await initWasm(source);
      initialized = true;
    })();
  }
  return ready;
}

/** Synchronous init with already-fetched bytes or a compiled module. */
export function initSync(module: BufferSource | WebAssembly.Module): void {
  wasm.initSync({ module });
  initialized = true;
  ready = Promise.resolve();
}

export function isReady(): boolean {
  return initialized;
}

function ensure(): void {
  if (!initialized) throw new Error('geoverse-precise: call `await init()` before using the library');
}

// The pure-TypeScript helper layer (constructors, iteration, unit conversion)
// lives in its own module and is re-exported from here, so `geoverse-precise` is a
// single import the way `@turf/turf` is.
export * from './helpers.js';
import {
  areaFactors,
  azimuthToBearing,
  bearingToAzimuth,
  clusterEach,
  clusterReduce,
  collectionOf,
  containsNumber,
  convertArea,
  convertLength,
  coordAll,
  coordEach,
  coordReduce,
  degreesToRadians,
  earthRadius,
  factors,
  feature,
  featureCollection,
  featureEach,
  featureReduce,
  featureOf,
  findPoint,
  findSegment,
  flattenEach,
  flattenReduce,
  geojsonType,
  geomEach,
  geomReduce,
  geometry,
  geometryCollection,
  getCluster,
  getCoord,
  getCoords,
  getGeom,
  getType,
  isNumber,
  isObject,
  lengthToDegrees,
  lengthToRadians,
  lineEach,
  lineReduce,
  lineString,
  lineStrings,
  multiLineString,
  multiPoint,
  multiPolygon,
  point,
  points,
  polygon,
  polygons,
  propEach,
  propReduce,
  radiansToDegrees,
  radiansToLength,
  randomLineString,
  randomPoint,
  randomPolygon,
  randomPosition,
  segmentEach,
  segmentReduce,
} from './helpers.js';

// ---------------------------------------------------------------- helpers

function crsStr(c?: Crs): string {
  if (c === undefined || c === null) return '';
  return typeof c === 'string' ? c : JSON.stringify(c);
}

function unitsStr(u?: Units): string {
  return u ?? 'kilometers';
}

function coordOf(p: PointLike): Position {
  if (Array.isArray(p)) return p;
  if (p && (p as Feature).type === 'Feature') {
    const g = (p as Feature<Point>).geometry;
    if (!g || g.type !== 'Point') throw new TypeError('expected a Point feature');
    return g.coordinates;
  }
  if (p && (p as Point).type === 'Point') return (p as Point).coordinates;
  throw new TypeError('expected [lng, lat], a Point or a Point feature');
}

function pointFeature<P extends GeoJsonProperties = GeoJsonProperties>(
  c: ArrayLike<number>,
  properties?: P,
): Feature<Point, P> {
  return {
    type: 'Feature',
    properties: (properties ?? {}) as P,
    geometry: { type: 'Point', coordinates: [c[0], c[1]] },
  };
}

const js = JSON.stringify;

// ---------------------------------------------------------------- measure

/** Geodesic (WGS84 ellipsoid) distance. */
export function distance(from: PointLike, to: PointLike, options: UnitsOption & CrsOption = {}): number {
  ensure();
  const [a, b] = [coordOf(from), coordOf(to)];
  return wasm.distance(a[0], a[1], b[0], b[1], unitsStr(options.units), crsStr(options.crs));
}

/** Initial bearing in degrees (-180, 180]; with `final: true` the arrival bearing in [0, 360). */
export function bearing(start: PointLike, end: PointLike, options: { final?: boolean } & CrsOption = {}): number {
  ensure();
  const [a, b] = [coordOf(start), coordOf(end)];
  return wasm.bearing(a[0], a[1], b[0], b[1], !!options.final, crsStr(options.crs));
}

export function destination<P extends GeoJsonProperties = GeoJsonProperties>(
  origin: PointLike,
  dist: number,
  bearingDeg: number,
  options: UnitsOption & CrsOption & { properties?: P } = {},
): Feature<Point, P> {
  ensure();
  const o = coordOf(origin);
  const r = wasm.destination(o[0], o[1], dist, bearingDeg, unitsStr(options.units), crsStr(options.crs));
  return pointFeature(r, options.properties);
}

/** Geodesic midpoint. */
export function midpoint(a: PointLike, b: PointLike, options: CrsOption = {}): Feature<Point> {
  ensure();
  const [p, q] = [coordOf(a), coordOf(b)];
  return pointFeature(wasm.midpoint(p[0], p[1], q[0], q[1], crsStr(options.crs)));
}

/** Total geodesic length of all lines / rings. */
export function length(geojson: GeoJSON, options: UnitsOption & CrsOption & EdgesOption = {}): number {
  ensure();
  return wasm.length(js(geojson), unitsStr(options.units), crsStr(options.crs), options.edges ?? '');
}

/** Geodesic area in square meters. */
export function area(geojson: GeoJSON, options: CrsOption & EdgesOption = {}): number {
  ensure();
  return wasm.area(js(geojson), crsStr(options.crs), options.edges ?? '');
}

/** Point at a distance along a line. */
export function along(
  line: LineString | Feature<LineString>,
  dist: number,
  options: UnitsOption & CrsOption & EdgesOption = {},
): Feature<Point> {
  ensure();
  return pointFeature(wasm.along(js(line), dist, unitsStr(options.units), crsStr(options.crs), options.edges ?? ''));
}

export interface NearestPointProperties {
  /** Distance from the query point (in `units`). */
  dist: number;
  /** Distance along the line part from its start (in `units`). */
  location: number;
  /** Index of the segment start vertex. */
  index: number;
  /** Index of the LineString within a MultiLineString. */
  multiFeatureIndex: number;
  [k: string]: unknown;
}

/** Closest point on a (multi)line, solved on the ellipsoid (Karney interception). */
export function nearestPointOnLine(
  line: LineLike,
  pt: PointLike,
  options: UnitsOption & CrsOption & EdgesOption = {},
): Feature<Point, NearestPointProperties> {
  ensure();
  const p = coordOf(pt);
  return JSON.parse(
    wasm.nearestPointOnLine(js(line), p[0], p[1], unitsStr(options.units), crsStr(options.crs), options.edges ?? ''),
  );
}

export function pointToLineDistance(
  pt: PointLike,
  line: LineLike,
  options: UnitsOption & CrsOption & EdgesOption = {},
): number {
  ensure();
  const p = coordOf(pt);
  return wasm.pointToLineDistance(p[0], p[1], js(line), unitsStr(options.units), crsStr(options.crs), options.edges ?? '');
}

// ----------------------------------------------------------------- buffer

/** Geodesic circle (`steps` = vertices on the full circle, default 64, like turf.circle). */
export function circle<P extends GeoJsonProperties = GeoJsonProperties>(
  center: PointLike,
  radius: number,
  options: UnitsOption & CrsOption & { steps?: number; properties?: P } = {},
): Feature<Polygon, P> {
  ensure();
  const c = coordOf(center);
  const f = JSON.parse(wasm.circle(c[0], c[1], radius, unitsStr(options.units), options.steps ?? 64, crsStr(options.crs)));
  f.properties = options.properties ?? {};
  return f;
}

export interface BufferOptions extends UnitsOption, CrsOption, EdgesOption {
  /** Segments per quarter circle (turf semantics). Default 16 (64 per circle). */
  steps?: number;
  /**
   * `geodesic` (default): every distance-defining vertex is solved on the ellipsoid.
   * `projected`: local transverse Mercator + planar buffer; faster, for small extents.
   */
  method?: 'geodesic' | 'projected';
  /** Max deviation (m) of output chords from the true offset curves. Default 0.01. */
  tolerance?: number;
}

type BufferResult<T> = T extends FeatureCollection
  ? FeatureCollection<Polygon | MultiPolygon>
  : Feature<Polygon | MultiPolygon> | undefined;

/**
 * Buffer with true metric distance. Negative radius shrinks polygons.
 * Like turf, returns `undefined` for an empty single result and drops empty
 * features from collections.
 */
export function buffer<T extends GeoJSON>(geojson: T, radius: number, options: BufferOptions = {}): BufferResult<T> {
  ensure();
  const opts = js({
    units: unitsStr(options.units),
    steps: options.steps,
    method: options.method,
    edges: options.edges,
    tolerance: options.tolerance,
    crs: options.crs === undefined ? undefined : crsStr(options.crs),
  });
  const out = JSON.parse(wasm.buffer(js(geojson), radius, opts));
  if (out.type === 'FeatureCollection') {
    out.features = out.features.filter((f: Feature) => f.geometry);
    return out;
  }
  return (out.geometry ? out : undefined) as BufferResult<T>;
}

// ---------------------------------------------------------------- overlay

export interface OverlayOptions<P extends GeoJsonProperties = GeoJsonProperties> extends CrsOption, EdgesOption {
  /** Max deviation (m) of long edges in the working plane. Default 0.01. */
  tolerance?: number;
  properties?: P;
}

type OverlayFn = (a: string, b: string, options: string) => string;

function overlayOpts(o: OverlayOptions): string {
  return js({
    edges: o.edges,
    tolerance: o.tolerance,
    crs: o.crs === undefined ? undefined : crsStr(o.crs),
    properties: o.properties,
  });
}

function runOverlay(
  fn: OverlayFn,
  a: PolygonLike | FeatureCollection<Polygon | MultiPolygon>,
  b: PolygonLike | OverlayOptions | undefined,
  options: OverlayOptions | undefined,
): Feature<Polygon | MultiPolygon> | null {
  ensure();
  // turf v7 style: fn(featureCollection, options)
  if ((a as FeatureCollection).type === 'FeatureCollection') {
    const opts = (b as OverlayOptions) ?? {};
    const feats = (a as FeatureCollection<Polygon | MultiPolygon>).features;
    if (feats.length < 2) return null;
    let acc: Feature<Polygon | MultiPolygon> | null = feats[0];
    const s = overlayOpts({ ...opts, properties: undefined });
    for (let i = 1; i < feats.length && acc; i++) {
      acc = JSON.parse(fn(js(acc), js(feats[i]), s));
    }
    if (acc) acc.properties = opts.properties ?? {};
    return acc;
  }
  return JSON.parse(fn(js(a), js(b), overlayOpts(options ?? {})));
}

/** Intersection. `intersect(a, b, opts)` or turf v7 style `intersect(featureCollection, opts)`. */
export function intersect(
  a: PolygonLike | FeatureCollection<Polygon | MultiPolygon>,
  b?: PolygonLike | OverlayOptions,
  options?: OverlayOptions,
): Feature<Polygon | MultiPolygon> | null {
  return runOverlay(wasm.intersect, a, b, options);
}

/** Difference `a − b`. Also accepts a FeatureCollection (first minus the rest). */
export function difference(
  a: PolygonLike | FeatureCollection<Polygon | MultiPolygon>,
  b?: PolygonLike | OverlayOptions,
  options?: OverlayOptions,
): Feature<Polygon | MultiPolygon> | null {
  return runOverlay(wasm.difference, a, b, options);
}

/** Symmetric difference. */
export function xor(a: PolygonLike, b: PolygonLike, options?: OverlayOptions): Feature<Polygon | MultiPolygon> | null {
  return runOverlay(wasm.xor, a, b, options);
}

/** Union. `union(a, b, opts)` or turf v7 style `union(featureCollection, opts)` (single pass). */
export function union(
  a: PolygonLike | FeatureCollection<Polygon | MultiPolygon>,
  b?: PolygonLike | OverlayOptions,
  options?: OverlayOptions,
): Feature<Polygon | MultiPolygon> | null {
  if ((a as FeatureCollection).type === 'FeatureCollection') {
    return unionAll(a as FeatureCollection<Polygon | MultiPolygon>, (b as OverlayOptions) ?? {});
  }
  return runOverlay(wasm.union, a, b, options);
}

/** Union of all polygons of a collection in one pass. */
export function unionAll(
  fc: FeatureCollection<Polygon | MultiPolygon>,
  options: OverlayOptions = {},
): Feature<Polygon | MultiPolygon> | null {
  ensure();
  return JSON.parse(wasm.unionAll(js(fc), overlayOpts(options)));
}

// ------------------------------------------------------------------ batch

/** Accepts either an interleaved Float64Array or an array of positions. */
export type Points = Float64Array | ArrayLike<number> | Position[];

function toFlat(points: Points): Float64Array {
  if (points instanceof Float64Array) return points;
  const arr = points as unknown[];
  if (arr.length > 0 && Array.isArray(arr[0])) {
    const out = new Float64Array(arr.length * 2);
    for (let i = 0; i < arr.length; i++) {
      const p = arr[i] as Position;
      out[2 * i] = p[0];
      out[2 * i + 1] = p[1];
    }
    return out;
  }
  return Float64Array.from(points as ArrayLike<number>);
}

/** Distances for `[x1, y1, x2, y2, …]` pairs, one WASM call for the whole array. */
export function distanceBatch(pairs: Float64Array | ArrayLike<number>, options: UnitsOption & CrsOption = {}): Float64Array {
  ensure();
  return wasm.distanceBatch(Float64Array.from(pairs), unitsStr(options.units), crsStr(options.crs));
}

/** Distance from one origin to many points. */
export function distanceToBatch(origin: PointLike, points: Points, options: UnitsOption & CrsOption = {}): Float64Array {
  ensure();
  const o = coordOf(origin);
  return wasm.distanceToBatch(o[0], o[1], toFlat(points), unitsStr(options.units), crsStr(options.crs));
}

/** Destination points for `[x, y, distance, bearing, …]` rows; returns interleaved `[x, y, …]`. */
export function destinationBatch(rows: Float64Array | ArrayLike<number>, options: UnitsOption & CrsOption = {}): Float64Array {
  ensure();
  return wasm.destinationBatch(Float64Array.from(rows), unitsStr(options.units), crsStr(options.crs));
}

// -------------------------------------------------------- prepared geometry

export interface PrepareOptions extends CrsOption, UnitsOption {}

/**
 * A geometry parsed and indexed once in WASM memory, for repeated point
 * queries. Avoids re-parsing GeoJSON on every call — the difference is two
 * orders of magnitude for bulk workloads.
 *
 * Call {@link Prepared.free} when finished.
 */
export class Prepared {
  /** @internal */
  private handle: InstanceType<typeof wasm.PreparedGeometry> | null;

  constructor(geojson: GeoJSON, options: PrepareOptions = {}) {
    ensure();
    this.handle = new wasm.PreparedGeometry(js(geojson), crsStr(options.crs), unitsStr(options.units));
  }

  private get h(): InstanceType<typeof wasm.PreparedGeometry> {
    if (!this.handle) throw new Error('geoverse-precise: this Prepared geometry has been freed');
    return this.handle;
  }

  /** Number of indexed segments. */
  get segmentCount(): number {
    return this.h.segmentCount();
  }

  /** True when the geometry has area (polygonal). */
  get isAreal(): boolean {
    return this.h.isAreal();
  }

  bbox(): [number, number, number, number] {
    const b = this.h.bbox();
    return [b[0], b[1], b[2], b[3]];
  }

  /** Point in polygon (areal geometries only). */
  contains(pt: PointLike, options: { ignoreBoundary?: boolean } = {}): boolean {
    const p = coordOf(pt);
    return this.h.containsPoint(p[0], p[1], !!options.ignoreBoundary);
  }

  /** One byte per point: 1 = inside. */
  containsMany(points: Points, options: { ignoreBoundary?: boolean } = {}): Uint8Array {
    return this.h.containsPoints(toFlat(points), !!options.ignoreBoundary);
  }

  /** Closest point on the geometry (its boundary, for polygons). */
  nearest(pt: PointLike): Feature<Point, NearestPointProperties> {
    const p = coordOf(pt);
    const r = this.h.nearest(p[0], p[1]);
    return pointFeature(r, {
      dist: r[2],
      location: r[3],
      index: r[4],
      multiFeatureIndex: r[5],
    }) as Feature<Point, NearestPointProperties>;
  }

  /** Six values per query point: `[x, y, dist, location, index, part]`. */
  nearestMany(points: Points): Float64Array {
    return this.h.nearestBatch(toFlat(points));
  }

  /** Distance from a point to the geometry (0 inside an areal geometry). */
  distance(pt: PointLike): number {
    const p = coordOf(pt);
    return this.h.distanceBatch(new Float64Array([p[0], p[1]]))[0];
  }

  distanceMany(points: Points): Float64Array {
    return this.h.distanceBatch(toFlat(points));
  }

  /** Indices of the points within `radius` of the geometry. */
  within(points: Points, radius: number): Uint32Array {
    return this.h.pointsWithin(toFlat(points), radius);
  }

  /** Releases the WASM-side geometry and index. */
  free(): void {
    this.handle?.free();
    this.handle = null;
  }
}

/** Shorthand for `new Prepared(geojson, options)`. */
export function prepare(geojson: GeoJSON, options: PrepareOptions = {}): Prepared {
  return new Prepared(geojson, options);
}

// ------------------------------------------------------------- predicates

/** Point-in-polygon, evaluated in the input coordinates (both must share a CRS). */
export function booleanPointInPolygon(
  pt: PointLike,
  polygon: PolygonLike,
  options: { ignoreBoundary?: boolean } = {},
): boolean {
  ensure();
  const p = coordOf(pt);
  return wasm.booleanPointInPolygon(p[0], p[1], js(polygon), !!options.ignoreBoundary);
}

export function booleanIntersects(a: Geometry | Feature, b: Geometry | Feature): boolean {
  ensure();
  return wasm.booleanIntersects(js(a), js(b));
}

// -------------------------------------------------------------------- CRS

/** Deep-copies `geojson` with all coordinates transformed (properties, ids and Z kept). */
export function transform<T extends GeoJSON>(geojson: T, from: Crs, to: Crs): T {
  ensure();
  return JSON.parse(wasm.transform(js(geojson), crsStr(from), crsStr(to)));
}

/** Transform a single position. */
export function convert(position: Position, from: Crs, to: Crs): Position {
  ensure();
  const buf = new Float64Array([position[0], position[1]]);
  wasm.transformCoords(buf, 2, crsStr(from), crsStr(to));
  return position.length > 2 ? [buf[0], buf[1], ...position.slice(2)] : [buf[0], buf[1]];
}

/**
 * Batch transform of interleaved coordinates (`stride` 2 = xy, 3 = xyz …).
 * A Float64Array is transformed **in place** and returned; other arrays are copied.
 */
export function transformCoords(
  coords: Float64Array | ArrayLike<number>,
  from: Crs,
  to: Crs,
  stride = 2,
): Float64Array {
  ensure();
  const buf = coords instanceof Float64Array ? coords : Float64Array.from(coords);
  wasm.transformCoords(buf, stride, crsStr(from), crsStr(to));
  return buf;
}

export const gcj02ToWgs84 = (p: Position): Position => convert(p, 'GCJ02', 'WGS84');
export const wgs84ToGcj02 = (p: Position): Position => convert(p, 'WGS84', 'GCJ02');
export const bd09ToWgs84 = (p: Position): Position => convert(p, 'BD09', 'WGS84');
export const wgs84ToBd09 = (p: Position): Position => convert(p, 'WGS84', 'BD09');
export const gcj02ToBd09 = (p: Position): Position => convert(p, 'GCJ02', 'BD09');
export const bd09ToGcj02 = (p: Position): Position => convert(p, 'BD09', 'GCJ02');

/** Canonical id of a CRS (throws if unsupported). */
export function normalizeCrs(crs: Crs): string {
  ensure();
  return wasm.normalizeCrs(crsStr(crs));
}

/** CGCS2000 Gauss-Krüger CRS for a longitude, e.g. `gaussKrugerCrs(120.3)` → `EPSG:4549`. */
export function gaussKrugerCrs(lon: number, options: { zoneWidth?: 3 | 6; zonePrefix?: boolean } = {}): string {
  ensure();
  return wasm.gaussKrugerCrs(lon, options.zoneWidth ?? 3, !!options.zonePrefix);
}

/** WGS84 UTM zone for a position, e.g. `EPSG:32650`. */
export function utmCrs(lon: number, lat: number): string {
  ensure();
  return wasm.utmCrs(lon, lat);
}

export function version(): string {
  ensure();
  return wasm.version();
}

// ------------------------------------------------------------ geometry ops

export interface ToleranceOptions extends UnitsOption, CrsOption {
  /** Simplification tolerance, in `units` (default kilometers). */
  tolerance?: number;
  /** Use topology-preserving Visvalingam–Whyatt instead of Douglas–Peucker. */
  preserveTopology?: boolean;
}

function opts(o: object): string {
  const clean: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(o as Record<string, unknown>)) {
    if (v === undefined) continue;
    clean[k] = k === 'crs' ? crsStr(v as Crs) : v;
  }
  return js(clean);
}

/** Simplify with a metric tolerance (default unit: kilometers). */
export function simplify<T extends GeoJSON>(geojson: T, tolerance: number, options: ToleranceOptions = {}): T {
  ensure();
  return JSON.parse(
    wasm.simplify(js(geojson), tolerance, opts({ ...options, units: options.units ?? 'kilometers' })),
  );
}

export function convexHull(geojson: GeoJSON, options: CrsOption & { properties?: GeoJsonProperties } = {}): Feature<Polygon> {
  ensure();
  return JSON.parse(wasm.convexHull(js(geojson), opts(options)));
}

/** Concave hull; `maxEdge` (in `units`) controls how tightly it wraps. */
export function concaveHull(
  geojson: GeoJSON,
  options: UnitsOption & CrsOption & { maxEdge?: number; properties?: GeoJsonProperties } = {},
): Feature<Polygon> {
  ensure();
  return JSON.parse(wasm.concaveHull(js(geojson), opts({ units: options.units ?? 'kilometers', ...options })));
}

/** Mean of all vertices (turf-compatible). */
export function centroid(geojson: GeoJSON, options: CrsOption & { properties?: GeoJsonProperties } = {}): Feature<Point> {
  ensure();
  return JSON.parse(wasm.center(js(geojson), 'centroid', opts(options)));
}

/** Area-weighted centre of mass. */
export function centerOfMass(geojson: GeoJSON, options: CrsOption & { properties?: GeoJsonProperties } = {}): Feature<Point> {
  ensure();
  return JSON.parse(wasm.center(js(geojson), 'centerOfMass', opts(options)));
}

/** A point guaranteed to lie on the feature (inside, for polygons). */
export function pointOnFeature(geojson: GeoJSON, options: CrsOption = {}): Feature<Point> {
  ensure();
  return JSON.parse(wasm.center(js(geojson), 'pointOnFeature', opts(options)));
}

export type BBox4 = [number, number, number, number];

export function bbox(geojson: GeoJSON, options: CrsOption = {}): BBox4 {
  ensure();
  const b = wasm.bbox(js(geojson), opts(options));
  return [b[0], b[1], b[2], b[3]];
}

export function bboxPolygon(b: BBox4, options: CrsOption & { properties?: GeoJsonProperties } = {}): Feature<Polygon> {
  ensure();
  return JSON.parse(wasm.bboxPolygon(Float64Array.from(b), opts(options)));
}

/** Clip to a bounding box (polygons via overlay, lines by segment clipping). */
export function bboxClip<T extends GeoJSON>(geojson: T, b: BBox4, options: CrsOption = {}): T {
  ensure();
  return JSON.parse(wasm.bboxClip(js(geojson), Float64Array.from(b), opts(options)));
}

/** Rotate clockwise by `angle` degrees around `pivot` (default: vertex centroid). */
export function transformRotate<T extends GeoJSON>(
  geojson: T,
  angle: number,
  options: CrsOption & { pivot?: Position } = {},
): T {
  ensure();
  return JSON.parse(wasm.transformGeometry(js(geojson), 'rotate', angle, 0, opts(options)));
}

/** Move every vertex `distance` along `bearing` degrees. */
export function transformTranslate<T extends GeoJSON>(
  geojson: T,
  distance: number,
  bearing: number,
  options: UnitsOption & CrsOption = {},
): T {
  ensure();
  return JSON.parse(
    wasm.transformGeometry(js(geojson), 'translate', distance, bearing, opts({ units: options.units ?? 'kilometers', ...options })),
  );
}

/** Scale distances from `origin` (default: vertex centroid). */
export function transformScale<T extends GeoJSON>(
  geojson: T,
  factor: number,
  options: CrsOption & { origin?: Position } = {},
): T {
  ensure();
  return JSON.parse(wasm.transformGeometry(js(geojson), 'scale', factor, 0, opts(options)));
}

export function lineSliceAlong(
  line: LineString | Feature<LineString>,
  start: number,
  stop: number,
  options: UnitsOption & CrsOption & EdgesOption = {},
): Feature<LineString> {
  ensure();
  return JSON.parse(wasm.lineSliceAlong(js(line), start, stop, opts({ units: options.units ?? 'kilometers', ...options })));
}

export function lineSlice(
  start: PointLike,
  stop: PointLike,
  line: LineString | Feature<LineString>,
  options: CrsOption & EdgesOption = {},
): Feature<LineString> {
  ensure();
  const a = coordOf(start);
  const b = coordOf(stop);
  return JSON.parse(wasm.lineSlice(js(line), a[0], a[1], b[0], b[1], opts(options)));
}

export function lineChunk(
  line: LineString | Feature<LineString>,
  length: number,
  options: UnitsOption & CrsOption & EdgesOption = {},
): FeatureCollection<LineString> {
  ensure();
  return JSON.parse(wasm.lineChunk(js(line), length, opts({ units: options.units ?? 'kilometers', ...options })));
}

export function lineIntersect(a: GeoJSON, b: GeoJSON, options: CrsOption = {}): FeatureCollection<Point> {
  ensure();
  return JSON.parse(wasm.lineIntersect(js(a), js(b), opts(options)));
}

/** Densified geodesic between two points. */
export function greatCircle(
  from: PointLike,
  to: PointLike,
  options: CrsOption & { steps?: number; properties?: GeoJsonProperties } = {},
): Feature<LineString> {
  ensure();
  const a = coordOf(from);
  const b = coordOf(to);
  return JSON.parse(wasm.greatCircle(a[0], a[1], b[0], b[1], opts(options)));
}

export function sector(
  center: PointLike,
  radius: number,
  bearing1: number,
  bearing2: number,
  options: UnitsOption & CrsOption & { steps?: number; properties?: GeoJsonProperties } = {},
): Feature<Polygon> {
  ensure();
  const c = coordOf(center);
  return JSON.parse(
    wasm.sector(c[0], c[1], radius, bearing1, bearing2, opts({ units: options.units ?? 'kilometers', ...options })),
  );
}

/** Closest point of a collection to a target. */
export function nearestPoint(
  target: PointLike,
  points: GeoJSON,
  options: UnitsOption & CrsOption = {},
): Feature<Point, { index: number; distance: number }> {
  ensure();
  const t = coordOf(target);
  const r = wasm.nearestPoint(t[0], t[1], js(points), opts({ units: options.units ?? 'kilometers', ...options }));
  return pointFeature([r[2], r[3]], { index: r[0], distance: r[1] });
}

/** Split multi-part geometries into one feature per part. */
export function flatten(geojson: GeoJSON, options: CrsOption = {}): FeatureCollection {
  ensure();
  return JSON.parse(wasm.reshape(js(geojson), 'flatten', opts(options)));
}

/** Every vertex as a point feature. */
export function explode(geojson: GeoJSON, options: CrsOption = {}): FeatureCollection<Point> {
  ensure();
  return JSON.parse(wasm.reshape(js(geojson), 'explode', opts(options)));
}

export function polygonToLine<T extends GeoJSON>(geojson: T, options: CrsOption = {}): T {
  ensure();
  return JSON.parse(wasm.reshape(js(geojson), 'polygonToLine', opts(options)));
}

export function lineToPolygon<T extends GeoJSON>(geojson: T, options: CrsOption = {}): T {
  ensure();
  return JSON.parse(wasm.reshape(js(geojson), 'lineToPolygon', opts(options)));
}

/** Enforce RFC 7946 ring winding. */
export function rewind<T extends GeoJSON>(geojson: T, options: CrsOption = {}): T {
  ensure();
  return JSON.parse(wasm.reshape(js(geojson), 'rewind', opts(options)));
}

/** Remove repeated (and, with a tolerance, near-collinear) vertices. */
export function cleanCoords<T extends GeoJSON>(geojson: T, options: UnitsOption & CrsOption & { tolerance?: number } = {}): T {
  ensure();
  return JSON.parse(wasm.reshape(js(geojson), 'cleanCoords', opts({ units: options.units ?? 'meters', ...options })));
}

/** Round coordinates to `precision` decimal places (default 6). */
export function truncate<T extends GeoJSON>(geojson: T, options: CrsOption & { precision?: number } = {}): T {
  ensure();
  return JSON.parse(wasm.reshape(js(geojson), 'truncate', opts(options)));
}

/** Union polygons that share a property value. */
export function dissolve(
  fc: FeatureCollection<Polygon | MultiPolygon>,
  options: CrsOption & EdgesOption & { propertyName?: string } = {},
): FeatureCollection<Polygon | MultiPolygon> {
  ensure();
  return JSON.parse(wasm.dissolve(js(fc), options.propertyName ?? '', opts(options)));
}

/** Points of a collection that fall inside a polygon. */
export function pointsWithinPolygon(points: GeoJSON, polygon: PolygonLike, options: CrsOption = {}): FeatureCollection<Point> {
  ensure();
  return JSON.parse(wasm.pointsWithinPolygon(js(points), js(polygon), opts(options)));
}

// -------------------------------------------------------------- predicates

export type SpatialPredicate =
  | 'intersects'
  | 'disjoint'
  | 'contains'
  | 'within'
  | 'covers'
  | 'coveredBy'
  | 'touches'
  | 'crosses'
  | 'overlaps'
  | 'equals';

/** The nine-character DE-9IM matrix, e.g. `"212101212"`. */
export function relate(a: GeoJSON, b: GeoJSON, options: CrsOption & EdgesOption = {}): string {
  ensure();
  return wasm.relate(js(a), js(b), opts(options));
}

/** Test a DE-9IM pattern such as `"T*F**F***"`. */
export function relatePattern(a: GeoJSON, b: GeoJSON, pattern: string, options: CrsOption & EdgesOption = {}): boolean {
  ensure();
  return wasm.relatePattern(js(a), js(b), pattern, opts(options));
}

/** Evaluate a named topological relation. */
export function booleanRelation(
  a: GeoJSON,
  b: GeoJSON,
  predicate: SpatialPredicate,
  options: CrsOption & EdgesOption = {},
): boolean {
  ensure();
  return wasm.booleanRelation(js(a), js(b), predicate, opts(options));
}

export const booleanContains = (a: GeoJSON, b: GeoJSON, o: CrsOption & EdgesOption = {}) => booleanRelation(a, b, 'contains', o);
export const booleanWithin = (a: GeoJSON, b: GeoJSON, o: CrsOption & EdgesOption = {}) => booleanRelation(a, b, 'within', o);
export const booleanCrosses = (a: GeoJSON, b: GeoJSON, o: CrsOption & EdgesOption = {}) => booleanRelation(a, b, 'crosses', o);
export const booleanTouches = (a: GeoJSON, b: GeoJSON, o: CrsOption & EdgesOption = {}) => booleanRelation(a, b, 'touches', o);
export const booleanOverlap = (a: GeoJSON, b: GeoJSON, o: CrsOption & EdgesOption = {}) => booleanRelation(a, b, 'overlaps', o);
export const booleanDisjoint = (a: GeoJSON, b: GeoJSON, o: CrsOption & EdgesOption = {}) => booleanRelation(a, b, 'disjoint', o);
export const booleanEqual = (a: GeoJSON, b: GeoJSON, o: CrsOption & EdgesOption = {}) => booleanRelation(a, b, 'equals', o);

/** Is the point on the line, within `tolerance` (default unit: meters)? */
export function booleanPointOnLine(
  pt: PointLike,
  line: LineLike,
  options: UnitsOption & CrsOption & EdgesOption & { tolerance?: number; ignoreEndVertices?: boolean } = {},
): boolean {
  ensure();
  const p = coordOf(pt);
  return wasm.booleanPointOnLine(
    p[0],
    p[1],
    js(line),
    options.tolerance ?? 0.01,
    !!options.ignoreEndVertices,
    opts({ units: options.units ?? 'meters', ...options }),
  );
}

// ---------------------------------------------------------------- topology

export type IssueSeverity = 'error' | 'warning';

export interface ValidationIssue {
  code: string;
  message: string;
  severity: IssueSeverity;
  feature?: number;
  part?: number;
  ring?: number;
  index?: number;
  at?: [number, number];
}

export interface ValidationReport {
  valid: boolean;
  issues: ValidationIssue[];
}

export interface ValidateOptions extends CrsOption {
  checkSpikes?: boolean;
  checkWinding?: boolean;
  checkAntimeridian?: boolean;
  /** Distance (m) under which consecutive vertices count as duplicates. */
  duplicateTolerance?: number;
}

/** Check geometries against OGC validity and the GeoJSON conventions. */
export function validate(geojson: GeoJSON, options: ValidateOptions = {}): ValidationReport {
  ensure();
  return JSON.parse(wasm.validate(js(geojson), opts(options)));
}

export interface MakeValidOptions extends CrsOption {
  /** Snap coordinates to this grid (m) first. */
  snapGrid?: number;
  /** Drop parts below this area (m²). */
  minArea?: number;
  /** Remove vertices within this distance (m) of the line between their neighbours. */
  cleanTolerance?: number;
}

/** Repair geometries; the result carries a `geoverse-precise:fixes` list. */
export function makeValid<T extends GeoJSON>(geojson: T, options: MakeValidOptions = {}): T & { 'geoverse-precise:fixes': string[] } {
  ensure();
  return JSON.parse(wasm.makeValid(js(geojson), opts(options)));
}

export interface CoverageOptions extends CrsOption, EdgesOption {
  /** Ignore overlaps below this area (m²). Default 0.001. */
  minOverlap?: number;
  /** Ignore gaps below this area (m²). Default 1. */
  minGap?: number;
  /** Ignore gaps above this area (m²) — they are probably intentional. */
  maxGap?: number;
  /**
   * Also find slivers that are open at both ends, by closing the union with
   * this width (m). 0 (default) reports enclosed holes only.
   */
  gapTolerance?: number;
}

export interface CoverageReport {
  overlaps: { a: number; b: number; areaM2: number; geometry: Geometry }[];
  gaps: { areaM2: number; geometry: Geometry }[];
  unionAreaM2: number;
  totalAreaM2: number;
}

/** Overlaps and gaps in a set of polygons that should tile an area. */
export function coverageIssues(fc: GeoJSON, options: CoverageOptions = {}): CoverageReport {
  ensure();
  return JSON.parse(wasm.coverageIssues(js(fc), opts(options)));
}

export interface NetworkOptions extends CrsOption, EdgesOption, UnitsOption {
  /** Endpoints closer than this count as connected. Default 0.01 m. */
  tolerance?: number;
  /** Report endpoints that stop within this distance of another line. */
  maxUndershoot?: number;
  /** Report dangling stubs shorter than this past the last junction. */
  maxOvershoot?: number;
}

export interface NetworkReport {
  dangles: { at: [number, number]; degree: number; lines: number[] }[];
  pseudo_nodes: { at: [number, number]; degree: number; lines: number[] }[];
  self_intersections: { at: [number, number]; lines: number[] }[];
  crossings_without_node: { at: [number, number]; lines: number[] }[];
  duplicates: [number, number][];
  undershoots: { at: [number, number]; lines: number[]; distance_m?: number }[];
  overshoots: { at: [number, number]; lines: number[]; distance_m?: number }[];
}

/** Dangles, pseudo nodes, crossings, duplicates, under- and overshoots. */
export function networkIssues(fc: GeoJSON, options: NetworkOptions = {}): NetworkReport {
  ensure();
  return JSON.parse(wasm.networkIssues(js(fc), opts({ units: options.units ?? 'meters', ...options })));
}

/** Round coordinates onto a metric grid. */
export function snapRound<T extends GeoJSON>(geojson: T, grid: number, options: UnitsOption & CrsOption = {}): T {
  ensure();
  return JSON.parse(wasm.snapRound(js(geojson), grid, opts({ units: options.units ?? 'meters', ...options })));
}

/** Snap vertices onto a reference layer; result carries `geoverse-precise:moved`. */
export function snapTo<T extends GeoJSON>(
  geojson: T,
  reference: GeoJSON,
  tolerance: number,
  options: UnitsOption & CrsOption = {},
): T & { 'geoverse-precise:moved': number } {
  ensure();
  return JSON.parse(
    wasm.snapTo(js(geojson), js(reference), tolerance, opts({ units: options.units ?? 'meters', ...options })),
  );
}

// ================================================================ rhumb lines

/** Rhumb-line (constant bearing) distance — longer than the geodesic. */
export function rhumbDistance(from: PointLike, to: PointLike, options: UnitsOption & CrsOption = {}): number {
  ensure();
  const [a, b] = [coordOf(from), coordOf(to)];
  return wasm.rhumbDistance(a[0], a[1], b[0], b[1], unitsStr(options.units), crsStr(options.crs));
}

/** The single bearing a rhumb line from `start` to `end` holds throughout. */
export function rhumbBearing(start: PointLike, end: PointLike, options: { final?: boolean } & CrsOption = {}): number {
  ensure();
  const [a, b] = [coordOf(start), coordOf(end)];
  return wasm.rhumbBearing(a[0], a[1], b[0], b[1], !!options.final, crsStr(options.crs));
}

/** Where a constant bearing takes you after `distance`. */
export function rhumbDestination<P extends GeoJsonProperties = GeoJsonProperties>(
  origin: PointLike,
  distance: number,
  bearing: number,
  options: UnitsOption & CrsOption & { properties?: P } = {},
): Feature<Point, P> {
  ensure();
  const o = coordOf(origin);
  const p = wasm.rhumbDestination(o[0], o[1], distance, bearing, unitsStr(options.units), crsStr(options.crs));
  return pointFeature(p, options.properties);
}

// ================================================================ line tools

/** Every two-position segment of the input lines. */
export function lineSegment(geojson: GeoJSON, options: CrsOption = {}): FeatureCollection<LineString> {
  ensure();
  return JSON.parse(wasm.lineSegment(js(geojson), opts(options)));
}

/** Split a line wherever `splitter` crosses or touches it. */
export function lineSplit(line: LineLike, splitter: GeoJSON, options: CrsOption = {}): FeatureCollection<LineString> {
  ensure();
  return JSON.parse(wasm.lineSplit(js(line), js(splitter), opts(options)));
}

export interface OffsetOptions extends UnitsOption, CrsOption, EdgesOption {
  /** Vertices used per 360° of arc on convex corners. Default 32. */
  arcSteps?: number;
}

/**
 * Offset a line sideways by a metric distance. Positive is left of travel.
 *
 * Convex corners get a rounded joint, so every vertex there is exactly
 * `distance` away. Concave corners keep both perpendicular feet without
 * mitring, which puts those vertices slightly *inside* the nominal offset —
 * the same trade turf makes, and it avoids a spike on a sharp corner.
 */
export function lineOffset(line: LineLike, distance: number, options: OffsetOptions = {}): Feature<LineString> {
  ensure();
  return JSON.parse(wasm.lineOffset(js(line), distance, opts(options)));
}

/** The stretches two lines share, within `tolerance` (default unit: kilometers). */
export function lineOverlap(
  line1: GeoJSON,
  line2: GeoJSON,
  options: UnitsOption & CrsOption & { tolerance?: number } = {},
): FeatureCollection<LineString> {
  ensure();
  return JSON.parse(wasm.lineOverlap(js(line1), js(line2), opts(options)));
}

/** Which of `points` lies closest to `line`; `dist` is added to its properties. */
export function nearestPointToLine(
  points: GeoJSON,
  line: LineLike,
  options: UnitsOption & CrsOption & EdgesOption = {},
): Feature<Point> {
  ensure();
  return JSON.parse(wasm.nearestPointToLine(js(points), js(line), opts(options)));
}

/** Distance from a point to a polygon: negative when the point is inside. */
export function pointToPolygonDistance(
  point: PointLike,
  polygon: PolygonLike,
  options: UnitsOption & CrsOption & EdgesOption = {},
): number {
  ensure();
  const p = coordOf(point);
  return wasm.pointToPolygonDistance(p[0], p[1], js(polygon), opts(options));
}

/** Angle at `mid` between `mid→start` and `mid→end`, in degrees. */
export function angle(
  start: PointLike,
  mid: PointLike,
  end: PointLike,
  options: CrsOption & { explementary?: boolean } = {},
): number {
  ensure();
  const [a, b, c] = [coordOf(start), coordOf(mid), coordOf(end)];
  return wasm.angle(a[0], a[1], b[0], b[1], c[0], c[1], opts(options));
}

// ===================================================================== grids

export interface GridOptions extends UnitsOption, CrsOption {
  /** Keep only the cells touching this geometry. */
  mask?: PolygonLike;
  properties?: GeoJsonProperties;
}

function gridCall(
  b: BBox4,
  gridType: string,
  cellWidth: number,
  cellHeight: number,
  options: GridOptions,
): FeatureCollection {
  ensure();
  const { mask: maskGeom, ...rest } = options;
  return JSON.parse(
    wasm.grid(
      Float64Array.from(b),
      opts({ ...rest, gridType, cellWidth, cellHeight, mask: maskGeom ? js(maskGeom) : undefined }),
    ),
  );
}

/**
 * A lattice of points spaced `cellSide` apart.
 *
 * The spacing is metric: cells are laid out in a local transverse Mercator
 * plane, so a 1 km grid has 1 km sides at 60°N as well as at the equator —
 * turf's degree-based spacing stretches east-west as you leave it.
 */
export function pointGrid(b: BBox4, cellSide: number, options: GridOptions = {}): FeatureCollection<Point> {
  return gridCall(b, 'point', cellSide, cellSide, options) as FeatureCollection<Point>;
}

export function squareGrid(b: BBox4, cellSide: number, options: GridOptions = {}): FeatureCollection<Polygon> {
  return gridCall(b, 'square', cellSide, cellSide, options) as FeatureCollection<Polygon>;
}

export function rectangleGrid(
  b: BBox4,
  cellWidth: number,
  cellHeight: number,
  options: GridOptions = {},
): FeatureCollection<Polygon> {
  return gridCall(b, 'rectangle', cellWidth, cellHeight, options) as FeatureCollection<Polygon>;
}

export function triangleGrid(b: BBox4, cellSide: number, options: GridOptions = {}): FeatureCollection<Polygon> {
  return gridCall(b, 'triangle', cellSide, cellSide, options) as FeatureCollection<Polygon>;
}

/** Flat-top hexagons of side `cellSide`. */
export function hexGrid(b: BBox4, cellSide: number, options: GridOptions = {}): FeatureCollection<Polygon> {
  return gridCall(b, 'hex', cellSide, cellSide, options) as FeatureCollection<Polygon>;
}

/** Pad the shorter side of a bbox until it is square. */
export function square(b: BBox4): BBox4 {
  ensure();
  const out = wasm.squareBbox(Float64Array.from(b));
  return [out[0], out[1], out[2], out[3]];
}

/** The bounding-box polygon of a geometry. */
export function envelope(geojson: GeoJSON, options: CrsOption & { properties?: GeoJsonProperties } = {}): Feature<Polygon> {
  ensure();
  return JSON.parse(wasm.envelope(js(geojson), opts(options)));
}

// ==================================================================== shapes

export interface EllipseOptions extends UnitsOption, CrsOption {
  /** Vertices around the ellipse. Default 64. */
  steps?: number;
  /** Rotation in degrees, positive clockwise. */
  angle?: number;
  properties?: GeoJsonProperties;
}

/** An ellipse with metric semi-axes (x is east-west before rotation). */
export function ellipse(
  center: PointLike,
  xSemiAxis: number,
  ySemiAxis: number,
  options: EllipseOptions = {},
): Feature<Polygon> {
  ensure();
  const c = coordOf(center);
  return JSON.parse(wasm.ellipse(c[0], c[1], xSemiAxis, ySemiAxis, opts(options)));
}

/** Chaikin corner cutting, applied in the local metric plane. */
export function polygonSmooth<T extends GeoJSON>(
  geojson: T,
  options: CrsOption & { iterations?: number } = {},
): FeatureCollection<Polygon | MultiPolygon> {
  ensure();
  return JSON.parse(wasm.polygonSmooth(js(geojson), opts(options)));
}

/** The two points of a polygon's outline visible at its silhouette from `pt`. */
export function polygonTangents(pt: PointLike, polygon: PolygonLike, options: CrsOption = {}): FeatureCollection<Point> {
  ensure();
  const p = coordOf(pt);
  return JSON.parse(wasm.polygonTangents(p[0], p[1], js(polygon), opts(options)));
}

/** `maskPolygon` with `polygon` punched out of it. */
export function mask(
  polygon: PolygonLike,
  maskPolygon: PolygonLike,
  options: CrsOption & { properties?: GeoJsonProperties } = {},
): Feature<Polygon | MultiPolygon> {
  ensure();
  return JSON.parse(wasm.mask(js(polygon), js(maskPolygon), opts(options)));
}

/** Does the polygon have a reflex corner? */
export function booleanConcave(polygon: PolygonLike, options: CrsOption = {}): boolean {
  ensure();
  return wasm.booleanConcave(js(polygon), opts(options));
}

/** Are two lines parallel, corner for corner, within `tolerance` degrees? */
export function booleanParallel(
  line1: LineLike,
  line2: LineLike,
  options: CrsOption & { tolerance?: number } = {},
): boolean {
  ensure();
  return wasm.booleanParallel(js(line1), js(line2), opts(options));
}

/** Swap x and y in every position — for data that arrived as [lat, lng]. */
export function flip<T extends GeoJSON>(geojson: T, options: CrsOption = {}): FeatureCollection {
  ensure();
  return JSON.parse(wasm.flip(js(geojson), opts(options)));
}

export interface CentreOptions extends CrsOption {
  /** Property holding each feature's weight. */
  weight?: string;
  properties?: GeoJsonProperties;
}

/** Weighted mean centre of a point set. */
export function centerMean(geojson: GeoJSON, options: CentreOptions = {}): Feature<Point> {
  ensure();
  const { weight, ...rest } = options;
  return JSON.parse(wasm.centerMean(js(geojson), opts({ ...rest, weightProperty: weight })));
}

/**
 * Weighted median centre: the point with the least total distance to the set.
 *
 * Solved by Weiszfeld iteration in the local plane, so it minimises real
 * distance rather than degree distance.
 */
export function centerMedian(
  geojson: GeoJSON,
  options: CentreOptions & UnitsOption & { tolerance?: number } = {},
): Feature<Point> {
  ensure();
  const { weight, ...rest } = options;
  return JSON.parse(wasm.centerMedian(js(geojson), opts({ ...rest, weightProperty: weight })));
}

/** Catmull-Rom spline through the line's vertices. */
export function bezierSpline(
  line: LineLike,
  options: CrsOption & { sharpness?: number; steps?: number } = {},
): Feature<LineString> {
  ensure();
  return JSON.parse(wasm.bezierSpline(js(line), opts(options)));
}

/** Build polygons from a set of lines that enclose faces. */
export function polygonize(geojson: GeoJSON, options: CrsOption = {}): FeatureCollection<Polygon> {
  ensure();
  return JSON.parse(wasm.polygonize(js(geojson), opts(options)));
}

// ============================================================= interpolation

export interface InterpolateOptions extends UnitsOption, CrsOption {
  /** Output cell shape. Default `square`. */
  gridType?: 'point' | 'square' | 'rectangle' | 'triangle' | 'hex';
  /** Property holding each sample's value. Default: `elevation`, then `z`, then `value`. */
  property?: string;
  /** Exponent on 1/distance (turf calls this `weight`). Default 1. */
  weight?: number;
  /** Ignore samples beyond this distance, in `units`. */
  searchRadius?: number;
  mask?: PolygonLike;
  bbox?: BBox4;
}

/** Inverse-distance weighting onto a metric grid. */
export function interpolate(points: GeoJSON, cellSize: number, options: InterpolateOptions = {}): FeatureCollection {
  ensure();
  const { property, weight, mask: maskGeom, ...rest } = options;
  return JSON.parse(
    wasm.interpolate(
      js(points),
      opts({
        ...rest,
        cellWidth: cellSize,
        cellHeight: cellSize,
        gridType: options.gridType ?? 'square',
        zProperty: property,
        power: weight,
        mask: maskGeom ? js(maskGeom) : undefined,
      }),
    ),
  );
}

export interface ContourOptions extends CrsOption {
  /** Property holding each sample's value. Default `elevation`. */
  zProperty?: string;
  properties?: GeoJsonProperties;
}

/**
 * Contour lines through a point lattice.
 *
 * Levels are traced across lattice triangles, where the interpolant is linear
 * and so has a single unambiguous crossing — the saddle cases that a
 * marching-squares table has to guess at never arise.
 */
export function isolines(
  pointGridFc: GeoJSON,
  breaks: number[],
  options: ContourOptions = {},
): FeatureCollection<MultiLineString> {
  ensure();
  return JSON.parse(wasm.isolines(js(pointGridFc), Float64Array.from(breaks), opts(options)));
}

/** Filled bands between consecutive breaks; they tile the domain exactly. */
export function isobands(
  pointGridFc: GeoJSON,
  breaks: number[],
  options: ContourOptions = {},
): FeatureCollection<MultiPolygon> {
  ensure();
  return JSON.parse(wasm.isobands(js(pointGridFc), Float64Array.from(breaks), opts(options)));
}

/**
 * Delaunay triangulation, with each triangle carrying its corners' values as
 * `a`, `b` and `c`. Triangulated in the local metric plane, so the shapes are
 * well formed on the ground.
 */
export function tin(points: GeoJSON, z?: string, options: CrsOption = {}): FeatureCollection<Polygon> {
  ensure();
  return JSON.parse(wasm.tin(js(points), opts({ ...options, zProperty: z })));
}

/** Voronoi cells, clipped to `bbox` (default: the points' padded extent). */
export function voronoi(points: GeoJSON, options: CrsOption & { bbox?: BBox4 } = {}): FeatureCollection<Polygon> {
  ensure();
  return JSON.parse(wasm.voronoi(js(points), opts(options)));
}

/** Interpolate a value inside a triangle carrying `a`, `b`, `c` properties. */
export function planepoint(point: PointLike, triangle: Feature<Polygon> | Polygon, options: CrsOption = {}): number {
  ensure();
  const p = coordOf(point);
  return wasm.planepoint(p[0], p[1], js(triangle), opts(options));
}

// ================================================================ clustering

export interface DbscanOptions extends UnitsOption, CrsOption {
  /** Points needed within `maxDistance` (counting the point itself) to be core. Default 3. */
  minPoints?: number;
}

export type DbscanRole = 'core' | 'edge' | 'noise';

/** Density clustering; each point gets `dbscan` and, unless noise, `cluster`. */
export function clustersDbscan(
  points: GeoJSON,
  maxDistance: number,
  options: DbscanOptions = {},
): FeatureCollection<Point, { dbscan: DbscanRole; cluster?: number } & GeoJsonProperties> {
  ensure();
  return JSON.parse(wasm.clustersDbscan(js(points), maxDistance, opts(options)));
}

/** k-means clustering; each point gets `cluster` and its `centroid`. */
export function clustersKmeans(
  points: GeoJSON,
  options: CrsOption & { numberOfClusters?: number } = {},
): FeatureCollection<Point, { cluster: number; centroid: Position } & GeoJsonProperties> {
  ensure();
  return JSON.parse(wasm.clustersKmeans(js(points), opts(options)));
}

export interface NnaResult {
  units: string;
  arealUnits: string;
  numberOfPoints: number;
  observedMeanDistance: number;
  expectedMeanDistance: number;
  studyAreaSize: number;
  nearestNeighborIndex: number;
  zScore: number;
}

/**
 * Nearest-neighbour index: below 1 is clustered, above 1 dispersed.
 *
 * Returned on the study area feature, as turf does, under
 * `nearestNeighborAnalysis`.
 */
export function nearestNeighborAnalysis(
  points: GeoJSON,
  options: UnitsOption & CrsOption & { studyArea?: PolygonLike } = {},
): Feature<Polygon | MultiPolygon, { nearestNeighborAnalysis: NnaResult }> {
  ensure();
  const { studyArea, ...rest } = options;
  return JSON.parse(wasm.nearestNeighborAnalysis(js(points), opts({ ...rest, studyArea: studyArea ? js(studyArea) : undefined })));
}

export interface SdeResult {
  numberOfFeatures: number;
  meanCenterCoordinates: Position;
  semiMajorAxis: number;
  semiMinorAxis: number;
  /** turf's θ: the clockwise rotation applied to the ellipse. */
  rotation: number;
  /** Bearing of the long axis in [0°, 180°) — unambiguous, unlike θ. */
  majorAxisBearing: number;
  numberOfFeaturesContained: number;
  percentageOfFeaturesContained: number;
}

/** Standard deviational ellipse, fitted in metres rather than degrees. */
export function standardDeviationalEllipse(
  points: GeoJSON,
  options: UnitsOption & CrsOption & { weight?: string; steps?: number } = {},
): Feature<Polygon, { standardDeviationalEllipse: SdeResult }> {
  ensure();
  const { weight, ...rest } = options;
  return JSON.parse(wasm.standardDeviationalEllipse(js(points), opts({ ...rest, weightProperty: weight })));
}

export interface DirectionalMeanResult {
  countOfLines: number;
  bearingAngle: number;
  cartesianAngle: number;
  /** 0 when every line points the same way, 1 when they cancel out. */
  circularVariance: number;
  averageLength: number;
  totalLength: number;
}

/** Mean direction of a set of lines, from geodesic start-to-end azimuths. */
export function directionalMean(lines: GeoJSON, options: UnitsOption & CrsOption = {}): DirectionalMeanResult {
  ensure();
  return JSON.parse(wasm.directionalMean(js(lines), opts(options)));
}

export interface PathOptions extends UnitsOption, CrsOption {
  obstacles?: GeoJSON;
  /** Lattice spacing in `units`. Default 1 km. */
  resolution?: number;
  /** Extra room around the extent, in `units`. */
  padding?: number;
  properties?: GeoJsonProperties;
}

/**
 * Shortest route from `start` to `end` around `obstacles`.
 *
 * A* over a metric lattice, then pulled taut: with nothing in the way the
 * result is the straight line, not a staircase.
 */
export function shortestPath(start: PointLike, end: PointLike, options: PathOptions = {}): Feature<LineString> {
  ensure();
  const [a, b] = [coordOf(start), coordOf(end)];
  const { obstacles, ...rest } = options;
  return JSON.parse(
    wasm.shortestPath(a[0], a[1], b[0], b[1], opts({ ...rest, obstacles: obstacles ? js(obstacles) : undefined })),
  );
}

// ================================================== turf-compatible aliases

/** Every point where a line or ring crosses itself. */
export function kinks(geojson: GeoJSON, options: CrsOption = {}): FeatureCollection<Point> {
  ensure();
  return JSON.parse(wasm.kinks(js(geojson), opts(options)));
}

/** Split self-intersecting polygons into valid pieces. */
export function unkinkPolygon(geojson: GeoJSON, options: CrsOption = {}): FeatureCollection<Polygon> {
  ensure();
  return JSON.parse(wasm.unkinkPolygon(js(geojson), opts(options)));
}

/** Project to Web Mercator (EPSG:3857) metres. */
export function toMercator<T extends GeoJSON>(geojson: T): T {
  ensure();
  return JSON.parse(wasm.toMercator(js(geojson)));
}

/** Back from Web Mercator metres to lon/lat. */
export function toWgs84<T extends GeoJSON>(geojson: T): T {
  ensure();
  return JSON.parse(wasm.toWgs84(js(geojson)));
}

/** Gather a property from the points inside each polygon into an array. */
export function collect(
  polygons: GeoJSON,
  points: GeoJSON,
  inProperty: string,
  outProperty: string,
  options: CrsOption = {},
): FeatureCollection {
  ensure();
  return JSON.parse(wasm.collect(js(polygons), js(points), inProperty, outProperty, opts(options)));
}

/** Copy a polygon's property onto the points that fall inside it. */
export function tag(
  points: GeoJSON,
  polygons: GeoJSON,
  field: string,
  outField: string,
  options: CrsOption = {},
): FeatureCollection<Point> {
  ensure();
  return JSON.parse(wasm.tag(js(points), js(polygons), field, outField, opts(options)));
}

// =========================================== arcs, tesselation, statistics

/** The arc of a circle between two bearings, as a line. */
export function lineArc(
  center: PointLike,
  radius: number,
  bearing1: number,
  bearing2: number,
  options: UnitsOption & CrsOption & { steps?: number; properties?: GeoJsonProperties } = {},
): Feature<LineString> {
  ensure();
  const c = coordOf(center);
  return JSON.parse(wasm.lineArc(c[0], c[1], radius, bearing1, bearing2, opts(options)));
}

/** Triangulate a polygon, holes included. */
export function tesselate(polygon: PolygonLike, options: CrsOption = {}): FeatureCollection<Polygon> {
  ensure();
  return JSON.parse(wasm.tesselate(js(polygon), opts(options)));
}

export interface WeightOptions extends UnitsOption, CrsOption {
  /** Neighbourhood radius in `units`. Default 10 km. */
  threshold?: number;
  /** Distance decay exponent used when `binary` is false. Default -1. */
  alpha?: number;
  /** Every neighbour counts 1. Default true. */
  binary?: boolean;
  /** `row` (default) scales each row to sum to 1; `raw` leaves them alone. */
  standardization?: 'row' | 'raw';
}

/** The spatial weight matrix over a point set. */
export function distanceWeight(points: GeoJSON, options: WeightOptions = {}): number[][] {
  ensure();
  return JSON.parse(wasm.distanceWeight(js(points), opts(options)));
}

export interface MoranResult {
  moranIndex: number;
  expectedMoranIndex: number;
  varianceMoranIndex: number;
  zNorm: number;
  pNorm: number;
}

/**
 * Moran's I: positive when like values cluster, negative when they alternate.
 *
 * The neighbourhood is a geodesic radius, so the same `threshold` means the same
 * ground distance wherever the data sits.
 */
export function moranIndex(points: GeoJSON, options: WeightOptions & { zProperty?: string } = {}): MoranResult {
  ensure();
  return JSON.parse(wasm.moranIndex(js(points), opts(options)));
}

export interface QuadratResult {
  quadrats: number;
  xQuadrats: number;
  yQuadrats: number;
  numberOfPoints: number;
  counts: number[];
  expected: number;
  /** 1 under complete spatial randomness, above 1 clustered, below 1 dispersed. */
  varianceMeanRatio: number;
  chiSquared: number;
  degreesOfFreedom: number;
  criticalValue: number;
  /** χ² under the 95% critical value: randomness is not rejected. */
  isRandom: boolean;
}

/** Quadrat count test for complete spatial randomness, on equal-area quadrats. */
export function quadratAnalysis(
  points: GeoJSON,
  options: CrsOption & { studyArea?: PolygonLike; xQuadrats?: number; yQuadrats?: number } = {},
): QuadratResult {
  ensure();
  const { studyArea, ...rest } = options;
  return JSON.parse(wasm.quadratAnalysis(js(points), opts({ ...rest, studyArea: studyArea ? js(studyArea) : undefined })));
}

// ================================================= turf naming compatibility

/** The centre of a geometry's bounding box (turf's `center`). */
export function center(geojson: GeoJSON, options: CrsOption & { properties?: GeoJsonProperties } = {}): Feature<Point> {
  const [x0, y0, x1, y1] = bbox(geojson, options);
  return pointFeature([(x0 + x1) / 2, (y0 + y1) / 2], options.properties ?? {});
}

/** Alias of {@link convexHull} (turf's `convex`). */
export const convex = convexHull;

/** Alias of {@link concaveHull} (turf's `concave`). */
export const concave = concaveHull;

/** Is the geometry valid? The full report is available from {@link validate}. */
export function booleanValid(geojson: GeoJSON, options: CrsOption & EdgesOption = {}): boolean {
  return validate(geojson, options).valid;
}

/** turf's `@turf/meta` namespace, for `meta.coordEach(...)` style calls. */
export const meta = {
  coordEach,
  coordReduce,
  coordAll,
  propEach,
  propReduce,
  featureEach,
  featureReduce,
  geomEach,
  geomReduce,
  flattenEach,
  flattenReduce,
  segmentEach,
  segmentReduce,
  lineEach,
  lineReduce,
  findSegment,
  findPoint,
};

/** turf's `@turf/helpers` namespace. */
export const helpers = {
  feature,
  featureCollection,
  geometry,
  geometryCollection,
  point,
  points,
  lineString,
  lineStrings,
  polygon,
  polygons,
  multiPoint,
  multiLineString,
  multiPolygon,
  earthRadius,
  factors,
  areaFactors,
  radiansToLength,
  lengthToRadians,
  lengthToDegrees,
  radiansToDegrees,
  degreesToRadians,
  bearingToAzimuth,
  azimuthToBearing,
  convertLength,
  convertArea,
  isNumber,
  isObject,
};

/** turf's `@turf/invariant` namespace. */
export const invariant = {
  getCoord,
  getCoords,
  getGeom,
  getType,
  geojsonType,
  featureOf,
  collectionOf,
  containsNumber,
};

/** turf's `@turf/projection` namespace. */
export const projection = { toMercator, toWgs84 };

/** turf's `@turf/random` namespace. */
export const random = { randomPosition, randomPoint, randomLineString, randomPolygon };

/** turf's `@turf/clusters` namespace. */
export const clusters = { getCluster, clusterEach, clusterReduce };
