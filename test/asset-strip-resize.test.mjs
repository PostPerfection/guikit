import assert from 'node:assert/strict';
import test from 'node:test';

import { initAssetStripResize } from '../src/asset-strip-resize.js';

const POINTER = 7;
const START_HEIGHT = 300;
const PRIMARY = 0;
const SECONDARY = 2;
const NO_BUTTONS = 0;
const PRIMARY_HELD = 1;

function fakeStrip() {
  const strip = {
    height: START_HEIGHT,
    style: { setProperty: (_, value) => (strip.height = parseInt(value)) },
    getBoundingClientRect: () => ({ height: strip.height }),
    after(element) {
      strip.handle = element;
    },
  };
  return strip;
}

function fakeHandle() {
  return {
    handlers: {},
    captured: new Set(),
    setAttribute() {},
    addEventListener(name, handler) {
      this.handlers[name] = handler;
    },
    setPointerCapture(id) {
      this.captured.add(id);
    },
    hasPointerCapture(id) {
      return this.captured.has(id);
    },
    releasePointerCapture(id) {
      this.captured.delete(id);
    },
  };
}

function setUp() {
  const strip = fakeStrip();
  const saved = new Map();
  globalThis.document = { querySelector: () => strip, createElement: () => fakeHandle() };
  globalThis.localStorage = { getItem: (key) => saved.get(key) ?? null, setItem: (key, value) => saved.set(key, value) };
  initAssetStripResize('wizard');
  const { handle } = strip;
  const fire = (name, event = {}) => handle.handlers[name]({ pointerId: POINTER, ...event });
  return { strip, handle, saved, fire };
}

test('a drag follows the pointer and saves the height on release', () => {
  const { strip, saved, fire } = setUp();
  fire('pointerdown', { button: PRIMARY, clientY: 100 });
  fire('pointermove', { buttons: PRIMARY_HELD, clientY: 150 });
  assert.equal(strip.height, START_HEIGHT + 50);
  fire('pointerup');
  assert.equal(saved.get('wizard-asset-strip-height'), String(START_HEIGHT + 50));
});

test('a move with no button held ends a drag whose release was missed', () => {
  const { strip, handle, saved, fire } = setUp();
  fire('pointerdown', { button: PRIMARY, clientY: 100 });
  fire('pointermove', { buttons: PRIMARY_HELD, clientY: 120 });
  fire('pointermove', { buttons: NO_BUTTONS, clientY: 400 });
  fire('pointermove', { buttons: PRIMARY_HELD, clientY: 500 });
  assert.equal(strip.height, START_HEIGHT + 20);
  assert.equal(saved.get('wizard-asset-strip-height'), String(START_HEIGHT + 20));
  // a capture left on the handle would take every later click in the app
  assert.equal(handle.hasPointerCapture(POINTER), false);
});

test('losing the capture ends the drag', () => {
  const { strip, fire } = setUp();
  fire('pointerdown', { button: PRIMARY, clientY: 100 });
  fire('lostpointercapture');
  fire('pointermove', { buttons: PRIMARY_HELD, clientY: 300 });
  assert.equal(strip.height, START_HEIGHT);
});

test('a secondary button press does not start a drag', () => {
  const { strip, handle, fire } = setUp();
  fire('pointerdown', { button: SECONDARY, clientY: 100 });
  fire('pointermove', { buttons: PRIMARY_HELD, clientY: 300 });
  assert.equal(strip.height, START_HEIGHT);
  assert.equal(handle.hasPointerCapture(POINTER), false);
});
