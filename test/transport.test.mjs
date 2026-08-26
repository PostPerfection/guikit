// The transport bar in src/preview.js, driven headless: the real module is
// imported, the buttons are fake elements and the stubbed bridge records what
// each click asked the backend for. `node --test test/`.
import assert from 'node:assert/strict';
import { register } from 'node:module';
import test from 'node:test';

register('./tauri-core-hooks.mjs', import.meta.url);

const bridge = await import('./tauri-core-stub.mjs');
const preview = await import('../src/preview.js');

const SKIP = preview.PREVIEW_SEEK_SECONDS;

const TRANSPORT_IDS = [
  'timeline-start-btn',
  'timeline-skip-back-btn',
  'timeline-frame-back-btn',
  'timeline-play-btn',
  'timeline-frame-forward-btn',
  'timeline-skip-forward-btn',
];

function fakeButton() {
  return {
    title: '',
    handlers: {},
    addEventListener(name, handler) {
      this.handlers[name] = handler;
    },
  };
}

const elements = new Map([...TRANSPORT_IDS, 'timeline-scrubber'].map((id) => [id, fakeButton()]));
globalThis.document = { getElementById: (id) => elements.get(id) ?? null };

preview.initPreview();
// the panel is not on this page, so the poll would only ask for metadata forever
preview.stopScrubberPolling();

function click(id) {
  bridge.forgetInvocations();
  elements.get(id).handlers.click();
  return bridge.invocations;
}

test('each transport button invokes its own command', () => {
  assert.deepEqual(click('timeline-start-btn'), [['preview_seek_absolute', { seconds: 0 }]]);
  assert.deepEqual(click('timeline-skip-back-btn'), [['preview_seek', { seconds: -SKIP }]]);
  assert.deepEqual(click('timeline-frame-back-btn'), [['preview_frame_back_step', undefined]]);
  assert.deepEqual(click('timeline-play-btn'), [['preview_play_pause', undefined]]);
  assert.deepEqual(click('timeline-frame-forward-btn'), [['preview_frame_step', undefined]]);
  assert.deepEqual(click('timeline-skip-forward-btn'), [['preview_seek', { seconds: SKIP }]]);
});

test('the skip buttons say how far they go', () => {
  assert.equal(elements.get('timeline-skip-back-btn').title, `Back ${SKIP} seconds`);
  assert.equal(elements.get('timeline-skip-forward-btn').title, `Forward ${SKIP} seconds`);
});
