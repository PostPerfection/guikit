import assert from 'node:assert/strict';
import { register } from 'node:module';
import test from 'node:test';

register('./tauri-core-hooks.mjs', import.meta.url);

const bridge = await import('./tauri-core-stub.mjs');
const preview = await import('../src/preview.js');

const SCRUBBER_IDLE_CLASS = 'timeline-scrubber-idle';
const SCRUBBER_RECTANGLE = { left: 0, top: 0, width: 400, height: 10 };
const DURATION_SECONDS = 120;

function fakeElement() {
  const classes = new Set();
  return {
    title: '',
    textContent: '',
    disabled: false,
    dataset: {},
    style: {},
    handlers: {},
    classList: {
      add: (name) => classes.add(name),
      remove: (name) => classes.delete(name),
      contains: (name) => classes.has(name),
    },
    addEventListener(name, handler) {
      this.handlers[name] = handler;
    },
    getBoundingClientRect: () => SCRUBBER_RECTANGLE,
    setPointerCapture() {},
  };
}

const elements = new Map(
  ['timeline-play-btn', 'timeline-scrubber', 'timeline-playhead', 'timeline-duration'].map((id) => [id, fakeElement()]),
);
elements.get('timeline-duration').dataset.raw = String(DURATION_SECONDS);
globalThis.document = { getElementById: (id) => elements.get(id) ?? null };

preview.initLiveTransport();
preview.stopScrubberPolling();

test('the transport is live with nothing loaded from this page', () => {
  assert.equal(elements.get('timeline-play-btn').disabled, false);
  assert.equal(elements.get('timeline-scrubber').classList.contains(SCRUBBER_IDLE_CLASS), false);
});

test('the play button and the scrubber drive the player', () => {
  bridge.forgetInvocations();
  elements.get('timeline-play-btn').handlers.click();
  elements.get('timeline-scrubber').handlers.pointerdown({ pointerId: 1, clientX: 100 });
  assert.deepEqual(bridge.invocations, [
    ['preview_play_pause', undefined],
    ['preview_seek_absolute', { seconds: 30 }],
  ]);
});
