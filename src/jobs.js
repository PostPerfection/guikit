// The Jobs panel: the rows the app's own backend is running, and whatever else
// an app queues elsewhere, in one table.
import { invoke } from '@tauri-apps/api/core';
import { escapeHtml } from './html.js';

const JOBS_TABLE_COLUMNS = 6;
const QUEUED_JOB_STATE = 'queued';
const CANCELLABLE_JOB_STATES = ['running', QUEUED_JOB_STATE];
const GUI_JOB_SOURCE = 'gui';
const NO_EXTRA_ROWS_STATUS = 'Ready';
const DEFAULT_POLL_INTERVAL_MILLISECONDS = 3000;
const PLACEHOLDER_ROW = `<tr><td colspan="${JOBS_TABLE_COLUMNS}" style="text-align:center">No jobs</td></tr>`;
const CANCEL_BUTTON_CLASS = 'btn-cancel';
const MOVE_TO_FRONT_BUTTON = { className: 'btn-move-top', text: '⤒', title: 'Run next' };
const MOVE_EARLIER_BUTTON = { className: 'btn-move-up', text: '↑', title: 'Earlier' };
const MOVE_LATER_BUTTON = { className: 'btn-move-down', text: '↓', title: 'Later' };
const TABLE_BUTTON_ACTIONS = {
  [CANCEL_BUTTON_CLASS]: (row) => row.cancel(),
  [MOVE_TO_FRONT_BUTTON.className]: (row) => row.move(null),
  [MOVE_EARLIER_BUTTON.className]: (row) => row.move(queuedNeighbours(row).previous.id),
  // the backend only moves a job before another one
  [MOVE_LATER_BUTTON.className]: (row) => queuedNeighbours(row).next.move(row.id),
};
const TABLE_BUTTON_SELECTOR = Object.keys(TABLE_BUTTON_ACTIONS).map((className) => `.${className}`).join(', ');

let jobsTableBody = null;
let jobsStatusBadge = null;
let extraRowsHook = null;
let pollIntervalMilliseconds = DEFAULT_POLL_INTERVAL_MILLISECONDS;
let pollTimer = null;
// The rows on screen now, in render order, which is what a button's index points
// into.
let renderedRows = [];

/// Take over a Jobs table the app has in its own markup. `tableBody` is the
/// `<tbody>` the rows go in and `statusBadge` the element the source status is
/// written to. `refreshButton` gets a click handler when it is given, and
/// `pollIntervalMs` replaces the three second poll.
///
/// `extraRows` is an app's second source of jobs, an async function returning
/// `{ source, status, rows }`: `source` names the rows in the Source column,
/// `status` goes in the badge, and each row is
/// `{ id, label, state, progress, message, cancel, move }` with `cancel` an async
/// function the ✕ button calls. `move(beforeId)` is optional and puts the job
/// before the row with that id, or at the front for `null`, and a queued row
/// without it gets no reorder buttons. Leave the hook out and the panel shows
/// the backend's jobs alone, with the badge reading "Ready".
export function initJobsPanel({ tableBody, statusBadge, refreshButton, pollIntervalMs, extraRows } = {}) {
  jobsTableBody = tableBody ?? null;
  jobsStatusBadge = statusBadge ?? null;
  extraRowsHook = extraRows ?? null;
  pollIntervalMilliseconds = pollIntervalMs ?? DEFAULT_POLL_INTERVAL_MILLISECONDS;
  jobsTableBody?.addEventListener('click', handleTableClick);
  refreshButton?.addEventListener('click', refreshJobs);
}

/// Read both sources and render the table.
export async function refreshJobs() {
  if (!jobsTableBody) return;

  const backendJobs = await invoke('list_jobs').catch(() => []);
  const rows = backendJobs.map(backendRow);
  let status = NO_EXTRA_ROWS_STATUS;

  if (extraRowsHook) {
    const extra = await extraRowsHook();
    status = extra.status;
    rows.push(...extra.rows.map((row) => ({ source: extra.source, ...row })));
  }

  renderedRows = rows;
  if (jobsStatusBadge) jobsStatusBadge.textContent = status;
  jobsTableBody.innerHTML = rows.length ? rows.map(rowMarkup).join('') : PLACEHOLDER_ROW;
}

export function startJobsPolling() {
  if (!pollTimer) pollTimer = setInterval(refreshJobs, pollIntervalMilliseconds);
}

export function stopJobsPolling() {
  if (pollTimer) {
    clearInterval(pollTimer);
    pollTimer = null;
  }
}

/// One row of the shared `JobInfo` the backend lists.
function backendRow(job) {
  return {
    source: GUI_JOB_SOURCE,
    id: job.id,
    label: job.title,
    state: job.state,
    progress: job.percent > 0 ? `${Math.round(job.percent)}%` : '',
    message: job.message,
    cancel: () => invoke('cancel_job', { jobId: Number(job.id) }),
    move: (beforeId) => invoke('move_job', { jobId: Number(job.id), beforeJobId: beforeId === null ? null : Number(beforeId) }),
  };
}

function isQueued(row) {
  return row.state.toLowerCase() === QUEUED_JOB_STATE;
}

function queuedNeighbours(row) {
  const queuedOfSource = renderedRows.filter((other) => other.source === row.source && isQueued(other));
  const position = queuedOfSource.indexOf(row);
  return { previous: queuedOfSource[position - 1], next: queuedOfSource[position + 1] };
}

function moveButton({ className, text, title }, index) {
  return `<button class="btn-sm ${className}" data-job-index="${index}" title="${title}">${text}</button>`;
}

function moveButtons(row, index) {
  if (!row.move || !isQueued(row)) return '';
  const { previous, next } = queuedNeighbours(row);
  const earlierButtons = previous ? moveButton(MOVE_TO_FRONT_BUTTON, index) + moveButton(MOVE_EARLIER_BUTTON, index) : '';
  const laterButton = next ? moveButton(MOVE_LATER_BUTTON, index) : '';
  return earlierButtons + laterButton;
}

function rowMarkup(row, index) {
  const { source, id, label, state, progress, message } = row;
  const lowercaseState = state.toLowerCase();
  const cancel = CANCELLABLE_JOB_STATES.includes(lowercaseState)
    ? `<button class="btn-sm ${CANCEL_BUTTON_CLASS}" data-job-index="${index}">✕</button>`
    : '';
  const rowTitle = message ? ` title="${escapeHtml(message)}"` : '';
  return `<tr${rowTitle}><td>${escapeHtml(id)}</td><td>${escapeHtml(source)}</td><td>${escapeHtml(label)}</td>` +
    `<td>${escapeHtml(lowercaseState)}</td><td>${escapeHtml(progress)}</td><td>${moveButtons(row, index)}${cancel}</td></tr>`;
}

async function handleTableClick(event) {
  const button = event.target.closest(TABLE_BUTTON_SELECTOR);
  if (!button) return;
  const className = Object.keys(TABLE_BUTTON_ACTIONS).find((name) => button.classList.contains(name));
  await TABLE_BUTTON_ACTIONS[className](renderedRows[Number(button.dataset.jobIndex)]);
  refreshJobs();
}
