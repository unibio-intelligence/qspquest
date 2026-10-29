import copy
import json
import unittest
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import qspquest

ROOT=Path(__file__).resolve().parents[3]
SOURCE=(ROOT/'docs/examples/profiled-proportional.qsp').read_text()
def request():return json.loads((ROOT/'docs/examples/profiled-proportional-fit.json').read_text())
def model():return qspquest.CompiledSensitivities(SOURCE,['theta'])

class ProfiledProportionalTests(unittest.TestCase):
    def test_versioned_fit_recovers_two_independent_scales(self):
        result=model().fit(request());fit=result['fit']
        self.assertEqual(fit['status'],'converged')
        self.assertAlmostEqual(fit['parameters']['theta']['value'],2.5,places=5)
        self.assertAlmostEqual(result['residual_scales']['y1'],0.0894427191,places=7)
        self.assertAlmostEqual(result['residual_scales']['y2'],0.1442220510,places=7)

    def test_planning_checks_profiled_rows_without_solver_work(self):
        compiled=model();valid=request();valid['problem']['request']['objective']['simulation']['run']['budgets']['solver_callbacks']=1
        plan=compiled.plan_fit(valid)
        self.assertEqual(plan['request']['problem']['kind'],'profiled_proportional')
        invalid=copy.deepcopy(valid)
        invalid['problem']['request']['objective']['observations'][0]['error']['proportional_sd']=.5
        with self.assertRaises(qspquest.QSPQuestError):compiled.plan_fit(invalid)
        with self.assertRaises(qspquest.QSPQuestError):compiled.fit(invalid)

    def test_profiled_result_cannot_be_bound_as_fixed_error_evidence(self):
        # The identity helper accepts native model JSON, so exercise its
        # evidence guard with a fully compiled JSON model and valid result.
        document=dict(schema='qspquest.model/v0.1',model_id='profiled_normal',description='',time_unit='h',
            parameters=[dict(name='theta',default=dict(value=1.8,unit='mg/L'),bounds=None,transform='identity',fixed=False,description=''),
                dict(name='decay',default=dict(value=1.,unit='1/h'),bounds=None,transform='identity',fixed=True,description='')],
            states=[dict(name='unused',unit='1',initial=dict(kind='quantity',quantity=dict(value=1.,unit='1')),
                rhs=dict(kind='binary',operator='multiply',arguments=[
                    dict(kind='unary',operator='negative',arguments=[dict(kind='symbol',name='decay')]),
                    dict(kind='symbol',name='unused')]),dosing=None)],
            outputs=[dict(name='y1',unit='mg/L',expression=dict(kind='symbol',name='theta')),
                dict(name='y2',unit='mg/L',expression=dict(kind='binary',operator='multiply',
                    arguments=[dict(kind='literal',value=2.),dict(kind='symbol',name='theta')]))])
        self.assertEqual(qspquest.CompiledModel(document).model_content_hash,qspquest.CompiledModel(SOURCE).model_content_hash)
        result=qspquest.CompiledSensitivities(document,['theta']).fit(request())
        known=request();known['problem']['kind']='individual'
        with self.assertRaisesRegex(qspquest.QSPQuestError,'without profiled residual scales'):
            qspquest.fitting_identity(document,known,'fit_individual',result=result)

    def test_evaluation_limit_is_incomplete(self):
        limited=request();limited['problem']['request']['max_evaluations']=1
        result=model().fit(limited)
        self.assertEqual(result['fit']['status'],'evaluation_limit')
        self.assertEqual(result['fit']['evaluations'],1)

    def test_concurrent_fits_share_no_mutable_scale_state(self):
        compiled=model();expected=compiled.fit(request())
        with ThreadPoolExecutor(max_workers=2) as pool:
            results=list(pool.map(compiled.fit,[request() for _ in range(4)]))
        for result in results:self.assertEqual(result,expected)

if __name__=='__main__':unittest.main()
