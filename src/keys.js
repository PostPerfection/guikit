import { escapeHtml } from './html.js';
import { formatDateTime } from './time-format.js';

const KEYS_TABLE_COLUMNS = 6;
const PLACEHOLDER_ROW = `<tr><td colspan="${KEYS_TABLE_COLUMNS}" style="text-align:center">No KDMs</td></tr>`;
const WINDOW_SEPARATOR = ' to ';
const PATH_SEPARATORS = /[\\/]/;
const FIT_WORDS = {
  Valid: 'valid',
  NotYetValid: 'not yet valid',
  Expired: 'expired',
  WrongRecipient: 'wrong recipient',
};
const REMOVE_BUTTON = { className: 'btn-remove', text: 'Remove' };

let keysTableBody = null;
let keysStatusBadge = null;
let loadKeys = null;
let keysActions = null;
let renderedKdms = [];

/// Take over a Keys table the app has in its own markup. `tableBody` is the
/// `<tbody>` the rows go in and `statusBadge` the element the status is written
/// to. `ingestButton` gets a click handler calling `actions.ingest()` when it is
/// given.
///
/// `load` is an async function returning `{ status, kdms }`, each KDM
/// `{ path, cplId, contentTitle, notValidBefore, notValidAfter, fit }`.
/// `actions` holds `ingest()` and `remove(kdm)`, and each is followed by a
/// refresh.
export function initKeysPanel({ tableBody, statusBadge, ingestButton, load, actions }) {
  keysTableBody = tableBody ?? null;
  keysStatusBadge = statusBadge ?? null;
  loadKeys = load;
  keysActions = actions;
  keysTableBody?.addEventListener('click', handleTableClick);
  ingestButton?.addEventListener('click', ingest);
}

/// Read the KDMs and render the table.
export async function refreshKeys() {
  if (!keysTableBody) return;

  const { status, kdms } = await loadKeys();
  renderedKdms = kdms;

  if (keysStatusBadge) keysStatusBadge.textContent = status;
  keysTableBody.innerHTML = kdms.length ? kdms.map(rowMarkup).join('') : PLACEHOLDER_ROW;
}

async function ingest() {
  await keysActions.ingest();
  refreshKeys();
}

function fileName(path) {
  return path.split(PATH_SEPARATORS).pop();
}

function rowMarkup(kdm, index) {
  const validityWindow = formatDateTime(kdm.notValidBefore) + WINDOW_SEPARATOR + formatDateTime(kdm.notValidAfter);
  const removeButton = `<button class="btn-sm ${REMOVE_BUTTON.className}" data-kdm-index="${index}">${REMOVE_BUTTON.text}</button>`;
  return `<tr><td>${escapeHtml(kdm.contentTitle)}</td><td>${escapeHtml(kdm.cplId)}</td><td>${validityWindow}</td>` +
    `<td>${escapeHtml(FIT_WORDS[kdm.fit])}</td><td title="${escapeHtml(kdm.path)}">${escapeHtml(fileName(kdm.path))}</td>` +
    `<td>${removeButton}</td></tr>`;
}

async function handleTableClick(event) {
  const button = event.target.closest(`.${REMOVE_BUTTON.className}`);
  if (!button) return;
  await keysActions.remove(renderedKdms[Number(button.dataset.kdmIndex)]);
  refreshKeys();
}
