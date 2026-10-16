import assert from 'node:assert/strict';
import { register } from 'node:module';
import test from 'node:test';

register('./tauri-core-hooks.mjs', import.meta.url);

const bridge = await import('./tauri-core-stub.mjs');

const DOCKED_CLASS = 'preview-docked';
const FULLSCREEN_CLASS = 'preview-fullscreen';
const SURFACE_RECTANGLE = { left: 0, top: 0, width: 640, height: 360 };

function fakeElement(name) {
  const classes = new Set();
  const listeners = {};
  const element = {
    name,
    hidden: false,
    children: [],
    parentElement: null,
    classList: {
      add: (value) => classes.add(value),
      remove: (value) => classes.delete(value),
      contains: (value) => classes.has(value),
      toggle(value, force = !classes.has(value)) {
        if (force) classes.add(value);
        else classes.delete(value);
      },
    },
    get firstElementChild() {
      return element.children[0] ?? null;
    },
    get nextElementSibling() {
      const siblings = element.parentElement?.children ?? [];
      return siblings[siblings.indexOf(element) + 1] ?? null;
    },
    append(...children) {
      for (const child of children) element.insertBefore(child, null);
    },
    prepend(child) {
      element.insertBefore(child, element.firstElementChild);
    },
    insertBefore(child, reference) {
      const oldSiblings = child.parentElement?.children;
      oldSiblings?.splice(oldSiblings.indexOf(child), 1);
      const index = reference ? element.children.indexOf(reference) : element.children.length;
      element.children.splice(index, 0, child);
      child.parentElement = element;
    },
    addEventListener(event, handler) {
      listeners[event] = handler;
    },
    click() {
      listeners.click();
    },
    getBoundingClientRect: () => SURFACE_RECTANGLE,
    querySelector: () => null,
  };
  return element;
}

const mainArea = fakeElement('main-area');
const content = fakeElement('content');
const panel = fakeElement('preview-panel');
const transport = fakeElement('global-transport');
const workspaceMain = fakeElement('workspace-main');
const assetsPanel = fakeElement('assets');
const surface = fakeElement('preview-surface');
const fullscreenButton = fakeElement('preview-fullscreen');
mainArea.append(content, panel, transport);
workspaceMain.append(assetsPanel);
panel.append(surface);

const elements = new Map([
  ['preview-panel', panel],
  ['preview-surface', surface],
  ['preview-fullscreen', fullscreenButton],
]);
globalThis.document = {
  body: fakeElement('body'),
  getElementById: (id) => elements.get(id) ?? null,
  addEventListener() {},
  querySelectorAll: () => [],
};
globalThis.window = { addEventListener() {}, removeEventListener() {} };
globalThis.ResizeObserver = class {
  observe() {}
};

const preview = await import('../src/preview.js');

bridge.answerWith('preview_is_embedded', true);
preview.initPreview();
// the full screen button is wired once the backend has answered that it is embedded
await new Promise((resolve) => setTimeout(resolve, 0));

function surfaceReportCount() {
  return bridge.invocations.filter(([command]) => command === 'preview_set_surface').length;
}

function childNames(element) {
  return element.children.map((child) => child.name);
}

test('docking moves the panel to the top of the container and reports the surface', () => {
  bridge.forgetInvocations();
  preview.dockPreviewPanel(workspaceMain);

  assert.deepEqual(childNames(workspaceMain), ['preview-panel', 'assets']);
  assert.deepEqual(childNames(mainArea), ['content', 'global-transport']);
  assert.ok(panel.classList.contains(DOCKED_CLASS));
  assert.equal(surfaceReportCount(), 1);

  preview.dockPreviewPanel(workspaceMain);
  assert.equal(surfaceReportCount(), 1, 'docking a docked panel reported again');
  preview.undockPreviewPanel();
});

test('undocking puts the panel back where it started and reports the surface', () => {
  preview.dockPreviewPanel(workspaceMain);
  bridge.forgetInvocations();
  preview.undockPreviewPanel();

  assert.deepEqual(childNames(mainArea), ['content', 'preview-panel', 'global-transport']);
  assert.deepEqual(childNames(workspaceMain), ['assets']);
  assert.ok(!panel.classList.contains(DOCKED_CLASS));
  assert.equal(surfaceReportCount(), 1);

  preview.undockPreviewPanel();
  assert.equal(surfaceReportCount(), 1, 'undocking an undocked panel reported again');
});

test('full screen takes a docked panel home and leaving it docks the panel again', () => {
  preview.dockPreviewPanel(workspaceMain);

  fullscreenButton.click();
  assert.ok(panel.classList.contains(FULLSCREEN_CLASS));
  assert.deepEqual(childNames(mainArea), ['content', 'preview-panel', 'global-transport']);
  assert.ok(!panel.classList.contains(DOCKED_CLASS));

  fullscreenButton.click();
  assert.ok(!panel.classList.contains(FULLSCREEN_CLASS));
  assert.deepEqual(childNames(workspaceMain), ['preview-panel', 'assets']);
  assert.ok(panel.classList.contains(DOCKED_CLASS));
  preview.undockPreviewPanel();
});
