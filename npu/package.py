#!/usr/bin/env python3
"""Offline DNE package inspection/assembly. Never opens an NPU or runs a model.

Wire layout recovered from libnpu.so.1.0.0, SHA256 in npu-result.md.
This assembles compiled streams; it does not compile neural-network operators.
"""
import argparse
import hashlib
import json
import struct
from pathlib import Path

MAX_BYTES = 64 * 1024 * 1024
MAX_ITEMS = 65536
# Only these package tensor type codes are accepted by this libnpu's parser.
TENSOR_BYTES = {2: 1, 6: 4, 8: 1}
REGIONS = ((0xCC880000, 0xCC8800FC), (0xCC880100, 0xCC8801FC),
           (0xCC880200, 0xCC88020C), (0xCC880300, 0xCC88094C),
           (0xCC881200, 0xCC88184C), (0xCC882100, 0xCC8822BC),
           (0xCC882700, 0xCC8828BC))


class PackageError(ValueError):
    pass


def sha256(data):
    return hashlib.sha256(data).hexdigest()


class Reader:
    def __init__(self, data):
        if not 8 <= len(data) <= MAX_BYTES:
            raise PackageError('package size outside 8 bytes..64 MiB')
        self.data = data
        self.root = self.u32(0)
        if self.root < 4 or self.root % 4:
            raise PackageError('invalid root offset')
        self.table(self.root)

    def need(self, pos, size):
        if pos < 0 or size < 0 or pos > len(self.data) - size:
            raise PackageError('truncated or out-of-range field')

    def scalar(self, fmt, pos):
        self.need(pos, struct.calcsize(fmt))
        return struct.unpack_from(fmt, self.data, pos)[0]

    def u32(self, pos):
        return self.scalar('<I', pos)

    def table(self, pos):
        if pos % 4:
            raise PackageError('unaligned table')
        vt = pos - self.scalar('<i', pos)
        self.need(vt, 4)
        length, size = struct.unpack_from('<HH', self.data, vt)
        if vt % 2 or length < 4 or length % 2 or size < 4:
            raise PackageError('invalid vtable')
        self.need(vt, length)
        self.need(pos, size)
        return vt, length, size

    def field(self, pos, index, size=4, required=True):
        vt, length, obj_size = self.table(pos)
        slot = 4 + 2 * index
        offset = self.scalar('<H', vt + slot) if slot < length else 0
        if offset == 0:
            if required:
                raise PackageError(f'missing field {index}')
            return None
        if offset < 4 or offset > obj_size - size:
            raise PackageError('field outside table object')
        self.need(pos + offset, size)
        return pos + offset

    def indirect(self, pos):
        offset = self.u32(pos)
        if offset < 4 or offset >= 0x80000000:
            raise PackageError('invalid forward offset')
        self.need(pos + offset, 4)
        return pos + offset

    def child(self, pos, index):
        child = self.indirect(self.field(pos, index))
        self.table(child)
        return child

    def vector(self, pos, index, width):
        vec = self.indirect(self.field(pos, index))
        if vec % 4:
            raise PackageError('unaligned vector')
        count = self.u32(vec)
        if count > MAX_BYTES // width or (width > 1 and count > MAX_ITEMS):
            raise PackageError('oversized vector')
        self.need(vec + 4, count * width)
        return vec + 4, count

    def blob(self, index):
        tab = self.child(self.root, index)
        start, size = self.vector(tab, 2, 1)
        if not size:
            raise PackageError('empty program or parameter stream')
        return start, size

    def tensors(self, index):
        start, count = self.vector(self.root, index, 4)
        if not 1 <= count <= 32:
            raise PackageError('tensor count outside 1..32')
        result = []
        for i in range(count):
            tab = self.indirect(start + i * 4)
            typ_pos = self.field(tab, 1, 1, required=False)
            typ = self.scalar('<b', typ_pos) if typ_pos is not None else 0
            dims, rank = self.vector(tab, 0, 2)
            if not 1 <= rank <= 8 or typ not in TENSOR_BYTES:
                raise PackageError('unsupported tensor type or rank')
            shape = list(struct.unpack_from('<' + 'H' * rank, self.data, dims))
            size = TENSOR_BYTES[typ]
            for dim in shape:
                if not dim:
                    raise PackageError('zero tensor dimension')
                size *= dim
            if size > MAX_BYTES:
                raise PackageError('tensor exceeds 64 MiB')
            result.append({'type': typ, 'shape': shape})
        return result

    def describe(self):
        typ_pos = self.field(self.root, 0, 1, required=False)
        typ = self.scalar('<b', typ_pos) if typ_pos is not None else 0
        if typ != 1 or self.scalar('<B', self.field(self.root, 6, 1)) != 2:
            raise PackageError('only DNE packages with DNE private data are supported')
        priv = self.child(self.root, 7)
        manifest = {'npu_type': typ, 'inputs': self.tensors(1), 'outputs': self.tensors(2),
                    'feature_bytes': self.u32(self.field(self.root, 5))}
        start, count = self.vector(priv, 2, 4)
        manifest['strides'] = [{'type': self.scalar('<b', start + i * 4),
                                'value': self.scalar('<H', start + i * 4 + 2)} for i in range(count)]
        start, count = self.vector(priv, 3, 8)
        manifest['registers'] = [{'address': self.u32(start + i * 8),
                                  'value': self.u32(start + i * 8 + 4)} for i in range(count)]
        for index, key in ((4, 'input_offsets'), (5, 'output_offsets')):
            start, count = self.vector(priv, index, 4)
            manifest[key] = list(struct.unpack_from('<' + 'I' * count, self.data, start))
        streams = {}
        for index, key in ((3, 'program'), (4, 'parameters')):
            start, count = self.blob(index)
            streams[key] = {'offset': start, 'bytes': count,
                            'sha256': sha256(self.data[start:start + count])}
        return {'manifest': manifest, 'streams': streams,
                'sha256': sha256(self.data), 'bytes': len(self.data),
                'hardware_execution_verified': False}


def integer(value, low, high, name):
    if isinstance(value, bool) or not isinstance(value, int) or not low <= value <= high:
        raise PackageError(f'{name} outside {low}..{high}')
    return value


def validate_manifest(m):
    if m.get('npu_type') != 1:
        raise PackageError('npu_type must be 1 (DNE)')
    integer(m['feature_bytes'], 1, MAX_BYTES, 'feature_bytes')
    for key in ('inputs', 'outputs'):
        if not 1 <= len(m[key]) <= 32:
            raise PackageError('tensor count outside 1..32')
        for t in m[key]:
            if t['type'] not in TENSOR_BYTES or not 1 <= len(t['shape']) <= 8:
                raise PackageError('unsupported tensor type or rank')
            size = TENSOR_BYTES[t['type']]
            for d in t['shape']:
                size *= integer(d, 1, 65535, 'dimension')
            if size > MAX_BYTES:
                raise PackageError('tensor exceeds 64 MiB')
    if sorted(s['type'] for s in m['strides']) != list(range(5)):
        raise PackageError('exactly one stride each for program, parameter, feature, input, output is required')
    for s in m['strides']:
        integer(s['value'], 1, 65535, 'stride')
    if not 1 <= len(m['registers']) <= MAX_ITEMS:
        raise PackageError('register count outside 1..65536')
    for r in m['registers']:
        a = integer(r['address'], 0, 0xFFFFFFFF, 'register address')
        if a % 4 or not any(lo <= a <= hi for lo, hi in REGIONS):
            raise PackageError('register address outside published O22 DNE blocks')
        integer(r['value'], 0, 0xFFFFFFFF, 'register value')
    for key, tensors in (('input_offsets', 'inputs'), ('output_offsets', 'outputs')):
        if len(m[key]) != len(m[tensors]):
            raise PackageError('offset count must match tensor count')
        for offset in m[key]:
            integer(offset, 0, 0xFFFFFFFF, key)


def pack(manifest, program, parameters):
    """Wrap existing compiled bytes. Requires recovered strides/registers, never guesses them."""
    import flatbuffers
    validate_manifest(manifest)
    if not program or not parameters or len(program) + len(parameters) > MAX_BYTES // 2:
        raise PackageError('streams must be nonempty and total at most 32 MiB')
    b = flatbuffers.Builder(1024)

    def vector(values, width, method):
        b.StartVector(width, len(values), min(width, 4))
        for value in reversed(values):
            getattr(b, method)(value)
        return b.EndVector()

    def tensors(items):
        offsets = []
        for t in items:
            shape = vector(t['shape'], 2, 'PrependUint16')
            b.StartObject(2)
            b.PrependUOffsetTRelativeSlot(0, shape, 0)
            b.PrependInt8Slot(1, t['type'], 0)
            offsets.append(b.EndObject())
        return vector(offsets, 4, 'PrependUOffsetTRelative')

    def blob(data):
        data = b.CreateByteVector(data)
        b.StartObject(3)
        b.PrependUOffsetTRelativeSlot(2, data, 0)
        return b.EndObject()

    inputs, outputs = tensors(manifest['inputs']), tensors(manifest['outputs'])
    prog, params = blob(program), blob(parameters)
    b.StartVector(4, len(manifest['strides']), 4)
    for s in reversed(manifest['strides']):
        b.PrependUint16(s['value'])
        b.PrependUint8(0)
        b.PrependInt8(s['type'])
    strides = b.EndVector()
    b.StartVector(8, len(manifest['registers']), 4)
    for r in reversed(manifest['registers']):
        b.PrependUint32(r['value'])
        b.PrependUint32(r['address'])
    registers = b.EndVector()
    ins = vector(manifest['input_offsets'], 4, 'PrependUint32')
    outs = vector(manifest['output_offsets'], 4, 'PrependUint32')
    b.StartObject(6)
    for slot, value in ((2, strides), (3, registers), (4, ins), (5, outs)):
        b.PrependUOffsetTRelativeSlot(slot, value, 0)
    private = b.EndObject()
    b.StartObject(8)
    b.PrependInt8Slot(0, 1, 0)
    for slot, value in ((1, inputs), (2, outputs), (3, prog), (4, params), (7, private)):
        b.PrependUOffsetTRelativeSlot(slot, value, 0)
    b.PrependUint32Slot(5, manifest['feature_bytes'], 0)
    b.PrependUint8Slot(6, 2, 0)
    root = b.EndObject()
    b.Finish(root)
    result = bytes(b.Output())
    Reader(result).describe()
    return result


def replace_parameters(data, replacement, expected_sha256):
    if sha256(data) != expected_sha256.lower():
        raise PackageError('source SHA256 mismatch')
    reader = Reader(data)
    reader.describe()
    start, size = reader.blob(4)
    if len(replacement) != size:
        raise PackageError('parameter length must stay unchanged')
    # Preserve every byte outside the parameter vector, including unknown metadata.
    return data[:start] + replacement + data[start + size:]


def main():
    p = argparse.ArgumentParser(description=__doc__)
    subs = p.add_subparsers(dest='command', required=True)
    inspect = subs.add_parser('inspect')
    inspect.add_argument('package', type=Path)
    extract = subs.add_parser('extract')
    extract.add_argument('package', type=Path)
    extract.add_argument('directory', type=Path)
    build = subs.add_parser('pack')
    build.add_argument('manifest', type=Path)
    build.add_argument('program', type=Path)
    build.add_argument('parameters', type=Path)
    build.add_argument('output', type=Path)
    patch = subs.add_parser('replace-parameters')
    patch.add_argument('package', type=Path)
    patch.add_argument('parameters', type=Path)
    patch.add_argument('output', type=Path)
    patch.add_argument('--sha256', required=True)
    args = p.parse_args()
    try:
        if args.command == 'pack':
            data = pack(json.loads(args.manifest.read_text()), args.program.read_bytes(), args.parameters.read_bytes())
        else:
            data = args.package.read_bytes()
        if args.command == 'replace-parameters':
            data = replace_parameters(data, args.parameters.read_bytes(), args.sha256)
        report = Reader(data).describe()
        if args.command in ('pack', 'replace-parameters'):
            with args.output.open('xb') as f:
                f.write(data)
        if args.command == 'extract':
            args.directory.mkdir()  # Refuse overwriting an existing extraction.
            (args.directory / 'manifest.json').write_text(json.dumps(report['manifest'], indent=2) + '\n')
            for key, meta in report['streams'].items():
                (args.directory / (key + '.bin')).write_bytes(data[meta['offset']:meta['offset'] + meta['bytes']])
        print(json.dumps(report, indent=2))
    except (PackageError, OSError, KeyError, TypeError, ValueError) as exc:
        p.exit(1, f'{exc}\n')


if __name__ == '__main__':
    main()
