#!/usr/bin/env python3
"""Renders the Rust shell's Settings (`lumatv-ui --host --kinds settings`: the engine in-process on Mesa, as the compositor runs it) against
the compositor's stand-in channel, LG's settings service stood in (`qmlhost/lgsettings.py`) and a small fake of the helper, dark and light,
at 100 % and 200 % text, into docs/mocks/host-rust/settings/ as 1280-wide JPEGs (a lean set: the pages one by one, the saver page with its
picker, and a few 200 % and light ones).

    python3 tests/render_rust_settings.py [--out docs/mocks/host-rust/settings] [--only saver,ambient] [--list]

A scene is the page to open, what the helper and the TV answer, and the keys a person presses; the picture is taken at the end. Each
render also runs the host's layout and contrast lint (`--eval lint`); `render` returns its findings. `SCENES` and `render` are what
tests/test_rust_settings.py uses.
"""
import argparse
import concurrent.futures
import copy
import json
import os
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, '..'))
sys.path.insert(0, HERE)
import rust_settings_host as host  # noqa: E402
import lgsettings  # noqa: E402

# the connection manager's status: the bar is quiet while the TV is online
ONLINE = {'connectionmanager/getStatus': {'returnValue': True, 'wired': {'state': 'connected'}, 'wifi': {'state': 'disconnected'}}}

OUT = os.path.join(ROOT, 'docs', 'mocks', 'host-rust', 'settings')
WIDTH = 1280
QUALITY = 80
# the first report of the client, then the page opens, then the keys; a loaded host takes its time
OPEN_AT = 900
KEY_GAP = 450
SETTLE = 1100


def saver_rules():
    answers = [{'id': 'over', 'glyph': 'picture-in-picture', 'title': 'Over'}, {'id': 'inside', 'glyph': 'moon', 'title': 'Inside'},
               {'id': 'wake', 'glyph': 'sun', 'title': 'Wake'}, {'id': 'hide', 'glyph': 'eye-off', 'title': 'Hide'}]
    four = ['over', 'inside', 'wake', 'hide']

    def cls(id, title, glyph, choice, choices=four):
        return {'id': id, 'title': title, 'glyph': glyph, 'choice': choice, 'default': choice, 'choices': choices}
    return {'ok': True, 'answers': answers, 'classes': [
        cls('camera.popup', 'Camera video', 'video', 'hide', ['over', 'wake', 'hide']), cls('camera.still', 'Camera events', 'camera', 'inside'),
        cls('doorbell', 'Doorbell', 'bell-ring', 'wake'), cls('ha', 'Home Assistant', 'house-wifi', 'inside'), cls('timer', 'Timers', 'timer', 'wake'),
        cls('complication', 'Saver items', 'sparkles', 'inside', ['inside', 'hide']),
        cls('ntfy', 'ntfy', 'bell', 'inside'), cls('toast', 'System notices', 'message-square-warning', 'hide'),
        cls('plugin:chores', 'Chores', 'plug', 'inside')]}


def owner_doc(fields):
    """A helper owner's document (`GET /v1/settings/<owner>`) from (key, kind, value)."""
    return {'ok': True, 'version': 1, 'effective': {k: {'value': v} for k, t, v in fields},
            'schema': {'sections': [{'id': 's', 'fields': [{'key': k, 'kind': {'type': t}} for k, t, v in fields]}]}}


AMBIENT = owner_doc([('enabled', 'bool', True), ('brightness', 'integer', 70), ('mode', 'choice', 'picture'), ('sound', 'choice', 'picture'),
                     ('top', 'integer', 18), ('right', 'integer', 11), ('bottom', 'integer', 18), ('left', 'integer', 11), ('delay_ms', 'integer', 40),
                     ('still_colour', 'choice', 'warm'), ('saturation', 'integer', 100), ('dark_hold', 'bool', False), ('skip_bars', 'bool', True),
                     ('game_edges', 'bool', False)])
AMBIENT_ROUTES = {'GET /v1/settings/base.ambient': AMBIENT, 'GET /v1/ambient': {'ok': True, 'state': 'running', 'strip': True, 'sections': 58}}


APPS = {'listLaunchPoints': {'returnValue': True, 'launchPoints': [
    {'id': 'netflix', 'title': 'Netflix', 'launchPointId': 'netflix_default'}, {'id': 'youtube.leanback.v4', 'title': 'YouTube'},
    {'id': 'amazon', 'title': 'Prime Video'}, {'id': 'com.webos.app.lgchannels', 'title': 'LG Channels', 'systemApp': True, 'hidden': True},
    {'id': 'plex', 'title': 'Plex'}, {'id': 'com.webos.app.livetv', 'title': 'Live TV', 'systemApp': True}]}}


def with_apps():
    return lgsettings.LgService(answers=dict(ONLINE, **APPS))


def with_hook():
    """The same apps with LG Input Hook installed, which the Buttons tab then says."""
    points = APPS['listLaunchPoints']['launchPoints'] + [{'id': 'org.webosbrew.inputhook', 'title': 'LG Input Hook'}]
    return lgsettings.LgService(answers=dict(ONLINE, listLaunchPoints={'returnValue': True, 'launchPoints': points}))


# The host eats the first key after the page opens: this one goes up to the tab bar and over to Buttons.
TO_BUTTONS = ['up', 'up', 'right', 'right']

# what the owner set: an app, nothing, the quick panel, another button, an input, Home and an imported code the list lacks
BUTTONS_SET = {'remote': {'buttons': {'netflix': 'app:plex', 'amazon': 'nothing', 'red': 'quick', 'green': 'as:blue', 'blue': 'app:com.webos.app.hdmi2',
                                      'list': 'home', 'code:1234': 'nothing'}}}


class Scene:
    def __init__(self, name, page, keys=(), routes=None, topics=None, theme=None, lg=None, tab=None, layout=None):
        self.name, self.page, self.keys = name, page, list(keys)
        self.routes, self.topics, self.theme, self.lg, self.tab, self.layout = routes or {}, topics or {}, theme, lg, tab, layout

    def helper_routes(self):
        routes = dict(self.routes)
        routes['topics'] = dict(self.topics)
        return routes


SCENES = [
    Scene('saver', 'duringsaver', routes={'GET /v1/saver/rules': lambda b: saver_rules()}, topics={'saver': {'rules': {}}},
          keys=['right', 'down', 'down', 'down', 'down']),
    Scene('saver-picker', 'duringsaver', routes={'GET /v1/saver/rules': lambda b: saver_rules()}, topics={'saver': {'rules': {}}},
          keys=['right', 'down', 'down', 'down', 'down', 'ok', 'down']),
]
SCENES += [
    Scene('ambient', 'ambient', routes=AMBIENT_ROUTES, keys=['right', 'down', 'down']),
    Scene('ambient-layout', 'ambient', routes=AMBIENT_ROUTES, keys=['right', 'up', 'right', 'down']),
]
SCENES += [
    Scene('remote', 'remote', lg=with_apps, keys=['right', 'down']),
    Scene('remote-keys', 'remote', lg=with_apps, keys=['up', 'up', 'right'] + ['down'] * 9),
    Scene('remote-buttons', 'remote', lg=with_apps, theme=BUTTONS_SET, keys=TO_BUTTONS + ['down', 'down']),
    Scene('remote-buttons-hook', 'remote', lg=with_hook, theme=BUTTONS_SET, keys=TO_BUTTONS + ['down']),
    Scene('appkey', 'remote', lg=with_apps, keys=TO_BUTTONS + ['down', 'ok', 'down', 'down']),
    Scene('button-picker', 'remote', lg=with_apps, theme=BUTTONS_SET, keys=TO_BUTTONS + ['down'] * 5 + ['ok', 'down', 'down', 'down']),
    Scene('number-key', 'remote', lg=with_apps, theme={'remote': {'quickAccess': {'on': True, 'slots': {}}}}, keys=['up', 'up', 'right'] + ['down'] * 10 + ['ok', 'down']),
]

WIFI_STATUS = {'returnValue': True, 'wired': {'state': 'disconnected'}, 'wifi': {
    'state': 'connected', 'ssid': "Nate's Home", 'signalBars': 3, 'signalLevel': -58, 'channelFrequency': 5180, 'ipAddress': '192.168.1.57',
    'netmask': '255.255.255.0', 'gateway': '192.168.1.1', 'dns1': '192.168.1.1', 'method': 'dhcp'}}
FOUND = {'returnValue': True, 'foundNetworks': [
    {'networkInfo': {'ssid': "Nate's Home", 'availableSecurityTypes': ['psk'], 'signalBars': 3}},
    {'networkInfo': {'ssid': 'Neighbour 2G', 'availableSecurityTypes': ['psk', 'wpa'], 'signalBars': 1}},
    {'networkInfo': {'ssid': 'Cafe Guest', 'availableSecurityTypes': ['none'], 'signalBars': 2}},
    {'networkInfo': {'ssid': 'Basement', 'availableSecurityTypes': ['psk'], 'signalBars': 0}}]}


def with_network(status=WIFI_STATUS):
    """The TV's network: the link the connection manager gives, the networks Wi-Fi finds, and the two LG keys a TV may or may not have."""
    values = copy.deepcopy(lgsettings.VALUES)
    values['network'].update({'ipv6Enable': 'true', 'bleAdvertisingOnOff': 'false'})
    return lgsettings.LgService(values=values, answers={'connectionmanager/getStatus': status, 'wifi/findnetworks': FOUND})


MANUAL = copy.deepcopy(WIFI_STATUS)
MANUAL['wifi']['method'] = 'manual'
SCENES += [
    Scene('network', 'network', lg=with_network, keys=['right', 'down']),
    Scene('network-details', 'network', lg=with_network, keys=['right', 'up', 'right', 'right', 'down', 'down', 'down']),
    Scene('wifi', 'wifi', lg=with_network, keys=['right', 'down']),
    Scene('ipsettings', 'ipsettings', lg=lambda: with_network(MANUAL), keys=['right', 'down', 'down']),
]
# ---- AI access: the helper's `ai` topic and the audit
AI = {'listener': {'enabled': True, 'bound': True, 'addr': '192.168.1.71:8723', 'port': 8723, 'error': None}, 'pairing': None,
      'assistants': [{'id': 'a1', 'name': 'Claude', 'paused': False, 'lastSeenAt': None,
                      'levels': {'state': 'always', 'playback': 'ask', 'launch': 'never', 'power': 'never'}},
                     {'id': 'a2', 'name': 'Voice assistant', 'paused': True,
                      'levels': {'state': 'ask', 'playback': 'ask', 'launch': 'ask', 'power': 'never'}}],
      'limits': {'maxVolume': 60},
      'groups': [{'id': 'state', 'title': 'State'}, {'id': 'playback', 'title': 'Playback'}, {'id': 'launch', 'title': 'Launch'}, {'id': 'power', 'title': 'Power'}]}
AI_OFF = dict(AI, listener={'enabled': False, 'bound': False}, assistants=[])


def ai_audit(_=None):
    import time
    now = int(time.time() * 1000)
    return {'ok': True, 'entries': [
        {'at': now - 7200000, 'tool': 'tv.set_volume', 'message': 'Set the volume to 18', 'outcome': 'ok', 'assistant': {'id': 'a1', 'name': 'Claude'}},
        {'at': now - 1500000, 'tool': 'tv.launch', 'message': 'Open Netflix', 'outcome': 'never', 'assistant': {'id': 'a1', 'name': 'Claude'}},
        {'at': now - 120000, 'tool': 'tv.play', 'message': 'Pause', 'outcome': 'ok', 'assistant': {'id': 'a1', 'name': 'Claude'}}]}


AI_ROUTES = {'GET /v1/ai/audit': ai_audit}
SCENES += [
    Scene('ai', 'ai', routes=AI_ROUTES, topics={'ai': AI}, keys=['right']),
    Scene('ai-assistants', 'ai', routes=AI_ROUTES, topics={'ai': AI}, keys=['right', 'up', 'right', 'down']),
    Scene('ai-allowed', 'ai', routes=AI_ROUTES, topics={'ai': AI}, keys=['right', 'up', 'right', 'right', 'down', 'down']),
    # access can only be turned on from the phone's approval, which is not connected here: the answer says so
    Scene('ai-unavailable', 'ai', routes=AI_ROUTES, topics={'ai': AI_OFF}, keys=['right', 'right', 'ok']),
]

# ---- Home Assistant and Cameras: the schemas the helper serves for the two plugins
HA = 'home-assistant'
HA_SCHEMA = {'sections': [
    {'id': 'connection', 'title': 'Connection', 'fields': [{'key': 'state', 'label': 'Connection', 'kind': {'type': 'read-only', 'value': {'capability': 'plugin.home-assistant.link'}}}]},
    {'id': 'room', 'title': 'This room', 'fields': [
        {'key': 'room', 'label': 'Room', 'kind': {'type': 'choice', 'options': {'provider': 'rooms'}}},
        {'key': 'controls', 'label': 'Lights and scenes', 'kind': {'type': 'ordered-set', 'max_items': 12, 'options': {'provider': 'controls'}}},
        {'key': 'summary', 'label': 'Watching', 'kind': {'type': 'read-only', 'value': {'capability': 'plugin.home-assistant.summary'}},
         'visible_when': {'type': 'capability-present', 'capability': 'plugin.home-assistant.summary'}}]},
    {'id': 'doorbell', 'title': 'Doorbell', 'fields': [
        {'key': 'doorbell', 'label': 'Doorbell', 'kind': {'type': 'ordered-set', 'max_items': 4, 'options': {'provider': 'doorbells'}}},
        {'key': 'notify_doorbell', 'label': 'Tell me when it rings', 'kind': {'type': 'bool'}}]},
    {'id': 'presence', 'title': 'Who is in the room', 'fields': [
        {'key': 'presence', 'label': 'Presence sensors', 'kind': {'type': 'ordered-set', 'max_items': 4, 'options': {'provider': 'presence'}}},
        {'key': 'hold', 'label': 'Still here for', 'kind': {'type': 'integer', 'min': 10, 'max': 600, 'step': 10, 'unit': 's'}}]},
    {'id': 'notifications', 'title': 'Notifications', 'fields': [{'key': 'notify_ha', 'label': "Home Assistant's notifications", 'kind': {'type': 'bool'}}]}]}
HA_VALUES = {'room': 'living_room', 'controls': ['light.lamp', 'scene.movie'], 'doorbell': ['event.front_doorbell'], 'notify_doorbell': True, 'presence': [], 'hold': 60, 'notify_ha': True}


def plugin_doc(schema, values, version=1):
    return {'ok': True, 'version': version, 'effective': {k: {'value': v} for k, v in values.items()}, 'schema': schema}


def choices(rows):
    return {'ok': True, 'plugin': HA, 'page': {'state': 'ready', 'rows': rows, 'more': False}}


HA_ROUTES = {
    'GET /v1/settings/home-assistant': plugin_doc(HA_SCHEMA, HA_VALUES),
    'GET /v1/plugins/home-assistant/choices/rooms': choices([{'id': 'kitchen', 'label': 'Kitchen'}, {'id': 'living_room', 'label': 'Living room'}, {'id': 'office', 'label': 'Office'}]),
    'GET /v1/plugins/home-assistant/choices/controls': choices([
        {'id': 'light.lamp', 'label': 'Floor lamp', 'detail': 'Living room'}, {'id': 'light.shelf', 'label': 'Shelf light', 'detail': 'Living room'},
        {'id': 'scene.movie', 'label': 'Movie night', 'detail': 'Living room'}, {'id': 'scene.bright', 'label': 'Bright', 'detail': 'Living room'}]),
    'GET /v1/plugins/home-assistant/choices/doorbells': choices([{'id': 'event.front_doorbell', 'label': 'Front doorbell'}, {'id': 'event.back_door', 'label': 'Back door'}]),
    'GET /v1/plugins/home-assistant/choices/presence': choices([]),
}
HA_TOPICS = {'plugins': {'plugins': [{'id': HA, 'name': 'Home Assistant', 'state': 'connected'}], 'firstParty': []},
             'ha.link': {'state': 'connected', 'address': 'http://homeassistant.local:8123', 'token': True},
             'plugin.home-assistant.facts': {'facts': {'link': 'Connected', 'summary': 'Living room: 3 lights on'}}}
SCENES += [
    Scene('homeassistant', 'homeassistant', routes=HA_ROUTES, topics=HA_TOPICS, keys=['right', 'down']),
    Scene('homeassistant-room', 'homeassistant', routes=HA_ROUTES, topics=HA_TOPICS, keys=['right', 'up', 'right', 'down', 'down']),
    Scene('homeassistant-unlinked', 'homeassistant', routes={}, topics=dict(HA_TOPICS, **{'ha.link': {'state': 'off'}}), keys=['right', 'down']),
]

SC = 'scrypted-cameras'
CAM_SCHEMA = {'revision': 1, 'sections': [
    {'id': 'link', 'title': 'Scrypted', 'fields': [{'key': 'link.status', 'label': 'Connection', 'kind': {'type': 'read-only', 'value': {'capability': 'plugin.scrypted-cameras.link'}}}]},
    {'id': 'show', 'title': 'Cameras', 'fields': [{'key': 'show', 'label': 'Show cameras', 'help': 'Empty shows every camera Scrypted shares.', 'kind': {'type': 'ordered-set', 'max_items': 128, 'options': {'provider': 'cameras'}}}]},
    {'id': 'popups', 'title': 'Pop-ups', 'fields': [
        {'key': 'popups.on', 'label': 'Show event pop-ups', 'kind': {'type': 'bool'}},
        {'key': 'popups.seconds', 'label': 'Stay for', 'kind': {'type': 'integer', 'min': 5, 'max': 60, 'step': 1, 'unit': 's'}},
        {'key': 'popups.cooldown', 'label': 'Same event again after', 'kind': {'type': 'integer', 'min': 0, 'max': 600, 'step': 5, 'unit': 's'}},
        {'key': 'popups.over_apps', 'label': 'Show pop-ups over playing apps', 'kind': {'type': 'bool'}}]},
    {'id': 'notify', 'title': 'Notify for', 'fields': [
        {'key': 'notify.front', 'label': 'Front door', 'kind': {'type': 'ordered-set', 'max_items': 8, 'options': {'provider': 'kinds'}}},
        {'key': 'notify.garage', 'label': 'Garage', 'kind': {'type': 'ordered-set', 'max_items': 8, 'options': {'provider': 'kinds'}}},
        {'key': 'notify.yard', 'label': 'Back yard', 'kind': {'type': 'ordered-set', 'max_items': 8, 'options': {'provider': 'kinds'}}}]},
    {'id': 'video', 'title': 'Video', 'fields': [
        {'key': 'video.live', 'label': 'Live quality', 'kind': {'type': 'choice', 'options': {'inline': [{'id': 'native', 'label': 'Full'}, {'id': 'balanced', 'label': 'Medium'}, {'id': 'light', 'label': 'Low'}]}}},
        {'key': 'video.grid', 'label': 'Grid', 'kind': {'type': 'choice', 'options': {'inline': [{'id': 'light', 'label': 'Light'}, {'id': 'balanced', 'label': 'Balanced'}, {'id': 'smooth', 'label': 'Smooth'}]}}}]}]}
CAM_VALUES = {'show': [], 'popups.on': True, 'popups.seconds': 12, 'popups.cooldown': 30, 'popups.over_apps': True, 'video.live': 'native', 'video.grid': 'balanced',
              'notify.front': ['person', 'doorbell'], 'notify.garage': ['person', 'vehicle', 'motion'], 'notify.yard': []}
CAM_ROUTES = {
    'GET /v1/settings/scrypted-cameras': plugin_doc(CAM_SCHEMA, CAM_VALUES, 3),
    'GET /v1/plugins/scrypted-cameras/choices/cameras': {'ok': True, 'page': {'state': 'ready', 'more': False, 'rows': [
        {'id': 'front', 'label': 'Front door', 'detail': 'Porch'}, {'id': 'garage', 'label': 'Garage', 'detail': 'Garage'}, {'id': 'yard', 'label': 'Back yard', 'detail': 'Garden'}]}},
    'GET /v1/plugins/scrypted-cameras/choices/kinds': {'ok': True, 'page': {'state': 'ready', 'more': False, 'rows': []}},
}
CAM_TOPICS = {'plugins': {'plugins': [{'id': SC, 'name': 'Cameras', 'state': 'connected'}], 'firstParty': []},
              'cameras': {'state': 'ready', 'cameras': [{'id': 'front', 'name': 'Front door'}, {'id': 'garage', 'name': 'Garage'}, {'id': 'yard', 'name': 'Back yard'}]},
              'plugin.scrypted-cameras.facts': {'facts': {'link': 'Connected', 'device': 'Living room TV', 'verifier': '9f2c41ab7d03e5681c4fa2b9d0e73a58b1c6f492ae07d3510b8e64c92f1a7d30'}}}
SCENES += [
    Scene('cameras', 'cameras', routes=CAM_ROUTES, topics=CAM_TOPICS, keys=['right', 'down', 'down']),
    Scene('cameras-notify', 'cameras', routes=CAM_ROUTES, topics=CAM_TOPICS, keys=['right', 'up', 'right', 'right', 'right', 'down', 'down']),
    Scene('cameras-video', 'cameras', routes=CAM_ROUTES, topics=CAM_TOPICS, keys=['right', 'up', 'right', 'right', 'right', 'right', 'down']),
    Scene('camera-kinds', 'cameras', routes=CAM_ROUTES, topics=CAM_TOPICS, keys=['right', 'up', 'right', 'right', 'right', 'down', 'down', 'ok', 'down', 'down']),
    Scene('cameras-unlinked', 'cameras', routes={}, topics={'cameras': {'state': 'unlinked', 'cameras': []}}, keys=['right']),
]

# ---- Profiles: the people with their bundled avatars (the pictures are larger than the tile they sit in)
def profiles_topic():
    avatar = os.path.join(ROOT, 'payload', 'avatars', 'critters-01.png')
    return {'ready': True, 'active': 'nitin', 'owner': 'nitin', 'canAdmin': True, 'pickerOnWake': False, 'startWith': 'last', 'max': 6, 'switchedAt': 1, 'profiles': [
        {'id': 'nitin', 'name': 'Nitin', 'initial': 'N', 'color': '#A79BF5', 'avatar': {'kind': 'initial'}, 'avatarFile': None, 'owner': True, 'active': True, 'locked': False},
        {'id': 'isabel', 'name': 'Isabel', 'initial': 'I', 'color': '#EB6F92', 'avatar': {'kind': 'gallery', 'id': 'critters-01'}, 'avatarFile': avatar, 'owner': False, 'active': False, 'locked': True}]}


SCENES += [Scene('profiles', 'profiles', topics={'profiles': profiles_topic()}, keys=['right'])]

# ---- Plex: the code to scan while the TV signs in, and the account signed in with two servers to choose from
PLEX_BEAST = {'id': 'beast', 'name': 'BeastNAS', 'address': '192.168.1.69:32400'}
PLEX_DEN = {'id': 'den', 'name': 'Den', 'address': '192.168.1.80:32400'}
SCENES += [
    Scene('plex-code', 'plex', keys=['right'], topics={'plex.account': {'state': 'code', 'code': 'WBVQ', 'url': 'https://plex.tv/link/?pin=WBVQ',
                                                                     'expiresAt': 1}}),
    Scene('plex-signed-in', 'plex', keys=['right'], topics={'plex.account': {'state': 'signed-in', 'user': 'nitin', 'server': PLEX_BEAST,
                                                                          'servers': [PLEX_BEAST, PLEX_DEN]}}),
]

# ---- Privacy: the helper's menu (`GET /v1/privacy/menu`), and its two-step change
HELD = [('option.livePlus', 'Live Plus content recognition', 'off'), ('option.watchedListCollection', 'viewing history collection', 'off'),
        ('option.thirdPartyCookie', 'third-party cookies', 'off'), ('option.dbgLogUpload', 'debug log upload', False), ('option.usageCare', 'usage statistics', False),
        ('general.customizedAd', 'personalised ads', 'off'), ('general.adCookie', 'ad cookie', 'on'), ('option.livePromotion', 'live promotion', 'off'),
        ('option.promotionStreamer', 'promotion streamer', 'off'), ('option.webOSPromotionVideo', 'promotion video', 'off'),
        ('general.homePromotion', 'home promotion', 'off'), ('general.screenSaverAd', 'screen saver ads', 'off'), ('general.sportsAlarm', 'sports alarm', 'off'),
        ('other.contentRecommendation', 'content recommendation', 'off'), ('general.aiNudge', 'AI nudge', 'off'), ('general.aiSettingsNudge', 'settings nudge', 'off')]


def privacy_menu(amazoff_on=False):
    def item(id, title, detail, on, **kw):
        return dict({'id': id, 'title': title, 'detail': detail, 'kind': 'switch', 'on': on, 'available': True, 'admin': True}, **kw)

    def consent(key, title, on, settable=True, agreements=()):
        return {'id': 'consent.' + key, 'title': title, 'kind': 'switch', 'on': on, 'available': True, 'settable': settable, 'admin': True,
                'agreements': [{'title': t} for t in agreements]}
    held = [{'key': k, 'target': 'off' if isinstance(v, str) else False, 'detail': d, 'value': v, 'available': True} for k, d, v in HELD]
    return {'ok': True, 'privacy': {'id': 'privacy', 'canAdmin': True, 'items': [
        item('ads', 'Block content recognition (ACR) and ads', 'Stops content recognition, ad overlays and banners.', True),
        item('settings', "Keep LG's viewing and ad settings off", 'Switches LG\'s 16 settings off.', True, settings=held),
        item('uploaders', 'Block telemetry uploads', 'Stops LG\'s uploads.', True),
        item('push', 'Block LG push promotions', 'Stops push notices.', False),
        item('hosts', 'Block ad & tracker hosts', 'Blocks advertising names.', True, effective=True, count=212),
        item('amazoff', 'Block Prime Video ads', 'Takes the ads out of Prime Video.', amazoff_on, protection='active' if amazoff_on else 'off'),
        consent('acrAllowed', 'Allow content recognition (ACR)', True, agreements=('Viewing Information Agreement',)),
        consent('customAdAllowed', 'Allow personalised ads', False, agreements=('Advertising Agreement',)),
        consent('cookiesAllowed', 'Allow advertising cookies', False),
        consent('voiceAllowed', 'Allow voice recordings', False, settable=False),
        consent('networkAllowed', "Accept LG's network service terms", True, settable=False),
        {'id': 'permissions', 'title': 'App Permissions', 'kind': 'link'}]}}


def privacy_prepare(b):
    return {'ok': True, 'confirmation': 'tok-' + b['id'], 'expiresInSeconds': 60, 'plan': {'key': b['id'], 'on': b['enabled']}}


def privacy_apply(b):
    return {'ok': True, 'changed': b['id'], 'on': b['enabled']}


PRIVACY_ROUTES = {'GET /v1/privacy/menu': lambda b: privacy_menu(), 'POST /v1/privacy/prepare': privacy_prepare, 'POST /v1/privacy/apply': privacy_apply}
SCENES += [
    Scene('privacy-blocking', 'privacy', routes=PRIVACY_ROUTES, keys=['right']),
    Scene('privacy-flip', 'privacy', routes=PRIVACY_ROUTES, keys=['right', 'ok']),
    Scene('privacy-prime', 'privacy', routes=PRIVACY_ROUTES, keys=['right'] + ['down'] * 6 + ['ok']),
    Scene('privacy-prime-yes', 'privacy', routes=PRIVACY_ROUTES, keys=['right'] + ['down'] * 6 + ['ok', 'right', 'ok']),
    Scene('privacy-held', 'privacy', routes=PRIVACY_ROUTES, keys=['right', 'down', 'down', 'ok']),
    Scene('privacy-collect', 'privacy', routes=PRIVACY_ROUTES, keys=['right', 'up', 'right', 'down']),
    Scene('privacy-ads', 'privacy', routes=PRIVACY_ROUTES, keys=['right', 'up', 'right', 'right', 'down']),
]

# ---- Home & look, Home tab: where the weather comes from
def weather_owner(source, entity=''):
    return owner_doc([('source', 'choice', source), ('entity', 'text', entity), ('zip', 'text', '30311'), ('units', 'choice', 'fahrenheit')])


WEATHER_ENTITIES = {'ok': True, 'entities': [{'id': 'weather.forecast_home', 'name': 'Forecast Home'}, {'id': 'weather.pirate_weather', 'name': 'Pirate Weather'}]}
TO_HOME = ['up', 'up', 'right']
WEATHER_BUILTIN = {'GET /v1/settings/base.weather': weather_owner('builtin')}
WEATHER_HA = {'GET /v1/settings/base.weather': weather_owner('homeassistant', 'weather.pirate_weather'), 'GET /v1/weather/entities': WEATHER_ENTITIES}
SCENES += [
    Scene('weather-source', 'look', routes=WEATHER_BUILTIN, keys=TO_HOME + ['down'] * 8),
    Scene('weather-source-ha', 'look', routes=WEATHER_HA, keys=TO_HOME + ['down'] * 8),
    Scene('weather-source-picker', 'look', routes=WEATHER_HA, keys=TO_HOME + ['down'] * 8 + ['ok']),
]

BY_NAME = {s.name: s for s in SCENES}


def jpeg(png, path):
    from PIL import Image
    im = Image.open(png).convert('RGB')
    im = im.resize((WIDTH, round(im.height * WIDTH / im.width)), Image.LANCZOS)
    im.save(path, 'JPEG', quality=QUALITY, optimize=True, progressive=True)


def play(scene, mode='dark', scale=1.0, wait=None, shot=None):
    """Runs the scene; returns (rc, output, helper, lint findings, the png's path or None). The helper keeps every request it was sent."""
    shot = shot or os.path.join(tempfile.mkdtemp(prefix='rsrender-'), scene.name + '.png')
    t = OPEN_AT + KEY_GAP * (len(scene.keys) + 1) + SETTLE + 200 * scene.keys.count('ok')
    extra = []
    for i, k in enumerate(scene.keys):
        at = OPEN_AT + KEY_GAP * (i + 1)
        extra += ['--key', '%d:%s' % (at, k)]
        if k == 'ok':
            # OK acts on its release (a hold is the help)
            extra += ['--key', '%d:ok:up' % (at + 80)]
    extra += ['--eval', '%d:lint' % (t - 200), '--eval', '%d:state' % (t - 200), '--snap', '%d:%s' % (t, shot)]
    steps = [(OPEN_AT - 300, lambda c: c.push('foreground', {'app': 'com.webos.app.home', 'home': True})),
             (OPEN_AT, lambda c: c.push('show', {'kind': 'settings', 'page': scene.page}))]
    helper = host.FakeHelper(scene.helper_routes())
    lg = scene.lg() if callable(scene.lg) else scene.lg or lgsettings.LgService(answers=ONLINE)
    rc, text, chan, lg, helper = host.run(steps=steps, wait=wait or t + 800, extra=extra, lg=lg, helper=helper, text_scale=scale, mode=mode, theme=scene.theme)
    evals = host.evals(text)
    findings = evals[0] if evals and isinstance(evals[0], list) else []
    return rc, text, helper, findings, shot if os.path.exists(shot) else None


def render(scene, mode='dark', scale=1.0, out=None, wait=None):
    """Runs the scene; returns (rc, output, lint findings, the png's path or None). `out` is a path for a JPEG."""
    rc, text, _, findings, png = play(scene, mode, scale, wait)
    if png and out:
        os.makedirs(os.path.dirname(out), exist_ok=True)
        jpeg(png, out)
    return rc, text, findings, png


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--out', default=OUT)
    ap.add_argument('--only', default='')
    ap.add_argument('--modes', default='dark')
    ap.add_argument('--scales', default='1')
    ap.add_argument('--list', action='store_true')
    ap.add_argument('--jobs', type=int, default=2)
    a = ap.parse_args()
    if a.list:
        print('\n'.join(s.name for s in SCENES))
        return 0
    if not host.available():
        print('needs rustui/target/debug/lumatv-ui and Mesa', file=sys.stderr)
        return 1
    only = [n for n in a.only.split(',') if n]
    jobs = []
    for s in SCENES:
        if only and s.name not in only:
            continue
        for mode in a.modes.split(','):
            for scale in a.scales.split(','):
                label = '%s-%s%s' % (s.name, mode, '' if float(scale) == 1.0 else '-%sx' % scale.replace('.0', ''))
                jobs.append((s, mode, float(scale), os.path.join(a.out, label + '.jpg')))
    bad = 0
    with concurrent.futures.ThreadPoolExecutor(a.jobs) as pool:
        for (s, mode, scale, path), (rc, text, findings, png) in zip(jobs, pool.map(lambda j: render(j[0], j[1], j[2], j[3]), jobs)):
            ok = rc == 0 and png is not None
            bad += not ok
            print('%s %s%s' % ('ok  ' if ok else 'FAIL', os.path.relpath(path, ROOT), ''.join('\n     lint: %s %s %s' % (f['kind'], f['subject'], f['detail']) for f in findings)))
            if not ok:
                print(text[-1500:])
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main())
