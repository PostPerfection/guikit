const PRIMARY_BUTTON = 0;
const PRIMARY_BUTTON_HELD = 1;

export function initAssetStripResize(wizard) {
  const heightKey = `${wizard}-asset-strip-height`;
  const strip = document.querySelector("#view-project .workspace-main");
  const setHeight = (height) => strip.style.setProperty("--asset-strip-height", `${height}px`);
  const savedHeight = localStorage.getItem(heightKey);
  if (savedHeight) setHeight(Number(savedHeight));

  const handle = document.createElement("div");
  handle.className = "asset-strip-handle";
  handle.title = "Drag to resize";
  handle.setAttribute("role", "separator");
  handle.setAttribute("aria-orientation", "horizontal");
  strip.after(handle);

  let dragStart = null;
  const endDrag = () => {
    if (!dragStart) return;
    // a held capture would keep every click in the app on this handle
    if (handle.hasPointerCapture(dragStart.pointerId)) handle.releasePointerCapture(dragStart.pointerId);
    dragStart = null;
    // the CSS clamp bounds the rendered height
    const height = Math.round(strip.getBoundingClientRect().height);
    setHeight(height);
    localStorage.setItem(heightKey, String(height));
  };
  handle.addEventListener("pointerdown", (event) => {
    if (event.button !== PRIMARY_BUTTON) return;
    dragStart = { pointerId: event.pointerId, pointerY: event.clientY, height: strip.getBoundingClientRect().height };
    handle.setPointerCapture(event.pointerId);
  });
  handle.addEventListener("pointermove", (event) => {
    if (!dragStart) return;
    // a fast drag can lose the release, so a move with no button held ends it
    if ((event.buttons & PRIMARY_BUTTON_HELD) === 0) {
      endDrag();
      return;
    }
    setHeight(dragStart.height + event.clientY - dragStart.pointerY);
  });
  for (const name of ["pointerup", "pointercancel", "lostpointercapture"]) handle.addEventListener(name, endDrag);
}
