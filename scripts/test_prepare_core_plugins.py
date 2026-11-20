import importlib.util
import json
import pathlib
import tempfile
import unittest
from unittest.mock import patch
from types import SimpleNamespace
spec = importlib.util.spec_from_file_location('prepare', pathlib.Path(__file__).with_name('prepare-core-plugins.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
class Upgrade(unittest.TestCase):
    def fixture(self, root):
        source, target = root/'builtin'/'qqmusic', root/'installed'/'qqmusic'
        source.mkdir(parents=True);target.mkdir(parents=True)
        for path, version in [(source,'0.1.14'),(target,'0.1.12')]:
            (path/'manifest.json').write_text(json.dumps({'id':'qqmusic','version':version}))
            (path/'provider.py').write_text(version)
            (path/'requirements.txt').write_text('package==1.0\n')
        (target/'credentials.json').write_text('fixture-private-state')
        return source,target
    def test_upgrade_preserves_state_and_backs_up_only_program(self):
        with tempfile.TemporaryDirectory() as folder:
            source,target=self.fixture(pathlib.Path(folder))
            with patch.object(module.subprocess,'run',return_value=SimpleNamespace(returncode=0,stdout='{"result":{"status":"ready"}}')):
                module.upgrade_source(source,target)
            self.assertEqual((target/'provider.py').read_text(),'0.1.14')
            self.assertEqual((target/'credentials.json').read_text(),'fixture-private-state')
            backups=list((target/'.source-backups').iterdir());self.assertEqual(len(backups),1)
            self.assertEqual({p.name for p in backups[0].iterdir()},{'provider.py','manifest.json','requirements.txt'})
            self.assertEqual((backups[0]/'provider.py').read_text(),'0.1.12')
    def test_health_failure_keeps_previous_source(self):
        with tempfile.TemporaryDirectory() as folder:
            source,target=self.fixture(pathlib.Path(folder))
            with patch.object(module.subprocess,'run',return_value=SimpleNamespace(returncode=1,stdout='')):
                with self.assertRaises(RuntimeError):module.upgrade_source(source,target)
            self.assertEqual((target/'provider.py').read_text(),'0.1.12')
    def test_support_module_is_published_before_entrypoint(self):
        with tempfile.TemporaryDirectory() as folder:
            source,target=self.fixture(pathlib.Path(folder));(source/'qq_artwork.py').write_text('helper')
            order=[];real_replace=module.os.replace
            def replace(a,b):order.append(b.name);real_replace(a,b)
            with patch.object(module.subprocess,'run',return_value=SimpleNamespace(returncode=0,stdout='{"result":{"status":"ready"}}')),patch.object(module.os,'replace',side_effect=replace):module.upgrade_source(source,target)
            self.assertLess(order.index('qq_artwork.py'),order.index('provider.py'));self.assertEqual((target/'qq_artwork.py').read_text(),'helper');self.assertEqual((target/'credentials.json').read_text(),'fixture-private-state')
if __name__=='__main__':unittest.main()
