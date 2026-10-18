import assert from "node:assert/strict";
import { register } from "node:module";
import test from "node:test";

register("./module-hooks.mjs", import.meta.url);

const previewStub = await import("./preview-stub.mjs");
const { boundedPropertiesWidth, initProjectPropertiesLayout, propertiesLayout, syncPreviewDock } = await import(
  "../src/properties-layout.js"
);

const POINTER = 7;
const PRIMARY_BUTTON = 0;
const PRIMARY_BUTTON_HELD = 1;
const WORKSPACE_WIDTH = 1000;
const START_WIDTH = 360;
const SAVED_WIDTH = 420;
const COLLAPSED_CLASS = "properties-collapsed";

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
    removeAttribute(name) {
      delete this.attributes[name];
    },
  };
}

function setUp(initiallySaved = {}) {
  const saved = new Map(Object.entries(initiallySaved));
  const workspace = {
    width: WORKSPACE_WIDTH,
    classList: fakeClassList(),
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
    removeItem: (key) => saved.delete(key),
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
  return { workspace, panel, handle, projectView, workspaceMain, saved, fireHandle };
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

test("collapsing reports zero width and the show label, expanding reports the saved width", () => {
  assert.deepEqual(propertiesLayout(true, SAVED_WIDTH), {
    panelWidth: 0,
    handleLabel: "Show Properties",
    handleTitle: "Show Properties",
  });
  assert.deepEqual(propertiesLayout(false, SAVED_WIDTH), {
    panelWidth: SAVED_WIDTH,
    handleLabel: "Resize Properties",
    handleTitle: "Drag to resize, double-click to hide",
  });
});

test("a double click collapses the panel and a click on the strip restores its saved width", () => {
  const { workspace, panel, handle, saved, fireHandle } = setUp({ "wizard-properties-width": String(SAVED_WIDTH) });

  fireHandle("click", { detail: 1 });
  fireHandle("click", { detail: 2 });
  fireHandle("dblclick");
  assert.equal(workspace.classList.contains(COLLAPSED_CLASS), true);
  assert.equal(handle.attributes["aria-label"], "Show Properties");
  assert.equal(handle.attributes.title, "Show Properties");
  assert.equal(saved.get("wizard-properties-collapsed"), "1");

  panel.width = 0;
  fireHandle("click", { detail: 1 });
  assert.equal(workspace.classList.contains(COLLAPSED_CLASS), false);
  assert.equal(panel.width, SAVED_WIDTH);
  assert.equal(handle.attributes["aria-label"], "Resize Properties");
  assert.equal(handle.attributes.title, "Drag to resize, double-click to hide");
  assert.equal(saved.has("wizard-properties-collapsed"), false);
});

test("a double click on the collapsed strip leaves the panel shown", () => {
  const { workspace, fireHandle } = setUp({ "wizard-properties-collapsed": "1" });

  fireHandle("click", { detail: 1 });
  fireHandle("click", { detail: 2 });
  fireHandle("dblclick");
  assert.equal(workspace.classList.contains(COLLAPSED_CLASS), false);
});

test("enter and space toggle the panel from the focused handle", () => {
  const { workspace, fireHandle } = setUp();

  assert.equal(fireHandle("keydown", { key: "Enter" }).defaultPrevented, true);
  assert.equal(workspace.classList.contains(COLLAPSED_CLASS), true);
  assert.equal(fireHandle("keydown", { key: " " }).defaultPrevented, true);
  assert.equal(workspace.classList.contains(COLLAPSED_CLASS), false);
  assert.equal(fireHandle("keydown", { key: "Enter", ctrlKey: true }).defaultPrevented, false);
  assert.equal(workspace.classList.contains(COLLAPSED_CLASS), false);
});

test("a saved collapsed panel starts collapsed", () => {
  const { workspace, handle } = setUp({ "wizard-properties-collapsed": "1" });

  assert.equal(workspace.classList.contains(COLLAPSED_CLASS), true);
  assert.equal(handle.attributes["aria-label"], "Show Properties");
});

test("dragging the collapsed strip left restores the panel from zero width", () => {
  const { workspace, panel, saved, fireHandle } = setUp({
    "wizard-properties-width": String(SAVED_WIDTH),
    "wizard-properties-collapsed": "1",
  });

  fireHandle("pointerdown", { button: PRIMARY_BUTTON, clientX: 700 });
  fireHandle("pointermove", { buttons: PRIMARY_BUTTON_HELD, clientX: 698 });
  assert.equal(workspace.classList.contains(COLLAPSED_CLASS), true);

  fireHandle("pointermove", { buttons: PRIMARY_BUTTON_HELD, clientX: 400 });
  assert.equal(workspace.classList.contains(COLLAPSED_CLASS), false);
  assert.equal(panel.width, 300);
  fireHandle("pointerup");
  assert.equal(saved.get("wizard-properties-width"), "300");
  assert.equal(saved.has("wizard-properties-collapsed"), false);
});

test("a click on the collapsed strip keeps the saved width", () => {
  const { panel, saved, fireHandle } = setUp({
    "wizard-properties-width": String(SAVED_WIDTH),
    "wizard-properties-collapsed": "1",
  });

  panel.width = 0;
  fireHandle("pointerdown", { button: PRIMARY_BUTTON, clientX: 700 });
  fireHandle("pointerup");
  fireHandle("click", { detail: 1 });
  assert.equal(panel.width, SAVED_WIDTH);
  assert.equal(saved.get("wizard-properties-width"), String(SAVED_WIDTH));
});

test("arrow keys leave a collapsed panel alone", () => {
  const { workspace, saved, fireHandle } = setUp({ "wizard-properties-collapsed": "1" });

  assert.equal(fireHandle("keydown", { key: "ArrowLeft" }).defaultPrevented, false);
  assert.equal(workspace.classList.contains(COLLAPSED_CLASS), true);
  assert.equal(saved.has("wizard-properties-width"), false);
});
