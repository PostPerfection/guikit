import assert from 'node:assert/strict';
import { register } from 'node:module';
import test from 'node:test';

register('./tauri-core-hooks.mjs', import.meta.url);

const bridge = await import('./tauri-core-stub.mjs');

const WINDOW_RECTANGLE = { left: 0, top: 0, width: 1920, height: 1080 };
const FULL_SCREEN_RECTANGLE = { left: 0, top: 0, width: 3840, height: 2160 };

let surfaceRectangle = WINDOW_RECTANGLE;

function fakeElement() {
  return {
    listeners: {},
    addEventListener(name, handler) {
      this.listeners[name] = handler;
    },
    getBoundingClientRect: () => surfaceRectangle,
  };
}

const surface = fakeElement();
const playButton = fakeElement();
const scrubber = fakeElement();
const elements = new Map([
  ['preview-surface', surface],
  ['timeline-play-btn', playButton],
  ['timeline-scrubber', scrubber],
]);

const watchers = {};
globalThis.document = {
  getElementById: (id) => elements.get(id) ?? null,
  addEventListener(name, handler) {
    watchers[name] = handler;
  },
};
globalThis.window = {
  addEventListener(name, handler) {
    watchers[name] = handler;
  },
};
globalThis.ResizeObserver = class {
  constructor(handler) {
    watchers.surfaceSize = handler;
  }
  observe() {}
};

const preview = await import('../src/preview.js');

preview.initFullPageSurface();

function surfaceReports() {
  return bridge.invocations.filter(([command]) => command === 'preview_set_surface').map(([, placement]) => placement);
}

test('the surface is reported shown straight away', () => {
  assert.deepEqual(surfaceReports(), [{ x: 0, y: 0, width: 1920, height: 1080, visible: true }]);
});

test('a window resize reports the new size once', () => {
  surfaceRectangle = FULL_SCREEN_RECTANGLE;
  watchers.resize({});
  watchers.surfaceSize([]);
  assert.deepEqual(surfaceReports().at(-1), { x: 0, y: 0, width: 3840, height: 2160, visible: true });
  assert.equal(surfaceReports().length, 2);
});

test('the page asks the backend for nothing but the surface placement', () => {
  assert.deepEqual(
    bridge.invocations.map(([command]) => command).filter((command) => command !== 'preview_set_surface'),
    [],
  );
});

test('the transport and the keyboard are left alone', () => {
  assert.deepEqual(playButton.listeners, {});
  assert.deepEqual(scrubber.listeners, {});
  assert.equal(watchers.keydown, undefined);
});
