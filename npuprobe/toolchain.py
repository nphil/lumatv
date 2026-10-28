#!/usr/bin/env python3
"""Unpacks the compilers the probe is built with under $NPU_TOOLS (default /tmp/lumatv-npu-tools). Nothing is installed.

  cross   Ubuntu 22.04 GCC 11.4 + glibc 2.35 for arm-linux-gnueabi (soft-float EABI, like the TV), Debian 12 binutils
  native  Debian 12 GCC 12 for the build host (stub-delegate tests)
  cmake 3.27.9 and ninja from PyPI; qemu-arm-static from Debian 12 (runs the ARM build on the host)

Needs: python3, dpkg-deb, pip, network. Writes $NPU_TOOLS/env.sh; `. $NPU_TOOLS/env.sh` puts everything on PATH.
"""
import gzip
import hashlib
import lzma
import os
import re
import subprocess
import sys
import urllib.request

TOOLS = os.environ.get("NPU_TOOLS", "/tmp/lumatv-npu-tools")

DEBIAN = "http://deb.debian.org/debian"
UBUNTU = "http://archive.ubuntu.com/ubuntu"

# Debian 12 (bookworm), amd64 and all
DEBIAN_CROSS_BINUTILS = ["binutils-arm-linux-gnueabi", "binutils-common", "libbinutils", "libjansson4", "libisl23",
                         "libmpfr6", "libmpc3"]
DEBIAN_NATIVE = ["binutils", "binutils-x86-64-linux-gnu", "binutils-common", "libbinutils", "libctf-nobfd0", "libctf0",
                 "libgprofng0", "libjansson4", "cpp-12", "libcc1-0", "libitm1", "libatomic1", "libasan8", "liblsan0",
                 "libtsan2", "libubsan1", "libgcc-12-dev", "gcc-12", "libc-dev-bin", "linux-libc-dev", "libcrypt-dev",
                 "libtirpc-dev", "libnsl-dev", "rpcsvc-proto", "libc6-dev", "libstdc++-12-dev", "g++-12", "libisl23",
                 "libmpfr6", "libmpc3"]
DEBIAN_QEMU = ["qemu-user-static"]
# Ubuntu 22.04 (jammy, with -updates), amd64 and all
UBUNTU_CROSS = ["gcc-11-arm-linux-gnueabi", "cpp-11-arm-linux-gnueabi", "g++-11-arm-linux-gnueabi",
                "gcc-11-arm-linux-gnueabi-base", "gcc-11-cross-base", "libc6-armel-cross", "libc6-dev-armel-cross",
                "libgcc-s1-armel-cross", "libgcc-11-dev-armel-cross", "libstdc++6-armel-cross",
                "libstdc++-11-dev-armel-cross", "linux-libc-dev-armel-cross", "libgomp1-armel-cross",
                "libatomic1-armel-cross"]


def log(msg):
    print(msg, flush=True)


def fetch(url):
    with urllib.request.urlopen(url, timeout=120) as r:
        return r.read()


def parse_packages(text):
    out = {}
    for block in text.split("\n\n"):
        m = re.search(r"^Package: (.+)$", block, re.M)
        if not m:
            continue
        rec = {k: v for k, v in re.findall(r"^(Version|Filename|SHA256): (.+)$", block, re.M)}
        name = m.group(1)
        if name not in out or subprocess.run(["dpkg", "--compare-versions", rec["Version"], "gt", out[name]["Version"]]).returncode == 0:
            out[name] = rec
    return out


def debian_index():
    raw = lzma.decompress(fetch(f"{DEBIAN}/dists/bookworm/main/binary-amd64/Packages.xz")).decode("utf-8", "replace")
    recs = parse_packages(raw)
    return recs


def ubuntu_index():
    recs = {}
    for suite in ("jammy", "jammy-updates", "jammy-security"):
        for comp in ("main", "universe"):
            try:
                raw = gzip.decompress(fetch(f"{UBUNTU}/dists/{suite}/{comp}/binary-amd64/Packages.gz")).decode("utf-8", "replace")
            except Exception as e:  # a suite may lack a component
                log(f"  skip {suite}/{comp}: {e}")
                continue
            for name, rec in parse_packages(raw).items():
                if name not in recs or subprocess.run(["dpkg", "--compare-versions", rec["Version"], "gt", recs[name]["Version"]]).returncode == 0:
                    recs[name] = rec
    return recs


def install(base, index, names, root):
    cache = os.path.join(TOOLS, "debs")
    os.makedirs(cache, exist_ok=True)
    os.makedirs(root, exist_ok=True)
    for n in names:
        rec = index.get(n)
        if rec is None:
            sys.exit(f"package {n} not found in the index")
        path = os.path.join(cache, os.path.basename(rec["Filename"]))
        if not (os.path.exists(path) and hashlib.sha256(open(path, "rb").read()).hexdigest() == rec["SHA256"]):
            log(f"  fetch {n} {rec['Version']}")
            with open(path, "wb") as f:
                f.write(fetch(f"{base}/{rec['Filename']}"))
            if hashlib.sha256(open(path, "rb").read()).hexdigest() != rec["SHA256"]:
                sys.exit(f"checksum mismatch for {n}")
        subprocess.run(["dpkg-deb", "-x", path, root], check=True)


def main():
    os.makedirs(TOOLS, exist_ok=True)
    log("Debian 12 packages")
    deb = debian_index()
    deb_root = os.path.join(TOOLS, "root")
    install(DEBIAN, deb, DEBIAN_CROSS_BINUTILS + DEBIAN_QEMU, deb_root)
    host_root = os.path.join(TOOLS, "hostroot")
    install(DEBIAN, deb, DEBIAN_NATIVE, host_root)
    # the native linker scripts name /lib/x86_64-linux-gnu and /lib64 inside the sysroot: point them at the real ones
    for link, target in (("lib", "/lib"), ("lib64", "/lib64")):
        p = os.path.join(host_root, link)
        if not os.path.lexists(p):
            os.symlink(target, p)

    log("Ubuntu 22.04 packages")
    ubu = ubuntu_index()
    cross_root = os.path.join(TOOLS, "root-u")
    install(UBUNTU, ubu, UBUNTU_CROSS, cross_root)
    # GCC 11 looks for as and ld beside itself and in <root>/usr/arm-linux-gnueabi/bin: use Debian's cross binutils
    for sub in ("usr/bin", "usr/arm-linux-gnueabi/bin"):
        os.makedirs(os.path.join(cross_root, sub), exist_ok=True)
    for tool in ("as", "ld", "ld.bfd", "ld.gold", "ar", "ranlib", "strip", "nm", "objcopy", "objdump", "readelf", "size", "strings"):
        src = os.path.join(deb_root, "usr/bin", f"arm-linux-gnueabi-{tool}")
        if not os.path.exists(src):
            continue
        for dst in (os.path.join(cross_root, "usr/bin", f"arm-linux-gnueabi-{tool}"),
                    os.path.join(cross_root, "usr/arm-linux-gnueabi/bin", tool)):
            if os.path.lexists(dst):
                os.remove(dst)
            os.symlink(src, dst)

    log("cmake and ninja")
    py = os.path.join(TOOLS, "py")
    subprocess.run([sys.executable, "-m", "pip", "install", "-q", "--target", py, "cmake==3.27.9", "ninja"],
                   check=True)
    bindir = os.path.join(TOOLS, "bin")
    os.makedirs(bindir, exist_ok=True)
    for name, src in (("cmake", "cmake/data/bin/cmake"), ("ctest", "cmake/data/bin/ctest"), ("ninja", "bin/ninja")):
        dst = os.path.join(bindir, name)
        if os.path.lexists(dst):
            os.remove(dst)
        os.symlink(os.path.join(py, src), dst)

    env = f"""# source this: cross (ARM soft-float, GCC 11 + glibc 2.35) and native (GCC 12) toolchains, cmake, ninja, qemu
export NPUT={TOOLS}
export CROSS_ROOT=$NPUT/root-u
export CROSS_CC_VER=11
export HOST_ROOT=$NPUT/hostroot
export LD_LIBRARY_PATH=$HOST_ROOT/usr/lib/x86_64-linux-gnu:$NPUT/root/usr/lib/x86_64-linux-gnu${{LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}}
export PATH=$NPUT/bin:$CROSS_ROOT/usr/bin:$NPUT/root/usr/bin:$HOST_ROOT/usr/bin:$PATH
export QEMU_ARM=$NPUT/root/usr/bin/qemu-arm-static
"""
    with open(os.path.join(TOOLS, "env.sh"), "w") as f:
        f.write(env)
    log(f"done: . {TOOLS}/env.sh")


if __name__ == "__main__":
    main()
