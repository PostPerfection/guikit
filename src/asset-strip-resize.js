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
  handle.addEventListener("pointerdown", (event) => {
    dragStart = { pointerY: event.clientY, height: strip.getBoundingClientRect().height };
    handle.setPointerCapture(event.pointerId);
  });
  handle.addEventListener("pointermove", (event) => {
    if (dragStart) setHeight(dragStart.height + event.clientY - dragStart.pointerY);
  });
  handle.addEventListener("pointerup", () => {
    dragStart = null;
    // the CSS clamp bounds the rendered height
    const height = Math.round(strip.getBoundingClientRect().height);
    setHeight(height);
    localStorage.setItem(heightKey, String(height));
  });
}
