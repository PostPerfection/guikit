// The text dialog in src/text-dialog.js, driven headless: the real module builds
// its dialog out of fake elements and the stubbed bridge records what the preview
// surface was told while it was open. `node --test test/`.
import assert from 'node:assert/strict';
import { register } from 'node:module';
import test from 'node:test';

register('./tauri-core-hooks.mjs', import.meta.url);

const bridge = await import('./tauri-core-stub.mjs');

const TITLE = 'New composition';
const LABEL = 'Composition name';
const PREFILL = 'CPL 2';
const TYPED = '  Reel One  ';

const created = [];

function fakeElement(tag) {
  return {
    tag,
    className: '',
    textContent: '',
    value: '',
    type: '',
    open: false,
    hidden: false,
    children: [],
    handlers: {},
    appendChild(child) {
      this.children.push(child);
    },
    addEventListener(name, handler) {
      this.handlers[name] = handler;
    },
    getBoundingClientRect: () => ({ left: 0, top: 0, width: 640, height: 360 }),
    querySelector: () => null,
    focus() {},
    select() {},
    showModal() {
      this.open = true;
    },
    close() {
      this.open = false;
    },
  };
}

const panel = fakeElement('div');
const surface = fakeElement('div');
const body = fakeElement('body');
const elements = new Map([
  ['preview-panel', panel],
  ['preview-surface', surface],
]);

globalThis.document = {
  body,
  getElementById: (id) => elements.get(id) ?? null,
  addEventListener: () => {},
  createElement(tag) {
    const element = fakeElement(tag);
    created.push(element);
    return element;
  },
};
globalThis.window = { addEventListener: () => {} };
globalThis.ResizeObserver = class {
  observe() {}
};

const preview = await import('../src/preview.js');
const { askForText } = await import('../src/text-dialog.js');

bridge.answerWith('preview_is_embedded', true);
preview.initPreview();
// the surface is reported once the backend has answered that it is embedded
await new Promise((resolve) => setTimeout(resolve, 0));

function lastSurfaceVisibility() {
  const reports = bridge.invocations.filter(([command]) => command === 'preview_set_surface');
  assert.notEqual(reports.length, 0, 'the surface was never reported');
  return reports.at(-1)[1].visible;
}

function madeButton(text) {
  const button = created.find((element) => element.tag === 'button' && element.textContent === text);
  assert.ok(button, `the dialog has no ${text} button`);
  return button;
}

function openDialog() {
  bridge.forgetInvocations();
  const asked = askForText({ title: TITLE, label: LABEL, value: PREFILL });
  const dialog = created.find((element) => element.tag === 'dialog');
  assert.ok(dialog, 'the dialog element was never built');
  return { asked, dialog };
}

test('the surface is reported visible with the panel showing', () => {
  assert.equal(lastSurfaceVisibility(), true);
});

test('the open dialog takes the surface off the screen and OK answers the trimmed text', async () => {
  const { asked, dialog } = openDialog();
  assert.equal(dialog.open, true);
  assert.equal(dialog.children[0].textContent, TITLE);
  const input = created.find((element) => element.tag === 'input');
  assert.equal(input.value, PREFILL);
  assert.equal(lastSurfaceVisibility(), false);

  input.value = TYPED;
  bridge.forgetInvocations();
  madeButton('OK').handlers.click();
  assert.equal(await asked, TYPED.trim());
  assert.equal(dialog.open, false);
  assert.equal(lastSurfaceVisibility(), true);
});

test('Cancel answers nothing and puts the surface back', async () => {
  const { asked, dialog } = openDialog();
  assert.equal(lastSurfaceVisibility(), false);

  bridge.forgetInvocations();
  madeButton('Cancel').handlers.click();
  assert.equal(await asked, null);
  assert.equal(dialog.open, false);
  assert.equal(lastSurfaceVisibility(), true);
});

test('Escape answers nothing and puts the surface back', async () => {
  const { asked, dialog } = openDialog();
  assert.equal(lastSurfaceVisibility(), false);

  bridge.forgetInvocations();
  dialog.handlers.cancel();
  assert.equal(await asked, null);
  assert.equal(dialog.open, false);
  assert.equal(lastSurfaceVisibility(), true);
});

test('Enter in the field answers the trimmed text', async () => {
  const { asked, dialog } = openDialog();
  const input = created.find((element) => element.tag === 'input');
  input.value = TYPED;

  bridge.forgetInvocations();
  input.handlers.keydown({ key: 'Enter' });
  assert.equal(await asked, TYPED.trim());
  assert.equal(dialog.open, false);
  assert.equal(lastSurfaceVisibility(), true);
});

test('the hidden panel keeps the surface off the screen after the dialog closes', async () => {
  panel.hidden = true;
  const { asked } = openDialog();
  assert.equal(lastSurfaceVisibility(), false);

  bridge.forgetInvocations();
  madeButton('Cancel').handlers.click();
  await asked;
  assert.equal(lastSurfaceVisibility(), false);
  panel.hidden = false;
});
