import copy
import json
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from package import PackageError, Reader, pack, replace_parameters, sha256


def manifest():
    # Serialization fixture only: these bytes are never submitted to hardware.
    return {'npu_type': 1, 'inputs': [{'type': 8, 'shape': [1, 4, 4, 1]}],
            'outputs': [{'type': 8, 'shape': [1, 4, 4, 1]}], 'feature_bytes': 4096,
            'strides': [{'type': i, 'value': 32} for i in range(5)],
            'registers': [{'address': 0xCC880300, 'value': 1}],
            'input_offsets': [0], 'output_offsets': [0]}


class PackageTests(unittest.TestCase):
    def setUp(self):
        self.meta = manifest()
        self.program = bytes(range(32))
        self.parameters = bytes(range(64))
        self.data = pack(self.meta, self.program, self.parameters)

    def test_roundtrip_metadata_and_streams(self):
        report = Reader(self.data).describe()
        self.assertEqual(report['manifest'], self.meta)
        self.assertFalse(report['hardware_execution_verified'])
        for name, stream in [('program', self.program), ('parameters', self.parameters)]:
            loc = report['streams'][name]
            self.assertEqual(self.data[loc['offset']:loc['offset'] + loc['bytes']], stream)
            self.assertEqual(loc['sha256'], sha256(stream))

    def test_multiple_tensors(self):
        self.meta['inputs'].append({'type': 6, 'shape': [1, 8]})
        self.meta['input_offsets'].append(4096)
        self.assertEqual(Reader(pack(self.meta, self.program, self.parameters)).describe()['manifest'], self.meta)

    def test_replace_preserves_every_other_byte(self):
        loc = Reader(self.data).describe()['streams']['parameters']
        modified = replace_parameters(self.data, bytes(64), sha256(self.data))
        start, end = loc['offset'], loc['offset'] + loc['bytes']
        self.assertEqual(modified[:start], self.data[:start])
        self.assertEqual(modified[end:], self.data[end:])
        self.assertEqual(modified[start:end], bytes(64))
        self.assertEqual(Reader(modified).describe()['manifest'], self.meta)

    def test_replace_rejects_wrong_hash(self):
        with self.assertRaises(PackageError):
            replace_parameters(self.data, bytes(64), '0' * 64)

    def test_replace_rejects_length_change(self):
        with self.assertRaises(PackageError):
            replace_parameters(self.data, bytes(63), sha256(self.data))

    def test_truncated_package(self):
        for size in (0, 1, 4, 8, 24, 90, len(self.data) // 2):
            with self.subTest(size=size), self.assertRaises(PackageError):
                Reader(self.data[:size]).describe()

    def test_root_out_of_bounds(self):
        data = bytearray(self.data)
        struct.pack_into('<I', data, 0, 0xFFFFFFFF)
        with self.assertRaises(PackageError):
            Reader(data)

    def test_vector_length_out_of_bounds(self):
        reader = Reader(self.data)
        start, _ = reader.blob(3)
        data = bytearray(self.data)
        struct.pack_into('<I', data, start - 4, 0xFFFFFFFF)
        with self.assertRaises(PackageError):
            Reader(data).describe()

    def test_field_outside_object(self):
        data = bytearray(self.data)
        reader = Reader(data)
        vt, _, _ = reader.table(reader.root)
        struct.pack_into('<H', data, vt + 10, 65535)
        with self.assertRaises(PackageError):
            Reader(data).describe()

    def test_rejects_other_npu(self):
        self.meta['npu_type'] = 2
        with self.assertRaises(PackageError):
            pack(self.meta, self.program, self.parameters)

    def test_rejects_unknown_tensor_type(self):
        self.meta['inputs'][0]['type'] = 0
        with self.assertRaises(PackageError):
            pack(self.meta, self.program, self.parameters)

    def test_rejects_zero_and_huge_dimensions(self):
        for shape in ([1, 0], [65536], [65535, 65535]):
            self.meta['inputs'][0]['shape'] = shape
            with self.assertRaises(PackageError):
                pack(self.meta, self.program, self.parameters)

    def test_rejects_outside_or_unaligned_registers(self):
        for addr in (0xCC080000, 0xCC880301, 0xCC8828C0, 0x300):
            self.meta['registers'][0]['address'] = addr
            with self.assertRaises(PackageError):
                pack(self.meta, self.program, self.parameters)

    def test_rejects_missing_duplicate_zero_strides(self):
        for strides in (self.meta['strides'][:-1], [self.meta['strides'][0]] * 5,
                        [{'type': i, 'value': 0} for i in range(5)]):
            m = copy.deepcopy(self.meta)
            m['strides'] = strides
            with self.assertRaises(PackageError):
                pack(m, self.program, self.parameters)

    def test_rejects_offset_count_mismatch(self):
        self.meta['input_offsets'] = []
        with self.assertRaises(PackageError):
            pack(self.meta, self.program, self.parameters)

    def test_rejects_empty_streams(self):
        for program, params in ((b'', self.parameters), (self.program, b'')):
            with self.assertRaises(PackageError):
                pack(self.meta, program, params)

    def test_cli_extract_pack_and_refuse_overwrite(self):
        with tempfile.TemporaryDirectory() as directory:
            d = Path(directory)
            source = d / 'source.lgnpu'
            source.write_bytes(self.data)
            extract = d / 'extracted'
            cmd = [sys.executable, str(ROOT / 'package.py')]
            subprocess.run(cmd + ['extract', str(source), str(extract)], check=True, capture_output=True)
            self.assertEqual((extract / 'program.bin').read_bytes(), self.program)
            out = d / 'new.lgnpu'
            build = cmd + ['pack', str(extract / 'manifest.json'), str(extract / 'program.bin'),
                           str(extract / 'parameters.bin'), str(out)]
            subprocess.run(build, check=True, capture_output=True)
            self.assertEqual(Reader(out.read_bytes()).describe()['manifest'], self.meta)
            self.assertNotEqual(subprocess.run(build, capture_output=True).returncode, 0)
            self.assertEqual(Reader(out.read_bytes()).describe()['manifest'], self.meta)


@unittest.skipUnless(os.environ.get('LGNPU_LIBRARY'), 'LG library supplied locally, never committed')
class OriginalRuntimeTests(unittest.TestCase):
    def test_generated_package_accepted_by_lg_arm_code(self):
        from vendor_verify import verify
        data = pack(manifest(), bytes(range(32)), bytes(64))
        self.assertTrue(verify(os.environ['LGNPU_LIBRARY'], data))

    def test_truncated_package_rejected_by_lg_arm_code(self):
        from vendor_verify import verify
        data = pack(manifest(), bytes(range(32)), bytes(64))
        self.assertFalse(verify(os.environ['LGNPU_LIBRARY'], data[:90]))

    def test_changed_parameters_accepted_by_lg_arm_code(self):
        from vendor_verify import verify
        data = pack(manifest(), bytes(range(32)), bytes(64))
        modified = replace_parameters(data, bytes(range(64)), sha256(data))
        self.assertTrue(verify(os.environ['LGNPU_LIBRARY'], modified))

    def test_other_library_refused(self):
        from vendor_verify import verify
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'wrong.so'
            path.write_bytes(b'not the pinned library')
            with self.assertRaisesRegex(ValueError, 'unknown libnpu'):
                verify(path, bytes(64))


@unittest.skipUnless((ROOT / 'build/host/lumatv-npu-probe').exists(), 'build.sh required')
class RuntimeProbeTests(unittest.TestCase):
    def run_probe(self, args=(), enabled=False):
        env = dict(os.environ)
        env.pop('LUMATV_NPU_EXPERIMENT', None)
        if enabled:
            env['LUMATV_NPU_EXPERIMENT'] = '1'
        return subprocess.run([str(ROOT / 'build/host/lumatv-npu-probe'), *args],
                              env=env, capture_output=True, text=True)

    def test_default_is_disabled(self):
        p = self.run_probe()
        self.assertEqual(p.returncode, 0)
        self.assertIn('disabled', p.stdout)

    def test_load_requires_explicit_switch(self):
        p = self.run_probe(['--symbols', '/does/not/exist'])
        self.assertEqual(p.returncode, 2)
        self.assertIn('explicit experiment', p.stderr)

    def test_absolute_path_required(self):
        p = self.run_probe(['--symbols', 'relative.so'], enabled=True)
        self.assertEqual(p.returncode, 2)
        self.assertIn('absolute', p.stderr)

    def test_failed_load_is_not_success(self):
        p = self.run_probe(['--symbols', '/does/not/exist'], enabled=True)
        self.assertEqual(p.returncode, 1)
        self.assertIn('dlopen', p.stderr)


if __name__ == '__main__':
    unittest.main()
