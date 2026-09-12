// One line of text asked for in the page, since the webview's own prompt() draws
// behind the preview surface, which is a native widget over the page.
import { coverPreviewSurface, uncoverPreviewSurface } from './preview.js';

let dialog = null;
let heading = null;
let labelText = null;
let input = null;
let answer = null;

/// Ask for a line of text, resolving to the trimmed answer, or null when the
/// dialog is cancelled or Escape closes it.
export function askForText({ title = '', label = '', value = '' } = {}) {
  if (!dialog) buildDialog();
  heading.textContent = title;
  labelText.textContent = label;
  input.value = value;
  coverPreviewSurface();
  dialog.showModal();
  input.focus();
  input.select();
  return new Promise((resolve) => {
    answer = resolve;
  });
}

function finish(text) {
  if (!answer) return;
  const resolve = answer;
  answer = null;
  dialog.close();
  uncoverPreviewSurface();
  resolve(text);
}

function buildDialog() {
  dialog = document.createElement('dialog');
  dialog.className = 'text-dialog';

  heading = document.createElement('h2');
  dialog.appendChild(heading);

  const label = document.createElement('label');
  labelText = document.createElement('span');
  input = document.createElement('input');
  input.type = 'text';
  label.appendChild(labelText);
  label.appendChild(input);
  dialog.appendChild(label);

  const buttons = document.createElement('div');
  buttons.className = 'text-dialog-buttons';
  const cancelButton = document.createElement('button');
  cancelButton.className = 'btn-sm';
  cancelButton.textContent = 'Cancel';
  const okButton = document.createElement('button');
  okButton.className = 'btn-sm primary';
  okButton.textContent = 'OK';
  buttons.appendChild(cancelButton);
  buttons.appendChild(okButton);
  dialog.appendChild(buttons);

  okButton.addEventListener('click', () => finish(input.value.trim()));
  cancelButton.addEventListener('click', () => finish(null));
  dialog.addEventListener('cancel', () => finish(null));
  input.addEventListener('keydown', (event) => {
    if (event.key === 'Enter') finish(input.value.trim());
  });

  document.body.appendChild(dialog);
}
