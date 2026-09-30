"""Independent cross-check of the distance "stick" used in accuracy.mjs.

For every buffer vertex written to bench/out/buffers.json, recompute its
distance to the input geometry with pyproj.Geod (PROJ's GeographicLib port)
and compare with the value geoprecise reported.

    python bench/verify_buffer.py
"""
import json
import os

import numpy as np
from pyproj import Geod

G = Geod(ellps="WGS84")
HERE = os.path.dirname(__file__)
data = json.load(open(os.path.join(HERE, "out", "buffers.json")))


def dist_to_point(verts, c):
    v = np.asarray(verts)
    _, _, d = G.inv(np.full(len(v), c[0]), np.full(len(v), c[1]), v[:, 0], v[:, 1])
    return d


def dist_to_line(verts, coords, samples=64, refine=40):
    """Min geodesic distance from each vertex to a polyline of geodesic segments."""
    coords = np.asarray(coords)
    a, b = coords[:-1], coords[1:]
    az, _, seglen = G.inv(a[:, 0], a[:, 1], b[:, 0], b[:, 1])
    out = []
    for v in verts:
        # coarse: sample every segment
        t = np.linspace(0, 1, samples)
        lon, lat, _ = G.fwd(
            np.repeat(a[:, 0], samples), np.repeat(a[:, 1], samples),
            np.repeat(az, samples), (seglen[:, None] * t[None, :]).ravel(),
        )
        _, _, d = G.inv(np.full(lon.size, v[0]), np.full(lon.size, v[1]), lon, lat)
        d = d.reshape(len(a), samples)
        best = float("inf")
        # refine the 3 most promising segments with golden-section search
        for s in np.argsort(d.min(axis=1))[:3]:
            k = int(np.argmin(d[s]))
            lo, hi = max(0, k - 1) / (samples - 1), min(samples - 1, k + 1) / (samples - 1)
            f = lambda tt: G.inv(v[0], v[1], *G.fwd(a[s, 0], a[s, 1], az[s], seglen[s] * tt)[:2])[2]
            gr = (5 ** 0.5 - 1) / 2
            x1, x2 = hi - gr * (hi - lo), lo + gr * (hi - lo)
            f1, f2 = f(x1), f(x2)
            for _ in range(refine):
                if f1 < f2:
                    hi, x2, f2 = x2, x1, f1
                    x1 = hi - gr * (hi - lo)
                    f1 = f(x1)
                else:
                    lo, x1, f1 = x1, x2, f2
                    x2 = lo + gr * (hi - lo)
                    f2 = f(x2)
            best = min(best, f1, f2, d[s].min())
        out.append(best)
    return np.asarray(out)


worst_overall = 0.0
for name, s in data.items():
    geom, d = s["input"], s["d"]
    for method in ("turf", "geodesic", "projected"):
        verts = s[method]
        if geom["type"] == "Point":
            true = dist_to_point(verts, geom["coordinates"])
        else:
            ring = geom["coordinates"][0] if geom["type"] == "Polygon" else geom["coordinates"]
            # vertices are checked in a subsample for speed
            idx = np.linspace(0, len(verts) - 1, min(len(verts), 150)).astype(int)
            verts = [verts[i] for i in idx]
            true = dist_to_line(verts, ring)
            stick = np.asarray(s["stick"][method])[idx]
        if geom["type"] == "Point":
            stick = np.asarray(s["stick"][method])
        err_true = true - d
        disagreement = np.abs(err_true - stick).max()
        worst_overall = max(worst_overall, disagreement)
        print(f"{name:45s} {method:10s} max|err| (pyproj) = {np.abs(err_true).max():12.6f} m   "
              f"stick disagreement = {disagreement:.2e} m")
print(f"\nmax disagreement between geoprecise stick and pyproj: {worst_overall:.2e} m")
