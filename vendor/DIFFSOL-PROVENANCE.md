# Vendored Diffsol

QSPQuest builds with Diffsol 0.17.1 and its diffsol-la and diffsol-nl crates,
using the nalgebra, faer and deterministic features with default features disabled.
The included workspace contains these three crates, their build metadata,
source, README and [MIT license](diffsol/LICENSE.txt). Examples, C bindings,
benchmarks, CUDA kernels and development configuration are excluded. Other
Diffsol feature combinations are outside the supported QSPQuest build.

Diffsol is developed by Martin Robinson and contributors. Its source is based
on [Diffsol](https://github.com/martinjrobins/diffsol) commit
`8cd236f1cbd545796880c572683a8674b82df69c` with the included modifications.
Required copyright and license notices are preserved.

Rodas5P and Rosenbrock23 use a generic tableau-driven solver. The implementation
supports analytic time partials, bounded finite differences, incoming-side
forcing stops and a maximum step bound. QSPQuest marks dosing and forcing
discontinuities through this API. The deterministic feature pins transcendental
arithmetic to libm 0.2.16. Builder-provided structural Jacobian sparsity supports
coloring even when couplings are zero at the initial state.

BDF retains the accepted step's backward-difference polynomial for dense
observations. Stop-time clipping is applied before predicting the next step,
so rescaling for a short final step cannot change interpolation on the
previous accepted interval.

The retained source receipt records the shared numerical files;
`python scripts/check_diffsol_rosenbrock_sync.py` verifies their hashes.
The build hashes vendored files into the compiler identity and reports the
source hash in `backend_version`. See the [numerical qualification scope](../bindings/mcp/qualification/README.md)
for measured tolerances and limitations. Numerical agreement does not imply
dimensional or biological qualification.
