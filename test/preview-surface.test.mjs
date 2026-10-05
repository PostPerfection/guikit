import assert from 'node:assert/strict';
import { register } from 'node:module';
import test from 'node:test';

register('./tauri-core-hooks.mjs', import.meta.url);

const bridge = await import('./tauri-core-stub.mjs');

const SHOWN_RECTANGLE = { left: 0, top: 0, width: 640, height: 360 };
const SCROLLED_RECTANGLE = { left: 0, top: -120, width: 640, height: 360 };
const RESIZED_RECTANGLE = { left: 0, top: -120, width: 800, height: 450 };
const SCROLL_EVENTS = 3;

let surfaceRectangle = SHOWN_RECTANGLE;

function fakeElement() {
  return {
    hidden: false,
    textContent: '',
    classList: { add() {}, remove() {}, toggle() {} },
    addEventListener() {},
    getBoundingClientRect: () => surfaceRectangle,
    querySelector: () => null,
  };
}

const panel = fakeElement();
const surface = fakeElement();
const elements = new Map([
  ['preview-panel', panel],
  ['preview-surface', surface],
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

bridge.answerWith('preview_is_embedded', true);
preview.initPreview();
// the watchers are registered once the backend has answered that it is embedded
await new Promise((resolve) => setTimeout(resolve, 0));

function surfaceReports() {
  return bridge.invocations.filter(([command]) => command === 'preview_set_surface').map(([, placement]) => placement);
}

function scroll() {
  watchers.scroll({});
}

function showPanel() {
  surfaceRectangle = SHOWN_RECTANGLE;
  preview.showEmbeddedPanel();
  bridge.forgetInvocations();
}

test('the first report is sent and the surface watchers are registered', () => {
  assert.deepEqual(surfaceReports(), [{ x: 0, y: 0, width: 640, height: 360, visible: true }]);
  assert.ok(watchers.scroll && watchers.resize && watchers.surfaceSize, 'a surface watcher was never registered');
});

test('scrolling, resizing and size changes report nothing while the panel is hidden', () => {
  showPanel();
  preview.closePreview();
  bridge.forgetInvocations();

  surfaceRectangle = SCROLLED_RECTANGLE;
  for (let event = 0; event < SCROLL_EVENTS; event++) scroll();
  watchers.resize({});
  watchers.surfaceSize([]);
  assert.deepEqual(surfaceReports(), []);
});

test('closing the panel reports the surface hidden once and scrolling adds nothing', () => {
  showPanel();
  preview.closePreview();
  for (let event = 0; event < SCROLL_EVENTS; event++) scroll();

  const reports = surfaceReports();
  assert.equal(reports.length, 1);
  assert.equal(reports[0].visible, false);
});

test('scrolling with the panel shown reports each new placement once', () => {
  showPanel();

  surfaceRectangle = SCROLLED_RECTANGLE;
  scroll();
  scroll();
  assert.deepEqual(surfaceReports(), [{ x: 0, y: -120, width: 640, height: 360, visible: true }]);

  surfaceRectangle = RESIZED_RECTANGLE;
  scroll();
  assert.deepEqual(surfaceReports().at(-1), { x: 0, y: -120, width: 800, height: 450, visible: true });
  assert.equal(surfaceReports().length, 2);
});
