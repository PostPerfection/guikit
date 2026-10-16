import { dockPreviewPanel, undockPreviewPanel } from "./preview.js";

const PRIMARY_BUTTON = 0;
const PRIMARY_BUTTON_HELD = 1;
const ACTIVE_VIEW_CLASS = "active";
const MINIMUM_PROPERTIES_WIDTH = 280;
const MAXIMUM_PROPERTIES_WIDTH_FRACTION = 0.6;
const KEYBOARD_RESIZE_STEP = 24;

export function boundedPropertiesWidth(width, workspaceWidth) {
  const maximumWidth = Math.max(MINIMUM_PROPERTIES_WIDTH, workspaceWidth * MAXIMUM_PROPERTIES_WIDTH_FRACTION);
  return Math.round(Math.min(Math.max(width, MINIMUM_PROPERTIES_WIDTH), maximumWidth));
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

  const savedWidth = Number(localStorage.getItem(widthKey));
  if (Number.isFinite(savedWidth) && savedWidth > 0) setWidth(savedWidth);
  syncPreviewDock();

  let dragStart = null;
  const endDrag = () => {
    if (!dragStart) return;
    if (handle.hasPointerCapture(dragStart.pointerId)) handle.releasePointerCapture(dragStart.pointerId);
    dragStart = null;
    saveWidth();
  };
  handle.addEventListener("pointerdown", (event) => {
    if (event.button !== PRIMARY_BUTTON) return;
    event.preventDefault();
    dragStart = {
      pointerId: event.pointerId,
      pointerX: event.clientX,
      width: panel.getBoundingClientRect().width,
    };
    handle.setPointerCapture(event.pointerId);
  });
  handle.addEventListener("pointermove", (event) => {
    if (!dragStart) return;
    if ((event.buttons & PRIMARY_BUTTON_HELD) === 0) {
      endDrag();
      return;
    }
    setWidth(dragStart.width + dragStart.pointerX - event.clientX);
  });
  for (const name of ["pointerup", "pointercancel", "lostpointercapture"]) {
    handle.addEventListener(name, endDrag);
  }
  handle.addEventListener("keydown", (event) => {
    if (!["ArrowLeft", "ArrowRight"].includes(event.key)) return;
    event.preventDefault();
    const direction = event.key === "ArrowLeft" ? 1 : -1;
    setWidth(panel.getBoundingClientRect().width + direction * KEYBOARD_RESIZE_STEP);
    saveWidth();
  });
}
