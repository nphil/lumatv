# Hardware JPEG: the decoder service and the helper's picture route

## Resume here

Done offline (2026-10-25), not run on the TV:

- `DILE_JPEG_Open/Close/Decode` recovered from LG's libraries (evidence below); header `jpeg/dilejpeg.h`.
- `lumatv-jpeg` (`jpeg/service.c`), a 32-bit service: JPEG bytes in, NV12 or RGBA/BGRA out, at 1/1, 1/2, 1/4 or 1/8 per axis. When the block
  is missing, refuses a file or fails on it, libjpeg.so.62 gives the same planes (`jpeg/cpu.c`). NEON converters (`jpeg/convert_neon.c`).
  `jpeg/build.sh` builds the set's binary and a host build against a fake library (`jpeg/fake_dilephoto.c`).
- The helper's `/v1/image/{root}/{file}?w=&h=` (`helper/app/src/images.rs`): the decoded picture as a BMP at the scale asked for, or the
  file as it is where there is no decoder. `lumatv` stages the decoder beside the helper; `tools/build-ipk.sh` packs it.
- The TV step list (below), `jpeg/make_samples.py` for its files and `jpeg/compare.py` for reading the result.

Open until a slot: the layout of the library's buffer (alignment, range, chroma order), the speed, and whether `KADP_InitSystem` in a
new process is welcome (LG's libraries have refused unknown callers before: `ambient-result.md`). Nothing in the shell calls the route yet.

Not done: encode (the signature is recovered, there is no consumer), `DILE_VO_DisplayPicture` (a still on a video window: census rank 3,
second run), EXIF orientation (the block rotates; the route does not ask it to).

## The API

Source: `libdile_photo.so.1.0.1` (md5 9127769b47e7009e5cc870b16c301f5b) and `libhal_photo.so.2.0.1` (md5 68bbd1f2b88ac95f73999ec97710bf7f),
webOS 10.2.1, 32-bit ARM Thumb-2, from the TV's `/usr/lib` (census copy `/tmp/hwcensus/tv/libs`, scratch). Disassembled with capstone; addresses
are file offsets of the `.text` of each library. LG publishes no header for `DILE_JPEG`; the kernel side is in the GPL tree
(`research/lgsrc/k/soc_GO/linux-and-kdriver_paluma-5/kdriver/include/mjpeg_kapi.h`).

| Function | Address | What it does |
|---|---|---|
| `DILE_JPEG_Open()` | dile `0xe84` | `HAL_JPEG_Open`; 0 ok, 1 failed (message `DILE_PHOTO_DEBUG_MSG`). |
| `DILE_JPEG_Close()` | dile `0xea8` | `HAL_JPEG_Close`; 0 ok, 1 failed. |
| `DILE_JPEG_Decode(src, dst, scaling)` | dile `0x1390` | Returns 2 for a NULL `src`/`dst` or `src->data` (`0x13b8-0x13cc`), 1 for a failed `malloc` (`0x1492`) or a failed HAL call (`0x1416`), else the HAL's 0. |
| `DILE_JPEG_Encode(src, dst)` | dile `0x10bc` | Same shape; source formats 8, 13, 14 only (HAL `0x1d9c`). Recovered, not used. |
| `HAL_JPEG_Open` | hal `0x1568` | `KADP_InitSystem`, `KADP_GAL_Open`, `KADP_MJPEG_Open`, `KADP_MJPEG_GetCfg`, two semaphores; "JPEG is already opened" if it is. |
| `HAL_JPEG_Decode` | hal `0x1830` | Locks the semaphore, `KADP_MJPEG_InitDecoder`, `KADP_MJPEG_DecodeImage`, `KADP_MJPEG_CloseDecoder`, unlocks: the decoder is set up and torn down inside every call. |

`DILE_JPEG_Decode` in detail (`0x1390-0x1458`):

- `scaling` is 1..4; `_get_scaling_shift_integer` (`0x1038`) returns `scaling - 1` and `_get_hal_jpeg_scaling_option` (`0x1064`) returns `scaling`
  itself; anything else logs and counts as 1/1. `HAL_JPEG_Decode` (`0x18ba-0x18c0`, `0x1a92`, `0x1b8a`) turns 2, 3, 4 into the scale codes 1, 2, 3 of
  `LX_MJPEG_SCALING_T` (`mjpeg_kapi.h:308-313`: 1/2, 1/4, 1/8), both axes.
- It `malloc`s the output: `(src.width >> shift) * (src.height >> shift) * 3` bytes (`0x13dc-0x1400`). The caller's `src.width/height` (struct offsets `0x1c`,
  `0x20`) only size that buffer. The HAL writes the pixels into it (`HAL_JPEG_Decode` `0x1910-0x1920`: dst data and size go into the decode
  parameter), and the result's `width`, `height` (from the kernel adapter's answer, `0x1934`) and `pixel_format` land in `dst` (`0x1448-0x1456`).
- `dst.size` is set to the allocation, not to what was written. The caller frees `dst.data`.
- The struct is `0x28` bytes; the fields the library reads or writes are `data` `0x00`, `size` `0x0c`, `width` `0x1c`, `height` `0x20`, `pixel_format` `0x24`
  (`jpeg/dilejpeg.h`).

Pixel format of the result, `HAL_JPEG_Decode` `0x1934-0x1ab4`: the kernel adapter's chroma format (`LX_MJPEG_FORMAT_*`, `mjpeg_kapi.h:215-219`) and
its `bUVInterleaved` flag (`mjpeg_kapi.h:256`, read at `sp+0xac`) choose the code.

| Hardware format | Planar (flag 0) | Interleaved CbCr (flag 1) |
|---|---|---|
| 4:2:0 (0) | 12 | 13 (NV12) |
| 4:2:2 (1) | 9 | 10 |
| 4:2:4 (2, chroma halved vertically) | 15 | 16 |
| 4:4:4 (3) | 7 | 8 |
| 4:0:0 (4) | 14 | 14 |

The jump table is at `0x194e`; format 3 is tested before it (`0x18ae`). The block takes baseline, extended-sequential and progressive Huffman JPEG
(`SOF_MARKER`, `SOF_MARKER_PDCT_HD`, `mjpeg_kapi.h:409-410`), 16 to 32768 pixels a side (`:51-54`).

The kernel adapter's answer also holds the buffer geometry (`LX_MJPEG_IMAGE_BUF_T`: `ui32StrideY`, `ui32StrideC`, `ui32SizeY..`, `mjpeg_kapi.h:246-257`
at `sp+0x8c`), and `HAL_JPEG_Decode` does not hand it up. How the planes lie in the caller's buffer (a stride equal to the width, or the block's own) is therefore
not in the API: it is the first thing the TV slot reads (step 6). `lumatv-jpeg` takes it as arguments (`-A` row alignment, `-B` plane-height alignment,
`-S` Cr before Cb, `-L` studio range), default 1, 1: planes packed at the reported width.

## The service

`lumatv-jpeg` is started by the helper with a connected socket as fd 0 and speaks `jpeg/jpegproto.h`: one request (op, format, shift, flags, `max_pixels`,
the JPEG file) and one answer (status, size, stride, microseconds for the decode and the conversion, what the hardware returned, flags, the pixels).

- The library is loaded at the first decode and the device opened with it; after 3 s without a request the device is given back (`DILE_JPEG_Close`),
  after 30 s the process ends, and the helper starts it again when a picture is wanted. A closed socket or SIGTERM gives the device back first.
- A file the block does not take (not 8-bit baseline/extended/progressive Huffman with the four samplings, under 16 or over 32768 a side), or
  a library that is missing, fails to open, fails to decode or returns an image this build cannot read: libjpeg.so.62 decodes it and the answer carries
  `JP_RFLAG_CPU` and the hardware's status in bits 8..15. Request flags choose the hardware only (`-H`) or the CPU only (`-C`).
- Both paths give planes; the same converters turn them into NV12 (a 4:2:0 semi-planar buffer is copied as it is) and RGBA/BGRA (integer BT.601,
  full range, or studio range expanded with `-L`; NEON where the kernel reports NEON and hardware divide). The libjpeg path uses raw data output (no
  colour conversion, no chroma upsampling), so the two paths differ only by what the block and libjpeg do to the IDCT.
- Nothing leaves the process: no network, no file, no bus call. `-n` resolves the three functions and prints them; `-b FILE -N COUNT -z SHIFT -F fmt
  [-C|-H] [-o DIR]` is the slot's tool: per-iteration times, CPU time, `JpgDec` interrupts and `CmaFree` before and after, and the library's raw buffer
  and the converted picture for `jpeg/compare.py`.

## The helper

`GET /v1/image/{root}/{file}?w=&h=` answers a JPEG of a folder the config names (`images.roots`, plus `images/` in the data dir and the package's
`art`). The scale is the largest of 1/1..1/8 that keeps `w` and `h`: a 4K painting for a 1080p window arrives at 1920x1080 and costs a 1080p copy. The
answer is a 32-bit bottom-up BMP (Qt's own reader; the picture is copied once) with `X-Luma-Decoder: hardware|cpu`. Where there is no decoder, it
fails, or the picture is over `images.maxPixels`, the answer is the file as `image/jpeg` and `X-Luma-Decoder: file`, which is what the shell does today.
Names are plain (`[A-Za-z0-9._-]`, no leading dot), the file has to stay inside its folder once links are followed, and only `.jpg`/`.jpeg` that parse as
JPEG are served. `GET /v1/images` reports the folders, whether the decoder is there and running, and how many pictures each path gave.
`images.decoderArgs` carries the slot's findings (`["-A","64","-B","16"]`).

For the QML: `Image { source: Helper.base + "/v1/image/art/great-wave.jpg?w=1920&h=1080"; sourceSize: ... }`. `X-Luma-Decoder` is not visible to QML; the route is
the same whichever decoder answered. Qt 5.15 on the workstation reads the BMP bottom-up with the right colours; the set's 5.12 has not been run.

## Not known until the TV run

| Question | Where it shows |
|---|---|
| Does a new process get `KADP_InitSystem` and the JPEG device without LG's permission flag or a luna role? | step 3: `DILE_JPEG_Open` result; a `LSHUB_NO_ROLE_FILE` line would mean the role mechanism of `payload/capture/capture.sh` |
| Row and plane alignment of the buffer, chroma order, range | step 6 (`compare.py`) |
| Time per picture, CPU per picture, against libjpeg | step 4 |
| Whether the 3x-pixels allocation is enough for 4:4:4 at every size | step 3 on `4k444.jpg` (a refused buffer is `status=6`) |
| Whether the helper's route works with the package's staging | step 8 |

[INFERENCE, to measure] libjpeg-turbo on one core of the set decodes about 70-120 MPix/s (census), so 1080p takes 17-30 ms and 4K 70-120 ms; the block is
worth wiring in if it takes less CPU time per picture, whatever the wall time.

## TV step list

Needs Main's OK first: it starts LG's JPEG library in a new process (`KADP_InitSystem`, `KADP_GAL_Open`, the JPEG device). Set at Home, no playback (the
block needs CMA); nothing else on the set is touched. Stop at the first anomaly: a compositor stall, a `dmesg` line from the JPEG driver, a process stuck on
`/dev/lg/mjpeg0` (`fuser /dev/lg/mjpeg0`; kill the program, which closes the device on SIGTERM).

On the workstation:

1. `jpeg/build.sh`; `python3 jpeg/make_samples.py /tmp/jpegslot` (1080p 4:2:0, 4K 4:2:0, 1003x707 4:2:0, 4K 4:4:4, 1080p progressive).
   Copy `jpeg/build/arm/lumatv-jpeg` and the five files to `/tmp/jpegslot` on the set.

On the set, as root, in `/tmp/jpegslot` (`J=./lumatv-jpeg`):

2. Read only: `ls -l /dev/lg/mjpeg0 /usr/lib/libdile_photo.so.1 /usr/lib/libhal_photo.so.2 /usr/lib/libjpeg.so.62`; `grep JpgDec /proc/interrupts`
   (0 since boot); `grep -i cma /proc/meminfo`; `$J -n` (prints the three function addresses and touches nothing).
3. First hardware decode, one picture each, the text kept for step 6: `mkdir o1080 oodd o4k o444 oprog`;
   `$J -b 1080.jpg -N 1 -H -o o1080 | tee o1080/bench.txt`, then `odd.jpg` into `oodd`, `4k.jpg` into `o4k`, `4k444.jpg` into `o444`, `1080p.jpg` into `oprog`. Per run read
   `status`, `hw_pixfmt` (13 for 4:2:0, 8 for 4:4:4), `hw_width/hw_height`, `jpgdec_irq_after` above the before (the block ran), `cma_free_kb_min`.
   If the open fails or the process dies, stop and copy the last 20 lines of the output and `dmesg | tail`.
4. Timing, 20 each, scale 1/1 and 1/2: `$J -b 1080.jpg -N 20 -H -F bgra`, `-N 20 -C -F bgra` (libjpeg on the CPU), the same with `-z 1`, and `4k.jpg` the same.
   Each prints `decode_us`, `convert_us` (min/median/mean/max), `wall_ms`, `cpu_ms` and `cpu_per_image_ms`. The hardware's cost to the CPU is `cpu_per_image_ms`
   against the baseline's; the first line of each run says `convert=neon` (or `scalar`, which would be a finding).
5. NV12: `$J -b 4k.jpg -N 20 -H -F nv12` and the same with `-C`.
6. The layout, on the workstation after copying `o*` back: `python3 jpeg/compare.py odd.jpg --dump oodd/hw-s0.bin --report oodd/bench.txt`; then the same for
   `1080.jpg` and `4k.jpg`. `odd.jpg` has no side that is a multiple of 16, so it tells the alignment; the other two confirm it. It prints the `-A`, `-B`, `-S`, `-L`
   that reproduce the picture (and says when it cannot: the dump then has to be read by hand). Run step 4 again with those arguments added and check
   `compare.py --out oodd/out-s0.rgba`.
7. Idle: after the last run `grep JpgDec /proc/interrupts` (the count only went up), `fuser /dev/lg/mjpeg0` (nobody), `CmaFree` back to its first value.
8. Through the helper, with the package installed (or `lumatv-jpeg` copied to `/var/lib/lumatv/bin/` and the args put in `helper.json`):
   `mkdir -p /var/lib/lumatv/images && cp 4k.jpg /var/lib/lumatv/images/a.jpg`;
   `curl -s -D - -o /tmp/a.bmp 'http://127.0.0.1:18731/v1/image/images/a.jpg?w=1920&h=1080'` (`X-Luma-Decoder: hardware`, `X-Luma-Size: 1920x1080`, 8 294 454 bytes);
   `curl -s http://127.0.0.1:18731/v1/images`; `cp /tmp/a.bmp` back and look at it. Wait 35 s: `pidof lumatv-jpeg` is empty and `fuser /dev/lg/mjpeg0` nobody.
9. Clean up: `rm -rf /tmp/jpegslot /var/lib/lumatv/images/a.jpg`.

Record in this file: the three numbers per picture (wall, CPU, interrupts), the alignment, the range, whether the open needed anything.

## Files

`jpeg/`: `service.c` (service and bench), `cpu.c`/`cpu.h` (libjpeg), `convert.c`, `convert_neon.c`, `convert_core.h`, `convert.h`, `dilejpeg.h`, `jpegproto.h`, `fake_dilephoto.c`, `convert_test.c`,
`compare.py`, `make_samples.py`, `libjpeg62/` (the four headers of the set's libjpeg, see its `NOTICE.md`), `build.sh`.
`helper/app/src/images.rs`, `config.rs` (`images`), `api.rs`, `http.rs` (a response may name its own `Content-Type` and `Cache-Control`).
`tools/lumatv` (`jpeg_stage`), `tools/build-ipk.sh`.

Tests: `python3 -m unittest tests.test_jpeg_service tests.test_jpeg_compare` (after `jpeg/build.sh`; the service against the fake library for every chroma layout and scale,
the fallback, the protocol edges, the converters against double precision, and under qemu-arm the NEON converters against the scalar ones and the set's own binary against an ARM build of the fake library), `helper/build.sh test`
(`images::tests`: names and links, scale, BMP, fallback, a decoder that ends or lies).
