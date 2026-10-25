"""Run with: python3 -m unittest discover -s tests -p 'test_scenes_loader.py'

The contract the other slices rely on in tests/qmlhost/scenes.py: a `scenes_<slice>.py` module registers its scenes with
`from scenes import scene`, a duplicate name or a module that does not import is reported without taking the others
down, and a runner scene is called instead of the Home host. No host and no display are needed.
"""
import os
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "qmlhost"))
import scenes  # noqa: E402


class Loader(unittest.TestCase):
    def setUp(self):
        self.before = (set(scenes.SCENES), len(scenes.LOAD_ERRORS))

    def tearDown(self):
        for name in set(scenes.SCENES) - self.before[0]:
            del scenes.SCENES[name]
            del scenes.META[name]
        del scenes.LOAD_ERRORS[self.before[1]:]
        for name in [m for m in sys.modules if m.startswith("scenes_zz")]:
            del sys.modules[name]

    def load(self, **files):
        with tempfile.TemporaryDirectory() as d:
            for name, text in files.items():
                with open(os.path.join(d, name + ".py"), "w") as f:
                    f.write(text)
            scenes.load_modules(d)

    def test_a_module_registers_its_scenes_through_the_shared_registry(self):
        self.load(scenes_zz_a='from scenes import scene, READY\n@scene("zz-a")\ndef _():\n    return [("snap", READY, "zz-a")]\n')
        self.assertEqual(scenes.SCENES["zz-a"](), [("snap", scenes.READY, "zz-a")])
        self.assertEqual(scenes.META["zz-a"], {"runner": False, "materials": ("dark", "light")})
        self.assertEqual(scenes.LOAD_ERRORS[self.before[1]:], [])

    def test_a_duplicate_name_and_a_module_that_does_not_import_are_reported_and_the_rest_load(self):
        one = 'from scenes import scene\n@scene("zz-one")\ndef _():\n    return []\n'
        self.load(scenes_zz_1=one, scenes_zz_2=one, scenes_zz_3="def broken(:\n", scenes_zz_4=one.replace("zz-one", "zz-four"))
        self.assertIn("zz-one", scenes.SCENES)
        self.assertIn("zz-four", scenes.SCENES)
        errors = scenes.LOAD_ERRORS[self.before[1]:]
        self.assertEqual(len(errors), 2, errors)
        self.assertTrue(errors[0].startswith("scenes_zz_2: ValueError") and "registered twice" in errors[0])
        self.assertTrue(errors[1].startswith("scenes_zz_3: SyntaxError"))

    def test_a_runner_scene_renders_itself(self):
        self.load(scenes_zz_r='from scenes import scene\n@scene("zz-r", runner=True, materials=("dark",))\n'
                              'def _(out_dir, material):\n    return 0, [], "ran %s %s" % (out_dir, material)\n')
        self.assertEqual(scenes.META["zz-r"], {"runner": True, "materials": ("dark",)})
        self.assertEqual(scenes.run_scene("zz-r", "/tmp/x", "dark"), (0, [], "ran /tmp/x dark"))


if __name__ == "__main__":
    unittest.main()
