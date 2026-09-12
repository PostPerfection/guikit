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

const DEFAULT_TITLE = 'Preview';
const SCRUBBER_IDLE_CLASS = 'timeline-scrubber-idle';
const LOADED_DIRECTORY = '/some/dir/Movie_DCP/';

function fakeElement() {
  const classes = new Set();
  return {
    title: '',
    textContent: '',
    disabled: false,
    handlers: {},
    classList: {
      add: (name) => classes.add(name),
      remove: (name) => classes.delete(name),
      contains: (name) => classes.has(name),
    },
    addEventListener(name, handler) {
      this.handlers[name] = handler;
    },
  };
}

const elements = new Map(
  [...TRANSPORT_IDS, 'timeline-scrubber', 'preview-title'].map((id) => [id, fakeElement()]),
);
globalThis.document = { getElementById: (id) => elements.get(id) ?? null };

preview.initPreview();
// the panel is not on this page, so the poll would only ask for metadata forever
preview.stopScrubberPolling();

function click(id) {
  bridge.forgetInvocations();
  const button = elements.get(id);
  assert.equal(button.disabled, false, `${id} is disabled, a browser would not click it`);
  button.handlers.click();
  return bridge.invocations;
}

function transportDisabled() {
  return TRANSPORT_IDS.map((id) => elements.get(id).disabled);
}

test('the transport is disabled with nothing loaded', () => {
  assert.deepEqual(transportDisabled(), TRANSPORT_IDS.map(() => true));
  assert.equal(elements.get('timeline-scrubber').classList.contains(SCRUBBER_IDLE_CLASS), true);
  assert.equal(elements.get('preview-title').textContent, DEFAULT_TITLE);
});

test('the scrubber ignores a press with nothing loaded', () => {
  bridge.forgetInvocations();
  elements.get('timeline-scrubber').handlers.pointerdown({ pointerId: 1, clientX: 10 });
  assert.deepEqual(bridge.invocations, []);
});

test('a load enables the transport and names what is showing', () => {
  preview.previewDcp(LOADED_DIRECTORY);
  preview.stopScrubberPolling();
  assert.deepEqual(transportDisabled(), TRANSPORT_IDS.map(() => false));
  assert.equal(elements.get('timeline-scrubber').classList.contains(SCRUBBER_IDLE_CLASS), false);
  assert.equal(elements.get('preview-title').textContent, 'Preview: Movie_DCP');
  assert.equal(elements.get('preview-title').title, LOADED_DIRECTORY);
});

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

test('stopping the player disables the transport and restores the label', () => {
  preview.stopPreview();
  assert.deepEqual(transportDisabled(), TRANSPORT_IDS.map(() => true));
  assert.equal(elements.get('timeline-scrubber').classList.contains(SCRUBBER_IDLE_CLASS), true);
  assert.equal(elements.get('preview-title').textContent, DEFAULT_TITLE);
  assert.equal(elements.get('preview-title').title, '');
});
