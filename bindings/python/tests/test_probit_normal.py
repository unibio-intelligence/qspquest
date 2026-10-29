import copy
import math
import unittest
from concurrent.futures import ThreadPoolExecutor
import qspquest

def request():
    return dict(schema='qspquest.probit-normal-binomial-fit/v0.1',
        observations=[dict(successes=y,total=2,group=g) for g in range(2) for y in range(3)],
        initial_means=[0.,0.],mean_bounds=[[-4.,4.],[-4.,4.]],initial_sds=[0.,0.],sd_bounds=[[0.,3.],[0.,3.]],
        max_evaluations=1000,max_iterations=200,gradient_tolerance=1e-8,max_quadrature_evaluations=2000000)

class ProbitNormalTests(unittest.TestCase):
    def test_zero_sd_start_recovers_known_litter_distribution(self):
        result=qspquest.fit_probit_normal_binomial(request())
        self.assertEqual(result['status'],'converged');self.assertEqual(result['observations'],6)
        self.assertAlmostEqual(result['negative_log_likelihood'],6*math.log(3),places=10)
        for mean,sd in zip(result['means'],result['sds']):
            self.assertAlmostEqual(mean,0.,places=8);self.assertAlmostEqual(sd,1.,places=7)

    def test_invalid_counts_groups_and_budget_refuse_then_recover(self):
        for key,value in [('successes',1.5),('successes',-1),('successes',3),('group',2),('total',0)]:
            invalid=request();invalid['observations'][0][key]=value
            with self.assertRaises(qspquest.QSPQuestError):qspquest.fit_probit_normal_binomial(invalid)
        invalid=request();invalid['max_quadrature_evaluations']=1
        with self.assertRaises(qspquest.QSPQuestError):qspquest.fit_probit_normal_binomial(invalid)
        self.assertEqual(qspquest.fit_probit_normal_binomial(request())['status'],'converged')

    def test_evaluation_limit_is_incomplete(self):
        limited=request();limited['max_evaluations']=1
        result=qspquest.fit_probit_normal_binomial(limited)
        self.assertEqual(result['status'],'evaluation_limit');self.assertEqual(result['evaluations'],1)

    def test_concurrent_native_calls_are_deterministic(self):
        expected=qspquest.fit_probit_normal_binomial(request())
        with ThreadPoolExecutor(max_workers=2) as pool:
            actual=list(pool.map(qspquest.fit_probit_normal_binomial,[copy.deepcopy(request()) for _ in range(4)]))
        for row in actual:self.assertEqual(row,expected)

if __name__=='__main__':unittest.main()
