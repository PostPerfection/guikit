// A drag shows the whole asset row as its image, which covers the reel tracks
// it is dropped on. This swaps in a one line label with the asset's name.
const DRAG_LABEL_CLASS = 'drag-label';
const DRAG_LABEL_MAX_CHARACTERS = 40;
const ELLIPSIS = '…';

/// `text` cut to `maxCharacters` with an ellipsis in the middle, so the start
/// and the extension both stay readable.
export function shortenLabel(text, maxCharacters = DRAG_LABEL_MAX_CHARACTERS) {
  if (text.length <= maxCharacters) return text;
  const kept = maxCharacters - ELLIPSIS.length;
  const head = Math.ceil(kept / 2);
  return text.slice(0, head) + ELLIPSIS + text.slice(text.length - (kept - head));
}

/// Make `text` the drag image of a dragstart event. The label element is in the
/// page only until the browser has copied it.
export function setDragLabel(event, text) {
  const label = document.createElement('div');
  label.className = DRAG_LABEL_CLASS;
  label.textContent = shortenLabel(text);
  document.body.appendChild(label);
  event.dataTransfer.setDragImage(label, 0, 0);
  setTimeout(() => label.remove(), 0);
}
