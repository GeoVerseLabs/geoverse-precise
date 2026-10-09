/**
 * The turf helper layer: constructors, iteration, unit conversion and the small
 * utilities that turf code reaches for on every line.
 *
 * None of this touches WebAssembly — it is plain GeoJSON bookkeeping, so it
 * works before `init()` has resolved and costs nothing to call.
 *
 * Signatures and callback argument order follow `@turf/helpers`, `@turf/meta`
 * and `@turf/invariant` so existing code ports across unchanged.
 */
import type {
  BBox,
  Feature,
  FeatureCollection,
  GeoJSON,
  GeoJsonProperties,
  Geometry,
  GeometryCollection,
  LineString,
  MultiLineString,
  MultiPoint,
  MultiPolygon,
  Point,
  Polygon,
  Position,
} from 'geojson';

export type Id = string | number;

export interface FeatureOptions {
  bbox?: BBox;
  id?: Id;
}

// =========================================================== constructors

/** Wrap a geometry in a Feature. */
export function feature<G extends Geometry, P extends GeoJsonProperties = GeoJsonProperties>(
  geom: G | null,
  properties?: P,
  options: FeatureOptions = {},
): Feature<G, P> {
  const f = { type: 'Feature' as const, properties: (properties ?? {}) as P, geometry: geom as G };
  if (options.id !== undefined) (f as Feature<G, P>).id = options.id;
  if (options.bbox) (f as Feature<G, P>).bbox = options.bbox;
  return f;
}

/** A bare geometry object of the given type. */
export function geometry(
  type: 'Point' | 'LineString' | 'Polygon' | 'MultiPoint' | 'MultiLineString' | 'MultiPolygon',
  coordinates: unknown,
): Geometry {
  switch (type) {
    case 'Point':
      return { type, coordinates: coordinates as Position };
    case 'LineString':
    case 'MultiPoint':
      return { type, coordinates: coordinates as Position[] };
    case 'Polygon':
    case 'MultiLineString':
      return { type, coordinates: coordinates as Position[][] };
    case 'MultiPolygon':
      return { type, coordinates: coordinates as Position[][][] };
    default:
      throw new Error(`unknown geometry type ${type as string}`);
  }
}

export function point<P extends GeoJsonProperties = GeoJsonProperties>(
  coordinates: Position,
  properties?: P,
  options: FeatureOptions = {},
): Feature<Point, P> {
  if (!Array.isArray(coordinates)) throw new TypeError('coordinates must be an array');
  if (coordinates.length < 2) throw new Error('coordinates must be at least 2 numbers long');
  if (!isNumber(coordinates[0]) || !isNumber(coordinates[1])) throw new Error('coordinates must contain numbers');
  return feature({ type: 'Point', coordinates }, properties, options);
}

export function points<P extends GeoJsonProperties = GeoJsonProperties>(
  coordinates: Position[],
  properties?: P,
  options: FeatureOptions = {},
): FeatureCollection<Point, P> {
  return featureCollection(
    coordinates.map((c) => point(c, properties)),
    options,
  );
}

export function lineString<P extends GeoJsonProperties = GeoJsonProperties>(
  coordinates: Position[],
  properties?: P,
  options: FeatureOptions = {},
): Feature<LineString, P> {
  if (coordinates.length < 2) throw new Error('coordinates must be an array of two or more positions');
  return feature({ type: 'LineString', coordinates }, properties, options);
}

export function lineStrings<P extends GeoJsonProperties = GeoJsonProperties>(
  coordinates: Position[][],
  properties?: P,
  options: FeatureOptions = {},
): FeatureCollection<LineString, P> {
  return featureCollection(
    coordinates.map((c) => lineString(c, properties)),
    options,
  );
}

/** Rings are closed for you if the last position does not repeat the first. */
export function polygon<P extends GeoJsonProperties = GeoJsonProperties>(
  coordinates: Position[][],
  properties?: P,
  options: FeatureOptions = {},
): Feature<Polygon, P> {
  for (const ring of coordinates) {
    if (ring.length < 4) throw new Error('each LinearRing needs four or more positions');
    const first = ring[0];
    const last = ring[ring.length - 1];
    if (first[0] !== last[0] || first[1] !== last[1]) ring.push([...first]);
  }
  return feature({ type: 'Polygon', coordinates }, properties, options);
}

export function polygons<P extends GeoJsonProperties = GeoJsonProperties>(
  coordinates: Position[][][],
  properties?: P,
  options: FeatureOptions = {},
): FeatureCollection<Polygon, P> {
  return featureCollection(
    coordinates.map((c) => polygon(c, properties)),
    options,
  );
}

export function multiPoint<P extends GeoJsonProperties = GeoJsonProperties>(
  coordinates: Position[],
  properties?: P,
  options: FeatureOptions = {},
): Feature<MultiPoint, P> {
  return feature({ type: 'MultiPoint', coordinates }, properties, options);
}

export function multiLineString<P extends GeoJsonProperties = GeoJsonProperties>(
  coordinates: Position[][],
  properties?: P,
  options: FeatureOptions = {},
): Feature<MultiLineString, P> {
  return feature({ type: 'MultiLineString', coordinates }, properties, options);
}

export function multiPolygon<P extends GeoJsonProperties = GeoJsonProperties>(
  coordinates: Position[][][],
  properties?: P,
  options: FeatureOptions = {},
): Feature<MultiPolygon, P> {
  return feature({ type: 'MultiPolygon', coordinates }, properties, options);
}

export function geometryCollection<P extends GeoJsonProperties = GeoJsonProperties>(
  geometries: Geometry[],
  properties?: P,
  options: FeatureOptions = {},
): Feature<GeometryCollection, P> {
  return feature({ type: 'GeometryCollection', geometries }, properties, options);
}

export function featureCollection<G extends Geometry, P extends GeoJsonProperties = GeoJsonProperties>(
  features: Array<Feature<G, P>>,
  options: FeatureOptions = {},
): FeatureCollection<G, P> {
  const fc: FeatureCollection<G, P> = { type: 'FeatureCollection', features };
  if (options.id !== undefined) (fc as { id?: Id }).id = options.id;
  if (options.bbox) fc.bbox = options.bbox;
  return fc;
}

// ======================================================== unit conversion

/**
 * turf's spherical earth radius, kept for compatibility with the converters
 * below. The library's own measurements use the WGS84 ellipsoid and never this
 * number.
 */
export const earthRadius = 6371008.8;

/** Multiplier from radians of arc to a length unit, on turf's sphere. */
export const factors: Record<string, number> = {
  centimeters: earthRadius * 100,
  centimetres: earthRadius * 100,
  degrees: 360 / (2 * Math.PI),
  feet: earthRadius * 3.28084,
  inches: earthRadius * 39.37,
  kilometers: earthRadius / 1000,
  kilometres: earthRadius / 1000,
  meters: earthRadius,
  metres: earthRadius,
  miles: earthRadius / 1609.344,
  millimeters: earthRadius * 1000,
  millimetres: earthRadius * 1000,
  nauticalmiles: earthRadius / 1852,
  radians: 1,
  yards: earthRadius / 0.9144,
};

/** Square metres per unit of area. */
export const areaFactors: Record<string, number> = {
  acres: 0.000247105,
  centimeters: 10000,
  centimetres: 10000,
  feet: 10.763910417,
  hectares: 0.0001,
  inches: 1550.003100006,
  kilometers: 0.000001,
  kilometres: 0.000001,
  meters: 1,
  metres: 1,
  miles: 3.86e-7,
  nauticalmiles: 2.9155334959812285e-7,
  millimeters: 1000000,
  millimetres: 1000000,
  yards: 1.195990046,
};

function factorOf(units: string): number {
  const f = factors[units];
  if (f === undefined) throw new Error(`${units} is not a supported unit`);
  return f;
}

/**
 * Arc length for an angle, on a sphere of turf's radius.
 *
 * Inherently spherical: an angle does not determine a length on an ellipsoid.
 * Prefer `distance()` when you want a real measurement.
 */
export function radiansToLength(radians: number, units = 'kilometers'): number {
  return radians * factorOf(units);
}

/** The inverse of {@link radiansToLength}. */
export function lengthToRadians(distance: number, units = 'kilometers'): number {
  return distance / factorOf(units);
}

/** A length as degrees of arc, on turf's sphere. */
export function lengthToDegrees(distance: number, units?: string): number {
  return radiansToDegrees(lengthToRadians(distance, units));
}

export function radiansToDegrees(radians: number): number {
  const r = radians % (2 * Math.PI);
  return (r * 180) / Math.PI;
}

export function degreesToRadians(degrees: number): number {
  const d = degrees % 360;
  return (d * Math.PI) / 180;
}

/** Bearing in (-180, 180] to an azimuth in [0, 360). */
export function bearingToAzimuth(bearing: number): number {
  let angle = bearing % 360;
  if (angle < 0) angle += 360;
  return angle;
}

/** Azimuth in [0, 360) to a bearing in (-180, 180]. */
export function azimuthToBearing(azimuth: number): number {
  const a = azimuth % 360;
  if (a > 180) return a - 360;
  if (a <= -180) return a + 360;
  return a;
}

/** Exact unit conversion — no earth model involved. */
export function convertLength(length: number, from = 'kilometers', to = 'kilometers'): number {
  if (!(length >= 0)) throw new Error('length must be a positive number');
  return radiansToLength(lengthToRadians(length, from), to);
}

/** Exact area conversion. */
export function convertArea(area: number, from = 'meters', to = 'kilometers'): number {
  if (!(area >= 0)) throw new Error('area must be a positive number');
  const f = areaFactors[from];
  const t = areaFactors[to];
  if (f === undefined) throw new Error(`invalid original units: ${from}`);
  if (t === undefined) throw new Error(`invalid final units: ${to}`);
  return (area / f) * t;
}

export function isNumber(n: unknown): boolean {
  return !isNaN(Number(n)) && n !== null && !Array.isArray(n) && typeof n !== 'boolean';
}

export function isObject(input: unknown): boolean {
  return input !== null && typeof input === 'object' && !Array.isArray(input);
}

/** Round to `precision` decimals (turf's `round`). */
export function round(num: number, precision = 0): number {
  if (precision < 0 || !Number.isInteger(precision)) throw new Error('precision must be a positive integer');
  const m = Math.pow(10, precision);
  return Math.round(num * m) / m;
}

// ============================================================== invariant

/** The position of a Point, however it was handed in. */
export function getCoord(coord: Feature<Point> | Point | Position): Position {
  if (Array.isArray(coord) && isNumber(coord[0]) && isNumber(coord[1])) return coord as Position;
  if (isObject(coord)) {
    const c = coord as Feature<Point> | Point;
    if (c.type === 'Feature' && (c as Feature<Point>).geometry?.type === 'Point') {
      return [...(c as Feature<Point>).geometry.coordinates];
    }
    if (c.type === 'Point') return [...(c as Point).coordinates];
  }
  throw new Error('coord must be GeoJSON Point or an Array of numbers');
}

/** The coordinates of any geometry, feature or raw coordinate array. */
export function getCoords(coords: unknown): unknown {
  if (Array.isArray(coords)) return coords;
  if (isObject(coords)) {
    const c = coords as Feature | Geometry;
    if (c.type === 'Feature') {
      const g = (c as Feature).geometry;
      if (g && 'coordinates' in g) return (g as { coordinates: unknown }).coordinates;
    } else if ('coordinates' in c) {
      return (c as { coordinates: unknown }).coordinates;
    }
  }
  throw new Error('coords must be GeoJSON Feature, Geometry Object or an Array');
}

export function getGeom<G extends Geometry>(geojson: Feature<G> | G): G | null {
  return (geojson as Feature<G>).type === 'Feature' ? (geojson as Feature<G>).geometry : (geojson as G);
}

export function getType(geojson: GeoJSON, _name = 'geojson'): string {
  const type = (geojson as { type: string }).type;
  if (type === 'FeatureCollection' || type === 'GeometryCollection') return type;
  if (type === 'Feature') {
    const g = (geojson as Feature).geometry;
    if (g) return g.type;
  }
  return type;
}

/** Throw unless the GeoJSON has the expected type. */
export function geojsonType(value: GeoJSON, type: string, name: string): void {
  if (!type || !name) throw new Error('type and name are required');
  if (!value || (value as Geometry).type !== type) {
    throw new Error(`Invalid input to ${name}: must be a ${type}, given ${(value as Geometry)?.type}`);
  }
}

/** Throw unless the Feature carries a geometry of the expected type. */
export function featureOf(f: Feature, type: string, name: string): void {
  if (!f) throw new Error('No feature passed');
  if (!name) throw new Error('.featureOf() requires a name');
  if (!f || f.type !== 'Feature' || !f.geometry) {
    throw new Error(`Invalid input to ${name}, Feature with geometry required`);
  }
  if (!f.geometry || f.geometry.type !== type) {
    throw new Error(`Invalid input to ${name}: must be a ${type}, given ${f.geometry.type}`);
  }
}

/** Throw unless every feature of the collection has the expected type. */
export function collectionOf(fc: FeatureCollection, type: string, name: string): void {
  if (!fc) throw new Error('No featureCollection passed');
  if (!name) throw new Error('.collectionOf() requires a name');
  if (fc.type !== 'FeatureCollection') throw new Error(`Invalid input to ${name}, FeatureCollection required`);
  for (const f of fc.features) {
    if (!f || f.type !== 'Feature' || !f.geometry) {
      throw new Error(`Invalid input to ${name}, Feature with geometry required`);
    }
    if (f.geometry.type !== type) throw new Error(`Invalid input to ${name}: must be a ${type}, given ${f.geometry.type}`);
  }
}

export function containsNumber(coordinates: unknown): boolean {
  const c = coordinates as unknown[];
  if (c.length > 1 && isNumber(c[0]) && isNumber(c[1])) return true;
  if (Array.isArray(c[0]) && c[0].length) return containsNumber(c[0]);
  throw new Error('coordinates must only contain numbers');
}

// ==================================================================== meta

type CoordCallback<T> = (
  currentCoord: Position,
  coordIndex: number,
  featureIndex: number,
  multiFeatureIndex: number,
  geometryIndex: number,
) => T;

/** Visit every position. With `excludeWrapCoord` a ring's closing position is skipped. */
export function coordEach(geojson: GeoJSON, callback: CoordCallback<void | false>, excludeWrapCoord = false): void {
  if (geojson === null) return;
  let coordIndex = 0;
  const isFC = geojson.type === 'FeatureCollection';
  const isF = geojson.type === 'Feature';
  const stop = isFC ? (geojson as FeatureCollection).features.length : 1;

  for (let featureIndex = 0; featureIndex < stop; featureIndex++) {
    const geometryMaybeCollection: Geometry | null = isFC
      ? ((geojson as FeatureCollection).features[featureIndex].geometry as Geometry)
      : isF
        ? ((geojson as Feature).geometry as Geometry)
        : (geojson as Geometry);
    if (!geometryMaybeCollection) continue;
    const isGeomCollection = geometryMaybeCollection.type === 'GeometryCollection';
    const stopG = isGeomCollection ? (geometryMaybeCollection as GeometryCollection).geometries.length : 1;

    for (let geomIndex = 0; geomIndex < stopG; geomIndex++) {
      let multiFeatureIndex = 0;
      let geometryIndex = 0;
      const geometry_ = isGeomCollection
        ? (geometryMaybeCollection as GeometryCollection).geometries[geomIndex]
        : geometryMaybeCollection;
      if (!geometry_) continue;
      const geomType = geometry_.type;
      const wrapShrink = excludeWrapCoord && (geomType === 'Polygon' || geomType === 'MultiPolygon') ? 1 : 0;

      switch (geomType) {
        case 'Point': {
          if (callback((geometry_ as Point).coordinates, coordIndex, featureIndex, multiFeatureIndex, geometryIndex) === false) return;
          coordIndex++;
          multiFeatureIndex++;
          break;
        }
        case 'LineString':
        case 'MultiPoint': {
          const coords = (geometry_ as LineString).coordinates;
          for (let j = 0; j < coords.length; j++) {
            if (callback(coords[j], coordIndex, featureIndex, multiFeatureIndex, geometryIndex) === false) return;
            coordIndex++;
            if (geomType === 'MultiPoint') multiFeatureIndex++;
          }
          if (geomType === 'LineString') multiFeatureIndex++;
          break;
        }
        case 'Polygon':
        case 'MultiLineString': {
          const coords = (geometry_ as Polygon).coordinates;
          for (let j = 0; j < coords.length; j++) {
            for (let k = 0; k < coords[j].length - wrapShrink; k++) {
              if (callback(coords[j][k], coordIndex, featureIndex, multiFeatureIndex, geometryIndex) === false) return;
              coordIndex++;
            }
            if (geomType === 'MultiLineString') multiFeatureIndex++;
            if (geomType === 'Polygon') geometryIndex++;
          }
          if (geomType === 'Polygon') multiFeatureIndex++;
          break;
        }
        case 'MultiPolygon': {
          const coords = (geometry_ as MultiPolygon).coordinates;
          for (let j = 0; j < coords.length; j++) {
            geometryIndex = 0;
            for (let k = 0; k < coords[j].length; k++) {
              for (let l = 0; l < coords[j][k].length - wrapShrink; l++) {
                if (callback(coords[j][k][l], coordIndex, featureIndex, multiFeatureIndex, geometryIndex) === false) return;
                coordIndex++;
              }
              geometryIndex++;
            }
            multiFeatureIndex++;
          }
          break;
        }
        case 'GeometryCollection': {
          for (const g of (geometry_ as GeometryCollection).geometries) {
            coordEach(g, callback, excludeWrapCoord);
          }
          break;
        }
        default:
          throw new Error('Unknown Geometry Type');
      }
    }
  }
}

export function coordReduce<T>(
  geojson: GeoJSON,
  callback: (
    previousValue: T,
    currentCoord: Position,
    coordIndex: number,
    featureIndex: number,
    multiFeatureIndex: number,
    geometryIndex: number,
  ) => T,
  initialValue?: T,
  excludeWrapCoord = false,
): T {
  let previous = initialValue as T;
  coordEach(
    geojson,
    (coord, coordIndex, featureIndex, multiFeatureIndex, geometryIndex) => {
      previous =
        coordIndex === 0 && initialValue === undefined
          ? (coord as unknown as T)
          : callback(previous, coord, coordIndex, featureIndex, multiFeatureIndex, geometryIndex);
    },
    excludeWrapCoord,
  );
  return previous;
}

export function propEach(
  geojson: GeoJSON,
  callback: (currentProperties: GeoJsonProperties, featureIndex: number) => void | false,
): void {
  if (geojson.type === 'FeatureCollection') {
    const fc = geojson as FeatureCollection;
    for (let i = 0; i < fc.features.length; i++) {
      if (callback(fc.features[i].properties, i) === false) break;
    }
  } else if (geojson.type === 'Feature') {
    callback((geojson as Feature).properties, 0);
  }
}

export function propReduce<T>(
  geojson: GeoJSON,
  callback: (previousValue: T, currentProperties: GeoJsonProperties, featureIndex: number) => T,
  initialValue?: T,
): T {
  let previous = initialValue as T;
  propEach(geojson, (props, i) => {
    previous = i === 0 && initialValue === undefined ? (props as unknown as T) : callback(previous, props, i);
  });
  return previous;
}

export function featureEach<G extends Geometry, P extends GeoJsonProperties = GeoJsonProperties>(
  geojson: GeoJSON,
  callback: (currentFeature: Feature<G, P>, featureIndex: number) => void | false,
): void {
  if (geojson.type === 'Feature') {
    callback(geojson as Feature<G, P>, 0);
  } else if (geojson.type === 'FeatureCollection') {
    const fc = geojson as FeatureCollection<G, P>;
    for (let i = 0; i < fc.features.length; i++) {
      if (callback(fc.features[i], i) === false) break;
    }
  }
}

export function featureReduce<T>(
  geojson: GeoJSON,
  callback: (previousValue: T, currentFeature: Feature, featureIndex: number) => T,
  initialValue?: T,
): T {
  let previous = initialValue as T;
  featureEach(geojson, (f, i) => {
    previous = i === 0 && initialValue === undefined ? (f as unknown as T) : callback(previous, f as Feature, i);
  });
  return previous;
}

/** Every position, flat. */
export function coordAll(geojson: GeoJSON): Position[] {
  const out: Position[] = [];
  coordEach(geojson, (coord) => {
    out.push(coord);
  });
  return out;
}

export function geomEach(
  geojson: GeoJSON,
  callback: (
    currentGeometry: Geometry | null,
    featureIndex: number,
    featureProperties: GeoJsonProperties,
    featureBBox?: BBox,
    featureId?: Id,
  ) => void | false,
): void {
  let featureIndex = 0;
  const isFC = geojson.type === 'FeatureCollection';
  const isF = geojson.type === 'Feature';
  const stop = isFC ? (geojson as FeatureCollection).features.length : 1;

  for (let i = 0; i < stop; i++) {
    const holder: Feature | null = isFC
      ? (geojson as FeatureCollection).features[i]
      : isF
        ? (geojson as Feature)
        : null;
    const geometryMaybeCollection: Geometry | null = holder ? holder.geometry : (geojson as Geometry);
    const featureProperties = holder ? holder.properties : {};
    const featureBBox = holder ? holder.bbox : (geojson as Feature).bbox;
    const featureId = holder ? holder.id : (geojson as Feature).id;

    const isGeomCollection = geometryMaybeCollection?.type === 'GeometryCollection';
    const stopG = isGeomCollection ? (geometryMaybeCollection as GeometryCollection).geometries.length : 1;
    for (let g = 0; g < stopG; g++) {
      const geometry_ = isGeomCollection
        ? (geometryMaybeCollection as GeometryCollection).geometries[g]
        : geometryMaybeCollection;
      if (geometry_ === null) {
        if (callback(null, featureIndex, featureProperties, featureBBox, featureId) === false) return;
        continue;
      }
      switch (geometry_.type) {
        case 'Point':
        case 'LineString':
        case 'MultiPoint':
        case 'Polygon':
        case 'MultiLineString':
        case 'MultiPolygon': {
          if (callback(geometry_, featureIndex, featureProperties, featureBBox, featureId) === false) return;
          break;
        }
        case 'GeometryCollection': {
          for (const inner of (geometry_ as GeometryCollection).geometries) {
            if (callback(inner, featureIndex, featureProperties, featureBBox, featureId) === false) return;
          }
          break;
        }
        default:
          throw new Error('Unknown Geometry Type');
      }
    }
    featureIndex++;
  }
}

export function geomReduce<T>(
  geojson: GeoJSON,
  callback: (
    previousValue: T,
    currentGeometry: Geometry | null,
    featureIndex: number,
    featureProperties: GeoJsonProperties,
    featureBBox?: BBox,
    featureId?: Id,
  ) => T,
  initialValue?: T,
): T {
  let previous = initialValue as T;
  let first = true;
  geomEach(geojson, (geom, featureIndex, props, bbox, id) => {
    if (first && initialValue === undefined) {
      previous = geom as unknown as T;
      first = false;
    } else {
      previous = callback(previous, geom, featureIndex, props, bbox, id);
    }
  });
  return previous;
}

/** Visit each single-part feature, splitting multi geometries apart. */
export function flattenEach(
  geojson: GeoJSON,
  callback: (currentFeature: Feature, featureIndex: number, multiFeatureIndex: number) => void | false,
): void {
  let stop = false;
  geomEach(geojson, (geometry_, featureIndex, properties, bbox, id) => {
    if (stop) return false;
    if (geometry_ === null) {
      return callback(feature(null as unknown as Geometry, properties, { bbox, id }), featureIndex, 0);
    }
    switch (geometry_.type) {
      case 'Point':
      case 'LineString':
      case 'Polygon': {
        if (callback(feature(geometry_, properties, { bbox, id }), featureIndex, 0) === false) stop = true;
        return stop ? false : undefined;
      }
      default:
        break;
    }
    const type = geometry_.type.replace('Multi', '') as 'Point' | 'LineString' | 'Polygon';
    const coords = (geometry_ as MultiPoint).coordinates as unknown as unknown[];
    for (let i = 0; i < coords.length; i++) {
      const geom = { type, coordinates: coords[i] } as Geometry;
      if (callback(feature(geom, properties), featureIndex, i) === false) {
        stop = true;
        return false;
      }
    }
    return undefined;
  });
}

export function flattenReduce<T>(
  geojson: GeoJSON,
  callback: (previousValue: T, currentFeature: Feature, featureIndex: number, multiFeatureIndex: number) => T,
  initialValue?: T,
): T {
  let previous = initialValue as T;
  let first = true;
  flattenEach(geojson, (f, featureIndex, multiFeatureIndex) => {
    if (first && initialValue === undefined) {
      previous = f as unknown as T;
      first = false;
    } else {
      previous = callback(previous, f, featureIndex, multiFeatureIndex);
    }
  });
  return previous;
}

/** Visit every two-position segment. */
export function segmentEach(
  geojson: GeoJSON,
  callback: (
    currentSegment: Feature<LineString>,
    featureIndex: number,
    multiFeatureIndex: number,
    geometryIndex: number,
    segmentIndex: number,
  ) => void | false,
): void {
  let stop = false;
  flattenEach(geojson, (f, featureIndex, multiFeatureIndex) => {
    if (stop) return false;
    let segmentIndex = 0;
    if (!f.geometry) return undefined;
    const type = f.geometry.type;
    if (type !== 'LineString' && type !== 'Polygon') return undefined;

    const rings: Position[][] =
      type === 'LineString' ? [(f.geometry as LineString).coordinates] : (f.geometry as Polygon).coordinates;
    for (let geometryIndex = 0; geometryIndex < rings.length; geometryIndex++) {
      const coords = rings[geometryIndex];
      for (let i = 0; i < coords.length - 1; i++) {
        const seg = lineString([coords[i], coords[i + 1]], f.properties);
        if (callback(seg, featureIndex, multiFeatureIndex, geometryIndex, segmentIndex) === false) {
          stop = true;
          return false;
        }
        segmentIndex++;
      }
    }
    return undefined;
  });
}

export function segmentReduce<T>(
  geojson: GeoJSON,
  callback: (
    previousValue: T,
    currentSegment: Feature<LineString>,
    featureIndex: number,
    multiFeatureIndex: number,
    geometryIndex: number,
    segmentIndex: number,
  ) => T,
  initialValue?: T,
): T {
  let previous = initialValue as T;
  let first = true;
  segmentEach(geojson, (seg, fi, mi, gi, si) => {
    if (first && initialValue === undefined) {
      previous = seg as unknown as T;
      first = false;
    } else {
      previous = callback(previous, seg, fi, mi, gi, si);
    }
  });
  return previous;
}

/** Visit each LineString, including each ring of a polygon. */
export function lineEach(
  geojson: GeoJSON,
  callback: (
    currentLine: Feature<LineString>,
    featureIndex: number,
    multiFeatureIndex: number,
    geometryIndex: number,
  ) => void | false,
): void {
  let stop = false;
  flattenEach(geojson, (f, featureIndex, multiFeatureIndex) => {
    if (stop || !f.geometry) return undefined;
    const type = f.geometry.type;
    if (type === 'LineString') {
      if (callback(f as Feature<LineString>, featureIndex, multiFeatureIndex, 0) === false) {
        stop = true;
        return false;
      }
    } else if (type === 'Polygon') {
      const rings = (f.geometry as Polygon).coordinates;
      for (let geometryIndex = 0; geometryIndex < rings.length; geometryIndex++) {
        if (callback(lineString(rings[geometryIndex], f.properties), featureIndex, multiFeatureIndex, geometryIndex) === false) {
          stop = true;
          return false;
        }
      }
    }
    return undefined;
  });
}

export function lineReduce<T>(
  geojson: GeoJSON,
  callback: (
    previousValue: T,
    currentLine: Feature<LineString>,
    featureIndex: number,
    multiFeatureIndex: number,
    geometryIndex: number,
  ) => T,
  initialValue?: T,
): T {
  let previous = initialValue as T;
  let first = true;
  lineEach(geojson, (line, fi, mi, gi) => {
    if (first && initialValue === undefined) {
      previous = line as unknown as T;
      first = false;
    } else {
      previous = callback(previous, line, fi, mi, gi);
    }
  });
  return previous;
}

export interface SegmentLocation {
  featureIndex?: number;
  multiFeatureIndex?: number;
  geometryIndex?: number;
  segmentIndex?: number;
  properties?: GeoJsonProperties;
}

/** The segment at a given location; negative indexes count from the end. */
export function findSegment(geojson: GeoJSON, options: SegmentLocation = {}): Feature<LineString> | null {
  let found: Feature<LineString> | null = null;
  const want = {
    featureIndex: options.featureIndex ?? 0,
    multiFeatureIndex: options.multiFeatureIndex ?? 0,
    geometryIndex: options.geometryIndex ?? 0,
    segmentIndex: options.segmentIndex ?? 0,
  };
  // negative indexes need the totals, so collect and pick afterwards
  const all: Array<[Feature<LineString>, number, number, number, number]> = [];
  segmentEach(geojson, (seg, fi, mi, gi, si) => {
    all.push([seg, fi, mi, gi, si]);
  });
  const resolve = (v: number, total: number) => (v < 0 ? total + v : v);
  const maxSeg = all.reduce((m, r) => Math.max(m, r[4]), -1) + 1;
  const maxFeat = all.reduce((m, r) => Math.max(m, r[1]), -1) + 1;
  const maxMulti = all.reduce((m, r) => Math.max(m, r[2]), -1) + 1;
  const maxGeom = all.reduce((m, r) => Math.max(m, r[3]), -1) + 1;
  for (const [seg, fi, mi, gi, si] of all) {
    if (
      fi === resolve(want.featureIndex, maxFeat) &&
      mi === resolve(want.multiFeatureIndex, maxMulti) &&
      gi === resolve(want.geometryIndex, maxGeom) &&
      si === resolve(want.segmentIndex, maxSeg)
    ) {
      found = options.properties ? lineString(seg.geometry.coordinates, options.properties) : seg;
      break;
    }
  }
  return found;
}

export interface PointLocation {
  featureIndex?: number;
  multiFeatureIndex?: number;
  geometryIndex?: number;
  coordIndex?: number;
  properties?: GeoJsonProperties;
}

/** The position at a given location, as a Point feature. */
export function findPoint(geojson: GeoJSON, options: PointLocation = {}): Feature<Point> | null {
  const seg = findSegment(geojson, {
    featureIndex: options.featureIndex,
    multiFeatureIndex: options.multiFeatureIndex,
    geometryIndex: options.geometryIndex,
    segmentIndex: options.coordIndex,
  });
  if (seg) return point(seg.geometry.coordinates[0], options.properties ?? seg.properties);
  // past the last segment: take the final position
  const coords = coordAll(geojson);
  if (coords.length === 0) return null;
  return point(coords[coords.length - 1], options.properties);
}

// ================================================================ clusters

/** Features whose properties match every key/value in `filter`. */
export function getCluster<G extends Geometry, P extends GeoJsonProperties = GeoJsonProperties>(
  geojson: FeatureCollection<G, P>,
  filter: GeoJsonProperties | string | number,
): FeatureCollection<G, P> {
  const matches = (props: GeoJsonProperties): boolean => {
    if (props === null) return false;
    if (typeof filter === 'string' || typeof filter === 'number') return filter in (props as object);
    for (const [k, v] of Object.entries(filter ?? {})) {
      if ((props as Record<string, unknown>)[k] !== v) return false;
    }
    return true;
  };
  return featureCollection(geojson.features.filter((f) => matches(f.properties)), {});
}

/** Visit each distinct value of `property`, with the features carrying it. */
export function clusterEach<G extends Geometry, P extends GeoJsonProperties = GeoJsonProperties>(
  geojson: FeatureCollection<G, P>,
  property: string,
  callback: (cluster: FeatureCollection<G, P>, clusterValue: unknown, currentIndex: number) => void,
): void {
  if (!property) throw new Error('property is required');
  const values: unknown[] = [];
  for (const f of geojson.features) {
    const v = (f.properties as Record<string, unknown> | null)?.[property];
    if (v !== undefined && !values.includes(v)) values.push(v);
  }
  values.forEach((v, i) => {
    const cluster = featureCollection(
      geojson.features.filter((f) => (f.properties as Record<string, unknown> | null)?.[property] === v),
      {},
    );
    callback(cluster, v, i);
  });
}

export function clusterReduce<T, G extends Geometry, P extends GeoJsonProperties = GeoJsonProperties>(
  geojson: FeatureCollection<G, P>,
  property: string,
  callback: (previousValue: T, cluster: FeatureCollection<G, P>, clusterValue: unknown, currentIndex: number) => T,
  initialValue?: T,
): T {
  let previous = initialValue as T;
  clusterEach(geojson, property, (cluster, value, i) => {
    previous = i === 0 && initialValue === undefined ? (cluster as unknown as T) : callback(previous, cluster, value, i);
  });
  return previous;
}

// ================================================================ utilities

/** A deep copy that keeps the GeoJSON shape (turf's `clone`). */
export function clone<T extends GeoJSON>(geojson: T): T {
  return structuredClone ? structuredClone(geojson) : (JSON.parse(JSON.stringify(geojson)) as T);
}

/** `num` features drawn without replacement (turf's `sample`). */
export function sample<G extends Geometry, P extends GeoJsonProperties = GeoJsonProperties>(
  fc: FeatureCollection<G, P>,
  num: number,
): FeatureCollection<G, P> {
  if (!fc) throw new Error('fc is required');
  if (num === undefined || num === null) throw new Error('num is required');
  if (!isNumber(num)) throw new Error('num must be a number');
  const pool = fc.features.slice();
  const out: Array<Feature<G, P>> = [];
  const take = Math.min(num, pool.length);
  for (let i = 0; i < take; i++) {
    out.push(pool.splice(Math.floor(Math.random() * pool.length), 1)[0]);
  }
  return featureCollection(out, {});
}

/** Merge single-part features into the matching multi geometries (turf's `combine`). */
export function combine(fc: FeatureCollection): FeatureCollection {
  const groups: Record<string, unknown[]> = { MultiPoint: [], MultiLineString: [], MultiPolygon: [] };
  for (const f of fc.features) {
    const g = f.geometry;
    if (!g) continue;
    switch (g.type) {
      case 'Point':
        groups.MultiPoint.push((g as Point).coordinates);
        break;
      case 'MultiPoint':
        groups.MultiPoint.push(...(g as MultiPoint).coordinates);
        break;
      case 'LineString':
        groups.MultiLineString.push((g as LineString).coordinates);
        break;
      case 'MultiLineString':
        groups.MultiLineString.push(...(g as MultiLineString).coordinates);
        break;
      case 'Polygon':
        groups.MultiPolygon.push((g as Polygon).coordinates);
        break;
      case 'MultiPolygon':
        groups.MultiPolygon.push(...(g as MultiPolygon).coordinates);
        break;
      default:
        break;
    }
  }
  const out: Feature[] = [];
  for (const [type, coords] of Object.entries(groups)) {
    if (coords.length > 0) {
      out.push(feature({ type, coordinates: coords } as unknown as Geometry, {}));
    }
  }
  return featureCollection(out, {});
}

/**
 * Is the ring wound clockwise?
 *
 * Judged by the sign of the shoelace sum in lon/lat, the same convention turf
 * uses — note that RFC 7946 asks for counter-clockwise exteriors.
 */
export function booleanClockwise(line: Feature<LineString> | LineString | Position[]): boolean {
  const ring = (Array.isArray(line) ? line : (getCoords(line) as Position[])) as Position[];
  let sum = 0;
  for (let i = 0; i < ring.length - 1; i++) {
    sum += (ring[i + 1][0] - ring[i][0]) * (ring[i + 1][1] + ring[i][1]);
  }
  return sum > 0;
}

export function booleanCounterClockwise(line: Feature<LineString> | LineString | Position[]): boolean {
  return !booleanClockwise(line);
}

// ================================================================== random

function randomPosition(bbox?: BBox): Position {
  const b = (bbox ?? [-180, -90, 180, 90]) as number[];
  return [Math.random() * (b[2] - b[0]) + b[0], Math.random() * (b[3] - b[1]) + b[1]];
}

export { randomPosition };

export function randomPoint(count = 1, options: { bbox?: BBox } = {}): FeatureCollection<Point> {
  const out: Array<Feature<Point>> = [];
  for (let i = 0; i < count; i++) out.push(point(randomPosition(options.bbox)));
  return featureCollection(out, {});
}

export function randomLineString(
  count = 1,
  options: { bbox?: BBox; num_vertices?: number; max_length?: number; max_rotation?: number } = {},
): FeatureCollection<LineString> {
  const num = Math.max(2, options.num_vertices ?? 10);
  const maxLength = options.max_length ?? 0.0001;
  const maxRotation = options.max_rotation ?? Math.PI / 8;
  const out: Array<Feature<LineString>> = [];
  for (let i = 0; i < count; i++) {
    const start = randomPosition(options.bbox);
    const coords: Position[] = [start];
    for (let j = 0; j < num - 1; j++) {
      const priorAngle =
        j === 0 ? Math.random() * 2 * Math.PI : Math.atan((coords[j][1] - coords[j - 1][1]) / (coords[j][0] - coords[j - 1][0]));
      const angle = priorAngle + (Math.random() - 0.5) * maxRotation * 2;
      const distance = Math.random() * maxLength;
      coords.push([coords[j][0] + distance * Math.cos(angle), coords[j][1] + distance * Math.sin(angle)]);
    }
    out.push(lineString(coords));
  }
  return featureCollection(out, {});
}

export function randomPolygon(
  count = 1,
  options: { bbox?: BBox; num_vertices?: number; max_radial_length?: number } = {},
): FeatureCollection<Polygon> {
  const num = Math.max(3, options.num_vertices ?? 10);
  const maxRadial = options.max_radial_length ?? 10;
  const out: Array<Feature<Polygon>> = [];
  for (let i = 0; i < count; i++) {
    const centre = randomPosition(options.bbox);
    const ring: Position[] = [];
    for (let j = 0; j < num; j++) {
      const angle = (2 * Math.PI * j) / num;
      const r = Math.random() * maxRadial;
      ring.push([centre[0] + r * Math.cos(angle), centre[1] + r * Math.sin(angle)]);
    }
    ring.push([...ring[0]]);
    out.push(polygon([ring]));
  }
  return featureCollection(out, {});
}

// ================================================== property-filter helpers

/** Does `properties` satisfy `filter`? (turf's `propertiesContainsFilter`) */
export function propertiesContainsFilter(properties: GeoJsonProperties, filter: GeoJsonProperties): boolean {
  if (!properties || !filter) return false;
  for (const key of Object.keys(filter)) {
    if ((properties as Record<string, unknown>)[key] !== (filter as Record<string, unknown>)[key]) return false;
  }
  return true;
}

/**
 * Match `properties` against a filter: a key name (present?), a list of filters
 * (all of them), or an object of key/value pairs (turf's `applyFilter`).
 */
export function applyFilter(
  properties: GeoJsonProperties,
  filter: string | number | Array<string | number | GeoJsonProperties> | GeoJsonProperties,
): boolean {
  if (properties === undefined || properties === null) return false;
  if (typeof filter === 'number' || typeof filter === 'string') {
    return (properties as Record<string, unknown>)[filter] !== undefined;
  }
  if (Array.isArray(filter)) {
    return filter.every((f) => applyFilter(properties, f as string | number | GeoJsonProperties));
  }
  return propertiesContainsFilter(properties, filter);
}

/** Keep only the listed keys (turf's `filterProperties`). */
export function filterProperties(properties: GeoJsonProperties, keys?: string[]): GeoJsonProperties {
  if (!keys || keys.length === 0 || !properties) return {};
  const out: Record<string, unknown> = {};
  for (const key of keys) {
    if (Object.prototype.hasOwnProperty.call(properties, key)) {
      out[key] = (properties as Record<string, unknown>)[key];
    }
  }
  return out;
}

/** Group feature indexes by the value of `property` (turf's `createBins`). */
export function createBins(geojson: FeatureCollection, property: string): Record<string, number[]> {
  const bins: Record<string, number[]> = {};
  featureEach(geojson, (feat, i) => {
    const v = (feat.properties as Record<string, unknown> | null)?.[property];
    if (v === undefined) return;
    const key = String(v);
    if (bins[key]) bins[key].push(i);
    else bins[key] = [i];
  });
  return bins;
}

/** A deep copy of a properties object (turf's `cloneProperties`). */
export function cloneProperties(properties: GeoJsonProperties): GeoJsonProperties {
  if (!properties) return {};
  return clone(properties as unknown as GeoJSON) as unknown as GeoJsonProperties;
}

/** Strip `bbox` from a GeoJSON object and everything inside it, in place. */
export function removeBbox<T extends GeoJSON>(geojson: T): T {
  const strip = (o: unknown): void => {
    if (!o || typeof o !== 'object') return;
    delete (o as { bbox?: BBox }).bbox;
    const obj = o as { features?: unknown[]; geometries?: unknown[]; geometry?: unknown };
    if (Array.isArray(obj.features)) obj.features.forEach(strip);
    if (Array.isArray(obj.geometries)) obj.geometries.forEach(strip);
    if (obj.geometry) strip(obj.geometry);
  };
  strip(geojson);
  return geojson;
}

/** Throw unless `bbox` is 4 or 6 finite numbers (turf's `validateBBox`). */
export function validateBBox(bbox: unknown): void {
  if (!bbox) throw new Error('bbox is required');
  if (!Array.isArray(bbox)) throw new Error('bbox must be an Array');
  if (bbox.length !== 4 && bbox.length !== 6) throw new Error('bbox must be an Array of 4 or 6 numbers');
  for (const v of bbox) {
    if (!isNumber(v)) throw new Error('bbox must only contain numbers');
  }
}

/** Throw unless `id` is a string or a number (turf's `validateId`). */
export function validateId(id: unknown): void {
  if (!id) throw new Error('id is required');
  if (['string', 'number'].indexOf(typeof id) === -1) throw new Error('id must be a number or a string');
}

// ============================================================ spatial index

/** The bounding box of any GeoJSON, computed without WebAssembly. */
function bboxOf(geojson: GeoJSON): [number, number, number, number] {
  let [x0, y0, x1, y1] = [Infinity, Infinity, -Infinity, -Infinity];
  coordEach(geojson, (c) => {
    if (c[0] < x0) x0 = c[0];
    if (c[1] < y0) y0 = c[1];
    if (c[0] > x1) x1 = c[0];
    if (c[1] > y1) y1 = c[1];
  });
  return [x0, y0, x1, y1];
}

type Box = [number, number, number, number];

function boxOf(input: GeoJSON | Box): Box {
  return Array.isArray(input) ? input : bboxOf(input);
}

const overlaps = (a: Box, b: Box): boolean => !(b[0] > a[2] || b[2] < a[0] || b[1] > a[3] || b[3] < a[1]);

/**
 * A bbox index with the surface of `@turf/geojson-rbush`.
 *
 * Behind the compatible API this is a uniform grid rather than an R-tree: the
 * grid is rebuilt when the extent or the population changes enough to matter, so
 * `insert` stays cheap and queries touch only the cells they overlap. Same
 * results, no dependency.
 */
export class GeoJsonIndex<G extends Geometry = Geometry, P extends GeoJsonProperties = GeoJsonProperties> {
  private items: Array<{ box: Box; feature: Feature<G, P> }> = [];
  private cells: Map<string, number[]> = new Map();
  private extent: Box = [Infinity, Infinity, -Infinity, -Infinity];
  private cellSize = 0;
  private builtFor = 0;

  insert(feat: Feature<G, P>): this {
    this.items.push({ box: bboxOf(feat as GeoJSON), feature: feat });
    this.cells.clear();
    return this;
  }

  load(features: Array<Feature<G, P>> | FeatureCollection<G, P>): this {
    const list = Array.isArray(features) ? features : features.features;
    for (const f of list) this.items.push({ box: bboxOf(f as GeoJSON), feature: f });
    this.cells.clear();
    return this;
  }

  /** Remove a feature; `equals` decides identity (reference equality by default). */
  remove(feat: Feature<G, P>, equals?: (a: Feature<G, P>, b: Feature<G, P>) => boolean): this {
    const same = equals ?? ((a, b) => a === b);
    const i = this.items.findIndex((it) => same(it.feature, feat));
    if (i >= 0) {
      this.items.splice(i, 1);
      this.cells.clear();
    }
    return this;
  }

  clear(): this {
    this.items = [];
    this.cells.clear();
    return this;
  }

  all(): FeatureCollection<G, P> {
    return featureCollection(this.items.map((it) => it.feature));
  }

  /** Features whose bbox overlaps the query. */
  search(input: GeoJSON | Box): FeatureCollection<G, P> {
    const box = boxOf(input);
    return featureCollection(this.candidates(box).filter((it) => overlaps(it.box, box)).map((it) => it.feature));
  }

  /** Is there at least one overlap? Stops at the first hit. */
  collides(input: GeoJSON | Box): boolean {
    const box = boxOf(input);
    return this.candidates(box).some((it) => overlaps(it.box, box));
  }

  toJSON(): Array<{ bbox: Box; feature: Feature<G, P> }> {
    return this.items.map((it) => ({ bbox: it.box, feature: it.feature }));
  }

  fromJSON(data: Array<{ bbox?: Box; feature: Feature<G, P> }>): this {
    this.items = data.map((d) => ({ box: d.bbox ?? bboxOf(d.feature as GeoJSON), feature: d.feature }));
    this.cells.clear();
    return this;
  }

  private build(): void {
    this.cells.clear();
    const n = this.items.length;
    this.builtFor = n;
    if (n === 0) return;
    let [x0, y0, x1, y1] = [Infinity, Infinity, -Infinity, -Infinity];
    for (const it of this.items) {
      if (it.box[0] < x0) x0 = it.box[0];
      if (it.box[1] < y0) y0 = it.box[1];
      if (it.box[2] > x1) x1 = it.box[2];
      if (it.box[3] > y1) y1 = it.box[3];
    }
    this.extent = [x0, y0, x1, y1];
    // aim for a handful of features per cell
    const side = Math.max(1, Math.round(Math.sqrt(n / 2)));
    this.cellSize = Math.max((x1 - x0) / side, (y1 - y0) / side, Number.MIN_VALUE);
    for (let i = 0; i < n; i++) this.index(i);
  }

  private index(i: number): void {
    const b = this.items[i].box;
    for (const key of this.keys(b)) {
      const bucket = this.cells.get(key);
      if (bucket) bucket.push(i);
      else this.cells.set(key, [i]);
    }
  }

  private keys(b: Box): string[] {
    const s = this.cellSize;
    const gx0 = Math.floor((b[0] - this.extent[0]) / s);
    const gy0 = Math.floor((b[1] - this.extent[1]) / s);
    const gx1 = Math.floor((b[2] - this.extent[0]) / s);
    const gy1 = Math.floor((b[3] - this.extent[1]) / s);
    const out: string[] = [];
    // a query far outside the extent still lands in the edge cells
    for (let gy = gy0; gy <= gy1; gy++) for (let gx = gx0; gx <= gx1; gx++) out.push(`${gx},${gy}`);
    return out;
  }

  private candidates(box: Box): Array<{ box: Box; feature: Feature<G, P> }> {
    if (this.items.length === 0) return [];
    if (this.cells.size === 0 || this.builtFor !== this.items.length) this.build();
    // a tiny population is not worth a grid walk
    if (this.items.length < 16) return this.items;
    const seen = new Set<number>();
    for (const key of this.keys(box)) {
      const bucket = this.cells.get(key);
      if (bucket) for (const i of bucket) seen.add(i);
    }
    return [...seen].map((i) => this.items[i]);
  }
}

/** An empty bbox index (turf's `geojsonRbush`). */
export function geojsonRbush<G extends Geometry = Geometry, P extends GeoJsonProperties = GeoJsonProperties>(): GeoJsonIndex<G, P> {
  return new GeoJsonIndex<G, P>();
}
