import assert from "node:assert/strict";
import { register } from "node:module";
import test from "node:test";

register("./module-hooks.mjs", import.meta.url);

const previewStub = await import("./preview-stub.mjs");
const { boundedPropertiesWidth, initProjectPropertiesLayout, syncPreviewDock } = await import(
  "../src/properties-layout.js"
);

const POINTER = 7;
const PRIMARY_BUTTON = 0;
const PRIMARY_BUTTON_HELD = 1;
const WORKSPACE_WIDTH = 1000;
const START_WIDTH = 360;

function fakeClassList() {
  const values = new Set();
  return {
    contains: (value) => values.has(value),
    toggle(value, enabled) {
      if (enabled) values.add(value);
      else values.delete(value);
    },
  };
}

function fakeControl() {
  return {
    handlers: {},
    attributes: {},
    addEventListener(name, handler) {
      this.handlers[name] = handler;
    },
    setAttribute(name, value) {
      this.attributes[name] = value;
    },
  };
}

function setUp() {
  const saved = new Map();
  const workspace = {
    width: WORKSPACE_WIDTH,
    style: {
      setProperty(_, value) {
        panel.width = Number.parseInt(value);
      },
    },
    getBoundingClientRect: () => ({ width: workspace.width }),
  };
  const panel = { width: START_WIDTH, getBoundingClientRect: () => ({ width: panel.width }) };
  const handle = {
    ...fakeControl(),
    captured: new Set(),
    setPointerCapture(pointer) {
      this.captured.add(pointer);
    },
    hasPointerCapture(pointer) {
      return this.captured.has(pointer);
    },
    releasePointerCapture(pointer) {
      this.captured.delete(pointer);
    },
  };
  const projectView = { classList: fakeClassList() };
  projectView.classList.toggle("active", true);
  const workspaceMain = {};
  globalThis.document = {
    querySelector(selector) {
      return { "#view-project .workspace": workspace, "#view-project .workspace-main": workspaceMain }[selector];
    },
    getElementById(id) {
      return {
        properties: panel,
        "properties-resize-handle": handle,
        "view-project": projectView,
      }[id];
    },
  };
  globalThis.localStorage = {
    getItem: (key) => saved.get(key) ?? null,
    setItem: (key, value) => saved.set(key, value),
  };
  previewStub.forgetCalls();
  initProjectPropertiesLayout("wizard");
  const fireHandle = (name, event = {}) => {
    const fired = {
      pointerId: POINTER,
      defaultPrevented: false,
      preventDefault: () => (fired.defaultPrevented = true),
      ...event,
    };
    handle.handlers[name](fired);
    return fired;
  };
  return { panel, handle, projectView, workspaceMain, saved, fireHandle };
}

test("dragging left widens the right panel and saves its width", () => {
  const { panel, handle, saved, fireHandle } = setUp();
  fireHandle("pointerdown", { button: PRIMARY_BUTTON, clientX: 700 });
  fireHandle("pointermove", { buttons: PRIMARY_BUTTON_HELD, clientX: 620 });

  assert.equal(panel.width, START_WIDTH + 80);
  fireHandle("pointerup");
  assert.equal(saved.get("wizard-properties-width"), String(START_WIDTH + 80));
  assert.equal(handle.hasPointerCapture(POINTER), false);
});

test("the panel width stays between its minimum and sixty percent", () => {
  assert.equal(boundedPropertiesWidth(100, WORKSPACE_WIDTH), 280);
  assert.equal(boundedPropertiesWidth(900, WORKSPACE_WIDTH), 600);
});

test("arrow keys resize the panel", () => {
  const { panel, saved, fireHandle } = setUp();

  assert.equal(fireHandle("keydown", { key: "ArrowLeft" }).defaultPrevented, true);
  assert.equal(panel.width, START_WIDTH + 24);
  assert.equal(saved.get("wizard-properties-width"), String(START_WIDTH + 24));
});

test("the project view docks the preview above the assets", () => {
  const { workspaceMain } = setUp();

  assert.deepEqual(previewStub.calls.at(-1), ["dock", workspaceMain]);
});

test("leaving the project view undocks the preview and returning docks it again", () => {
  const { projectView, workspaceMain } = setUp();

  projectView.classList.toggle("active", false);
  syncPreviewDock();
  assert.deepEqual(previewStub.calls.at(-1), ["undock"]);

  projectView.classList.toggle("active", true);
  syncPreviewDock();
  assert.deepEqual(previewStub.calls.at(-1), ["dock", workspaceMain]);
});
