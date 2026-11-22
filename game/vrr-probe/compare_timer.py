"""Compare two saved O22 ARM-side /proc/lxvdec/reg/vsync dumps, never write hardware."""
import argparse
from pathlib import Path
import re

BASE = 0xCE020700
ROW = re.compile(r'[0-9A-Fa-f]{8}:\s+(?:[0-9A-Fa-f]{8}\s+){3}[0-9A-Fa-f]{8}')


def channels(text):
    lines = [line.strip() for line in text.splitlines() if line.strip()]
    if len(lines) != 17 or lines[0] != 'sync':
        raise ValueError('expected a complete O22 ARM sync dump')
    words = []
    for i, line in enumerate(lines[1:]):
        if not ROW.fullmatch(line) or int(line[:8], 16) != BASE + i * 16:
            raise ValueError(f'invalid O22 sync row {i}')
        words.extend(int(x, 16) for x in line[9:].split())
    return [(words[4 + i * 2], words[5 + i * 2], (words[1] >> (24 + i)) & 1,
             (words[0] >> (24 + i)) & 1) for i in range(6)]


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('before', type=Path)
    parser.add_argument('after', type=Path)
    args = parser.parse_args()
    before, after = (channels(path.read_text()) for path in (args.before, args.after))
    print('ch (field0, field1, field-select, enable): before -> after')
    for i, (a, b) in enumerate(zip(before, after)):
        print(f'{i} {a} -> {b} {"equal" if a == b else "DIFFERENT"}')
    raise SystemExit(int(before != after))
