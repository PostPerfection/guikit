import { PREVIEW_DOCKED_CLASS } from "./preview.js";

const PRIMARY_BUTTON = 0;
const PRIMARY_BUTTON_HELD = 1;
const GROWS_WITH_DRAG_DOWN = 1;
const GROWS_WITH_DRAG_UP = -1;

export function initPreviewResize(wizard) {
  const heightKey = `${wizard}-preview-height`;
  const panel = document.getElementById("preview-panel");
  const surface = panel.querySelector(".preview-surface");
  const setHeight = (height) => panel.style.setProperty("--preview-surface-height", `${height}px`);
  const savedHeight = localStorage.getItem(heightKey);
  if (savedHeight) setHeight(Number(savedHeight));

  const handle = document.createElement("div");
  handle.className = "preview-resize-handle";
  handle.title = "Drag to resize";
  handle.setAttribute("role", "separator");
  handle.setAttribute("aria-orientation", "horizontal");
  panel.prepend(handle);

  let dragStart = null;
  const endDrag = () => {
    if (!dragStart) return;
    // a held capture would keep every click in the app on this handle
    if (handle.hasPointerCapture(dragStart.pointerId)) handle.releasePointerCapture(dragStart.pointerId);
    dragStart = null;
    // the CSS minimum bounds the rendered height
    const height = Math.round(surface.getBoundingClientRect().height);
    setHeight(height);
    localStorage.setItem(heightKey, String(height));
  };
  handle.addEventListener("pointerdown", (event) => {
    if (event.button !== PRIMARY_BUTTON) return;
    // left alone, the press starts a text selection that WebKit then drags away, release included
    event.preventDefault();
    dragStart = {
      pointerId: event.pointerId,
      pointerY: event.clientY,
      height: surface.getBoundingClientRect().height,
      // a docked panel shows the handle under the picture
      direction: panel.classList.contains(PREVIEW_DOCKED_CLASS) ? GROWS_WITH_DRAG_DOWN : GROWS_WITH_DRAG_UP,
    };
    handle.setPointerCapture(event.pointerId);
  });
  handle.addEventListener("pointermove", (event) => {
    if (!dragStart) return;
    // a fast drag can lose the release, so a move with no button held ends it
    if ((event.buttons & PRIMARY_BUTTON_HELD) === 0) {
      endDrag();
      return;
    }
    setHeight(dragStart.height + dragStart.direction * (event.clientY - dragStart.pointerY));
  });
  for (const name of ["pointerup", "pointercancel", "lostpointercapture"]) handle.addEventListener(name, endDrag);
}
