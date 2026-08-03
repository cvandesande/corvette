"""Diff two raw f32 output tensors from the same model and input.

Bitwise equality is not the bar -- both implementations drive the same shaders
on the same device, but nothing guarantees identical reduction order -- so
report the worst absolute and relative disagreement and let the caller judge.
"""

import sys

import numpy as np

a = np.fromfile(sys.argv[1], dtype=np.float32)
b = np.fromfile(sys.argv[2], dtype=np.float32)
if a.shape != b.shape:
    raise SystemExit(f"shape mismatch: {a.shape} vs {b.shape}")

diff = np.abs(a - b)
scale = np.maximum(np.abs(a), np.abs(b))
rel = np.where(scale > 1e-6, diff / np.maximum(scale, 1e-12), 0.0)
print(
    f"elements={a.size} identical={int((a == b).sum())} "
    f"max_abs_diff={diff.max():.6g} mean_abs_diff={diff.mean():.6g} "
    f"max_rel_diff={rel.max():.6g} "
    f"range=[{a.min():.6g}, {a.max():.6g}]"
)
raise SystemExit(0 if diff.max() < 1e-3 else 1)
