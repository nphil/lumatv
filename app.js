/**
 * {{displayName}} Settings App
 * Remote-control frontend for the {{displayName}} CLI via Homebrew Channel's exec service
 */

(function() {
  'use strict';

  /* ====== Bridge & CLI ====== */

  /**
   * Check if real webOS bridge is available
   */
  function hasRealBridge() {
    return typeof window.webOS === 'object' &&
           typeof window.webOS.service === 'object' &&
           typeof window.webOS.service.request === 'function';
  }

  /**
   * exec(cmd) -> Promise<{returnValue, stdoutString, stderrString, error}>
   * Calls luna://org.webosbrew.hbchannel.service/exec with the given shell command
   */
  async function exec(cmd) {
    if (hasRealBridge()) {
      return new Promise((resolve, reject) => {
        window.webOS.service.request('luna://org.webosbrew.hbchannel.service', {
          method: 'exec',
          parameters: { command: cmd },
          onSuccess: resolve,
          onFailure: (err) => reject(new Error(err?.errorText || 'exec failed'))
        });
      });
    }

    // Fallback: try PalmServiceBridge constructor
    if (typeof PalmServiceBridge === 'function') {
      const bridge = new PalmServiceBridge();
      return new Promise((resolve, reject) => {
        const callback = (msg) => {
          try {
            const parsed = JSON.parse(msg);
            resolve(parsed);
          } catch {
            reject(new Error('exec: invalid response'));
          }
        };
        bridge.onservicecallback = callback;
        bridge.call('palm://org.webosbrew.hbchannel.service/exec', JSON.stringify({ command: cmd }));
      });
    }

    // No real bridge; check if mock mode is enabled
    const isMockMode = new URLSearchParams(window.location.search).get('mock') === '1';
    if (isMockMode) {
      return mockExec(cmd);
    }

    // No bridge and not in mock mode: error
    throw new Error('Homebrew Channel root service unavailable');
  }

  /**
   * cli(verb, arg?) -> Promise<{ok:boolean, ...payload}>
   * Constructs and runs: /media/developer/apps/usr/palm/applications/{{appId}}/tools/{{shortId}} <verb> [arg]
   */
  async function cli(verb, arg) {
    let cmd = '/media/developer/apps/usr/palm/applications/{{appId}}/tools/{{shortId}} ' + verb;
    if (arg) {
      cmd += ' ' + arg;
    }

    try {
      const response = await exec(cmd);
      if (!response.stdoutString) {
        throw new Error('CLI returned no output');
      }
      const parsed = JSON.parse(response.stdoutString);
      if (!parsed.ok) {
        throw new Error(parsed.error || 'Unknown CLI error');
      }
      return parsed;
    } catch (err) {
      throw new Error(err.message || 'CLI failed');
    }
  }

  /* ====== Mock Bridge (browser preview only) ====== */

  const mockState = {
    compositor: { live: false, enabled: false, flag: 'off' },
    standby: 'system',
    ntfy: { enabled: false, url: '' },
    version: '0.3.5',
    firmware: '10.2.1',
    model: 'HE_DTV_W24G_AFABATAA'
  };

  const mockLog = [
    '[00:00] {{displayName}} settings app started',
    '[00:01] Status retrieved successfully',
    '[mock mode] No real Homebrew Channel available',
    '[mock mode] Using simulated CLI responses'
  ];

  const mockTheme = { wallpaperMotion: { enabled: true, amplitude: 4, speed: 1, depth: 0.25, scale: 1, detail: 0.35, fps: 30, vignette: 0, specular: 0, bloom: 0, saturation: 1, contrast: 1 } };

  async function mockExec(cmd) {
    // Simulate network delay
    await new Promise(r => setTimeout(r, 300));

    let stdoutString = '{"ok":true}';
    if (cmd.includes(' theme ')) {
      const m = /theme set '(.*)'$/.exec(cmd);
      if (m) Object.assign(mockTheme.wallpaperMotion, JSON.parse(m[1]).wallpaperMotion || {});
      return { returnValue: true, stdoutString: JSON.stringify({ ok: true, theme: mockTheme }) };
    }

    if (cmd.includes('status')) {
      stdoutString = JSON.stringify({
        ok: true,
        version: mockState.version,
        compositor: mockState.compositor,
        standby: mockState.standby,
        ntfy: mockState.ntfy,
        firmware: mockState.firmware,
        model: mockState.model
      });
    } else if (cmd.includes('compositor on')) {
      mockState.compositor = { live: true, enabled: true, flag: 'persistent' };
    } else if (cmd.includes('compositor off')) {
      mockState.compositor = { live: false, enabled: false, flag: 'off' };
    } else if (cmd.includes('launchDefaultApp') || cmd.includes('applicationManager/launch ')) {
      stdoutString = JSON.stringify({ returnValue: true });
    } else if (cmd.includes('standby warm')) {
      mockState.standby = 'warm';
      stdoutString = JSON.stringify({ ok: true, standby: 'warm' });
    } else if (cmd.includes('standby cold')) {
      mockState.standby = 'cold';
      stdoutString = JSON.stringify({ ok: true, standby: 'cold' });
    } else if (cmd.includes('ntfy set')) {
      const match = cmd.match(/ntfy set (.+)$/);
      if (match) {
        mockState.ntfy = { enabled: true, url: match[1] };
        stdoutString = JSON.stringify({ ok: true, ntfy: mockState.ntfy });
      } else {
        stdoutString = JSON.stringify({ ok: false, error: 'ntfy set: invalid URL' });
      }
    } else if (cmd.includes('ntfy off')) {
      mockState.ntfy = { enabled: false, url: '' };
      stdoutString = JSON.stringify({ ok: true, ntfy: mockState.ntfy });
    } else if (cmd.includes('ntfy test')) {
      stdoutString = JSON.stringify({ ok: true, tested: true });
    } else if (cmd.includes('log')) {
      stdoutString = JSON.stringify({ ok: true, lines: mockLog });
    } else if (cmd.includes('version')) {
      stdoutString = JSON.stringify({ ok: true, version: mockState.version });
    }

    return { returnValue: true, stdoutString, stderrString: '' };
  }

  /* ====== UI State & DOM ====== */

  const dom = {
    app: document.getElementById('app'),
    header: document.getElementById('header'),
    rows: document.getElementById('rows'),
    footer: document.getElementById('footer'),
    statusCard: document.getElementById('status-card'),
    statApplied: document.getElementById('stat-applied'),
    statMounts: document.getElementById('stat-mounts'),
    statVersion: document.getElementById('stat-version'),
    statFirmware: document.getElementById('stat-firmware'),
    statModel: document.getElementById('stat-model'),
    footerStatus: document.getElementById('footer-status'),
    toast: document.getElementById('toast'),

    // Overlays
    confirmRevert: document.getElementById('overlay-confirm-revert'),
    applyBusy: document.getElementById('overlay-apply-busy'),
    applyBusyText: document.getElementById('apply-busy-text'),
    enterFailed: document.getElementById('overlay-enter-failed'),
    enterReason: document.getElementById('enter-failed-reason'),
    busy: document.getElementById('overlay-busy'),
    ntfyEditor: document.getElementById('overlay-ntfy'),
    logPanel: document.getElementById('overlay-log'),

    // Controls
    confirmRevertYes: document.getElementById('confirm-revert-yes'),
    confirmRevertNo: document.getElementById('confirm-revert-no'),
    ntfyInput: document.getElementById('ntfy-input'),
    ntfySave: document.getElementById('ntfy-save'),
    ntfyOff: document.getElementById('ntfy-off'),
    ntfyCancel: document.getElementById('ntfy-cancel'),
    enterRetry: document.getElementById('enter-retry'),
    enterLg: document.getElementById('enter-lg'),
    logContent: document.getElementById('log-content'),
    wallpaperPanel: document.getElementById('overlay-wallpaper'),
    wallpaperRows: document.getElementById('wallpaper-rows'),
    wallpaperClose: document.getElementById('wallpaper-close'),
    logClose: document.getElementById('log-close'),
    toggleStandby: document.getElementById('toggle-standby'),
    ntfySub: document.getElementById('ntfy-sub')
  };

  const state = {
    currentMode: 'main', // 'main', 'confirmRevert', 'ntfy', 'log', 'wallpaper', 'applying', 'enterFailed'
    focusIndex: 0,
    status: null,
    isExecuting: false,
    entered: false
  };

  const rows = Array.from(document.querySelectorAll('.row.focusable'));

  /* ====== Focus & Keyboard ====== */

  function setFocus(index, mode) {
    if (mode) state.currentMode = mode;

    let focusable = [];
    if (state.currentMode === 'main') {
      focusable = rows;
    } else if (state.currentMode === 'confirmRevert') {
      focusable = [dom.confirmRevertYes, dom.confirmRevertNo];
    } else if (state.currentMode === 'ntfy') {
      focusable = [dom.ntfyInput, dom.ntfySave, dom.ntfyOff, dom.ntfyCancel];
    } else if (state.currentMode === 'log') {
      focusable = [dom.logContent, dom.logClose];
    } else if (state.currentMode === 'wallpaper') {
      focusable = Array.from(dom.wallpaperRows.querySelectorAll('.wp-row')).concat([dom.wallpaperClose]);
    } else if (state.currentMode === 'enterFailed') {
      focusable = [dom.enterRetry, dom.enterLg];
    }

    if (index < 0) index = 0;
    if (index >= focusable.length) index = focusable.length - 1;

    state.focusIndex = index;
    focusable[index]?.focus();
  }

  function moveFocus(direction) {
    setFocus(state.focusIndex + direction);
  }

  document.addEventListener('keydown', (e) => {
    // Support old webOS browser where keyCode is read-only or unavailable
    const code = e.keyCode || e.which || (e.key === 'ArrowUp' ? 38 : e.key === 'ArrowDown' ? 40 : e.key === 'Enter' ? 13 : null);

    if (state.currentMode === 'log') {
      // In log view, Up/Down scroll content, Back exits
      if (code === 38) { // Up
        dom.logContent.scrollTop -= 100;
        e.preventDefault();
      } else if (code === 40) { // Down
        dom.logContent.scrollTop += 100;
        e.preventDefault();
      } else if (code === 461) { // Back
        closeLogPanel();
        e.preventDefault();
      }
      return;
    }

    if (state.currentMode === 'wallpaper' && (code === 37 || code === 39)) { // Left / Right adjust
      adjustWallpaperRow(document.activeElement, code === 39 ? 1 : -1);
      e.preventDefault();
      return;
    }

    if (code === 38) { // Arrow Up
      moveFocus(-1);
      e.preventDefault();
    } else if (code === 40) { // Arrow Down
      moveFocus(1);
      e.preventDefault();
    } else if (code === 13) { // Enter
      const focused = document.activeElement;
      if (focused && focused.classList.contains('focusable')) {
        focused.click();
      }
      e.preventDefault();
    } else if (code === 461) { // Back
      if (state.currentMode !== 'main') {
        closeAllOverlays();
        state.currentMode = 'main';
        setFocus(state.focusIndex);
      } else if (typeof webOS !== 'undefined' && typeof webOS.platformBack === 'function') {
        webOS.platformBack();
      } else {
        window.close();
      }
      e.preventDefault();
    }
  });

  /* ====== Status & Rendering ====== */

  async function refreshStatus() {
    try {
      state.status = await cli('status');
      renderStatus();
    } catch (err) {
      showError(err.message);
      state.status = null;
    }
  }

  function renderStatus() {
    if (!state.status) {
      dom.statApplied.textContent = '?';
      dom.statMounts.textContent = '?';
      dom.statVersion.textContent = '?';
      dom.statFirmware.textContent = '?';
      dom.statModel.textContent = '?';
      return;
    }

    const compositor = state.status.compositor;
    dom.statApplied.textContent = compositor ? (compositor.live ? 'Yes' : 'No') : '?';
    dom.statMounts.textContent = compositor ? (compositor.flag === 'persistent' ? 'Yes' : 'No') : '?';
    dom.statVersion.textContent = state.status.version;
    dom.statFirmware.textContent = state.status.firmware;
    dom.statModel.textContent = state.status.model;

    // Update toggles
    dom.toggleStandby.setAttribute('data-on', state.status.standby === 'warm' ? 'true' : 'false');
    dom.ntfySub.textContent = state.status.ntfy?.enabled ? state.status.ntfy.url : 'off';
  }

  function showError(msg) {
    dom.toast.textContent = msg;
    dom.toast.removeAttribute('hidden');
    setTimeout(() => dom.toast.setAttribute('hidden', ''), 4000);
  }

  /* ====== Overlay Control ====== */

  function closeAllOverlays() {
    dom.confirmRevert.setAttribute('hidden', '');
    dom.applyBusy.setAttribute('hidden', '');
    dom.enterFailed.setAttribute('hidden', '');
    dom.busy.setAttribute('hidden', '');
    dom.ntfyEditor.setAttribute('hidden', '');
    dom.logPanel.setAttribute('hidden', '');
    dom.wallpaperPanel.setAttribute('hidden', '');
  }

  /* ====== Wallpaper motion ====== */

  const wallpaperControls = [
    { key: 'enabled', label: 'Motion', kind: 'toggle', def: true },
    { key: 'amplitude', label: 'Amount', min: 0, max: 16, step: 1, unit: ' px', def: 4 },
    { key: 'speed', label: 'Speed', min: 0.2, max: 3, step: 0.1, unit: 'x', def: 1 },
    { key: 'depth', label: 'Depth', min: 0, max: 1, step: 0.05, unit: '', def: 0.25 },
    { key: 'scale', label: 'Wave size', min: 0.5, max: 2.5, step: 0.1, unit: 'x', def: 1 },
    { key: 'detail', label: 'Detail', min: 0, max: 1, step: 0.05, unit: '', def: 0.35 },
    { key: 'fps', label: 'Frame rate', min: 15, max: 60, step: 5, unit: ' fps', def: 30 },
    { key: 'vignette', label: 'Vignette', min: 0, max: 1, step: 0.05, unit: '', def: 0 },
    { key: 'specular', label: 'Specular', min: 0, max: 1, step: 0.05, unit: '', def: 0 },
    { key: 'bloom', label: 'Bloom', min: 0, max: 1, step: 0.05, unit: '', def: 0 },
    { key: 'saturation', label: 'Saturation', min: 0.5, max: 1.5, step: 0.05, unit: '', def: 1 },
    { key: 'contrast', label: 'Contrast', min: 0.8, max: 1.3, step: 0.05, unit: '', def: 1 }
  ];
  const wallpaper = { values: {}, timer: null, pending: {} };

  function fmtWallpaper(c, v) {
    if (c.kind === 'toggle') return v ? 'On' : 'Off';
    return (c.step >= 1 ? Math.round(v) : Number(v).toFixed(c.step >= 0.1 ? 1 : 2)) + c.unit;
  }

  function renderWallpaperRows() {
    dom.wallpaperRows.innerHTML = '';
    for (const c of wallpaperControls) {
      const row = document.createElement('div');
      row.className = 'wp-row focusable';
      row.tabIndex = 0;
      row.dataset.key = c.key;
      const v = wallpaper.values[c.key] === undefined ? c.def : wallpaper.values[c.key];
      row.innerHTML = '<span class="wp-label">' + c.label + '</span><span class="wp-value">&#8249; ' + fmtWallpaper(c, v) + ' &#8250;</span>';
      row.addEventListener('click', () => adjustWallpaperRow(row, 1));
      dom.wallpaperRows.appendChild(row);
    }
  }

  function adjustWallpaperRow(row, dir) {
    if (!row || !row.dataset.key) return;
    const c = wallpaperControls.find(x => x.key === row.dataset.key);
    let v = wallpaper.values[c.key] === undefined ? c.def : wallpaper.values[c.key];
    if (c.kind === 'toggle') v = !v;
    else { v = Math.round((v + dir * c.step) / c.step) * c.step; v = Math.max(c.min, Math.min(c.max, Math.round(v * 100) / 100)); }
    wallpaper.values[c.key] = v;
    wallpaper.pending[c.key] = v;
    row.querySelector('.wp-value').innerHTML = '&#8249; ' + fmtWallpaper(c, v) + ' &#8250;';
    clearTimeout(wallpaper.timer);
    wallpaper.timer = setTimeout(flushWallpaper, 250);
  }

  async function flushWallpaper() {
    const patch = wallpaper.pending; wallpaper.pending = {};
    if (!Object.keys(patch).length) return;
    try {
      await cli('theme', "set '" + JSON.stringify({ wallpaperMotion: patch }).replace(/'/g, "'\\''") + "'");
      dom.footerStatus.textContent = 'Wallpaper updated';
    } catch (err) {
      showError(err.message);
    }
  }

  async function onWallpaperOpen() {
    try {
      const res = await cli('theme', 'get');
      wallpaper.values = Object.assign({}, (res.theme && res.theme.wallpaperMotion) || {});
    } catch (err) {
      showError(err.message);
      wallpaper.values = {};
    }
    renderWallpaperRows();
    dom.wallpaperPanel.removeAttribute('hidden');
    setFocus(0, 'wallpaper');
  }

  function onWallpaperClose() {
    clearTimeout(wallpaper.timer);
    flushWallpaper();
    dom.wallpaperPanel.setAttribute('hidden', '');
    state.currentMode = 'main';
    setFocus(rows.findIndex(r => r.id === 'row-wallpaper'));
  }

  /* ====== Row Actions ====== */

  async function openHome() {
    const response = await exec('luna-send -n 1 luna://com.webos.applicationManager/launchDefaultApp ' + "'" + JSON.stringify({ category: 'home', params: { launchType: 'homeKey', activateType: 'home' } }) + "'");
    const result = JSON.parse(response.stdoutString || '{}');
    if (!result.returnValue) throw new Error(result.errorText || 'Home did not open');
  }

  const launchDeps = { cli, exec, sleep: (ms) => new Promise((resolve) => setTimeout(resolve, ms)), now: () => Date.now() };

  function showEnterProgress(phase, reason) {
    dom.applyBusyText.textContent = phase === 'waiting'
      ? "Waiting for LG's Home to finish recovering: " + reason
      : 'If the compositor needs to start, the screen will flash and Home will restart. Please wait.';
  }

  function showEnterFailed(reason) {
    closeAllOverlays();
    dom.enterReason.textContent = reason;
    dom.enterFailed.removeAttribute('hidden');
    setFocus(0, 'enterFailed');
  }

  /**
   * Opens {{displayName}}: its compositor is started when it is not the compositor (after the escape, say), and its Home comes up. A compositor that
   * runs already is shown by opening Home when `openIfLive` (the row's own meaning) and left as it is otherwise (the page's start).
   */
  async function enterLumaTV(openIfLive) {
    if (state.isExecuting) return;
    state.isExecuting = true;
    state.currentMode = 'applying';
    showEnterProgress('starting');
    dom.applyBusy.removeAttribute('hidden');
    try {
      const result = await LumaLaunch.enter(launchDeps, showEnterProgress);
      if (result.outcome === 'failed') {
        showEnterFailed(result.reason);
        return;
      }
      if (result.outcome === 'live' && openIfLive) await openHome();
      await refreshStatus();
    } catch (err) {
      showEnterFailed(err.message);
      return;
    } finally {
      state.isExecuting = false;
    }
    closeAllOverlays();
    state.currentMode = 'main';
    setFocus(0);
  }

  /** The failure's way out: LG's own Home. */
  async function onEnterLg() {
    try {
      await LumaLaunch.openLgHome(launchDeps);
    } catch (err) {
      showError("LG's Home did not open: " + err.message);
    }
  }

  /** Opened from LG's launcher while {{displayName}} is not the compositor (after the escape, or a revert): started once, with no key pressed. */
  async function enterOnOpen() {
    if (state.entered) return;
    state.entered = true;
    const compositor = state.status && state.status.compositor;
    if (compositor && !compositor.live) await enterLumaTV(false);
  }

  async function onRevert() {
    if (state.isExecuting) return;
    closeAllOverlays();
    state.currentMode = 'confirmRevert';
    dom.confirmRevert.removeAttribute('hidden');
    setFocus(0);
  }

  async function confirmRevert() {
    if (state.isExecuting) return;
    state.isExecuting = true;
    closeAllOverlays();
    dom.busy.removeAttribute('hidden');
    document.getElementById('busy-message').textContent = 'Reverting...';

    try {
      await cli('compositor', 'off');
      await refreshStatus();
      showError('Reverted to stock home screen.');
    } catch (err) {
      showError('Revert failed: ' + err.message);
    } finally {
      state.isExecuting = false;
      dom.busy.setAttribute('hidden', '');
      state.currentMode = 'main';
      setFocus(0);
    }
  }


  async function onStandbyToggle() {
    if (state.isExecuting) return;
    state.isExecuting = true;

    // "system" (Quick Start+ on, SDDP still enabled) is the stock state and
    // reads as off: on a set with a VPN tun interface webos-sddp crashes on
    // the next network change and the next power-off reboots, so it is not
    // instant-on in practice.
    const warm = state.status.standby === 'warm';
    try {
      await cli('standby', warm ? 'cold' : 'warm');
      await refreshStatus();
    } catch (err) {
      showError('Instant on toggle failed: ' + err.message);
    } finally {
      state.isExecuting = false;
    }
  }

  function onNtfyOpen() {
    if (state.isExecuting) return;
    closeAllOverlays();
    state.currentMode = 'ntfy';
    dom.ntfyEditor.removeAttribute('hidden');
    dom.ntfyInput.value = state.status.ntfy?.enabled ? state.status.ntfy.url : '';
    setFocus(0, 'ntfy');
  }

  async function onNtfySave() {
    const url = dom.ntfyInput.value.trim();
    if (!url) {
      showError('Please enter a URL or click "Turn off"');
      return;
    }

    if (state.isExecuting) return;
    state.isExecuting = true;

    try {
      await cli('ntfy set', url);
      await refreshStatus();
      closeAllOverlays();
      state.currentMode = 'main';
      setFocus(0);
    } catch (err) {
      showError('ntfy set failed: ' + err.message);
    } finally {
      state.isExecuting = false;
    }
  }

  async function onNtfyOff() {
    if (state.isExecuting) return;
    state.isExecuting = true;

    try {
      await cli('ntfy off');
      await refreshStatus();
      closeAllOverlays();
      state.currentMode = 'main';
      setFocus(0);
    } catch (err) {
      showError('ntfy off failed: ' + err.message);
    } finally {
      state.isExecuting = false;
    }
  }

  function onNtfyCancel() {
    closeAllOverlays();
    state.currentMode = 'main';
    setFocus(0);
  }

  async function onNtfyTest() {
    if (state.isExecuting) return;
    if (!state.status.ntfy?.enabled) {
      showError('ntfy is not configured');
      return;
    }

    state.isExecuting = true;
    dom.busy.removeAttribute('hidden');
    document.getElementById('busy-message').textContent = 'Sending test notification...';

    try {
      await cli('ntfy test');
      showError('Test notification sent!');
    } catch (err) {
      showError('Test failed: ' + err.message);
    } finally {
      state.isExecuting = false;
      dom.busy.setAttribute('hidden', '');
    }
  }

  async function onViewLog() {
    if (state.isExecuting) return;
    state.isExecuting = true;
    closeAllOverlays();

    try {
      const result = await cli('log 40');
      dom.logContent.textContent = (result.lines || []).join('\n');
      state.currentMode = 'log';
      dom.logPanel.removeAttribute('hidden');
      setFocus(0, 'log');
    } catch (err) {
      showError('Log failed: ' + err.message);
    } finally {
      state.isExecuting = false;
    }
  }

  function closeLogPanel() {
    dom.logPanel.setAttribute('hidden', '');
    state.currentMode = 'main';
    setFocus(0);
  }

  /* ====== Event Listeners ====== */

  document.getElementById('row-apply').addEventListener('click', () => enterLumaTV(true));
  document.getElementById('row-revert').addEventListener('click', onRevert);
  document.getElementById('row-standby').addEventListener('click', onStandbyToggle);
  document.getElementById('row-ntfy').addEventListener('click', onNtfyOpen);
  document.getElementById('row-wallpaper').addEventListener('click', onWallpaperOpen);
  dom.wallpaperClose.addEventListener('click', onWallpaperClose);
  document.getElementById('row-ntfy-test').addEventListener('click', onNtfyTest);
  document.getElementById('row-log').addEventListener('click', onViewLog);

  dom.confirmRevertYes.addEventListener('click', confirmRevert);
  dom.confirmRevertNo.addEventListener('click', onNtfyCancel);

  dom.ntfySave.addEventListener('click', onNtfySave);
  dom.ntfyOff.addEventListener('click', onNtfyOff);
  dom.ntfyCancel.addEventListener('click', onNtfyCancel);

  dom.enterRetry.addEventListener('click', () => enterLumaTV(true));
  dom.enterLg.addEventListener('click', onEnterLg);

  dom.logClose.addEventListener('click', closeLogPanel);

  // Handle app suspend/resume for resilient apply flow
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible') {
      refreshStatus();
    }
  });

  window.addEventListener('focus', refreshStatus);

  /* ====== Initialization ====== */

  document.addEventListener('DOMContentLoaded', async () => {
    // On startup, refresh status, and start LumaTV when this is the way back to it
    await refreshStatus();
    await enterOnOpen();
    // Set initial focus on Apply row
    setFocus(0);
  });

  // If DOM is already loaded when script runs
  if (document.readyState !== 'loading') {
    refreshStatus().then(enterOnOpen).then(() => setFocus(0));
  }

})();
