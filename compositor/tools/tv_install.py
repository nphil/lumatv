#!/usr/bin/env python3
"""Quiet install to the TV: compares the built files with what is installed, and puts each changed part in by the route that does not
disturb what is on screen. Run from the workstation under the lease's renewal (`tv-lock.sh with` keeps it renewed; it inherits a lease the
lane already holds, and only takes one itself, which can wake the set, when there is none). This tool only calls `require`:

    tv-lock.sh with --why "<what>" -- python3 compositor/tools/tv_install.py [options]   (--check alone needs only `require`)

    tv_install.py [--check] [--now | --defer] [--only tool,helper,ui,compositor,feeder] [--helper PATH] [--deploy DIR] [--backup-dir DIR] [--host IP]

Parts and how each goes in:

  tool        tools/lumatv, tools/compositor.sh: backed up, then replaced atomically in the app's tools/. Nothing restarts. Always first: the staged
              install's boot hook lives in tools/lumatv. The result is checked on the set (hashes, both files parse, `helper status`
              answers); a failed check puts the previous files back and stops.
  helper      the helper binary (payload and /var/lib/lumatv/bin): both copies backed up, replaced atomically, then `tools/lumatv helper
              start`. Only the helper restarts; the compositor, the app in front and the video are not touched. A start that does not
              give a new running pid puts the previous copies back and starts them.
  ui          bin/lumatv-ui and assets/ui: `cutover.sh ui-swap`, which replaces them atomically and restarts only the shell client
              (`ctl shell restart`); the compositor keeps the app's surface and the video plane. It checks that the UI reconnects and Home
              draws, and puts the previous one back when it does not. Used when nothing of the compositor class changed.
  compositor, feeder
              bin/lumatv-compositor, bin/lumatv-sm, lib/*, recover-home.sh, logrelay.py, verified.list, the feeder and its libraries
              (the ui rides along when it changed too): staged on the set (/var/lib/lumatv/compositor/staged) with a manifest of hashes
              and applied by the set itself at the next boot, before the compositor starts. `--now` applies at once, but only with the
              panel on and Home in front and no game running (exit status 10 says not now, and the set stays staged). Nothing applies it in
              standby: that path is not proven not to wake the panel. The apply is the cutover's own disable, install, probe, enable and
              video check; any failure puts the previous files back, and a reboot is never done for the owner. A boot apply stays pending
              with the previous files kept until `tools/lumatv staged accept` is run by hand; while one is pending nothing is written. Never Wake-on-LAN.

  --defer     include the helper and UI in a boot-only staged set instead of restarting them. No running service is touched; an existing
              waiting or unconfirmed set is kept. Include `tool` if the installed boot hook needs updating.

Every install, staged ones included, backs up the binaries it replaces on the workstation as <name>-<8 hex of the old hash> (a `--now` apply drops the set's own previous files once the video check passes).
The result lists old -> new hashes (8 hex) for the HANDOFF entry."""
import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from collections import namedtuple

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", ".."))

TV_HOST = "192.168.1.242"
APP = "/media/developer/apps/usr/palm/applications/org.nphil.lumatv"
DATA = "/var/lib/lumatv"
COMP = DATA + "/compositor"
STAGED = COMP + "/staged"
QUIET = "/tmp/lumatv-quiet"
ASSETS_UI = COMP + "/assets/ui/"
HELPER_PAYLOAD = APP + "/payload/helper/lumatv-helper"
HELPER_BIN = DATA + "/bin/lumatv-helper"
TV_LOCK = os.environ.get("LUMATV_TV_LOCK", "/data/home/Marquee/tools/tv-lock.sh")

HELPER_WAIT = 20  # seconds the helper gets to show a new pid after `helper start`
SENTINEL = "LUMATV-INVENTORY-END"
PENDING = "LUMATV-STAGED-PENDING"
BOOT_PENDING = "LUMATV-BOOT-PENDING"
APPLYING = "LUMATV-STAGED-APPLYING"
DEFERRED = "LUMATV-STAGED-BOOT-ONLY"
HELPER_REL = "helper/lumatv-helper"

PARTS = ("tool", "helper", "ui", "compositor", "feeder")
STAGED_PARTS = ("compositor", "feeder")
# Files the staged apply needs beside the parts: the cutover's own scripts, which run from the staged set so that an apply is whole.
SUPPORT = ("cutover.sh", "compositor.sh", "logrelay.py", "feeder/feeder.sh", "feeder/check.h264")

# remote: the path on the set; local: the built file; rel: its path in the deploy tree (None for a part that is not staged).
Item = namedtuple("Item", "remote local rel")
Change = namedtuple("Change", "part remote old new rel local")


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def short(h):
    return h[:8] if h else "-"


def tree(root):
    """Every file under root as its relative path, sorted."""
    out = []
    for dirpath, _, names in os.walk(root):
        for n in names:
            out.append(os.path.relpath(os.path.join(dirpath, n), root))
    return sorted(out)


def local_parts(repo, deploy, helper):
    """part -> [Item]: what each part consists of, for the files that exist in this build."""
    parts = {p: [] for p in PARTS}

    def add(part, remote, local, rel=None):
        if os.path.isfile(local):
            parts[part].append(Item(remote, local, rel))

    add("tool", APP + "/tools/lumatv", os.path.join(repo, "tools", "lumatv"))
    add("tool", APP + "/tools/compositor.sh", os.path.join(repo, "tools", "compositor.sh"))
    add("helper", HELPER_PAYLOAD, helper)
    add("helper", HELPER_BIN, helper)
    add("ui", COMP + "/bin/lumatv-ui", os.path.join(deploy, "bin", "lumatv-ui"), "bin/lumatv-ui")
    ui_root = os.path.join(deploy, "assets", "ui")
    if os.path.isdir(ui_root):
        for rel in tree(ui_root):
            add("ui", ASSETS_UI + rel, os.path.join(ui_root, rel), "assets/ui/" + rel)
    for f in ("lumatv-compositor", "lumatv-sm"):
        add("compositor", COMP + "/bin/" + f, os.path.join(deploy, "bin", f), "bin/" + f)
    for f in ("recover-home.sh", "logrelay.py"):
        add("compositor", COMP + "/bin/" + f, os.path.join(deploy, f), f)
    add("compositor", COMP + "/verified.list", os.path.join(deploy, "verified.list"), "verified.list")
    lib = os.path.join(deploy, "lib")
    if os.path.isdir(lib):
        for f in sorted(os.listdir(lib)):
            add("compositor", COMP + "/lib/" + f, os.path.join(lib, f), "lib/" + f)
    add("feeder", DATA + "/bin/lumatv-gfn-feeder", os.path.join(deploy, "feeder", "lumatv-gfn-feeder"), "feeder/lumatv-gfn-feeder")
    flib = os.path.join(deploy, "feeder", "lib")
    if os.path.isdir(flib):
        for f in sorted(os.listdir(flib)):
            add("feeder", DATA + "/bin/lib/" + f, os.path.join(flib, f), "feeder/lib/" + f)
    return {p: items for p, items in parts.items() if items}


def hash_script(parts):
    """Commands that print `sha256  path` for every installed file of the parts, and the files of the installed assets/ui as ./rel. No exit
    here: whatever is appended after it has to run."""
    paths = sorted({i.remote for items in parts.values() for i in items if not i.remote.startswith(ASSETS_UI)})
    cmd = "nice -n 19 sha256sum " + " ".join(paths) + " 2>/dev/null"
    if "ui" in parts:
        cmd += "; cd " + ASSETS_UI.rstrip("/") + " 2>/dev/null && find . -type f | nice -n 19 xargs sha256sum 2>/dev/null"
    return cmd


def inventory_script(parts):
    """Installed hashes and staged state, followed by a sentinel that proves the script ran to its end."""
    return ("%s; [ -f %s/manifest ] && echo %s; [ -f %s/boot-pending ] && echo %s; "
            "[ -f %s/applying ] && echo %s; grep -q '  boot-only$' %s/manifest 2>/dev/null && echo %s; echo %s") % (
        hash_script(parts), STAGED, PENDING, STAGED, BOOT_PENDING, STAGED, APPLYING, STAGED, DEFERRED, SENTINEL)


def parse_hashes(out):
    """`sha256sum` output -> {remote path: hash}; ./rel lines are the installed assets/ui."""
    found = {}
    for line in out.splitlines():
        m = re.match(r"^([0-9a-f]{64})  (.+)$", line.strip())
        if not m:
            continue
        path = m.group(2)
        if path.startswith("./"):
            path = ASSETS_UI + path[2:]
        found[path] = m.group(1)
    return found


def compare(parts, remote):
    """part -> [Change] for what differs from the installed hashes; a part with none is left out. An installed assets/ui file the build
    no longer has is a change too (new is None). A feeder that is not installed on the set is not installed by this (cutover only updates
    one that `lumatv feeder install` put there)."""
    changes = {}
    for part, items in parts.items():
        if part == "feeder" and not any(remote.get(i.remote) for i in items if i.rel == "feeder/lumatv-gfn-feeder"):
            continue
        rows = []
        for i in items:
            new = sha256_file(i.local)
            old = remote.get(i.remote)
            if old != new:
                rows.append(Change(part, i.remote, old, new, i.rel, i.local))
        if part == "ui":
            have = {i.remote for i in items}
            rows += [Change(part, p, h, None, None, None) for p, h in sorted(remote.items()) if p.startswith(ASSETS_UI) and p not in have]
        if rows:
            changes[part] = rows
    return changes


def choose(changed, defer=False):
    """The actions for the changed parts, in order."""
    actions = [p for p in ("tool", "helper") if p in changed and (p != "helper" or not defer)]
    if changed.intersection(STAGED_PARTS) or (defer and changed.intersection(("helper", "ui"))):
        actions.append("staged")
    elif "ui" in changed:
        actions.append("ui")
    return actions


def staged_rels(parts, changes):
    """The deploy-relative files a staged set carries: the changed ones of the staged parts and, when the ui changed, all of it (assets/ui is
    one unit on the set), and the support files."""
    rels = {c.rel for p in STAGED_PARTS for c in changes.get(p, []) if c.rel}
    if "ui" in changes:
        rels |= {i.rel for i in parts["ui"]}
    return sorted(rels)


def manifest_text(deploy, rels):
    return "".join("%s  %s\n" % (sha256_file(os.path.join(deploy, r)), r) for r in rels)


def stale_deploy(repo, deploy):
    """Why the deploy tree cannot be installed from, or None: its scripts must be the repo's, and its files those it was built with."""
    for mine, theirs in ((os.path.join(repo, "compositor", "tools", "tv-cutover.sh"), "cutover.sh"), (os.path.join(repo, "tools", "compositor.sh"), "compositor.sh")):
        dest = os.path.join(deploy, theirs)
        if not os.path.isfile(dest) or sha256_file(dest) != sha256_file(mine):
            return "%s in the deploy tree is not the repo's: run compositor/tools/build-tv.sh" % theirs
    hashes = os.path.join(deploy, "HASHES")
    if os.path.isfile(hashes):
        for line in open(hashes):
            parts = line.split()
            if len(parts) == 2 and os.path.isfile(os.path.join(deploy, parts[1])) and sha256_file(os.path.join(deploy, parts[1])) != parts[0]:
                return "%s does not match HASHES: run compositor/tools/build-tv.sh" % parts[1]
    return None


class Tv:
    """The TV over the system ssh and scp (key login, no password)."""

    def __init__(self, host):
        self.target = "root@" + host
        self.opts = ["-o", "BatchMode=yes", "-o", "ConnectTimeout=10"]

    def run(self, cmd, timeout=900):
        r = subprocess.run(["ssh", *self.opts, self.target, cmd], stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=timeout)
        return r.returncode, r.stdout

    def put(self, local, remote):
        subprocess.run(["scp", "-q", *self.opts, local, "%s:%s" % (self.target, remote)], stdin=subprocess.DEVNULL, check=True)

    def put_tree(self, local_dir, remote_dir):
        subprocess.run(["scp", "-q", "-r", *self.opts, local_dir + "/.", "%s:%s/" % (self.target, remote_dir)], stdin=subprocess.DEVNULL, check=True)

    def get(self, remote, local):
        subprocess.run(["scp", "-q", *self.opts, "%s:%s" % (self.target, remote), local], stdin=subprocess.DEVNULL, check=True)


def outer_lease_problem(env):
    """An install runs inside `tv-lock.sh with --why ... --`, which keeps the lease renewed for as long as it runs; one without it could outlive
    the lease's expiry."""
    if env.get("PLX_TV_LOCK_INHERITED") == "1" and env.get("PLX_TV_LOCK_TOKEN"):
        return None
    return ("an install has to run under the lease's renewal: tv-lock.sh with --why \"<what>\" -- python3 compositor/tools/tv_install.py ... "
            "(with inherits a lease already held; it takes one itself only when there is none, which can wake the set; this tool never does)")


def require_lease():
    """Fails unless this lane holds the lease. `require` never wakes the set (`acquire` does, by Wake-on-LAN)."""
    try:
        r = subprocess.run([TV_LOCK, "require"], stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    except OSError as e:
        return "no TV lease check (%s)" % e
    return None if r.returncode == 0 else "the TV lease is not ours:\n" + r.stdout


def backup(tv, remote, old, backup_dir, say):
    """Keeps the file about to be replaced on the workstation as <name>-<8 hex>; returns its path (None when nothing was installed)."""
    if not old:
        return None
    os.makedirs(backup_dir, exist_ok=True)
    dest = os.path.join(backup_dir, "%s-%s" % (os.path.basename(remote), short(old)))
    if os.path.exists(dest):
        return dest
    part = dest + ".tmp"
    tv.get(remote, part)
    if sha256_file(part) != old:
        os.remove(part)
        raise RuntimeError("the backup of %s does not match its hash" % remote)
    os.replace(part, dest)
    say("  backup %s" % dest)
    return dest


def swap_file(tv, local, remote):
    """Uploads beside the target and renames over it: the target is always whole."""
    tv.put(local, remote + ".new")
    rc, out = tv.run("chmod 755 %s.new && mv -f %s.new %s" % (remote, remote, remote))
    if rc != 0:
        raise RuntimeError("could not put %s in place: %s" % (remote, out.strip()))


def helper_state(tv):
    """The helper's `status` as a dict ({} when it does not answer): running, pid (the supervisor's), healthy (null without a port)."""
    rc, out = tv.run("cd %s && ./tools/lumatv helper status" % APP)
    if rc != 0:
        return {}
    for line in reversed(out.strip().splitlines()):
        if line.startswith("{"):
            try:
                state = json.loads(line).get("helper")
            except ValueError:
                return {}
            return state if isinstance(state, dict) else {}
    return {}


def wait_helper(tv, before):
    """(pid, "") once the helper runs under a pid that is not `before` and does not report itself unhealthy; (None, why) after HELPER_WAIT s."""
    why = "it did not answer"
    for attempt in range(HELPER_WAIT):
        if attempt:
            time.sleep(1)
        s = helper_state(tv)
        pid = s.get("pid")
        if not s.get("running") or not pid:
            why = "it is not running (no pid)"
        elif before is not None and str(pid) == str(before):
            why = "its pid did not change (%s): it did not restart" % pid
        elif s.get("healthy") is False:
            why = "it runs (pid %s) but does not answer its health check" % pid
        else:
            return str(pid), ""
    return None, why


def start_helper(tv):
    rc, out = tv.run("cd %s && ./tools/lumatv helper start" % APP)
    return rc == 0 and '"ok":true' in out, out.strip()[-300:]


def tool_problem(tv, changes):
    """Why the replaced tool files cannot be trusted, or None: each has the hash that was sent, both parse, and `tools/lumatv` (which sources
    compositor.sh) still answers `helper status`."""
    rc, out = tv.run("nice -n 19 sha256sum " + " ".join(c.remote for c in changes) + " 2>/dev/null")
    have = parse_hashes(out) if rc == 0 else {}
    for c in changes:
        if have.get(c.remote) != c.new:
            return "%s on the set is not the file that was sent" % c.remote
    rc, out = tv.run("cd %s && sh -n tools/lumatv && sh -n tools/compositor.sh" % APP)
    if rc != 0:
        return "the new tool files do not parse: %s" % out.strip()[-200:]
    if not helper_state(tv):
        return "the new tools/lumatv does not answer `helper status`"
    return None


def rollback_tool(tv, attempted, saved):
    """Puts the previous tool files back for every file a swap was attempted on, then reads the set's hashes to prove it: a file that was not
    there before must be gone. Says what came of it; every file is tried even when one fails."""
    errors = []
    for c in attempted:
        try:
            if saved.get(c.remote):
                swap_file(tv, saved[c.remote], c.remote)
            else:
                rc, out = tv.run("rm -f %s %s.new" % (c.remote, c.remote))
                if rc != 0:
                    errors.append("could not remove %s: %s" % (c.remote, out.strip()))
        except (RuntimeError, subprocess.SubprocessError, OSError) as e:
            errors.append(str(e))
    try:
        rc, out = tv.run("nice -n 19 sha256sum " + " ".join(c.remote for c in attempted) + " 2>/dev/null")
        if rc not in (0, 1):
            errors.append("the set's files could not be read back (ssh exit %s)" % rc)
        else:
            have = parse_hashes(out)
            errors += ["%s holds %s, not %s" % (c.remote, short(have.get(c.remote)), short(c.old)) for c in attempted if have.get(c.remote) != c.old]
    except (subprocess.SubprocessError, OSError) as e:
        errors.append("the set's files could not be read back (%s)" % e)
    if errors:
        return "THE PREVIOUS TOOL FILES COULD NOT BE PUT BACK (%s): check tools/lumatv and tools/compositor.sh on the set" % "; ".join(errors)
    return "the previous tool files were put back and read back from the set"


def do_tool(tv, changes, backup_dir, say):
    saved = {c.remote: backup(tv, c.remote, c.old, backup_dir, say) for c in changes if c.old}
    attempted = []
    try:
        for c in changes:
            # Listed before the swap: an ssh that dies after the rename took effect cannot be told from one that never ran it.
            attempted.append(c)
            swap_file(tv, c.local, c.remote)
        problem = tool_problem(tv, changes)
    except (RuntimeError, subprocess.SubprocessError, OSError) as e:
        problem = str(e)
    if problem:
        raise RuntimeError("%s; %s" % (problem, rollback_tool(tv, attempted, saved)))
    return "replaced %s; nothing restarted" % ", ".join(os.path.basename(c.remote) for c in changes)


def do_helper(tv, changes, backup_dir, say):
    state = helper_state(tv)
    before = str(state["pid"]) if state.get("running") and state.get("pid") else None
    # Both installed copies hold one build; the payload copy is what `helper start` copies to /var/lib/lumatv/bin. Each is kept under its own old
    # hash, so a failed start can put both back.
    saved = {c.remote: backup(tv, c.remote, c.old, backup_dir, say) for c in changes if c.old}
    problem = None
    try:
        swap_file(tv, changes[0].local, HELPER_PAYLOAD)
        started, out = start_helper(tv)
        after, why = wait_helper(tv, before)
        if started and after:
            return "restarted (pid %s -> %s); the compositor was not touched" % (before or "-", after)
        problem = "helper start failed: %s" % out if not started else "the helper did not come up: %s" % why
    except (RuntimeError, subprocess.SubprocessError, OSError) as e:
        problem = str(e)
    raise RuntimeError("%s; %s" % (problem, rollback_helper(tv, saved, before)))


def rollback_helper(tv, saved, before):
    """Puts the previous helper files back and starts them; says what came of it. The stop comes first: `helper start` leaves a running
    helper alone when the two copies match, which would keep the failed one."""
    try:
        if not saved:
            return "no previous helper file was on the set to put back"
        tv.run("cd %s && ./tools/lumatv helper stop" % APP)
        for remote in (HELPER_PAYLOAD, HELPER_BIN):
            if remote in saved:
                swap_file(tv, saved[remote], remote)
        started, out = start_helper(tv)
        pid, why = wait_helper(tv, None)
        if started and pid:
            return "the previous helper was put back and runs (pid %s)" % pid
        return "THE PREVIOUS HELPER WAS PUT BACK BUT DID NOT COME UP (%s): check `tools/lumatv helper status` and the log" % (out if not started else why)
    except (RuntimeError, subprocess.SubprocessError, OSError) as e:
        return "THE PREVIOUS HELPER COULD NOT BE PUT BACK (%s): the helper may be down" % e


def do_ui(tv, deploy, changes, backup_dir, say):
    lumatv_ui = next((c for c in changes if c.remote == COMP + "/bin/lumatv-ui"), None)
    if lumatv_ui:
        backup(tv, lumatv_ui.remote, lumatv_ui.old, backup_dir, say)
    stage_to_quiet(tv, deploy)
    rc, out = tv.run("LUMATV_D=%s nice -n 19 sh %s/cutover.sh ui-swap; rc=$?; rm -rf %s; exit $rc" % (QUIET, QUIET, QUIET), timeout=300)
    for line in out.strip().splitlines()[-6:]:
        say("  | " + line)
    if rc != 0:
        raise RuntimeError("ui-swap failed (exit %d); the previous UI was put back unless the output above says otherwise" % rc)
    return "swapped; shell client restarted, compositor untouched"


def stage_to_quiet(tv, deploy):
    tv.run("rm -rf %s && mkdir -p %s/bin %s/assets" % (QUIET, QUIET, QUIET))
    tv.put(os.path.join(deploy, "bin", "lumatv-ui"), QUIET + "/bin/lumatv-ui")
    for f in ("cutover.sh", "compositor.sh", "logrelay.py"):
        tv.put(os.path.join(deploy, f), QUIET + "/" + f)
    tv.put_tree(os.path.join(deploy, "assets"), QUIET + "/assets")


def do_staged(tv, deploy, parts, changes, now, say, defer, held, backup_dir):
    # The apply drops the previous files once the video check passes, so a rollback copy is kept here, before anything is staged.
    for part in STAGED_PARTS:
        for c in changes.get(part, ()):
            backup(tv, c.remote, c.old, backup_dir, say)
    rels = staged_rels(parts, changes)
    sources = {r: os.path.join(deploy, r) for r in rels + list(SUPPORT) if os.path.isfile(os.path.join(deploy, r))}
    if defer and "helper" in changes:
        sources[HELPER_REL] = changes["helper"][0].local
        rels.append(HELPER_REL)
    with tempfile.TemporaryDirectory() as tmp:
        dep = os.path.join(tmp, "dep.new")
        os.makedirs(dep)
        if defer:
            with open(os.path.join(dep, "boot-only"), "w"):
                pass
        for rel, src in sources.items():
            dest = os.path.join(dep, rel)
            os.makedirs(os.path.dirname(dest), exist_ok=True)
            try:
                os.link(src, dest)
            except OSError:
                shutil.copy2(src, dest)
        man = os.path.join(tmp, "manifest.new")
        with open(man, "w") as f:
            f.write(manifest_text(dep, tree(dep)))
        rc, out = tv.run("mkdir -p %s && rm -rf %s/dep.new %s/manifest.new && mkdir -p %s/dep.new" % ((STAGED,) * 4))
        if rc != 0:
            raise RuntimeError("could not prepare the staging directory: %s" % out.strip()[-300:])
        tv.put_tree(os.path.join(tmp, "dep.new"), STAGED + "/dep.new")
        tv.put(man, STAGED + "/manifest.new")
    held()
    rc, out = tv.run("LUMATV_D=%s/dep.new sh %s/dep.new/cutover.sh staged commit; rc=$?; exit $rc" % (STAGED, STAGED))
    if rc != 0:
        raise RuntimeError("staging failed: %s" % out.strip()[-300:])
    say("  staged %d file(s): %s" % (len(rels), ", ".join(rels[:6]) + (", ..." if len(rels) > 6 else "")))
    if defer:
        return "staged; only the early boot hook can apply it (nothing restarted)"
    if not now:
        return "staged; applies at the next boot, before the compositor starts (or run again with --now)"
    held()
    rc, out = tv.run("LUMATV_D=%s/dep sh %s/dep/cutover.sh staged apply --now" % (STAGED, STAGED), timeout=900)
    last = out.strip().splitlines()[-1] if out.strip() else ""
    if rc == 0:
        return "applied now (disable, install, probe, enable, video check)"
    if rc == 10:
        return "staged, not applied now: %s" % last
    if rc == 12:
        raise RuntimeError("no picture with the new files or the previous ones; the previous files are back and the set was not rebooted: reboot it yourself (exit 12)")
    raise RuntimeError("apply failed and the previous files were put back (exit %d): %s" % (rc, last))


def report_lines(changes, notes):
    lines = []
    for part in PARTS:
        if part not in changes:
            continue
        rows = changes[part]
        if part == "ui":
            summary = "%d file(s)" % len(rows)
            main = next((c for c in rows if c.remote.endswith("/bin/lumatv-ui")), None)
            if main:
                summary += "; lumatv-ui %s -> %s" % (short(main.old), short(main.new))
        else:
            summary = "; ".join("%s %s -> %s" % (c.remote.replace(APP + "/", "").replace(COMP + "/", "").replace(DATA + "/", ""), short(c.old), short(c.new)) for c in rows[:8])
            if len(rows) > 8:
                summary += "; +%d more" % (len(rows) - 8)
        lines.append("- %s: %s%s" % (part, summary, " [%s]" % notes[part] if part in notes else ""))
    return lines


def inventory(tv, parts):
    """Installed hashes and staged-state markers. Fail closed if inventory was cut short."""
    rc, out = tv.run(inventory_script(parts))
    lines = out.splitlines()
    if rc != 0 or not lines or lines[-1].strip() != SENTINEL:
        raise RuntimeError("the inventory of the set failed (ssh exit %s): %s" % (rc, out.strip()[-200:]))
    remote = parse_hashes(out)
    if parts and not remote:
        raise RuntimeError("the set reported no installed file at the expected paths: this tool updates an installed set, it does not install one")
    return remote, {line.strip() for line in lines}


def discard_staged(tv):
    rc, out = tv.run("sh %s/dep/cutover.sh staged discard" % STAGED)
    if rc != 0:
        raise RuntimeError("the old staged set could not be discarded (exit %s): %s" % (rc, out.strip()[-200:]))


def refresher_problem(tv):
    """Why nothing may be written to the set now: LG's pixel refresher is running, and anything that wakes the set or restarts what is on
    screen cuts it short (compositor/tools/refresh_guard.py reads eplmanager's state file and the helper). None when it is not running."""
    if HERE not in sys.path:
        sys.path.insert(0, HERE)
    import refresh_guard

    def run(cmd, timeout=20):
        return tv.run(cmd, timeout)[1]
    return refresh_guard.refusal(run)


def install(tv, repo, deploy, helper, backup_dir, check=False, now=False, only=None, say=print, lease=None, defer=False):
    if defer and now:
        raise ValueError("--defer and --now cannot be used together")
    problem = stale_deploy(repo, deploy)
    if problem:
        say(problem)
        return 2
    parts = local_parts(repo, deploy, helper)
    if defer and only and "helper" in only and "helper" not in parts:
        say("FAILED before any change: the selected helper binary is missing")
        return 4
    if defer and only and "ui" in only and "ui" not in parts:
        say("FAILED before any change: the selected UI build is missing")
        return 4
    if only:
        parts = {p: v for p, v in parts.items() if p in only or (defer and p == "tool")}
    try:
        remote, marks = inventory(tv, parts)
    except (RuntimeError, subprocess.SubprocessError, OSError) as e:
        say("FAILED before any change: %s" % e)
        return 4
    changes = compare(parts, remote)
    pending, boot_pending = PENDING in marks, BOOT_PENDING in marks
    actions = choose(set(changes), defer)
    if boot_pending:
        say("a boot apply is awaiting confirmation, with the previous files kept for its rollback")
    if not check:
        why = refresher_problem(tv)
        if why:
            say("FAILED before any change: " + why)
            return 4
        if boot_pending:
            say("FAILED before any change: nothing is written while it is pending. Confirm it by hand with `tools/lumatv staged accept` "
                "(panel on, Home in front, a picture on the video check), then run this again")
            return 4
        if APPLYING in marks or (pending and (defer or DEFERRED in marks)):
            say("FAILED before any change: an existing staged set is kept; check `tools/lumatv staged status`")
            return 4
    if defer and only and "tool" not in only and "tool" in changes:
        say("FAILED before any change: the boot hook needs updating; include tool in --only")
        return 4
    if defer and "ui" in changes and not (os.path.isfile(os.path.join(deploy, "bin", "lumatv-ui")) and
                                          any(i.remote.startswith(ASSETS_UI) for i in parts["ui"])):
        say("FAILED before any change: deferring the UI requires its binary and asset tree")
        return 4

    def held():
        problem = (lease() if lease else None) or refresher_problem(tv)
        if problem:
            raise RuntimeError(problem)

    try:
        if not changes:
            say("nothing differs from what is installed")
            if pending and not check and not only:
                held()
                discard_staged(tv)
                say("a staged set that no longer differs from the installed files was discarded")
            return 0
        say("differs from the installed files: " + ", ".join(p for p in PARTS if p in changes) + " -> " + " + ".join(actions))
        if check:
            for line in report_lines(changes, {}):
                say(line)
            return 0
        if pending and "staged" not in actions and not only:
            held()
            discard_staged(tv)
            say("an older staged set no longer matches the build and was discarded")
    except (RuntimeError, subprocess.SubprocessError, OSError) as e:
        say("FAILED before any change: %s" % e)
        return 4
    notes = {}

    try:
        if "tool" in actions:
            held()
            notes["tool"] = do_tool(tv, changes["tool"], backup_dir, say)
            say("tool: " + notes["tool"])
        if "helper" in actions:
            held()
            notes["helper"] = do_helper(tv, changes["helper"], backup_dir, say)
            say("helper: " + notes["helper"])
        if "ui" in actions:
            held()
            notes["ui"] = do_ui(tv, deploy, changes["ui"], backup_dir, say)
            say("ui: " + notes["ui"])
        if "staged" in actions:
            held()
            how = do_staged(tv, deploy, parts, changes, now, say, defer, held, backup_dir)
            for p in STAGED_PARTS + ("ui",) + (("helper",) if defer else ()):
                if p in changes:
                    notes[p] = how
            say("staged: " + how)
    except (RuntimeError, subprocess.SubprocessError, OSError) as e:
        say("FAILED: %s" % e)
        for line in report_lines(changes, notes):
            say(line)
        return 1
    say("")
    say("For the HANDOFF entry (old -> new, 8 hex):")
    for line in report_lines(changes, notes):
        say(line)
    return 0


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--check", action="store_true", help="only compare and print what would be done")
    activation = ap.add_mutually_exclusive_group()
    activation.add_argument("--now", action="store_true", help="apply the staged part now when Home is in front and no game is running")
    activation.add_argument("--defer", action="store_true", help="stage helper/UI without restarting them; only the early boot hook applies the set")
    ap.add_argument("--only", help="comma-separated parts: " + ",".join(PARTS))
    ap.add_argument("--helper", default=os.path.join(REPO, "helper", "dist", "lumatv-helper"))
    ap.add_argument("--deploy", default=os.path.join(REPO, "compositor", "deploy"))
    ap.add_argument("--backup-dir", default=os.path.join(tempfile.gettempdir(), "lumatv-install-backup"))
    ap.add_argument("--host", default=TV_HOST)
    a = ap.parse_args(argv)
    only = set(a.only.split(",")) if a.only else None
    if only and not only <= set(PARTS):
        ap.error("--only takes: " + ",".join(PARTS))
    if not a.check:
        problem = outer_lease_problem(os.environ)
        if problem:
            print(problem)
            return 3
    problem = require_lease()
    if problem:
        print(problem)
        return 3
    return install(Tv(a.host), REPO, a.deploy, a.helper, a.backup_dir, a.check, a.now, only, lease=require_lease, defer=a.defer)


if __name__ == "__main__":
    sys.exit(main())
