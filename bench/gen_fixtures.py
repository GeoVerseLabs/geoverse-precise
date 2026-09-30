"""Generate reference data from GeographicLib (Karney) and PROJ (pyproj).

    pip install geographiclib pyproj
    python bench/gen_fixtures.py

Output goes to bench/fixtures/*.json and is used by the Rust tests
(crates/core/tests/reference.rs) and the Node accuracy report (bench/accuracy.mjs).
"""
import json
import math
import os
import random

from geographiclib.geodesic import Geodesic
from pyproj import Transformer

random.seed(20260916)
G = Geodesic.WGS84
OUT = os.path.join(os.path.dirname(__file__), "fixtures")
os.makedirs(OUT, exist_ok=True)


def dump(name, data):
    with open(os.path.join(OUT, name), "w") as f:
        json.dump(data, f, separators=(",", ":"))
    print(name, len(data) if isinstance(data, list) else "")


def china_pt():
    return random.uniform(73.5, 134.8), random.uniform(18.0, 53.5)


def global_pt():
    return random.uniform(-180, 180), math.degrees(math.asin(random.uniform(-0.99, 0.99)))


# --- inverse problem --------------------------------------------------------
inverse = []
for i in range(600):
    if i < 200:  # local (fleet / city scale)
        lon1, lat1 = china_pt()
        az, s = random.uniform(-180, 180), random.uniform(10, 50_000)
        d = G.Direct(lat1, lon1, az, s)
        lon2, lat2 = d["lon2"], d["lat2"]
        tag = "china-local"
    elif i < 400:  # regional
        (lon1, lat1), (lon2, lat2) = china_pt(), china_pt()
        tag = "china-regional"
    else:
        (lon1, lat1), (lon2, lat2) = global_pt(), global_pt()
        tag = "global"
    r = G.Inverse(lat1, lon1, lat2, lon2)
    inverse.append(dict(tag=tag, lon1=lon1, lat1=lat1, lon2=lon2, lat2=lat2,
                        s12=r["s12"], azi1=r["azi1"], azi2=r["azi2"]))
# nearly antipodal
for lat in (0.0, 0.5, 20.0):
    r = G.Inverse(lat, 0.0, -lat + 0.3, 179.7)
    inverse.append(dict(tag="antipodal", lon1=0.0, lat1=lat, lon2=179.7, lat2=-lat + 0.3,
                        s12=r["s12"], azi1=r["azi1"], azi2=r["azi2"]))
dump("inverse.json", inverse)

# --- direct problem ---------------------------------------------------------
direct = []
for i in range(300):
    lon, lat = china_pt() if i < 200 else global_pt()
    azi = random.uniform(0, 360)
    s = 10 ** random.uniform(1, 6.7)
    d = G.Direct(lat, lon, azi, s)
    direct.append(dict(lon=lon, lat=lat, azi=azi, s=s, lon2=d["lon2"], lat2=d["lat2"]))
dump("direct.json", direct)

# --- polygon area -----------------------------------------------------------
areas = []
for i in range(120):
    cx, cy = china_pt()
    radius = 10 ** random.uniform(2, 5.9)  # 100 m .. 800 km
    n = random.randint(3, 40)
    angles = sorted(random.uniform(0, 360) for _ in range(n))
    ring = []
    for a in angles:  # counter-clockwise in lon/lat => negative azimuth order
        d = G.Direct(cy, cx, -a, radius * random.uniform(0.5, 1.0))
        ring.append([d["lon2"], d["lat2"]])
    ring.append(ring[0])
    p = G.Polygon()
    for lon, lat in ring[:-1]:
        p.AddPoint(lat, lon)
    num, perim, area = p.Compute(False, True)
    areas.append(dict(ring=ring, area=area, perimeter=perim, radius=radius))
dump("area.json", areas)

# --- projections (PROJ) ------------------------------------------------------
proj = []
codes = [4491, 4495, 4501, 4502, 4509, 4512, 4513, 4527, 4533, 4534, 4546, 4549, 4554]
for code in codes:
    t = Transformer.from_crs("EPSG:4490", f"EPSG:{code}", always_xy=True)
    crs = t.target_crs
    lon0 = [p.value for p in crs.coordinate_operation.params if "longitude" in p.name.lower()][0]
    half = 3.0 if code <= 4512 else 1.5
    for _ in range(40):
        lon = random.uniform(lon0 - half, lon0 + half)
        lat = random.uniform(18, 53)
        x, y = t.transform(lon, lat)
        proj.append(dict(src="EPSG:4490", dst=f"EPSG:{code}", lon=lon, lat=lat, x=x, y=y))
for code in (32649, 32650, 32651, 32733):
    t = Transformer.from_crs("EPSG:4326", f"EPSG:{code}", always_xy=True)
    zone = code % 100
    lon0 = zone * 6 - 183
    for _ in range(40):
        lon = random.uniform(lon0 - 3, lon0 + 3)
        lat = random.uniform(0.5, 60) * (-1 if code > 32700 else 1)
        x, y = t.transform(lon, lat)
        proj.append(dict(src="EPSG:4326", dst=f"EPSG:{code}", lon=lon, lat=lat, x=x, y=y))
t = Transformer.from_crs("EPSG:4326", "EPSG:3857", always_xy=True)
for _ in range(60):
    lon, lat = random.uniform(-180, 180), random.uniform(-85, 85)
    x, y = t.transform(lon, lat)
    proj.append(dict(src="EPSG:4326", dst="EPSG:3857", lon=lon, lat=lat, x=x, y=y))
# wide zone: 6° GK far from CM (tests the Krüger series, not just the zone)
t = Transformer.from_crs("EPSG:4490", "EPSG:4509", always_xy=True)  # CM 117E
for _ in range(40):
    lon, lat = random.uniform(105, 129), random.uniform(18, 53)
    x, y = t.transform(lon, lat)
    proj.append(dict(src="EPSG:4490", dst="EPSG:4509", lon=lon, lat=lat, x=x, y=y, wide=True))
dump("projections.json", proj)
