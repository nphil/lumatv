// node --test tests/lumagame_player.test.cjs: the player page's keys and the states the frame hears (game/player/player.js).
const test = require('node:test');
const assert = require('node:assert');
const { action, ytState, refused } = require('../game/player/player.js');

test('OK toggles, the media keys set, Left and Right seek ten seconds', () => {
  assert.deepStrictEqual(action('ok', true), { op: 'pause' });
  assert.deepStrictEqual(action('ok', false), { op: 'play' });
  assert.deepStrictEqual(action('pause', false), { op: 'pause' });
  assert.deepStrictEqual(action('left', true), { op: 'seek', by: -10 });
  assert.deepStrictEqual(action('right', false), { op: 'seek', by: 10 });
  assert.strictEqual(action('up', true), null);
});

test('the frame hears playing, paused and ended, and the embed refusals', () => {
  assert.deepStrictEqual([1, 2, 0, 3, -1].map(ytState), ['playing', 'paused', 'ended', null, null]);
  assert.ok([101, 150, 153].every(refused));
  assert.ok(!refused(0));
});
