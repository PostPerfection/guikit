/// How long the full screen preview's controls stay up after the last pointer move or key.
export const FULLSCREEN_HUD_IDLE_TIMEOUT_MS = 3000;

/// Whether the full screen controls stay up when the idle timeout runs out.
export function fullscreenHudStaysShown({ paused, pointerOverHud }) {
  return paused || pointerOverHud;
}

/// Whether `point` lies inside any of `rectangles`, edges included, in the shape
/// getBoundingClientRect returns.
export function pointIsInsideAnyRectangle(point, rectangles) {
  return rectangles.some(
    (rectangle) =>
      point.x >= rectangle.left && point.x <= rectangle.right && point.y >= rectangle.top && point.y <= rectangle.bottom,
  );
}
