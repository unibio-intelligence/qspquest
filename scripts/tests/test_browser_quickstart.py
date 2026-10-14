"""Reject changed or unavailable model-library inputs before browser packaging."""
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
import zipfile

spec=importlib.util.spec_from_file_location('browser_packager',Path(__file__).resolve().parents[1]/'build_browser_quickstart.py')
packager=importlib.util.module_from_spec(spec);spec.loader.exec_module(packager)

class LibraryBoundary(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup);self.root=Path(self.temp.name)
        self.members={'native/model.json':b'{"model_id":"synthetic"}',
                 'native/run.json':json.dumps({'schema':'qspquest.run/v0.1','budgets':{'events':10000,'output_values':30000,'solver_callbacks':1000000}}).encode(),
                 'LICENSE':b'Synthetic test notice\n','manifest.xml':b'<manifest/>','NOTICE':b'Synthetic test fixture\n'}
        self.index={'profile':'ubi.omex.rust-ode/v1','model_id':'synthetic','model_version':'0.1',
               'scenarios':[{'scenario_id':'example'}],
               'native_execution':{'schema':'qspquest.models.native-execution/v1','model':'native/model.json','scenarios':{'example':'native/run.json'},'outputs':{}}}
        self.archive=self.root/'model.omex'
        self.row={'id':'synthetic','version':'0.1','name':'Synthetic','description':'Test only','scenario_ids':['example'],
                  'archive':'model.omex','license':'Apache-2.0'}
        self.repack()
    def repack(self):
        self.index['members']={n:hashlib.sha256(v).hexdigest() for n,v in self.members.items() if n!='manifest.xml'}
        raw=io.BytesIO()
        with zipfile.ZipFile(raw,'w') as archive:
            for n,v in self.members.items():archive.writestr(n,v)
            archive.writestr('index.json',json.dumps(self.index))
        self.archive.write_bytes(raw.getvalue())
        self.row['omex_sha256']=hashlib.sha256(raw.getvalue()).hexdigest()
        self.catalog()
    def catalog(self): (self.root/'catalog.json').write_text(json.dumps({'models':[self.row]}))
    def test_fixed_model_and_scenario_preserved_with_lower_budget(self):
        model,legal=packager.library_entry(self.root,'synthetic','0.1','example')
        self.assertEqual(model['source'],'{"model_id":"synthetic"}')
        self.assertEqual(model['request']['budgets']['output_values'],20000)
        self.assertEqual(legal['library/synthetic/0.1/LICENSE'],b'Synthetic test notice\n')
    def test_changed_archive_rejected(self):
        self.archive.write_bytes(self.archive.read_bytes()+b'changed')
        with self.assertRaisesRegex(ValueError,'checksum'):packager.library_entry(self.root,'synthetic','0.1','example')
    def test_optional_catalog_description_does_not_block_packaging(self):
        del self.row['description'];self.catalog()
        model,legal=packager.library_entry(self.root,'synthetic','0.1','example')
        self.assertEqual(model['description'],'')
        self.assertEqual(model['name'],'Synthetic')
        self.assertIn('library/synthetic/0.1/LICENSE',legal)
    def test_unavailable_scenario_rejected(self):
        with self.assertRaisesRegex(ValueError,'available native'):packager.library_entry(self.root,'synthetic','0.1','unknown')
    def test_catalog_cannot_relabel_archive_model_or_version(self):
        for field,value in [('id','different_model'),('version','9.9.9')]:
            with self.subTest(field=field):
                original=self.row[field];self.row[field]=value;self.catalog()
                with self.assertRaisesRegex(ValueError,'Archive model ID or version'):
                    packager.library_entry(self.root,self.row['id'],self.row['version'],'example')
                self.row[field]=original;self.catalog()
    def test_archive_identity_is_required(self):
        for field in ('model_id','model_version'):
            with self.subTest(field=field):
                original=self.index.pop(field);self.repack()
                with self.assertRaisesRegex(ValueError,'Archive model ID or version'):
                    packager.library_entry(self.root,'synthetic','0.1','example')
                self.index[field]=original;self.repack()
    def test_native_model_identity_must_match_even_with_valid_checksums(self):
        for model in ({'model_id':'different_model'},{}):
            with self.subTest(model=model):
                self.members['native/model.json']=json.dumps(model).encode();self.repack()
                with self.assertRaisesRegex(ValueError,'Native model ID'):
                    packager.library_entry(self.root,'synthetic','0.1','example')
    def test_selected_scenario_must_be_declared_once_in_both_inventories(self):
        for inventory in ('catalog','index'):
            for ids in ([],['different'],['example','example']):
                with self.subTest(inventory=inventory,ids=ids):
                    self.row['scenario_ids']=ids if inventory=='catalog' else ['example']
                    self.index['scenarios']=[{'scenario_id':s} for s in (ids if inventory=='index' else ['example'])]
                    self.repack()
                    with self.assertRaisesRegex(ValueError,'declared exactly once'):
                        packager.library_entry(self.root,'synthetic','0.1','example')
    def test_declared_unavailable_scenario_rejected(self):
        self.index['unavailable_scenario_ids']=['example'];self.repack()
        with self.assertRaisesRegex(ValueError,'unavailable for execution'):
            packager.library_entry(self.root,'synthetic','0.1','example')
    def test_scenario_inventory_cannot_be_a_string(self):
        self.row['scenario_ids']='example';self.catalog()
        with self.assertRaisesRegex(ValueError,'inventories must be lists'):
            packager.library_entry(self.root,'synthetic','0.1','example')
    def test_catalog_path_traversal_rejected(self):
        self.row['archive']='../model.omex';self.catalog()
        with self.assertRaisesRegex(ValueError,'outside'):packager.library_entry(self.root,'synthetic','0.1','example')
    def test_unsafe_requested_identifier_rejected(self):
        with self.assertRaisesRegex(ValueError,'plain model'):packager.library_entry(self.root,'../synthetic','0.1','example')

if __name__=='__main__':unittest.main()
