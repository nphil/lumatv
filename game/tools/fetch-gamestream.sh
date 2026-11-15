#!/bin/sh
# Build-host helper: the GPL engine remains a separate executable and repository.
set -eu
if [ "$#" -ne 1 ]; then echo 'usage: fetch-gamestream.sh <package-stage>' >&2; exit 2; fi
here=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
python3 - "$here/engines.lock" "$1" <<'PY'
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import sys
import tarfile
import tempfile
import urllib.request

lock = json.loads(Path(sys.argv[1]).read_text())
if lock.get('schema') != 1:
    raise SystemExit('Unsupported engine lock format')
pin = lock['gamestream']
if not re.fullmatch(r'[0-9]+\.[0-9]+\.[0-9]+', pin['version']):
    raise SystemExit('Invalid pinned GameStream version')
if not re.fullmatch(r'[a-f0-9]{64}', pin['sha256']) or not pin['url'].startswith('https://'):
    raise SystemExit('GameStream needs an HTTPS release URL and a pinned SHA-256')
engines = Path(sys.argv[2]).resolve() / 'engines'
engines.mkdir(parents=True, exist_ok=True)
with tempfile.TemporaryDirectory(prefix='.gamestream-', dir=engines) as temporary:
    work = Path(temporary)
    archive = work / 'release.tar.gz'
    digest = hashlib.sha256()
    with urllib.request.urlopen(pin['url'], timeout=60) as response, archive.open('wb') as out:
        while chunk := response.read(1024 * 1024):
            digest.update(chunk)
            out.write(chunk)
    if digest.hexdigest() != pin['sha256']:
        raise SystemExit('GameStream release checksum does not match engines.lock; nothing installed')
    unpacked = work / 'unpacked'
    with tarfile.open(archive, 'r:gz') as bundle:
        for member in bundle.getmembers():
            path = PurePosixPath(member.name)
            if member.isdir() and path == PurePosixPath('.'):
                continue
            if path.is_absolute() or '..' in path.parts or not path.parts or path.parts[0] != 'gamestream' or not (member.isdir() or member.isfile()):
                raise SystemExit('Unsafe entry in the GameStream release archive')
        for member in bundle.getmembers():
            destination = unpacked / member.name
            if member.isdir():
                destination.mkdir(parents=True, exist_ok=True)
            else:
                destination.parent.mkdir(parents=True, exist_ok=True)
                with bundle.extractfile(member) as incoming, destination.open('wb') as outgoing:
                    shutil.copyfileobj(incoming, outgoing)
    source = unpacked / 'gamestream'
    manifest = json.loads((source / 'engine.json').read_text())
    if manifest.get('id') != 'gamestream' or manifest.get('exec') != 'lumagame-gamestream' or manifest.get('protocol') != 1 or manifest.get('license') != 'GPL-3.0-only':
        raise SystemExit('Unexpected GameStream engine manifest')
    if manifest.get('source') != pin['source']:
        raise SystemExit('GameStream source version does not match engines.lock')
    executable = source / 'lumagame-gamestream'
    with executable.open('rb') as file:
        header = file.read(20)
    if header[:6] != b'\x7fELF\x02\x01' or int.from_bytes(header[18:20], 'little') != 183:
        raise SystemExit('The GameStream binary is not little-endian aarch64 ELF')
    for name in ('LICENSE', 'SOURCE.txt', 'NOTICE', 'LUMATV-LICENSE'):
        if not (source / name).is_file():
            raise SystemExit(f'The GameStream release is missing {name}')
    if not (source / 'licenses').is_dir():
        raise SystemExit('The GameStream release is missing dependency licences')
    executable.chmod(0o755)
    destination = engines / 'gamestream'
    previous = work / 'previous'
    if destination.is_symlink():
        raise SystemExit('Refusing to replace a symbolic-link engine directory')
    replaced = destination.exists()
    if replaced:
        os.replace(destination, previous)
    try:
        os.replace(source, destination)
    except OSError:
        if replaced:
            os.replace(previous, destination)
        raise
print(f"GameStream {pin['version']} staged; SHA-256 {pin['sha256']}")
PY
