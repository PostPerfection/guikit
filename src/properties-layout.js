import { dockPreviewPanel, undockPreviewPanel } from "./preview.js";

const PRIMARY_BUTTON = 0;
const PRIMARY_BUTTON_HELD = 1;
const ACTIVE_VIEW_CLASS = "active";
const MINIMUM_PROPERTIES_WIDTH = 280;
const MAXIMUM_PROPERTIES_WIDTH_FRACTION = 0.6;
const KEYBOARD_RESIZE_STEP = 24;
const TOGGLE_KEYS = ["Enter", " "];
const COLLAPSED_CLASS = "properties-collapsed";
const COLLAPSED_VALUE = "1";
// a click on the collapsed strip may move the pointer a little without meaning a drag
const DRAG_START_DISTANCE = 4;
const RESIZE_PROPERTIES_LABEL = "Resize Properties";
const SHOW_PROPERTIES_LABEL = "Show Properties";

export function boundedPropertiesWidth(width, workspaceWidth) {
  const maximumWidth = Math.max(MINIMUM_PROPERTIES_WIDTH, workspaceWidth * MAXIMUM_PROPERTIES_WIDTH_FRACTION);
  return Math.round(Math.min(Math.max(width, MINIMUM_PROPERTIES_WIDTH), maximumWidth));
}

export function propertiesLayout(collapsed, savedWidth) {
  if (collapsed) return { panelWidth: 0, handleLabel: SHOW_PROPERTIES_LABEL, handleTitle: SHOW_PROPERTIES_LABEL };
  return { panelWidth: savedWidth, handleLabel: RESIZE_PROPERTIES_LABEL, handleTitle: null };
}

export function syncPreviewDock() {
  if (document.getElementById("view-project").classList.contains(ACTIVE_VIEW_CLASS)) {
    dockPreviewPanel(document.querySelector("#view-project .workspace-main"));
  } else {
    undockPreviewPanel();
  }
}

export function initProjectPropertiesLayout(wizard) {
  const workspace = document.querySelector("#view-project .workspace");
  const panel = document.getElementById("properties");
  const handle = document.getElementById("properties-resize-handle");
  if (!workspace || !panel || !handle) return;

  const widthKey = `${wizard}-properties-width`;
  const collapsedKey = `${wizard}-properties-collapsed`;
  const setWidth = (width) => {
    const boundedWidth = boundedPropertiesWidth(width, workspace.getBoundingClientRect().width);
    workspace.style.setProperty("--project-properties-width", `${boundedWidth}px`);
    handle.setAttribute("aria-valuenow", String(boundedWidth));
  };
  const saveWidth = () => {
    const width = Math.round(panel.getBoundingClientRect().width);
    setWidth(width);
    localStorage.setItem(widthKey, String(width));
  };
  const savedWidth = () => {
    const width = Number(localStorage.getItem(widthKey));
    return Number.isFinite(width) && width > 0 ? width : null;
  };

  let collapsed = localStorage.getItem(collapsedKey) === COLLAPSED_VALUE;
  const applyLayout = () => {
    const layout = propertiesLayout(collapsed, savedWidth());
    workspace.classList.toggle(COLLAPSED_CLASS, collapsed);
    if (layout.panelWidth) setWidth(layout.panelWidth);
    handle.setAttribute("aria-label", layout.handleLabel);
    if (layout.handleTitle) handle.setAttribute("title", layout.handleTitle);
    else handle.removeAttribute("title");
  };
  const setCollapsed = (value) => {
    collapsed = value;
    if (collapsed) localStorage.setItem(collapsedKey, COLLAPSED_VALUE);
    else localStorage.removeItem(collapsedKey);
    applyLayout();
  };

  applyLayout();
  syncPreviewDock();

  let dragStart = null;
  let clicksStartedCollapsed = false;
  const endDrag = () => {
    if (!dragStart) return;
    if (handle.hasPointerCapture(dragStart.pointerId)) handle.releasePointerCapture(dragStart.pointerId);
    dragStart = null;
    if (!collapsed) saveWidth();
  };
  handle.addEventListener("pointerdown", (event) => {
    if (event.button !== PRIMARY_BUTTON) return;
    event.preventDefault();
    dragStart = {
      pointerId: event.pointerId,
      pointerX: event.clientX,
      width: propertiesLayout(collapsed, panel.getBoundingClientRect().width).panelWidth,
    };
    handle.setPointerCapture(event.pointerId);
  });
  handle.addEventListener("pointermove", (event) => {
    if (!dragStart) return;
    if ((event.buttons & PRIMARY_BUTTON_HELD) === 0) {
      endDrag();
      return;
    }
    const widthChange = dragStart.pointerX - event.clientX;
    if (collapsed && widthChange < DRAG_START_DISTANCE) return;
    if (collapsed) setCollapsed(false);
    setWidth(dragStart.width + widthChange);
  });
  for (const name of ["pointerup", "pointercancel", "lostpointercapture"]) {
    handle.addEventListener(name, endDrag);
  }
  handle.addEventListener("click", (event) => {
    if (event.detail > 1) return;
    clicksStartedCollapsed = collapsed;
    if (collapsed) setCollapsed(false);
  });
  // the first click of a double click on the collapsed strip has already shown the panel
  handle.addEventListener("dblclick", () => {
    if (!clicksStartedCollapsed) setCollapsed(true);
  });
  handle.addEventListener("keydown", (event) => {
    const modified = event.altKey || event.ctrlKey || event.metaKey || event.shiftKey;
    if (TOGGLE_KEYS.includes(event.key) && !modified) {
      event.preventDefault();
      setCollapsed(!collapsed);
      return;
    }
    if (collapsed || !["ArrowLeft", "ArrowRight"].includes(event.key)) return;
    event.preventDefault();
    const direction = event.key === "ArrowLeft" ? 1 : -1;
    setWidth(panel.getBoundingClientRect().width + direction * KEYBOARD_RESIZE_STEP);
    saveWidth();
  });
}
