// The queue in src/playlist.js, driven headless: the real module is imported and
// the player under it is the stub, so what is asserted here is the queue's own
// decisions. `node --test test/`.
import assert from 'node:assert/strict';
import { register } from 'node:module';
import test from 'node:test';

register('./module-hooks.mjs', import.meta.url);

const player = await import('./preview-stub.mjs');

const FIRST = '/packages/first';
const SECOND = '/packages/second';
const THIRD = '/packages/third';

/// A container the panel renders into, enough of an element for the queue: it
/// keeps the markup as text, and the click handler is called with a made-up event
/// rather than by parsing that markup back into elements.
function fakePanel() {
  return {
    innerHTML: '',
    classList: { add() {} },
    handlers: {},
    addEventListener(name, handler) {
      this.handlers[name] = handler;
    },
  };
}

let instances = 0;

/// A queue of its own per scenario, since the module holds the rows. The stub
/// stays one module, so the watchers it hands out are always the newest queue's.
async function freshQueue(...rows) {
  player.forgetCalls();
  const panel = fakePanel();
  const queue = await import(`../src/playlist.js?instance=${++instances}`);
  queue.initPlaylist(panel);
  for (const row of rows) queue.addToPlaylist(...[].concat(row));
  return { queue, panel };
}

/// A row playing, started the way the page starts one: clicked, then a poll that
/// reports the package loaded and paused.
function playRow(panel, index) {
  clickRow(panel, index);
  player.poll({ paused: true });
}

function clickRow(panel, index, control) {
  panel.handlers.click({
    target: {
      classList: { contains: (name) => name === control },
      closest: (selector) => (selector === '.playlist-row' ? { dataset: { index: String(index) } } : null),
    },
  });
}

function loads() {
  return player.calls.filter(([what]) => what === 'load').map(([, path]) => path);
}

test('a row is queued under the title the app gives it', async () => {
  const { panel } = await freshQueue([FIRST, 'The Feature (2K, EN-XX)']);
  assert.match(panel.innerHTML, /The Feature \(2K, EN-XX\)/);
  // the directory is still what the row points at, and its tooltip
  assert.match(panel.innerHTML, /title="\/packages\/first"/);
});

test("a row with no title of its own is the directory's name", async () => {
  const { panel } = await freshQueue(FIRST);
  assert.match(panel.innerHTML, />first</);
});

test("mpv's composition title replaces the one the app gave", async () => {
  const { panel } = await freshQueue([FIRST, 'Queued As This']);
  playRow(panel, 0);
  player.poll({ filename: 'THE COMPOSITION' });
  assert.match(panel.innerHTML, /THE COMPOSITION/);
  assert.doesNotMatch(panel.innerHTML, /Queued As This/);
});

test('removing a row that is not playing leaves playback alone', async () => {
  const { panel } = await freshQueue(FIRST, SECOND);
  playRow(panel, 0);
  player.forgetCalls();

  clickRow(panel, 1, 'playlist-remove');
  assert.deepEqual(player.calls, []);
  // the marker is still on the row playing, which is now the only row
  assert.match(panel.innerHTML, /playlist-row playing/);
});

test('removing the row playing stops the player and leaves the panel', async () => {
  const { panel } = await freshQueue(FIRST, SECOND);
  playRow(panel, 0);
  player.forgetCalls();

  clickRow(panel, 0, 'playlist-remove');
  assert.deepEqual(player.calls, [['stop']]);
  assert.doesNotMatch(panel.innerHTML, /playing/);
});

test('removing the last row clears the preview as well', async () => {
  const { panel } = await freshQueue(FIRST);
  playRow(panel, 0);
  // the row has played out and mpv is holding its last frame, which is still the
  // queue's package on screen
  player.poll({ eof: true, paused: true });
  player.forgetCalls();

  clickRow(panel, 0, 'playlist-remove');
  assert.deepEqual(player.calls, [['close']]);
});

test('removing rows the queue no longer owns touches nothing', async () => {
  const { panel } = await freshQueue(FIRST, SECOND);
  playRow(panel, 0);
  // the app previewed something of its own, so the queue let go
  player.loadedByTheApp('/somewhere/else');
  player.forgetCalls();

  clickRow(panel, 0, 'playlist-remove');
  clickRow(panel, 0, 'playlist-remove');
  assert.deepEqual(player.calls, []);
});

test('an end of file advances the queue once, whatever later polls report', async () => {
  const { panel } = await freshQueue(FIRST, SECOND, THIRD);
  playRow(panel, 0);
  assert.deepEqual(loads(), [FIRST]);

  // the first row reaches its end, and the queue takes the decision here
  player.poll({ eof: true, paused: true });
  assert.deepEqual(loads(), [FIRST, SECOND]);
  // the flag is still set while the player takes the load, and it must not count
  // as a second end of file
  player.poll({ eof: true, paused: true });
  assert.deepEqual(loads(), [FIRST, SECOND]);
  // the flag clears, which is the row being loaded, and the pause it inherited
  // is what the one play/pause is for
  player.poll({ eof: false, paused: true });
  assert.deepEqual(player.calls.at(-1), ['play_pause']);

  player.poll({ eof: true, paused: true });
  assert.deepEqual(loads(), [FIRST, SECOND, THIRD]);
});

test('a decision already made survives the flag being cleared under it', async () => {
  const { panel } = await freshQueue(FIRST, SECOND);
  playRow(panel, 0);

  player.poll({ eof: true, paused: true });
  assert.deepEqual(loads(), [FIRST, SECOND]);
  // whatever cleared mpv's flag, the row that was queued stays queued and the
  // queue does not go looking for the end of the row before it again
  player.poll({ eof: false, paused: true });
  player.poll({ eof: false, paused: false });
  assert.deepEqual(loads(), [FIRST, SECOND]);
});

test('the last row ends with the player holding its last frame', async () => {
  const { panel } = await freshQueue(FIRST);
  playRow(panel, 0);
  player.forgetCalls();

  player.poll({ eof: true, paused: true });
  player.poll({ eof: true, paused: true });
  assert.deepEqual(player.calls, []);
});

test('a row queued after the queue ran out does not start on its own', async () => {
  const { queue, panel } = await freshQueue(FIRST);
  playRow(panel, 0);
  player.poll({ eof: true, paused: true });
  player.forgetCalls();

  // the end of that row was already dealt with, and the player is sitting on its
  // last frame reporting the flag on every poll
  queue.addToPlaylist(SECOND);
  player.poll({ eof: true, paused: true });
  assert.deepEqual(player.calls, []);
  // clicking it still plays it
  clickRow(panel, 1);
  assert.deepEqual(loads(), [SECOND]);
});
