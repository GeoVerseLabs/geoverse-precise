import maplibregl from 'maplibre-gl';
import 'maplibre-gl/dist/maplibre-gl.css';
import * as turf from '@turf/turf';
import * as gp from 'geoverse-precise';

await gp.init();

const $ = (id) => document.getElementById(id);
const state = {
  mode: 'point',
  base: 'osm',
  drawing: [],
  geom: null,
  loaded: false,
};

const BASEMAPS = {
  osm: { tiles: ['https://tile.openstreetmap.org/{z}/{x}/{y}.png'], crs: 'WGS84', attribution: '© OpenStreetMap' },
  amap: {
    tiles: ['1', '2', '3', '4'].map((s) => `https://webrd0${s}.is.autonavi.com/appmaptile?lang=zh_cn&size=1&scale=1&style=8&x={x}&y={y}&z={z}`),
    crs: 'GCJ02',
    attribution: '© 高德地图',
  },
};

const map = new maplibregl.Map({
  container: 'map',
  center: [116.4, 39.9],
  zoom: 6,
  style: {
    version: 8,
    sources: { base: { type: 'raster', tiles: BASEMAPS.osm.tiles, tileSize: 256, attribution: BASEMAPS.osm.attribution } },
    layers: [
      { id: 'bg', type: 'background', paint: { 'background-color': '#eef0ec' } },
      { id: 'base', type: 'raster', source: 'base' },
    ],
  },
});

const empty = { type: 'FeatureCollection', features: [] };

map.on('load', () => {
  for (const id of ['input', 'turf', 'gp', 'drawing']) map.addSource(id, { type: 'geojson', data: empty });
  map.addLayer({ id: 'turf-fill', type: 'fill', source: 'turf', paint: { 'fill-color': '#d9480f', 'fill-opacity': 0.06 } });
  map.addLayer({ id: 'turf-line', type: 'line', source: 'turf', paint: { 'line-color': '#d9480f', 'line-width': 2, 'line-dasharray': [2, 2] } });
  map.addLayer({ id: 'gp-fill', type: 'fill', source: 'gp', paint: { 'fill-color': '#1c6dd0', 'fill-opacity': 0.08 } });
  map.addLayer({ id: 'gp-line', type: 'line', source: 'gp', paint: { 'line-color': '#1c6dd0', 'line-width': 2 } });
  map.addLayer({ id: 'input-line', type: 'line', source: 'input', filter: ['!=', '$type', 'Point'], paint: { 'line-color': '#1d1d1b', 'line-width': 1.5 } });
  map.addLayer({ id: 'input-pt', type: 'circle', source: 'input', filter: ['==', '$type', 'Point'], paint: { 'circle-radius': 4, 'circle-color': '#1d1d1b' } });
  map.addLayer({ id: 'drawing-pt', type: 'circle', source: 'drawing', paint: { 'circle-radius': 3, 'circle-color': '#6b6b66' } });
  state.loaded = true;
  state.geom = { type: 'Point', coordinates: [116.4, 39.9] };
  update();
});

/** Data is kept in WGS84; convert only for display on a GCJ-02 basemap. */
const display = (fc) => (BASEMAPS[state.base].crs === 'WGS84' ? fc : gp.transform(fc, 'WGS84', BASEMAPS[state.base].crs));
const fromScreen = (lngLat) => {
  const p = [lngLat.lng, lngLat.lat];
  return BASEMAPS[state.base].crs === 'WGS84' ? p : gp.convert(p, BASEMAPS[state.base].crs, 'WGS84');
};
const fc = (...geoms) => ({ type: 'FeatureCollection', features: geoms.filter(Boolean).map((g) => (g.type === 'Feature' ? g : { type: 'Feature', properties: {}, geometry: g })) });

map.on('click', (e) => {
  const p = fromScreen(e.lngLat);
  if (state.mode === 'point') {
    state.geom = { type: 'Point', coordinates: p };
    update();
  } else {
    state.drawing.push(p);
    map.getSource('drawing').setData(display(fc({ type: 'MultiPoint', coordinates: state.drawing })));
  }
});

$('finish').onclick = () => {
  const pts = state.drawing;
  if (state.mode === 'line' && pts.length >= 2) state.geom = { type: 'LineString', coordinates: pts };
  if (state.mode === 'polygon' && pts.length >= 3) state.geom = { type: 'Polygon', coordinates: [[...pts, pts[0]]] };
  state.drawing = [];
  map.getSource('drawing').setData(empty);
  update();
};
$('clear').onclick = () => {
  state.drawing = [];
  state.geom = null;
  for (const id of ['input', 'turf', 'gp', 'drawing']) map.getSource(id).setData(empty);
  for (const id of ['tErr', 'tRel', 'tMs', 'gErr', 'gRel', 'gMs']) $(id).textContent = '–';
};
$('sample').onclick = () => {
  // Beijing → Jinan → Nanjing → Shanghai, densified so edge interpretation does not matter
  const legs = [[116.397, 39.909], [117.2, 36.65], [118.8, 32.06], [121.47, 31.23]];
  const coords = [legs[0]];
  for (let i = 1; i < legs.length; i++) {
    for (let k = 1; k <= 60; k++) {
      const t = k / 60;
      coords.push([legs[i - 1][0] + (legs[i][0] - legs[i - 1][0]) * t, legs[i - 1][1] + (legs[i][1] - legs[i - 1][1]) * t]);
    }
  }
  state.geom = { type: 'LineString', coordinates: coords };
  $('radius').value = 20;
  map.fitBounds([[115, 30.5], [123, 40.5]], { padding: 40 });
  update();
};
document.querySelectorAll('#modes button').forEach((b) => {
  b.onclick = () => {
    state.mode = b.dataset.mode;
    state.drawing = [];
    document.querySelectorAll('#modes button').forEach((x) => x.classList.toggle('active', x === b));
  };
});
document.querySelectorAll('#basemaps button').forEach((b) => {
  b.onclick = () => {
    state.base = b.dataset.base;
    document.querySelectorAll('#basemaps button').forEach((x) => x.classList.toggle('active', x === b));
    const bm = BASEMAPS[state.base];
    map.removeLayer('base');
    map.removeSource('base');
    map.addSource('base', { type: 'raster', tiles: bm.tiles, tileSize: 256, attribution: bm.attribution });
    map.addLayer({ id: 'base', type: 'raster', source: 'base' }, 'turf-fill');
    update();
  };
});
for (const id of ['radius', 'projected', 'geodesicEdges']) $(id).addEventListener('input', update);

const fmt = (m) => (m < 0.01 ? `${(m * 1000).toFixed(2)} mm` : m < 1 ? `${(m * 100).toFixed(1)} cm` : m < 1000 ? `${m.toFixed(1)} m` : `${(m / 1000).toFixed(2)} km`);

function vertices(feature) {
  if (!feature) return [];
  const g = feature.geometry;
  const polys = g.type === 'Polygon' ? [g.coordinates] : g.coordinates;
  return polys.flatMap((p) => p.flatMap((r) => r.slice(0, -1)));
}

function maxError(verts, geom, d, edges) {
  let worst = 0;
  const target =
    geom.type === 'Polygon' ? { type: 'LineString', coordinates: geom.coordinates[0] } : geom.type === 'Point' ? null : geom;
  for (const v of verts) {
    const dist = target
      ? gp.pointToLineDistance(v, target, { units: 'meters', edges })
      : gp.distance(geom.coordinates, v, { units: 'meters' });
    worst = Math.max(worst, Math.abs(dist - d));
  }
  return worst;
}

function update() {
  $('radiusLabel').textContent = `${Number($('radius').value).toFixed(1)} km`;
  if (!state.geom || !state.loaded) return;
  const r = Number($('radius').value);
  const d = r * 1000;
  const edges = $('geodesicEdges').checked ? 'geodesic' : 'planar';
  const method = $('projected').checked ? 'projected' : 'geodesic';

  let t0 = performance.now();
  const tb = turf.buffer(state.geom, r, { units: 'kilometers', steps: 16 });
  const tMs = performance.now() - t0;
  t0 = performance.now();
  let gb;
  try {
    gb = gp.buffer(state.geom, r, { units: 'kilometers', steps: 16, edges, method });
  } catch (err) {
    $('note').textContent = String(err.message || err);
    return;
  }
  const gMs = performance.now() - t0;

  map.getSource('input').setData(display(fc(state.geom)));
  map.getSource('turf').setData(display(fc(tb)));
  map.getSource('gp').setData(display(fc(gb)));

  const te = maxError(vertices(tb), state.geom, d, edges);
  const ge = maxError(vertices(gb), state.geom, d, edges);
  $('tErr').textContent = fmt(te);
  $('gErr').textContent = fmt(ge);
  $('tRel').textContent = `${((te / d) * 100).toFixed(3)}%`;
  $('gRel').textContent = `${((ge / d) * 100).toFixed(5)}%`;
  $('tMs').textContent = `${tMs.toFixed(1)} ms`;
  $('gMs').textContent = `${gMs.toFixed(1)} ms`;
  $('note').textContent =
    state.geom.type === 'Point'
      ? '点缓冲：误差 = 顶点到圆心的椭球距离 − 半径。'
      : `误差 = 顶点到输入几何（边按${edges === 'planar' ? '经纬度直线' : '大地线'}）的椭球距离 − 半径。线在凹角处的交点受弦近似影响会略微偏内（至多一个弦高）。`;
}
