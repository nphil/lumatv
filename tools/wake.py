#!/usr/bin/env python3
"""Stage and recover the native bootd wake policy without rebooting the TV."""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import subprocess
import sys
import tempfile
import time
import uuid

HOME = 'com.webos.app.home'
SAVED = Path('/var/luna/preferences/bootd_last_foreground_app_id.json')
STOCK = Path('/etc/palm/action-condition.json')
STATE = Path('/mnt/lg/cmn_data/lumatv/wake')
POLICY = STATE / 'action-condition.json'
MAIN_DB = Path('/var/preferences/configd_db.json')
FACTORY_DB = Path('/var/preferences/configd_factory_db.json')
SNAPSHOT = STATE / 'original.json'
SELECTOR = STATE / 'selector.py'
TRIAL = STATE / 'trial.json'
PENDING = STATE / 'pending.json'
REPORT = STATE / 'boot-report.json'
FAILURE = STATE / 'failure.json'
DROPIN = Path('/run/systemd/system/bootd.service.d/90-lumatv-wake-trial.conf')
LOG = Path('/var/log/bootd.log')
PATH_KEY = 'com.webos.bootManager.actionConditionConfPath'
REASONS_KEY = 'com.webos.service.tvpower.fullHomeBootingPowerOnReasons'
REASONS = ('wakeOnLan', 'remoteKey', 'localKey', 'micomPower', 'lastPowerOn')
FINGERPRINT = '3965ad323e398dc6ac544c0ab7307a07e2df1c3e47e2e7ef7450e3881500cfaf'
OWNER = 'lumatv-native-wake-1'
GUARD = '# LumaTV native wake trial\n[Service]\nRestart=no\nStartLimitAction=none\n'
APP_ID = re.compile(r'[A-Za-z0-9][A-Za-z0-9._-]{0,127}\Z', re.ASCII)


def saved_id(path=None):
    try:
        with open(SAVED if path is None else path, 'rb') as f:
            raw = f.read(4097)
        if len(raw) <= 4096:
            value = json.loads(raw.decode('ascii')).get('lastAppId')
            if isinstance(value, str) and APP_ID.fullmatch(value):
                return value
    except (OSError, ValueError, UnicodeError, AttributeError):
        pass
    return None


def select(path=None):
    return saved_id(path) or HOME


def fingerprint(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':')).encode()).hexdigest()


def make_policy(stock, selector=None):
    selector = SELECTOR if selector is None else selector
    policy = json.loads(json.dumps(stock))
    actions = {a['name']: a for a in policy['actions']}
    cover = json.loads(json.dumps(actions['Cover App']))
    for name in ('Input App', 'Cover App'):
        actions[name]['conditions'].append('Luma Normal Resume - OFF')
    cover['name'] = 'Luma Cover App'
    cover['payload']['fallback']['id'] = '${Luma Resume App}'
    cover['payload']['finalFallback'] = {'id': HOME}
    cover['conditions'].extend(['Luma Normal Resume - ON', 'Luma Resume App'])
    policy['actions'].insert(policy['actions'].index(actions['Cover App']) + 1, cover)
    for name, conjunction, op in [('ON', 'or', 'equal'), ('OFF', 'and', 'notequal')]:
        policy['conditions'].append({
            'name': 'Luma Normal Resume - ' + name, 'type': 'AppLaunchCondition',
            'command': {'type': 'comp', 'target': '${PowerOnReason}'},
            'where': {conjunction: [{'properties': ['reason'], 'operation': op, 'value': reason}
                                    for reason in REASONS]},
            'conditions': ['PowerOnReason'],
        })
    # LG’s native shell condition removes the final stdout byte as a newline.
    command = "timeout 2 python3 %s select 2>/dev/null || printf '%%s\\n' %s" % (shlex.quote(str(selector)), HOME)
    policy['conditions'].append({
        'name': 'Luma Resume App', 'type': 'AbsCondition',
        'command': {'type': 'sh', 'target': command},
        'where': {'operation': 'notequal', 'value': ''},
        'conditions': ['Luma Normal Resume - ON'],
    })
    return policy


def atomic_bytes(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp = tempfile.mkstemp(prefix='.wake-', dir=str(path.parent))
    try:
        with os.fdopen(fd, 'wb') as f:
            f.write(data)
            f.flush()
            os.fsync(f.fileno())
        os.replace(tmp, path)
        directory = os.open(str(path.parent), os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        if os.path.exists(tmp):
            os.unlink(tmp)


def atomic(path, value):
    atomic_bytes(path, json.dumps(value, separators=(',', ':')).encode())


def read_json(path):
    with path.open('rb') as f:
        raw = f.read(65537)
    if len(raw) > 65536:
        raise RuntimeError('wake state too large')
    return json.loads(raw)


def call(uri, payload):
    result = subprocess.run(['luna-send', '-n', '1', uri, json.dumps(payload)],
                            stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=4)
    data = json.loads(result.stdout)
    if result.returncode or not data.get('returnValue'):
        raise RuntimeError('LS2 call failed: ' + uri)
    return data


def luna(method, payload):
    return call('luna://com.webos.service.config/' + method, payload)


def configs():
    return luna('getConfigs', {'configNames': [PATH_KEY, REASONS_KEY]}).get('configs', {})


def set_configs(values, volatile=False):
    payload = {'configs': values}
    if volatile:
        payload['volatile'] = True
    luna('setConfigs', payload)
    now = configs()
    if any(now.get(k) != v for k, v in values.items()):
        raise RuntimeError('configd readback differs: ' + json.dumps(
            {'requested': values, 'observed': now, 'volatile': volatile}))


def foreground(allow_empty=False):
    value = call('luna://com.webos.applicationManager/getForegroundAppInfo', {}).get('appId')
    if allow_empty and value == '':
        return value
    if not isinstance(value, str) or not APP_ID.fullmatch(value):
        raise RuntimeError('foreground unavailable')
    return value


def boot_status():
    return call('luna://com.webos.bootManager/getBootStatus', {})


def boot_id():
    return Path('/proc/sys/kernel/random/boot_id').read_text().strip()


def systemctl(*args):
    result = subprocess.run(['systemctl', '--no-pager', *args], stdin=subprocess.DEVNULL,
                            capture_output=True, text=True, timeout=6)
    if result.returncode:
        raise RuntimeError('systemctl ' + args[0] + ' failed')
    return result.stdout


def guard_install():
    if DROPIN.is_symlink() or (DROPIN.exists() and DROPIN.read_text() != GUARD):
        raise RuntimeError('foreign bootd trial drop-in')
    if not DROPIN.exists():
        atomic_bytes(DROPIN, GUARD.encode())
    systemctl('daemon-reload')
    props = dict(line.split('=', 1) for line in
                 systemctl('show', 'bootd.service', '--property=Restart', '--property=StartLimitAction').splitlines()
                 if '=' in line)
    if props.get('Restart') != 'no' or props.get('StartLimitAction') != 'none':
        raise RuntimeError('bootd restart guard not effective')


def guard_remove():
    if DROPIN.is_symlink() or (DROPIN.exists() and DROPIN.read_text() != GUARD):
        raise RuntimeError('foreign bootd trial drop-in')
    if DROPIN.exists():
        DROPIN.unlink()
        systemctl('daemon-reload')


class JobUnresolved(RuntimeError):
    pass


def service(action):
    try:
        systemctl(action, 'bootd.service')
    except subprocess.TimeoutExpired:
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            try:
                props = dict(line.split('=', 1) for line in systemctl(
                    'show', 'bootd.service', '--property=Job', '--property=ActiveState',
                    '--property=Result', '--property=ExecMainStatus').splitlines() if '=' in line)
            except (OSError, ValueError, RuntimeError, subprocess.SubprocessError):
                props = {}
            if 'Job' in props and not props['Job']:
                if (props.get('ActiveState') == 'active' and props.get('Result') == 'success'
                        and props.get('ExecMainStatus') == '0'):
                    return
                if all(key in props for key in ('ActiveState', 'Result', 'ExecMainStatus')):
                    raise RuntimeError('bootd job completed unsuccessfully')
            time.sleep(1)
        raise JobUnresolved('bootd job remains unresolved; guard retained')


def log_cursor():
    s = LOG.stat()
    return {'inode': s.st_ino, 'size': s.st_size, 'monotonic_ns': time.monotonic_ns()}


def log_witness(since=None):
    with LOG.open('rb') as f:
        stat = os.fstat(f.fileno())
        # bootd opens this log with O_TRUNC on every restart. Its native
        # PERFORMANCE clock is CLOCK_MONOTONIC once the TV is running.
        start = max(0, stat.st_size - 1048576)
        f.seek(start)
        lines = f.read(1048576).decode('utf-8', 'replace').splitlines()
    starts = [i for i, line in enumerate(lines) if 'Boot) BOOTD_START' in line]
    if not starts:
        return {'condition': False, 'action': False}
    lines = lines[starts[-1]:]
    if since is not None:
        stamp = re.fullmatch(r'PERFORMANCE\( *(\d+)\.(\d{9})\) : Boot\) BOOTD_START', lines[0])
        if stamp is None:
            raise RuntimeError('bootd restart timestamp missing or malformed')
        started_ns = int(stamp[1]) * 1000000000 + int(stamp[2])
        now_ns = time.monotonic_ns()
        if not since['monotonic_ns'] <= started_ns <= now_ns:
            raise RuntimeError('bootd restart timestamp is outside this trial: '
                               + str(started_ns) + ' not in [' + str(since['monotonic_ns'])
                               + ', ' + str(now_ns) + ']')
    expected = str(POLICY)
    condition = any('Condition) Read configuration (' + expected + ')' in line for line in lines)
    action = any('Action) Read configuration (' + expected + ')' in line for line in lines)
    return {'condition': condition, 'action': action}


def witness_ok(witness):
    return witness.get('condition') is True and witness.get('action') is True


def snapshot():
    saved = read_json(SNAPSHOT)
    if saved.get('owner') != OWNER or saved.get('stock_fingerprint') != FINGERPRINT:
        raise RuntimeError('foreign wake snapshot')
    return saved


def owned(saved):
    stock = read_json(STOCK)
    if fingerprint(stock) != FINGERPRINT:
        raise RuntimeError('unsupported native action policy')
    if (STATE.is_symlink() or any(p.is_symlink() for p in (POLICY, SELECTOR, SNAPSHOT, TRIAL, PENDING, REPORT, FAILURE))
            or read_json(POLICY) != make_policy(stock)
            or SELECTOR.read_bytes() != Path(__file__).read_bytes()):
        raise RuntimeError('wake files changed or are not owned')
    c = configs()
    if (c.get(PATH_KEY) not in (saved['prior_path'], str(STOCK), str(POLICY)) or
            c.get(REASONS_KEY) not in (saved['prior_reasons'], saved['active_reasons'])):
        raise RuntimeError('config ownership changed')
    return c


def original(saved):
    return {PATH_KEY: saved['prior_path'] or str(STOCK), REASONS_KEY: saved['prior_reasons']}


def active(saved):
    return {PATH_KEY: str(POLICY), REASONS_KEY: saved['active_reasons']}


def database_entries(path):
    with path.open('rb') as f:
        raw = f.read(1048577)
    if len(raw) > 1048576:
        raise RuntimeError('configd database too large')
    database = json.loads(raw)
    if not isinstance(database, dict):
        raise RuntimeError('invalid configd database')
    entries = {}
    for key in (PATH_KEY, REASONS_KEY):
        namespace, leaf = key.rsplit('.', 1)
        section = database.get(namespace, {})
        if not isinstance(section, dict):
            raise RuntimeError('invalid configd namespace')
        if leaf in section:
            entries[key] = section[leaf]
    return entries


def factory_preflight(saved):
    expected = original(saved)
    main = database_entries(MAIN_DB)
    if main.get(PATH_KEY, str(STOCK)) != expected[PATH_KEY] or main.get(REASONS_KEY) != expected[REASONS_KEY]:
        raise RuntimeError('unsafe configd main fallback')
    factory = database_entries(FACTORY_DB)
    if any(value != expected[key] for key, value in factory.items()):
        raise RuntimeError('foreign configd factory override')
    return factory


def stage():
    stock = read_json(STOCK)
    if fingerprint(stock) != FINGERPRINT:
        raise RuntimeError('unsupported native action policy')
    c = configs()
    if c.get(PATH_KEY) not in (None, str(STOCK)):
        raise RuntimeError('another owner controls bootd policy')
    reasons = c.get(REASONS_KEY)
    if not isinstance(reasons, list) or any(not isinstance(x, str) for x in reasons):
        raise RuntimeError('invalid full-home reason list')
    if STATE.is_symlink() or any(p.is_symlink() for p in (POLICY, SELECTOR, SNAPSHOT, TRIAL, PENDING, REPORT, FAILURE)):
        raise RuntimeError('unowned wake files')
    if PENDING.exists():
        raise RuntimeError('cold trial already pending')
    if SNAPSHOT.exists():
        saved = snapshot()
        owned(saved)
        if original(saved)[REASONS_KEY] != reasons or c.get(PATH_KEY) not in (saved['prior_path'], str(STOCK)):
            raise RuntimeError('existing original differs; refusing overwrite')
    else:
        if any(p.exists() for p in (POLICY, SELECTOR, TRIAL, REPORT, FAILURE)):
            raise RuntimeError('unowned wake files')
        saved = {'owner': OWNER, 'stock_fingerprint': FINGERPRINT, 'prior_path': c.get(PATH_KEY),
                 'prior_reasons': reasons, 'active_reasons': [x for x in reasons if x not in REASONS]}
        atomic(SNAPSHOT, saved)
        atomic_bytes(SELECTOR, Path(__file__).read_bytes())
        atomic(POLICY, make_policy(stock))
    if TRIAL.exists():
        TRIAL.unlink()
    return {'ok': True, 'phase': 'staged', 'original_path_was_absent': saved['prior_path'] is None}


def rollback(saved, restart):
    errors = []
    if restart:
        try:
            guard_install()
        except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as exc:
            errors.append(str(exc))
    # Persistent writes update Factory, which takes precedence over volatile values.
    for volatile in (False, True):
        try:
            set_configs(original(saved), volatile=volatile)
        except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as exc:
            errors.append(str(exc))
    if restart and not errors:
        try:
            service('restart')
        except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as exc:
            errors.append(str(exc))
    if not errors:
        try:
            deadline = time.monotonic() + 5
            while True:
                try:
                    boot_status()
                    break
                except (OSError, ValueError, RuntimeError, subprocess.SubprocessError):
                    if time.monotonic() >= deadline:
                        raise RuntimeError('restored bootd did not become ready; guard retained')
                    time.sleep(1)
            guard_remove()
        except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as exc:
            errors.append(str(exc))
    return errors


def record_failure(phase, exc, saved, restart, observations=None, mutated=True, allowed_factory=None):
    failure = {'ok': False, 'phase': phase, 'error': str(exc), 'rollback_started': mutated}
    if observations is not None:
        failure['observations'] = observations
    atomic(FAILURE, failure)
    if isinstance(exc, JobUnresolved):
        failure['rollback_errors'] = ['bootd job unresolved; no overlapping recovery']
        atomic(FAILURE, failure)
        return failure
    if not mutated:
        try:
            guard_remove()
            errors = []
        except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
            errors = [str(error)]
    else:
        errors = []
        if allowed_factory is not None:
            try:
                if database_entries(FACTORY_DB) not in allowed_factory:
                    raise RuntimeError('configd factory ownership changed; guard retained')
            except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
                errors.append(str(error))
        if not errors:
            errors = rollback(saved, restart)
        elif restart:
            try:
                guard_install()
            except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
                errors.append(str(error))
    failure['rollback_errors'] = errors
    atomic(FAILURE, failure)
    return failure


def trial():
    saved = snapshot()
    owned(saved)
    if PENDING.exists():
        raise RuntimeError('cold trial already pending')
    if TRIAL.exists():
        raise RuntimeError('trial already attempted; stage again before retry')
    factory = factory_preflight(saved)
    atomic(TRIAL, {'owner': OWNER, 'passed': False, 'prior_factory': factory})
    restart_attempted = False
    mutated = False
    observations = {name: {'value': None, 'error': None} for name in
                    ('foreground_before', 'cursor_before', 'readiness', 'native_policy_log',
                     'foreground_after', 'cursor_after')}

    def observe(name, check, *args, required=False):
        try:
            value = check(*args)
        except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
            observations[name]['error'] = {'type': type(error).__name__, 'message': str(error)}
            if required:
                raise
            return False, None
        observations[name]['value'] = value
        return True, value

    try:
        guard_install()
        _, before = observe('foreground_before', foreground, required=True)
        _, cursor = observe('cursor_before', log_cursor, required=True)
        if factory_preflight(saved) != factory:
            raise RuntimeError('configd factory changed during trial')
        if factory:
            mutated = True
            luna('setConfigs', {'configs': {key: None for key in factory}})
        if database_entries(FACTORY_DB) or factory_preflight(saved):
            raise RuntimeError('configd factory overrides remain')
        current = configs()
        if (current.get(PATH_KEY, str(STOCK)) != original(saved)[PATH_KEY] or
                current.get(REASONS_KEY) != original(saved)[REASONS_KEY]):
            raise RuntimeError('configd defaults changed during trial')
        mutated = True
        set_configs(active(saved), volatile=True)
        restart_attempted = True
        service('restart')
        deadline = time.monotonic() + 15
        while True:
            ready, _ = observe('readiness', boot_status)
            witnessed, witness = observe('native_policy_log', log_witness, cursor)
            foreground_read, after = observe('foreground_after', foreground)
            observe('cursor_after', log_cursor)
            if ready and witnessed and foreground_read and witness_ok(witness) and after == before:
                break
            if time.monotonic() >= deadline:
                raise RuntimeError('trial readiness, native policy log or foreground check failed')
            time.sleep(1)
        result = {'ok': True, 'phase': 'trial', 'passed': True, 'foreground': before,
                  'native_policy_log': witness, 'boot_id': boot_id()}
        guard_remove()
        atomic(TRIAL, dict(result, owner=OWNER, prior_factory=factory))
        return result
    except (OSError, ValueError, KeyError, RuntimeError, subprocess.SubprocessError) as exc:
        return record_failure('trial', exc, saved, restart_attempted, observations,
                              mutated=mutated, allowed_factory=({}, factory))


def persist():
    saved = snapshot()
    current = owned(saved)
    if any(current.get(k) != v for k, v in active(saved).items()):
        raise RuntimeError('runtime trial configuration no longer active')
    trial_result = read_json(TRIAL)
    if trial_result.get('owner') != OWNER or trial_result.get('passed') is not True or trial_result.get('boot_id') != boot_id():
        raise RuntimeError('passed runtime trial required in this boot')
    if PENDING.exists():
        raise RuntimeError('cold trial already pending')
    expected = foreground()
    pending = {'owner': OWNER, 'boot_id': boot_id(), 'expected_foreground': expected, 'state': 'armed',
               'activation_token': uuid.uuid4().hex}
    atomic(PENDING, pending)
    if REPORT.exists():
        REPORT.unlink()
    try:
        set_configs(active(saved))
        set_configs(active(saved), volatile=True)
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as exc:
        pending['state'] = 'failed'
        atomic(PENDING, pending)
        return record_failure('persist', exc, saved, True,
                              allowed_factory=({}, trial_result.get('prior_factory', {}),
                                               original(saved), active(saved)))
    return {'ok': True, 'phase': 'pending', 'acceptance': 'PENDING coldboot acceptance',
            'expected_foreground': expected, 'boot_id': pending['boot_id']}


def native_cycle():
    with LOG.open('rb') as f:
        f.seek(max(0, os.fstat(f.fileno()).st_size - 1048576))
        text = f.read().decode('utf-8', 'replace')
    lines = text.splitlines()
    starts = [i for i, line in enumerate(lines) if 'Boot) BOOTD_START' in line or
              ('onPowerStatusChange:' in line and 'subProcessing(prepare power on)' in line)]
    if not starts:
        raise RuntimeError('native wake cycle is missing')
    lines = lines[starts[-1]:]
    stamp = re.match(r'\w+\( *(\d+)\.(\d{9})\)', lines[0])
    if not stamp or not 0 <= time.monotonic_ns() - (int(stamp[1]) * 1000000000 + int(stamp[2])) <= 120000000000:
        raise RuntimeError('native wake cycle is stale')
    return '\n'.join(lines)


def wrapper_destination(raw):
    # The snapshot converter emits the logical input as a direct params field.
    # A truncated line is accepted only through its complete, unescaped prefix.
    def unique_pairs(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError('duplicate JSON field')
            result[key] = value
        return result
    try:
        payload = json.loads(raw, object_pairs_hook=unique_pairs)
    except json.JSONDecodeError:
        if '\\' in raw or len(re.findall(r'"changeRunningAppId"\s*:', raw)) != 1:
            return None
        prefix = re.match(r'\{\s*"id"\s*:\s*"com\.webos\.app\.inputcommon"\s*,\s*'
                          r'"noSplash"\s*:\s*true\s*,\s*"params"\s*:\s*\{', raw)
        if not prefix:
            return None
        decoder = json.JSONDecoder(object_pairs_hook=unique_pairs)
        pos = prefix.end()
        seen = set()
        while True:
            try:
                while pos < len(raw) and raw[pos].isspace():
                    pos += 1
                key, end = decoder.raw_decode(raw, pos)
                if not isinstance(key, str) or key in seen:
                    return None
                seen.add(key)
                pos = end
                while pos < len(raw) and raw[pos].isspace():
                    pos += 1
                if pos >= len(raw) or raw[pos] != ':':
                    return None
                pos += 1
                while pos < len(raw) and raw[pos].isspace():
                    pos += 1
                value, pos = decoder.raw_decode(raw, pos)
                if key == 'changeRunningAppId':
                    tail = raw[pos:].lstrip()
                    if not tail or tail[0] not in ',}':
                        return None
                    return value if isinstance(value, str) and APP_ID.fullmatch(value) else None
                while pos < len(raw) and raw[pos].isspace():
                    pos += 1
                if pos >= len(raw) or raw[pos] != ',':
                    return None
                pos += 1
            except ValueError:
                return None
    except ValueError:
        return None
    if (not isinstance(payload, dict) or payload.get('id') != 'com.webos.app.inputcommon' or
            len(re.findall(r'"id"\s*:\s*"com\.webos\.app\.inputcommon"', raw)) != 1):
        return None
    params = payload.get('params')
    app = params.get('changeRunningAppId') if isinstance(params, dict) else None
    literal = re.findall(r'"changeRunningAppId"\s*:\s*"([A-Za-z0-9._-]+)"', raw)
    return app if isinstance(app, str) and APP_ID.fullmatch(app) and literal == [app] else None


def native_delivered(text):
    lines = text.splitlines()
    # Inspect newest successful actions first; an ambiguous later wrapper must
    # not revive an earlier, superseded input.
    for i in range(len(lines) - 1, -1, -1):
        match = re.search(r'Execute AbsAction \(([^)]+)\) response \((.*)\)$', lines[i])
        if not match or match[1] in ('Cover App', 'Luma Cover App'):
            continue
        try:
            response = json.loads(match[2])
        except ValueError:
            continue
        if response.get('returnValue') is not True:
            continue
        for j in range(i - 1, max(-1, i - 13), -1):
            if 'Action) Execute AbsAction (' in lines[j] and ' response (' in lines[j]:
                break
            request = re.search(r'Client\) Request\s*:\s*(\{.*)$', lines[j])
            if not request:
                continue
            raw = request[1]
            prefix = re.match(r'\{\s*"id"\s*:\s*"([^"\\]*)"', raw)
            try:
                parsed = json.loads(raw)
            except ValueError:
                parsed = None
            is_wrapper = ((prefix and prefix[1] == 'com.webos.app.inputcommon') or
                          (isinstance(parsed, dict) and parsed.get('id') == 'com.webos.app.inputcommon'))
            if is_wrapper:
                if (match[1] != 'Input App' or j == 0 or
                        'Client) Call luna://com.webos.applicationManager/launch' not in lines[j-1] or
                        any('Client) Request' in line for line in lines[j+1:i]) or i + 1 >= len(lines) or
                        'Action) Set first action (Input App) appId (\u0024{Last Input App})' not in lines[i+1]):
                    return None
                inputs = []
                for event in lines[:j]:
                    hit = re.search(r"Event\) Trigger 'Last Input App' event \((.*)\)$", event)
                    if hit:
                        try:
                            inputs.append(json.loads(hit[1]))
                        except ValueError:
                            return None
                app = wrapper_destination(raw)
                if app and inputs and app == inputs[-1]:
                    return {'appId': app, 'params': None, 'source': 'native-input-id-only'}
                return None
            full = re.fullmatch(r'\{.*\}', raw)
            if full:
                payload = parsed
                if not isinstance(payload, dict):
                    return None
                app = payload.get('id')
                params = payload.get('params', {})
                if isinstance(app, str) and APP_ID.fullmatch(app) and isinstance(params, dict):
                    return {'appId': app, 'params': params,
                            'source': 'native-input' if match[1] == 'Input App' else 'native-action'}
            elif match[1] == 'Input App' and prefix:
                last_input = None
                for event in lines[:j]:
                    hit = re.search(r"Event\) Trigger 'Last Input App' event \((.*)\)$", event)
                    if hit:
                        try:
                            last_input = json.loads(hit[1])
                        except ValueError:
                            return None
                if prefix[1] == last_input and APP_ID.fullmatch(prefix[1]):
                    return {'appId': prefix[1], 'params': None, 'source': 'native-input-id-only'}
            return None
        return None
    return None


def boot_start_marker(text):
    match = re.search(r'PERFORMANCE\( *(\d+\.\d{9})\) : Boot\) BOOTD_START', text)
    return match[1] if match else None


def inherited_foreground(text):
    if not text.splitlines() or 'Boot) BOOTD_START' not in text.splitlines()[0]:
        return None
    for line in text.splitlines():
        match = re.search(r'Respawn\) (\{.*\})$', line)
        if match:
            try:
                data = json.loads(match[1])
            except ValueError:
                return None
            app = data.get('foregroundApp')
            if data.get('respawned') is True and isinstance(app, str) and APP_ID.fullmatch(app):
                return {'appId': app, 'params': None, 'source': 'native-respawn'}
            return None
    return None


def resolve_intent(text, saved_app):
    events = {}
    relevant = {'Hotkey', 'Hotkey - ON', 'Hotkey - OFF', 'OTT GIP', 'OTT GIP - ON',
                'OTT GIP - OFF', 'Luma Normal Resume - ON', 'Luma Normal Resume - OFF',
                'Luma Resume App', 'OnTimer - OFF', 'Remind - OFF', 'EWBS - OFF',
                'Auto Browser - OFF', 'Last App Booting - OFF'}
    for line in text.splitlines():
        match = re.search(r"Event\) Trigger '([^']+)' event \((.*)\)$", line)
        if match and match[1] in relevant:
            try:
                name = match[1]
                if name.endswith(' - ON'):
                    events.pop(name[:-5] + ' - OFF', None)
                elif name.endswith(' - OFF'):
                    events.pop(name[:-6] + ' - ON', None)
                events[name] = json.loads(match[2])
            except ValueError:
                raise RuntimeError('malformed native event')
    for name, source in (('Hotkey', 'hotkey'), ('OTT GIP', 'ott')):
        if name + ' - ON' in events:
            value = events.get(name)
            if (not isinstance(value, dict) or not isinstance(value.get('id'), str)
                    or not APP_ID.fullmatch(value['id']) or not isinstance(value.get('params', {}), dict)):
                raise RuntimeError('invalid native destination')
            return {'appId': value['id'], 'params': value.get('params', {}), 'source': source}
        if name + ' - OFF' not in events:
            raise RuntimeError('native ' + name + ' selection is ambiguous')
    delivered = native_delivered(text)
    if delivered and delivered['source'] == 'native-action':
        return delivered
    exclusions = ('OnTimer - OFF', 'Remind - OFF', 'EWBS - OFF', 'Auto Browser - OFF', 'Last App Booting - OFF')
    if 'Luma Normal Resume - ON' in events:
        if (not saved_app or events.get('Luma Resume App') != saved_app or
                any(name not in events for name in exclusions)):
            raise RuntimeError('native resume selection is missing or ambiguous')
        return {'appId': saved_app, 'params': {}, 'source': 'resume'}
    if 'Luma Normal Resume - OFF' in events and delivered:
        return delivered
    raise RuntimeError('native destination is missing or ambiguous')


def boot_ready(native):
    return (native.get('powerStatus') == 'active' and
            isinstance(native.get('signals'), dict) and native['signals'].get('boot-done') is True)


def boot_check(phase, token, context=None, restart_target=None):
    if (phase not in ('prepare', 'landing', 'finalize') or not re.fullmatch(r'[0-9a-f]{32}', token)
            or (context is None and restart_target is not None)
            or (context is not None and (phase != 'landing' or context != 'restart'
                or not isinstance(restart_target, str) or not APP_ID.fullmatch(restart_target)))):
        raise RuntimeError('boot-check requires a phase and invocation token')
    if not SNAPSHOT.exists():
        if configs().get(PATH_KEY) == str(POLICY):
            raise RuntimeError('owned policy has no original snapshot')
        return {'ok': True, 'phase': 'no-active-policy'}
    saved = snapshot()
    current = configs()
    if current.get(PATH_KEY) != str(POLICY) and not PENDING.exists():
        return {'ok': True, 'phase': 'no-active-policy'}
    pending = read_json(PENDING) if PENDING.exists() else None
    if pending is not None and (pending.get('owner') != OWNER or pending.get('state') != 'armed'
                                or not re.fullmatch(r'[0-9a-f]{32}', pending.get('activation_token', ''))):
        raise RuntimeError('pending activation is foreign or failed')
    c = owned(saved)
    if c != active(saved):
        if c.get(PATH_KEY) == str(POLICY):
            raise RuntimeError('active wake configuration differs')
        return {'ok': True, 'phase': 'no-active-policy'}
    if database_entries(FACTORY_DB) != active(saved):
        raise RuntimeError('durable wake configuration differs')
    if FAILURE.exists():
        raise RuntimeError('failed wake policy cannot restart automatically')
    if phase == 'prepare':
        old_report = read_json(REPORT) if REPORT.exists() else {}
        if old_report.get('invocation_token') == token:
            raise RuntimeError('this worker already prepared wake policy')
        activation = pending['activation_token'] if pending else old_report.get('activation_token')
        if not isinstance(activation, str) or not re.fullmatch(r'[0-9a-f]{32}', activation):
            raise RuntimeError('active wake policy has no activation token')
        saved_app = saved_id()
        before = foreground()
        cycle = native_cycle()
        delivered = native_delivered(cycle)
        if delivered is None and boot_start_marker(cycle) != old_report.get('reload_start'):
            delivered = inherited_foreground(cycle)
        cursor = log_cursor()
        report = {'ok': False, 'phase': 'preparing', 'boot_id': boot_id(), 'owner': OWNER,
                  'activation_token': activation, 'invocation_token': token, 'target': None,
                  'foreground_before': before, 'native_delivered': delivered,
                  'saved_fallback': saved_app, 'cursor': cursor,
                  'expected_foreground': pending.get('expected_foreground') if pending else None}
        atomic(REPORT, report)
        try:
            guard_install()
            service('restart')
            deadline = time.monotonic() + 40
            while True:
                try:
                    native = boot_status()
                    witness = log_witness(cursor)
                    if witness_ok(witness) and boot_ready(native):
                        break
                except (OSError, ValueError, RuntimeError, subprocess.SubprocessError):
                    pass
                if time.monotonic() >= deadline:
                    raise RuntimeError('fresh native policy load or boot readiness was not witnessed')
                time.sleep(1)
            guard_remove()
            # Keep the old process's wake intent; use only Luma conditions from the reload.
            deadline = time.monotonic() + 15
            while True:
                try:
                    fresh = LOG.read_text(errors='replace')
                    report['reload_start'] = boot_start_marker(fresh)
                    luma = '\n'.join(line for line in fresh.splitlines()
                                      if "Event) Trigger 'Luma " in line)
                    report['target'] = resolve_intent(cycle + '\n' + luma, saved_app)
                    break
                except (OSError, ValueError, RuntimeError) as exc:
                    if time.monotonic() >= deadline:
                        report.update(phase='intent-unavailable', error=str(exc), native_policy_log=witness,
                                      firstAppId=native.get('firstAppId'))
                        atomic(REPORT, report)
                        return report
                    time.sleep(1)
            selected = report['target']
            if before != selected['appId'] and delivered is None:
                report.update(phase='intent-unavailable', native_policy_log=witness,
                              firstAppId=native.get('firstAppId'),
                              error='foreground differs from selected app; native delivery provenance unavailable')
                atomic(REPORT, report)
                return report
            if before != selected['appId'] and before != delivered['appId']:
                report['native_target'] = selected
                report['target'] = {'appId': before, 'params': None, 'source': 'newer-foreground'}
            report.update(ok=True, phase='prepared', native_policy_log=witness,
                          firstAppId=native.get('firstAppId'))
            atomic(REPORT, report)
            return report
        except (OSError, ValueError, KeyError, RuntimeError, subprocess.SubprocessError) as exc:
            if pending:
                pending['state'] = 'failed'
                atomic(PENDING, pending)
            failure = record_failure('boot-check-prepare', exc, saved, True,
                                     allowed_factory=({}, original(saved), active(saved)))
            report.update(error=str(exc), rollback_errors=failure['rollback_errors'])
            atomic(REPORT, report)
            return report
    report = read_json(REPORT)
    activation = pending['activation_token'] if pending else report.get('activation_token')
    if (report.get('owner') != OWNER or report.get('phase') != 'prepared' or not report.get('ok')
            or report.get('invocation_token') != token or report.get('activation_token') != activation
            or report.get('boot_id') != boot_id() or not witness_ok(log_witness(report['cursor']))):
        raise RuntimeError('stale or foreign boot preparation')
    target = report['target']
    if context == 'restart' and restart_target != target['appId']:
        raise RuntimeError('restart target differs from validated wake target')
    fg = foreground(True) if context == 'restart' else foreground()
    fresh = LOG.read_text(errors='replace')
    later_cycle = 'subProcessing(prepare power on)' in fresh
    if phase == 'landing':
        owned_restart_transition = context == 'restart' and fg in ('', HOME) and not later_cycle
        if later_cycle or (not owned_restart_transition and fg not in (report['foreground_before'], target['appId'])):
            if not fg:
                raise RuntimeError('new wake cycle has no foreground')
            target = {'appId': fg, 'params': None, 'source': 'newer-foreground'}
        return {'ok': True, 'phase': 'landing', **target}
    native = boot_status()
    accepted = boot_ready(native) and not later_cycle and fg == target['appId']
    original_accepted = accepted and (not pending or
        (target['source'] == 'resume' and fg == pending['expected_foreground']))
    report.update(ok=bool(accepted and original_accepted), phase='boot-report', foreground=fg,
                  firstAppId=native.get('firstAppId'), native_ready=boot_ready(native),
                  destination_accepted=accepted, original_scenario_accepted=bool(original_accepted))
    if not report['ok']:
        report['error'] = 'original activation scenario was overridden or destination was not reached'
    atomic(REPORT, report)
    return report


def commit():
    pending = read_json(PENDING)
    report = read_json(REPORT)
    saved = snapshot()
    owned(saved)
    if (pending.get('owner') != OWNER or pending.get('state') != 'armed' or not report.get('ok')
            or report.get('phase') != 'boot-report' or report.get('boot_id') != boot_id()
            or report.get('activation_token') != pending.get('activation_token')
            or not report.get('original_scenario_accepted')):
        raise RuntimeError('passed boot report required before commit')
    PENDING.unlink()
    return {'ok': True, 'phase': 'committed', 'boot_id': report['boot_id']}


def off():
    if not SNAPSHOT.exists():
        if configs().get(PATH_KEY) == str(POLICY):
            raise RuntimeError('owned policy has no original snapshot')
        return {'ok': True, 'phase': 'off'}
    saved = snapshot()
    owned(saved)
    allowed_factory = [{}, original(saved), active(saved)]
    if TRIAL.exists():
        trial_result = read_json(TRIAL)
        if trial_result.get('owner') != OWNER or not isinstance(trial_result.get('prior_factory', {}), dict):
            raise RuntimeError('foreign trial record')
        allowed_factory.append(trial_result.get('prior_factory', {}))
    if database_entries(FACTORY_DB) not in allowed_factory:
        raise RuntimeError('configd factory ownership changed; no restore')
    if PENDING.exists():
        pending = read_json(PENDING)
        if pending.get('owner') != OWNER:
            raise RuntimeError('foreign pending marker')
        pending['state'] = 'off'
        atomic(PENDING, pending)
    errors = rollback(saved, True)
    if errors:
        atomic(FAILURE, {'ok': False, 'phase': 'off', 'rollback_errors': errors})
        return {'ok': False, 'phase': 'off', 'rollback_errors': errors}
    for path in (PENDING, TRIAL):
        if path.exists():
            path.unlink()
    return {'ok': True, 'phase': 'off', 'original_path_was_absent': saved['prior_path'] is None}


def status():
    current = configs()
    phase = 'pending' if PENDING.exists() else ('enabled' if current.get(PATH_KEY) == str(POLICY) else 'off')
    return {'ok': True, 'phase': phase, 'config': current,
            'trial': read_json(TRIAL) if TRIAL.exists() else None,
            'pending': read_json(PENDING) if PENDING.exists() else None,
            'boot_report': read_json(REPORT) if REPORT.exists() else None,
            'failure': read_json(FAILURE) if FAILURE.exists() else None}


def on():
    for step in (stage, trial, persist):
        result = step()
        if not result.get('ok'):
            return result
    return result


def main():
    commands = {'stage': stage, 'trial': trial, 'persist': persist,
                'commit': commit, 'on': on, 'off': off, 'status': status}
    if len(sys.argv) == 2 and sys.argv[1] == 'select':
        print(select())
        return 0
    is_check = len(sys.argv) in (4, 6) and sys.argv[1] == 'boot-check'
    if not is_check and (len(sys.argv) != 2 or sys.argv[1] not in commands):
        raise RuntimeError('usage: wake.py boot-check prepare|landing|finalize TOKEN, or stage|trial|persist|commit|on|off|status|select')
    if sys.argv[1] == 'status':
        result = status()
    else:
        fd = os.open('/tmp/lumatv-wake.lock', os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
        try:
            try:
                fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                raise RuntimeError('another wake operation is in progress')
            result = boot_check(sys.argv[2], sys.argv[3], *(sys.argv[4:] if len(sys.argv) == 6 else [])) if is_check else commands[sys.argv[1]]()
        finally:
            os.close(fd)
    print(json.dumps(result, separators=(',', ':')))
    return 0 if result.get('ok') else 1


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (OSError, ValueError, KeyError, TypeError, RuntimeError, subprocess.SubprocessError) as exc:
        print(json.dumps({'ok': False, 'error': str(exc)}, separators=(',', ':')))
        sys.exit(1)
