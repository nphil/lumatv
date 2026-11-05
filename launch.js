/**
 * Entering {{displayName}} from LG's launcher: the equivalent of `{{shortId}} compositor on`. The tool puts the Home stub in first, so that LG's
 * Flutter Home is ended and never comes back, then restarts the compositor and brings {{displayName}}'s Home up; where it fails it has put LG's
 * compositor and LG's Home back by itself. This file is the decision of the page and nothing else, apart from it so that it runs under node.
 *
 * deps: cli(verb, arg) -> Promise of the tool's JSON, rejected with the tool's own error text; exec(command) -> Promise of the root service's
 * answer; sleep(ms) -> Promise; now() -> ms.
 */
(function (root, factory) {
  var api = factory();
  if (typeof module === 'object' && module.exports) {
    module.exports = api;
  } else {
    root.LumaLaunch = api;
  }
})(typeof window !== 'undefined' ? window : this, function () {
  'use strict';

  var LG_HOME = 'com.webos.app.home';
  // What the tool says when it is only too early: LG's Home is still being brought back after an escape (recover-home.sh, up to 116 s of work),
  // or another run of the tool holds its lock. Both pass by themselves. Pinned to tools/compositor.sh and tools/lumatv by tests/test_launcher_app.py.
  var BUSY = /a recovery of LG's Home is running|another mutation is still running/i;
  var POLL_MS = 3000;
  var BUSY_MS = 120000;

  function reasonOf(err) {
    return (err && err.message) || 'unknown error';
  }

  /**
   * enter(deps, progress) -> { outcome: 'live' | 'started' | 'failed', reason }
   *
   * live: {{displayName}}'s compositor already is the compositor, and nothing was done. started: `compositor on` ended well. failed: it did not;
   * `reason` is the tool's text. `progress('starting')` and `progress('waiting', reason)` say what the page may show meanwhile.
   */
  async function enter(deps, progress) {
    var status = await deps.cli('compositor', 'status');
    if (status.compositor && status.compositor.live) {
      return { outcome: 'live' };
    }
    var begun = deps.now();
    for (;;) {
      progress('starting');
      try {
        await deps.cli('compositor', 'on');
        return { outcome: 'started' };
      } catch (err) {
        var reason = reasonOf(err);
        if (BUSY.test(reason) && deps.now() - begun < BUSY_MS) {
          progress('waiting', reason);
          await deps.sleep(POLL_MS);
          continue;
        }
        // It may have worked after all: the boot's own start came first, or the restart cut the answer off.
        try {
          var again = await deps.cli('compositor', 'status');
          if (again.compositor && again.compositor.live) {
            return { outcome: 'live' };
          }
        } catch (ignored) {
          // the status is no better than the failure already in hand
        }
        return { outcome: 'failed', reason: reason };
      }
    }
  }

  /** LG's own Home, launched by id: what the failure's way out does. */
  async function openLgHome(deps) {
    var response = await deps.exec('luna-send -n 1 luna://com.webos.applicationManager/launch \'' + JSON.stringify({ id: LG_HOME }) + '\'');
    var result = JSON.parse(response.stdoutString || '{}');
    if (!result.returnValue) {
      throw new Error(result.errorText || "LG's Home did not open");
    }
  }

  return { enter: enter, openLgHome: openLgHome, BUSY: BUSY, POLL_MS: POLL_MS, BUSY_MS: BUSY_MS, LG_HOME: LG_HOME };
});
