import copy
import math
import unittest
from concurrent.futures import ThreadPoolExecutor

import qspquest


def request():
    q=lambda value:{'value':value,'unit':'h'}
    return dict(schema='qspquest.gamma-poisson-fit/v0.1',
                observations=[dict(count=count,exposure=q(1.)) for count in [0,1]],
                initial_shape=1.,shape_bounds=[.01,100.],initial_rate=q(1.),rate_bounds=[q(.01),q(100.)],
                max_evaluations=1000,max_iterations=200,gradient_tolerance=1e-7)


class GammaPoissonTests(unittest.TestCase):
    def test_native_api_matches_bounded_closed_form(self):
        result=qspquest.fit_gamma_poisson(request())
        self.assertEqual(result['status'],'converged')
        self.assertEqual(result['parameters']['rate'],{'value':100.,'unit':'h'})
        self.assertAlmostEqual(result['parameters']['shape']['value'],1/(2*math.log1p(.01)),places=5)
        self.assertEqual(result['observations'],2)
        self.assertLessEqual(result['projected_gradient_norm'],1e-7)
        self.assertTrue(result['compiler_source_sha256'].startswith('sha256:'))

    def test_fractional_negative_and_incompatible_counts_refuse_then_recover(self):
        for value in [1.5,-1]:
            invalid=request();invalid['observations'][0]['count']=value
            with self.assertRaises(qspquest.QSPQuestError):qspquest.fit_gamma_poisson(invalid)
        invalid=request();invalid['observations'][0]['exposure']['unit']='mg'
        with self.assertRaises(qspquest.QSPQuestError):qspquest.fit_gamma_poisson(invalid)
        self.assertEqual(qspquest.fit_gamma_poisson(request())['status'],'converged')

    def test_work_limit_is_reported_as_incomplete(self):
        limited=request();limited['max_evaluations']=1
        result=qspquest.fit_gamma_poisson(limited)
        self.assertEqual(result['status'],'evaluation_limit')
        self.assertEqual(result['evaluations'],1)

    def test_independent_native_calls_are_deterministic(self):
        baseline=qspquest.fit_gamma_poisson(request())
        with ThreadPoolExecutor(max_workers=2) as pool:
            results=list(pool.map(qspquest.fit_gamma_poisson,[copy.deepcopy(request()) for _ in range(4)]))
        for result in results:self.assertEqual(result,baseline)


if __name__=='__main__':unittest.main()
