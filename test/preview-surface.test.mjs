import assert from 'node:assert/strict';
import { register } from 'node:module';
import test from 'node:test';

register('./tauri-core-hooks.mjs', import.meta.url);

const bridge = await import('./tauri-core-stub.mjs');
const tauriWindow = await import('./tauri-window-stub.mjs');

const SHOWN_RECTANGLE = { left: 0, top: 0, width: 640, height: 360 };
const SCROLLED_RECTANGLE = { left: 0, top: -120, width: 640, height: 360 };
const RESIZED_RECTANGLE = { left: 0, top: -120, width: 800, height: 450 };
const SCROLL_EVENTS = 3;
const FULLSCREEN_CLASS = 'preview-fullscreen';
const FULLSCREEN_HUD_HIDDEN_CLASS = 'preview-fullscreen-hud-hidden';
const FULLSCREEN_HUD_IDLE_TIMEOUT_MS = 3000;
const FULLSCREEN_SURFACE_RECTANGLE = { left: 0, top: 32, width: 1920, height: 1000 };
const IDLE_SURFACE_RECTANGLE = { left: 0, top: 0, width: 1920, height: 1080 };
const TRANSPORT_RECTANGLE = { left: 0, top: 1040, right: 1920, bottom: 1080 };
const POINTER_OVER_TRANSPORT = { clientX: 960, clientY: 1060 };
const POINTER_OVER_PICTURE = { clientX: 960, clientY: 540 };

let surfaceRectangle = SHOWN_RECTANGLE;

function fakeElement() {
  const classes = new Set();
  const listeners = {};
  return {
    hidden: false,
    textContent: '',
    classList: {
      add: (name) => classes.add(name),
      remove: (name) => classes.delete(name),
      contains: (name) => classes.has(name),
      toggle(name, force = !classes.has(name)) {
        if (force) classes.add(name);
        else classes.delete(name);
      },
    },
    addEventListener(name, handler) {
      listeners[name] = handler;
    },
    click() {
      listeners.click();
    },
    getBoundingClientRect: () => surfaceRectangle,
    querySelector: () => null,
  };
}

const panel = fakeElement();
const surface = fakeElement();
const fullscreenButton = fakeElement();
const body = fakeElement();
const transport = { getBoundingClientRect: () => TRANSPORT_RECTANGLE };
const elements = new Map([
  ['preview-panel', panel],
  ['preview-surface', surface],
  ['preview-fullscreen', fullscreenButton],
]);

// window and document listeners by event name, the size observer's under surfaceSize
const watchers = {};
function watch(name, handler) {
  watchers[name] ??= new Set();
  watchers[name].add(handler);
}
function dispatch(name, event) {
  for (const handler of watchers[name] ?? []) handler(event);
}
globalThis.document = {
  body,
  getElementById: (id) => elements.get(id) ?? null,
  addEventListener: watch,
  querySelectorAll: () => (panel.classList.contains(FULLSCREEN_CLASS) ? [transport] : []),
};
globalThis.window = {
  addEventListener: watch,
  removeEventListener(name, handler) {
    watchers[name]?.delete(handler);
  },
};
globalThis.ResizeObserver = class {
  constructor(handler) {
    watch('surfaceSize', handler);
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
  dispatch('scroll', {});
}

function showPanel() {
  surfaceRectangle = SHOWN_RECTANGLE;
  preview.showEmbeddedPanel();
  bridge.forgetInvocations();
  tauriWindow.fullscreenRequests.length = 0;
}

function pressEscape(defaultPrevented) {
  dispatch('keydown', { key: 'Escape', defaultPrevented, preventDefault() {} });
}

test('the first report is sent and the surface watchers are registered', () => {
  assert.deepEqual(surfaceReports(), [{ x: 0, y: 0, width: 640, height: 360, visible: true }]);
  assert.ok(
    watchers.scroll?.size && watchers.resize?.size && watchers.surfaceSize?.size,
    'a surface watcher was never registered',
  );
});

test('scrolling, resizing and size changes report nothing while the panel is hidden', () => {
  showPanel();
  preview.closePreview();
  bridge.forgetInvocations();

  surfaceRectangle = SCROLLED_RECTANGLE;
  for (let event = 0; event < SCROLL_EVENTS; event++) scroll();
  dispatch('resize', {});
  dispatch('surfaceSize', []);
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

test('the full screen button covers the window, restores it, and places the surface again each time', () => {
  showPanel();

  fullscreenButton.click();
  assert.ok(panel.classList.contains(FULLSCREEN_CLASS));
  assert.deepEqual(tauriWindow.fullscreenRequests, [true]);
  assert.equal(surfaceReports().length, 1);

  fullscreenButton.click();
  assert.ok(!panel.classList.contains(FULLSCREEN_CLASS));
  assert.deepEqual(tauriWindow.fullscreenRequests, [true, false]);
  assert.equal(surfaceReports().length, 2);
});

test('Escape restores the window unless another handler already took it', () => {
  showPanel();
  fullscreenButton.click();

  pressEscape(true);
  assert.ok(panel.classList.contains(FULLSCREEN_CLASS));

  pressEscape(false);
  assert.ok(!panel.classList.contains(FULLSCREEN_CLASS));
  assert.deepEqual(tauriWindow.fullscreenRequests, [true, false]);

  pressEscape(false);
  assert.deepEqual(tauriWindow.fullscreenRequests, [true, false]);
});

test('closing the preview in full screen restores the window', () => {
  showPanel();
  fullscreenButton.click();

  preview.closePreview();
  assert.ok(!panel.classList.contains(FULLSCREEN_CLASS));
  assert.deepEqual(tauriWindow.fullscreenRequests, [true, false]);
  assert.equal(surfaceReports().at(-1).visible, false);
});

test('full screen hides the controls when idle, places the surface over the whole screen and shows them on a pointer move', (context) => {
  context.mock.timers.enable({ apis: ['setTimeout'] });
  showPanel();
  surfaceRectangle = FULLSCREEN_SURFACE_RECTANGLE;
  fullscreenButton.click();
  assert.ok(!body.classList.contains(FULLSCREEN_HUD_HIDDEN_CLASS));

  surfaceRectangle = IDLE_SURFACE_RECTANGLE;
  context.mock.timers.tick(FULLSCREEN_HUD_IDLE_TIMEOUT_MS);
  assert.ok(body.classList.contains(FULLSCREEN_HUD_HIDDEN_CLASS));
  assert.deepEqual(surfaceReports().at(-1), { x: 0, y: 0, width: 1920, height: 1080, visible: true });

  surfaceRectangle = FULLSCREEN_SURFACE_RECTANGLE;
  dispatch('mousemove', POINTER_OVER_PICTURE);
  assert.ok(!body.classList.contains(FULLSCREEN_HUD_HIDDEN_CLASS));
  assert.deepEqual(surfaceReports().at(-1), { x: 0, y: 32, width: 1920, height: 1000, visible: true });

  preview.closePreview();
});

test('the controls stay up while the pointer rests on them and Escape does not count as activity', (context) => {
  context.mock.timers.enable({ apis: ['setTimeout'] });
  showPanel();
  fullscreenButton.click();

  dispatch('mousemove', POINTER_OVER_TRANSPORT);
  context.mock.timers.tick(FULLSCREEN_HUD_IDLE_TIMEOUT_MS * 2);
  assert.ok(!body.classList.contains(FULLSCREEN_HUD_HIDDEN_CLASS));

  // the window resizing under the pointer forgets where it was
  dispatch('resize', {});
  context.mock.timers.tick(FULLSCREEN_HUD_IDLE_TIMEOUT_MS);
  assert.ok(body.classList.contains(FULLSCREEN_HUD_HIDDEN_CLASS));

  pressEscape(true);
  assert.ok(body.classList.contains(FULLSCREEN_HUD_HIDDEN_CLASS));

  preview.closePreview();
});

test('leaving full screen shows the cursor and controls again and stops listening for activity', (context) => {
  context.mock.timers.enable({ apis: ['setTimeout'] });
  showPanel();
  fullscreenButton.click();
  context.mock.timers.tick(FULLSCREEN_HUD_IDLE_TIMEOUT_MS);
  assert.ok(body.classList.contains(FULLSCREEN_HUD_HIDDEN_CLASS));

  pressEscape(false);
  assert.ok(!body.classList.contains(FULLSCREEN_HUD_HIDDEN_CLASS));
  assert.equal(watchers.mousemove.size, 0);

  context.mock.timers.tick(FULLSCREEN_HUD_IDLE_TIMEOUT_MS);
  assert.ok(!body.classList.contains(FULLSCREEN_HUD_HIDDEN_CLASS));
});
