"""Host-only 1080p SDR picture stream and timing manifests; no TV connection."""
import argparse
import csv
from fractions import Fraction
import json
from pathlib import Path
import subprocess
import tempfile


COUNT = 720
PHASES = ('fixed120', 'shuffled', 'same-mean')
FIELDS = ('id', 'packet_offset', 'packet_bytes', 'patch_on', 'rate_num',
          'rate_den', 'period_ns', 'target_ns')


def period_ns(rate):
    if not (24 <= rate <= 120 and max(rate.numerator, rate.denominator) <= 65535):
        raise ValueError(f'rate outside VO bounds: {rate}')
    return 1_000_000_000 / Fraction(rate)


def rates(phase):
    if phase in ('fixed120', 'same-mean'):
        return [Fraction(120) if phase == 'fixed120' else Fraction(1000, 13)] * COUNT
    if phase != 'shuffled':
        raise ValueError(phase)
    # Fixed xorshift32/Fisher–Yates: exact 50:50 histogram, stable across hosts.
    seq = [Fraction(125, 2), Fraction(100)] * (COUNT // 2)
    state = 0xF123B5C7
    for i in range(COUNT - 1, 0, -1):
        state ^= (state << 13) & 0xffffffff
        state ^= state >> 17
        state ^= (state << 5) & 0xffffffff
        j = state % (i + 1)
        seq[i], seq[j] = seq[j], seq[i]
    return seq


def manifest(out, phase, packets):
    writer = csv.writer(out, lineterminator='\n')
    writer.writerow(FIELDS)
    target = Fraction(0)
    for i, (rate, packet) in enumerate(zip(rates(phase), packets, strict=True)):
        period = period_ns(rate)
        # Round cumulative deadlines, never individually round and accumulate.
        writer.writerow((i, *packet, i % 2, rate.numerator, rate.denominator,
                         str(period), round(target)))
        target += period


def generate(dest, font):
    if dest.exists():
        raise FileExistsError(dest)
    # Use a fixed relative font path: FFmpeg filters have their own path escaping.
    with tempfile.TemporaryDirectory(dir=dest.parent) as tmp:
        work = Path(tmp).resolve()
        (work / 'font.ttf').symlink_to(font.resolve(strict=True))
        video = work / 'frames.h264'
        # Patch and ID share a scanline. Neither stays bright; patch has two dark levels.
        filters = ("drawbox=x=96:y=80:w=128:h=128:c=0x484848:t=fill:enable='mod(n,2)',"
                   "drawtext=fontfile=font.ttf:text='%{n}':x=320+24*mod(trunc(n/60)\\,4):"
                   "y=84:fontsize=116:fontcolor=0x666666,setsar=1")
        subprocess.run(['ffmpeg', '-nostdin', '-v', 'error', '-f', 'lavfi', '-i',
                        'color=c=0x181818:s=1920x1080:r=60', '-vf', filters,
                        '-frames:v', str(COUNT), '-an', '-pix_fmt', 'yuv420p',
                        '-c:v', 'libx264', '-threads', '2', '-preset', 'ultrafast',
                        '-tune', 'zerolatency', '-crf', '16', '-x264-params',
                        'aud=1:bframes=0:scenecut=0:keyint=60:repeat-headers=1',
                        '-f', 'h264', str(video)], cwd=work, check=True)
        result = subprocess.run(['ffprobe', '-v', 'error', '-show_packets',
                                 '-show_entries', 'packet=pos,size', '-of', 'json',
                                 str(video)], capture_output=True, check=True)
        packets = [(int(p['pos']), int(p['size'])) for p in json.loads(result.stdout)['packets']]
        end = 0
        for offset, size in packets:
            if offset != end or size <= 0:
                raise ValueError('non-contiguous H.264 access units')
            end += size
        if len(packets) != COUNT or end != video.stat().st_size:
            raise ValueError('encoded stream does not match manifest')
        for phase in PHASES:
            with (work / f'{phase}.csv').open('w', newline='') as out:
                manifest(out, phase, packets)
        (work / 'font.ttf').unlink()
        dest.mkdir()
        for path in work.iterdir():
            path.rename(dest / path.name)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path, help='new directory under an existing parent')
    parser.add_argument('--font', type=Path, default=Path('/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf'))
    args = parser.parse_args()
    generate(args.output, args.font)
