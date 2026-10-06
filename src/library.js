import { escapeHtml } from './html.js';
import { formatDateTime, framesToTimecode } from './time-format.js';

const LIBRARY_TABLE_COLUMNS = 7;
const PLACEHOLDER_ROW = `<tr><td colspan="${LIBRARY_TABLE_COLUMNS}" style="text-align:center">No packages</td></tr>`;
const VERIFYING_VERDICT_STATE = 'Verifying';
const FAILED_VERDICT_STATE = 'Failed';
const ENCRYPTED_MARK = '🔒';
const ENCRYPTED_MARK_TITLE = 'Encrypted';
const PLAY_BUTTON = { className: 'btn-play', text: 'Play' };
const VERIFY_BUTTON = { className: 'btn-verify', text: 'Verify' };
const REMOVE_BUTTON = { className: 'btn-remove', text: 'Remove' };
const TABLE_BUTTON_ACTIONS = {
  [PLAY_BUTTON.className]: (row) => libraryActions.play(row.libraryPackage, row.composition),
  [VERIFY_BUTTON.className]: (row) => libraryActions.verify(row.libraryPackage),
  [REMOVE_BUTTON.className]: (row) => libraryActions.remove(row.libraryPackage),
};
const TABLE_BUTTON_SELECTOR = Object.keys(TABLE_BUTTON_ACTIONS).map((className) => `.${className}`).join(', ');

let libraryTableBody = null;
let libraryStatusBadge = null;
let loadLibrary = null;
let libraryActions = null;
let renderedRows = [];

/// Take over a Library table the app has in its own markup. `tableBody` is the
/// `<tbody>` the rows go in and `statusBadge` the element the status is written
/// to. `refreshButton` gets a click handler when it is given.
///
/// `load` is an async function returning `{ status, packages }`, each package
/// `{ directory, title, standard, compositions, verdict }`. `actions` holds
/// `play(package, composition)`, `verify(package)` and `remove(package)`, and
/// each is followed by a refresh.
export function initLibraryPanel({ tableBody, statusBadge, refreshButton, load, actions }) {
  libraryTableBody = tableBody ?? null;
  libraryStatusBadge = statusBadge ?? null;
  loadLibrary = load;
  libraryActions = actions;
  libraryTableBody?.addEventListener('click', handleTableClick);
  refreshButton?.addEventListener('click', refreshLibrary);
}

/// Read the packages and render the table.
export async function refreshLibrary() {
  if (!libraryTableBody) return;

  const { status, packages } = await loadLibrary();
  renderedRows = packages.flatMap((libraryPackage) =>
    libraryPackage.compositions.map((composition) => ({ libraryPackage, composition })),
  );

  if (libraryStatusBadge) libraryStatusBadge.textContent = status;
  libraryTableBody.innerHTML = renderedRows.length ? renderedRows.map(rowMarkup).join('') : PLACEHOLDER_ROW;
}

function verdictText({ state, errorCount }) {
  const lowercaseState = state.toLowerCase();
  if (state !== FAILED_VERDICT_STATE) return lowercaseState;
  return `${lowercaseState} (${errorCount})`;
}

function verdictCell(verdict) {
  const title = verdict.verifiedAt ? ` title="${escapeHtml(formatDateTime(verdict.verifiedAt))}"` : '';
  return `<td${title}>${escapeHtml(verdictText(verdict))}</td>`;
}

function encryptedCell(composition) {
  if (!composition.encrypted) return '<td></td>';
  return `<td title="${ENCRYPTED_MARK_TITLE}">${ENCRYPTED_MARK}</td>`;
}

function rowButton({ className, text }, index) {
  return `<button class="btn-sm ${className}" data-row-index="${index}">${text}</button>`;
}

function rowButtons(libraryPackage, index) {
  const play = rowButton(PLAY_BUTTON, index);
  if (libraryPackage.verdict.state === VERIFYING_VERDICT_STATE) return play;
  return play + rowButton(VERIFY_BUTTON, index) + rowButton(REMOVE_BUTTON, index);
}

function rowMarkup({ libraryPackage, composition }, index) {
  const duration = framesToTimecode(composition.durationFrames, composition.editRate);
  return `<tr><td>${escapeHtml(libraryPackage.title)}</td><td>${escapeHtml(composition.title)}</td>` +
    `<td>${duration}</td><td>${escapeHtml(libraryPackage.standard)}</td>${encryptedCell(composition)}` +
    `${verdictCell(libraryPackage.verdict)}<td>${rowButtons(libraryPackage, index)}</td></tr>`;
}

async function handleTableClick(event) {
  const button = event.target.closest(TABLE_BUTTON_SELECTOR);
  if (!button) return;
  const className = Object.keys(TABLE_BUTTON_ACTIONS).find((name) => button.classList.contains(name));
  await TABLE_BUTTON_ACTIONS[className](renderedRows[Number(button.dataset.rowIndex)]);
  refreshLibrary();
}
