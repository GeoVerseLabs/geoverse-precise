# turf coverage

Every one of the **184 exports of `@turf/turf` 7.2** has a same-named export in
`geoverse-precise`, so `import * as turf from '@turf/turf'` becomes
`import * as turf from 'geoverse-precise'` — with one call to `await turf.init()` at
start-up, because the geometry runs in WebAssembly.

The status column comes from `bench/turf-parity.mjs`, which runs both libraries
side by side and prints the numbers quoted here:

| | meaning |
| --- | --- |
| **=** | same result (to turf's own precision) |
| **+** | ellipsoid-accurate where turf is spherical or degree-based, so the numbers differ by a measurable amount |
| **~** | deliberately different; the note says how |

`bench/out/parity.json` holds the machine-readable form of the run.

---

## Measurement

| turf | geoverse-precise | | note |
| --- | --- | --- | --- |
| `distance` | `distance` | + | Karney geodesic. Beijing→Shanghai: turf 1 067 078.9 m, geoverse-precise 1 065 615.5 m — turf is 0.137% long |
| `bearing` | `bearing` | + | ellipsoidal azimuth; 0.0985° from turf's spherical one on the same pair |
| `destination` | `destination` | + | 1 022 m apart from turf after 500 km |
| `midpoint` | `midpoint` | + | 194 m apart from turf on a 1 000 km line |
| `length` | `length` | + | turf 153 796.7 m vs 154 055.8 m on a 3-vertex line |
| `area` | `area` | + | turf is 0.094% low on a 1°×1° polygon at 39°N |
| `along` | `along` | + | 116.6 m apart from turf at 50 km along |
| `nearestPointOnLine` | `nearestPointOnLine` | = | 157.5 m apart from turf, which is turf's spherical error, not a different answer |
| `pointToLineDistance` | `pointToLineDistance` | + | turf 9 161.7 m vs 9 302.6 m |
| `pointToPolygonDistance` | `pointToPolygonDistance` | + | signed: negative inside |
| `rhumbDistance` | `rhumbDistance` | + | meridian arc by Gauss–Legendre quadrature; 0.137% from turf |
| `rhumbBearing` | `rhumbBearing` | = | isometric latitude; agrees with turf to 0.001° |
| `rhumbDestination` | `rhumbDestination` | + | Newton inverse on the meridian arc; 978 m from turf at 500 km |
| `angle` | `angle` | = | geodesic bearings; `explementary` supported |
| `greatCircle` | `greatCircle` | + | densified so the chord error stays under the tolerance |

## Coordinate mutation and transforms

| turf | geoverse-precise | | note |
| --- | --- | --- | --- |
| `transformRotate` | `transformRotate` | + | rotates on the ellipsoid rather than in degree space |
| `transformTranslate` | `transformTranslate` | + | same |
| `transformScale` | `transformScale` | + | same |
| `toMercator` | `toMercator` | = | routed through the library's own CRS stack (EPSG:3857) |
| `toWgs84` | `toWgs84` | = | exact inverse |
| `flip` | `flip` | = | swaps x and y |
| `rewind` | `rewind` | = | |
| `cleanCoords` | `cleanCoords` | + | tolerance is metric, not degrees |
| `truncate` | `truncate` | = | |
| `clone` | `clone` | = | `structuredClone` where available |

**Beyond turf:** `transform`, `convert`, `transformCoords`, `normalizeCrs`,
`gaussKrugerCrs`, `utmCrs`, `wgs84ToGcj02` / `gcj02ToWgs84` /
`wgs84ToBd09` / `bd09ToWgs84` / `gcj02ToBd09` / `bd09ToGcj02`. Every measuring
and constructive function also takes `crs`, so GCJ-02 or CGCS2000 Gauss-Krüger
input is converted in, computed on the ellipsoid, and converted back out.

## Constructive

| turf | geoverse-precise | | note |
| --- | --- | --- | --- |
| `circle` | `circle` | + | every vertex exactly on the radius: turf's radii spread 39.7 m over a 10 km circle, geoverse-precise's 0.000 m |
| `buffer` | `buffer` | + | geodesic offset strips and wedges: turf's edge distance spreads 200 m at 50 km, geoverse-precise's 0.00 m |
| `ellipse` | `ellipse` | + | metric semi-axes: turf draws 20 050 m for a 20 km axis |
| `sector` | `sector` | + | radii exact |
| `lineArc` | `lineArc` | + | turf's radii spread 19.8 m, geoverse-precise's 0.0000 m |
| `bezierSpline` | `bezierSpline` | = | Catmull-Rom control points, evaluated in the local plane |
| `polygonSmooth` | `polygonSmooth` | + | Chaikin cutting in metres, so cells do not skew with latitude |
| `polygonTangents` | `polygonTangents` | = | |
| `mask` | `mask` | = | |
| `envelope` | `envelope` | = | identical bbox |
| `square` | `square` | = | identical |
| `bboxPolygon` | `bboxPolygon` | = | |
| `polygonize` | `polygonize` | = | noding plus a half-edge minimal-face walk |
| `tesselate` | `tesselate` | = | constrained Delaunay; 8 vs 8 triangles covering the polygon exactly |
| `convex` / `convexHull` | `convex`, `convexHull` | = | areas differ only by turf's spherical area |
| `concave` / `concaveHull` | `concave`, `concaveHull` | + | `maxEdge` is metric |
| `lineToPolygon` | `lineToPolygon` | = | |
| `polygonToLine` | `polygonToLine` | = | |

## Grids

turf converts a cell size to degrees once and uses it for both axes, so its cells
stretch east-west as you leave the equator. geoverse-precise lays the grid out in a
local transverse Mercator plane, so a 5 km cell is 5 km on the ground.

| turf | geoverse-precise | | note |
| --- | --- | --- | --- |
| `pointGrid` | `pointGrid` | + | 5 km grid at 39°N: turf spaces neighbours 4 992 m, geoverse-precise 5 000 m |
| `squareGrid` | `squareGrid` | + | cell areas vary by 0.146% of nominal for turf, 0.0005% for geoverse-precise |
| `rectangleGrid` | `rectangleGrid` | + | turf's median cell is 0.777× the requested area; geoverse-precise's is 1.000× |
| `triangleGrid` | `triangleGrid` | + | both 1.000× nominal; the cell counts differ because the cells are different sizes |
| `hexGrid` | `hexGrid` | + | flat-top hexagons; turf 1.001× nominal, geoverse-precise 1.000× |
| `interpolate` | `interpolate` | + | IDW with a geodesic search radius. On a 0.02°-tall bbox turf's degree-converted cells are taller than the box and it returns nothing; geoverse-precise returns 18 cells |

## Interpolation and contouring

| turf | geoverse-precise | | note |
| --- | --- | --- | --- |
| `isolines` | `isolines` | = | traced across lattice triangles, where the interpolant is linear — no marching-squares saddle to guess at. Contour positions agree with the level to 1.4e-13° |
| `isobands` | `isobands` | + | bands are built by clipping triangles and unioning, so they tile the domain with no slivers or overlaps |
| `tin` | `tin` | = | 400 vs 400 triangles covering the hull exactly. Triangulated in the metric plane, then the lon/lat-degenerate slivers that produces on a collinear lattice edge are dropped |
| `voronoi` | `voronoi` | ~ | cells are half-plane intersections against the Delaunay neighbours, exact in the plane. Written out as lon/lat chords they cover 0.99989 of the bbox against turf's 1.00000 — a hairline along each shared edge, ~1e-4 of the area over a 30 km domain |
| `planepoint` | `planepoint` | = | barycentric in the local plane |

## Overlay

| turf | geoverse-precise | | note |
| --- | --- | --- | --- |
| `intersect` | `intersect` | = | integer-kernel boolean (`i_overlay` via `geo`); areas differ only by turf's spherical area |
| `union` | `union` | = | same |
| `difference` | `difference` | = | same |
| — | `xor`, `unionAll`, `dissolve` | | `dissolve` groups by a property first |

`edges: 'geodesic'` makes an overlay treat edges as geodesics rather than
lon/lat-straight lines; `planar` (the default) is RFC 7946 and turf semantics.

## Predicates

| turf | geoverse-precise | | note |
| --- | --- | --- | --- |
| `booleanPointInPolygon` | `booleanPointInPolygon` | = | |
| `booleanIntersects` | `booleanIntersects` | = | |
| `booleanContains` | `booleanContains` | = | DE-9IM |
| `booleanWithin` | `booleanWithin` | = | DE-9IM |
| `booleanCrosses` | `booleanCrosses` | = | DE-9IM |
| `booleanTouches` | `booleanTouches` | = | DE-9IM |
| `booleanOverlap` | `booleanOverlap` | = | DE-9IM |
| `booleanDisjoint` | `booleanDisjoint` | = | DE-9IM |
| `booleanEqual` | `booleanEqual` | = | DE-9IM, not a coordinate comparison |
| `booleanPointOnLine` | `booleanPointOnLine` | + | metric tolerance |
| `booleanParallel` | `booleanParallel` | = | |
| `booleanConcave` | `booleanConcave` | = | |
| `booleanClockwise` | `booleanClockwise` | = | shoelace sign, turf's convention |
| `booleanValid` | `booleanValid` | = | `true` for a square, `false` for a bowtie — where turf reports `true` |

**Beyond turf:** `relate` returns the DE-9IM matrix, `relatePattern` tests a
pattern such as `T*F**FFF*`, `booleanRelation` takes the predicate by name, and
`booleanCounterClockwise` is the complement of `booleanClockwise`.

## Lines

| turf | geoverse-precise | | note |
| --- | --- | --- | --- |
| `lineSegment` | `lineSegment` | = | two-position LineStrings |
| `lineSplit` | `lineSplit` | = | |
| `lineSlice` | `lineSlice` | + | cut points are on the geodesic |
| `lineSliceAlong` | `lineSliceAlong` | + | same |
| `lineChunk` | `lineChunk` | + | chunk length is a real distance |
| `lineIntersect` | `lineIntersect` | = | |
| `lineOffset` | `lineOffset` | + | 1 km offset: turf lands at 999 m, geoverse-precise at 1 000 m. Convex corners get an arc; concave corners keep both perpendicular feet, so they sit at `d·cos(turn)` rather than folding over |
| `lineOverlap` | `lineOverlap` | = | tolerance is metric |
| `nearestPointToLine` | `nearestPointToLine` | = | |
| `nearestPoint` | `nearestPoint` | + | geodesic distances |
| `explode` | `explode` | = | |
| `flatten` | `flatten` | = | |
| `kinks` | `kinks` | = | 1 vs 1 self-intersection on a bowtie |
| `unkinkPolygon` | `unkinkPolygon` | = | 2 vs 2 pieces |
| `simplify` | `simplify` | + | tolerance is metric; `preserveTopology` uses Visvalingam–Whyatt |
| `bboxClip` | `bboxClip` | = | |

## Centres

| turf | geoverse-precise | | note |
| --- | --- | --- | --- |
| `center` | `center` | = | bbox centre; identical |
| `centroid` | `centroid` | = | mean of positions, with the ring's closing position excluded as turf does |
| `centerOfMass` | `centerOfMass` | + | area-weighted, in the local plane |
| `centerMean` | `centerMean` | = | identical on a symmetric cloud |
| `centerMedian` | `centerMedian` | = | Weiszfeld in metres. With one outlier among four clustered points it lands 423 m from the cluster centre against turf's 626 m, while the mean is dragged 28 007 m away |
| `pointOnFeature` | `pointOnFeature` | = | |

## Clustering and spatial statistics

| turf | geoverse-precise | | note |
| --- | --- | --- | --- |
| `clustersDbscan` | `clustersDbscan` | = | 2/2 clusters and 1/1 noise point on the same input; `maxDistance` is geodesic |
| `clustersKmeans` | `clustersKmeans` | + | k-means++ from a fixed generator, so repeated runs give the same clusters; turf's `skmeans` seeds at random |
| `getCluster` | `getCluster` | = | |
| `clusterEach` | `clusterEach` | = | |
| `clusterReduce` | `clusterReduce` | = | |
| `createBins` | `createBins` | = | |
| `nearestNeighborAnalysis` | `nearestNeighborAnalysis` | + | for two 24-point rings the closed form gives an index of 0.3362; geoverse-precise reports 0.3375 and turf 0.0877 |
| `standardDeviationalEllipse` | `standardDeviationalEllipse` | + | fitted in metres: 20.34 km × 0.51 km where turf reports 0.0052° × 0.2001°, which mixes the two axes' units. Also reports `majorAxisBearing`, which θ alone does not give |
| `directionalMean` | `directionalMean` | = | geodesic start-to-end azimuths; 45.000° against turf's 45.115° |
| `distanceWeight` | `distanceWeight` | + | the radius is a geodesic distance in `units`. turf thresholds a Minkowski distance on raw degrees: on a 1 km lattice at 39°N its interior point sees 63 neighbours at `threshold: 1.2` and 2 at `0.0095`, where a 1.2 km radius sees the 4 that are there |
| `moranIndex` | `moranIndex` | + | follows from the weights: a west-to-east ramp on a rook lattice gives 0.9375, where turf gives −0.0159 (its expectation) or 1.0000 depending on which degree threshold you pick |
| `quadratAnalysis` | `quadratAnalysis` | + | equal-area quadrats. On a perfect lattice turf's degree quadrats give uneven counts and reject randomness; geoverse-precise counts 4 per quadrat, so χ² is 0 and `varianceMeanRatio` of 0.00 is what reports the regularity. Note that χ² only tests the clustered tail — a regular pattern passes it too |

## Routing

| turf | geoverse-precise | | note |
| --- | --- | --- | --- |
| `shortestPath` | `shortestPath` | + | A* over a metric lattice, then pulled taut: 36.89 km around a wall where turf's staircase is 41.04 km (straight line 34.65 km). With nothing in the way the result is the straight line, to 0.1 m |

## Joins and aggregation

| turf | geoverse-precise | | note |
| --- | --- | --- | --- |
| `tag` | `tag` | = | |
| `collect` | `collect` | = | |
| `sample` | `sample` | = | |
| `combine` | `combine` | = | |
| `pointsWithinPolygon` | `pointsWithinPolygon` | = | prepared index behind it |

## Helper layer (pure TypeScript, no WebAssembly)

These need no `init()` and are byte-compatible with turf's.

| turf | geoverse-precise | | note |
| --- | --- | --- | --- |
| `feature`, `featureCollection`, `geometry`, `geometryCollection` | same | = | |
| `point`, `points`, `lineString`, `lineStrings`, `polygon`, `polygons` | same | = | rings are closed for you; the same validation errors |
| `multiPoint`, `multiLineString`, `multiPolygon` | same | = | |
| `earthRadius`, `factors`, `areaFactors` | same | = | turf's spherical constants, kept for the converters below |
| `radiansToLength`, `lengthToRadians`, `lengthToDegrees` | same | = | spherical by definition — an angle does not fix a length on an ellipsoid. Use `distance` for a measurement |
| `radiansToDegrees`, `degreesToRadians` | same | = | |
| `bearingToAzimuth`, `azimuthToBearing` | same | = | |
| `convertLength`, `convertArea` | same | = | exact unit conversion, no earth model |
| `round`, `isNumber`, `isObject` | same | = | |
| `coordEach`, `coordReduce`, `coordAll` | same | = | 10 vs 10 positions on a polygon with a hole; `excludeWrapCoord` honoured |
| `propEach`, `propReduce`, `featureEach`, `featureReduce` | same | = | |
| `geomEach`, `geomReduce`, `flattenEach`, `flattenReduce` | same | = | |
| `segmentEach`, `segmentReduce`, `lineEach`, `lineReduce` | same | = | 8 vs 8 segments on the same polygon |
| `findSegment`, `findPoint` | same | = | negative indexes count from the end |
| `getCoord`, `getCoords`, `getGeom`, `getType` | same | = | |
| `geojsonType`, `featureOf`, `collectionOf`, `containsNumber` | same | = | same error messages |
| `validateBBox`, `validateId` | same | = | |
| `applyFilter`, `propertiesContainsFilter`, `filterProperties`, `cloneProperties` | same | = | |
| `removeBbox` | `removeBbox` | = | strips `bbox` recursively, in place |
| `randomPosition`, `randomPoint`, `randomLineString`, `randomPolygon` | same | = | |
| `geojsonRbush` | `geojsonRbush` | = | 4 vs 4 hits on the same query. A uniform grid behind turf's R-tree surface (`insert`, `load`, `remove`, `search`, `collides`, `all`, `clear`, `toJSON`, `fromJSON`) — same results, no dependency |
| `meta`, `helpers`, `invariant`, `projection`, `random`, `clusters` | same | = | the namespace objects, for `meta.coordEach(…)` style calls |

## Only in geoverse-precise

Nothing in turf corresponds to these.

**Validity and repair** — `validate` (a report with severities and locations),
`makeValid`, `snapRound`, `snapTo`.

**Polygon coverage QA** — `coverageIssues` finds overlaps, gaps and slivers
across a set of polygons that should tile, including gaps that are open at one
end (found by morphological closing).

**Line network QA** — `networkIssues` finds dangles, pseudo-nodes, undershoots,
overshoots, duplicates and unnoded crossings.

**Prepared geometry** — `prepare()` builds a segment index once and answers
`contains`, `containsMany`, `nearest`, `nearestMany`, `distance`, `distanceMany`
and `within` against it.

**Batch entry points** — `distanceBatch`, `distanceToBatch`, `destinationBatch`
take and return `Float64Array`, so a million distances cross the WebAssembly
boundary once.

**CRS** — see the transforms section above.

---

## Porting notes

1. `await init()` once before the first geometry call. The helper layer works
   without it.
2. Lengths default to kilometres and areas are m², as in turf.
3. `units` is accepted wherever turf accepts it, and additionally on grid cell
   sizes, tolerances, search radii and routing resolutions — those are metric in
   geoverse-precise, where turf takes degrees.
4. `edges` chooses how an edge between two vertices is interpreted: `planar`
   (the default, RFC 7946 and turf) or `geodesic`.
5. `crs` is accepted throughout; leave it out for WGS84.
6. Functions that turf mutates in place (`clustersDbscan` with `mutate`) always
   return a new FeatureCollection here.
7. `bezierSpline` takes `steps` per segment rather than turf's `resolution` in
   milliseconds.
