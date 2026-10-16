import assert from 'node:assert/strict';
import { register } from 'node:module';
import test from 'node:test';

register('./tauri-core-hooks.mjs', import.meta.url);

const { initPreviewResize } = await import('../src/preview-resize.js');

const POINTER = 7;
const START_HEIGHT = 360;
const SAVED_HEIGHT = 500;
const PRIMARY = 0;
const SECONDARY = 2;
const PRIMARY_HELD = 1;
const DOCKED_CLASS = 'preview-docked';

function fakePanel(classes) {
  const surface = { height: START_HEIGHT, getBoundingClientRect: () => ({ height: surface.height }) };
  const panel = {
    surface,
    classList: { contains: (name) => classes.includes(name) },
    style: { setProperty: (_, value) => (surface.height = parseInt(value)) },
    querySelector: () => surface,
    prepend(element) {
      panel.handle = element;
    },
  };
  return panel;
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

function setUp({ classes = [], savedValues = {} } = {}) {
  const panel = fakePanel(classes);
  const saved = new Map(Object.entries(savedValues));
  globalThis.document = { getElementById: () => panel, createElement: () => fakeHandle() };
  globalThis.localStorage = { getItem: (key) => saved.get(key) ?? null, setItem: (key, value) => saved.set(key, value) };
  initPreviewResize('wizard');
  const { handle } = panel;
  const fire = (name, event = {}) => {
    const fired = { pointerId: POINTER, preventDefault() {}, ...event };
    handle.handlers[name](fired);
  };
  return { surface: panel.surface, handle, saved, fire };
}

test('dragging the handle up grows the picture and saves its height on release', () => {
  const { surface, saved, fire } = setUp();
  fire('pointerdown', { button: PRIMARY, clientY: 400 });
  fire('pointermove', { buttons: PRIMARY_HELD, clientY: 340 });
  assert.equal(surface.height, START_HEIGHT + 60);
  fire('pointerup');
  assert.equal(saved.get('wizard-preview-height'), String(START_HEIGHT + 60));
});

test('dragging the handle down grows the picture of a docked panel', () => {
  const { surface, fire } = setUp({ classes: [DOCKED_CLASS] });
  fire('pointerdown', { button: PRIMARY, clientY: 400 });
  fire('pointermove', { buttons: PRIMARY_HELD, clientY: 460 });
  assert.equal(surface.height, START_HEIGHT + 60);
});

test('the saved height is applied on start', () => {
  const { surface } = setUp({ savedValues: { 'wizard-preview-height': String(SAVED_HEIGHT) } });
  assert.equal(surface.height, SAVED_HEIGHT);
});

test('a secondary button press does not start a drag', () => {
  const { surface, handle, fire } = setUp();
  fire('pointerdown', { button: SECONDARY, clientY: 400 });
  fire('pointermove', { buttons: PRIMARY_HELD, clientY: 200 });
  assert.equal(surface.height, START_HEIGHT);
  assert.equal(handle.hasPointerCapture(POINTER), false);
});
