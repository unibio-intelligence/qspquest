"""Generate the independent Robertson comparator with SciPy Radau.

Numerical reference prepared by UniBio Intelligence, not exact truth or clinical evidence.
Copyright 2026 UniBio Intelligence. Licensed under Apache-2.0.
"""
import json
from pathlib import Path
import scipy
from scipy.integrate import solve_ivp

times = [0, .01, .02, .04, .1, .4, 1, 4, 10, 40, 100, 1000, 10000]

def rhs(t, y):
    a = -.04 * y[0] + 1e4 * y[1] * y[2]
    c = 3e7 * y[1] ** 2
    return [a, -a-c, c]

solution = solve_ivp(rhs, [0, 10000], [1, 0, 0], method='Radau',
                     rtol=2.3e-14, atol=1e-16, t_eval=times)
assert solution.success
Path(__file__).with_name('robertson-radau.json').write_text(json.dumps({
    'provenance': 'Numerical comparator prepared by UniBio Intelligence; classic Robertson equations; not exact or clinical evidence',
    'scipy_version': scipy.__version__, 'method': 'Radau', 'rtol': 2.3e-14,
    'atol': 1e-16, 'times': times, 'states': solution.y.T.tolist()
}, indent=2) + '\n')
