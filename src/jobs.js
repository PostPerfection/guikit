// The Jobs panel: the rows the app's own backend is running, and whatever else
// an app queues elsewhere, in one table.
import { invoke } from '@tauri-apps/api/core';

const JOBS_TABLE_COLUMNS = 6;
const CANCELLABLE_JOB_STATES = ['running', 'queued'];
const GUI_JOB_SOURCE = 'gui';
const NO_EXTRA_ROWS_STATUS = 'Ready';
const DEFAULT_POLL_INTERVAL_MILLISECONDS = 3000;
const PLACEHOLDER_ROW = `<tr><td colspan="${JOBS_TABLE_COLUMNS}" style="text-align:center">No jobs</td></tr>`;

let jobsTableBody = null;
let jobsStatusBadge = null;
let extraRowsHook = null;
let pollIntervalMilliseconds = DEFAULT_POLL_INTERVAL_MILLISECONDS;
let pollTimer = null;
// The rows on screen now, in render order, which is what a cancel button's index
// points into.
let renderedRows = [];

/// Take over a Jobs table the app has in its own markup. `tableBody` is the
/// `<tbody>` the rows go in and `statusBadge` the element the source status is
/// written to. `refreshButton` gets a click handler when it is given, and
/// `pollIntervalMs` replaces the three second poll.
///
/// `extraRows` is an app's second source of jobs, an async function returning
/// `{ source, status, rows }`: `source` names the rows in the Source column,
/// `status` goes in the badge, and each row is
/// `{ id, label, state, progress, message, cancel }` with `cancel` an async
/// function the ✕ button calls. Leave it out and the panel shows the backend's
/// jobs alone, with the badge reading "Ready".
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
    state: job.status,
    progress: job.percent > 0 ? `${Math.round(job.percent)}%` : '',
    message: job.message,
    cancel: () => invoke('cancel_job', { jobId: Number(job.id) }),
  };
}

function rowMarkup({ source, id, label, state, progress, message }, index) {
  const cancel = CANCELLABLE_JOB_STATES.includes(state)
    ? `<button class="btn-sm btn-cancel" data-job-index="${index}">✕</button>`
    : '';
  const rowTitle = message ? ` title="${escapeHtml(message)}"` : '';
  return `<tr${rowTitle}><td>${escapeHtml(id)}</td><td>${escapeHtml(source)}</td><td>${escapeHtml(label)}</td>` +
    `<td>${escapeHtml(state)}</td><td>${escapeHtml(progress)}</td><td>${cancel}</td></tr>`;
}

async function handleTableClick(event) {
  const button = event.target.closest('.btn-cancel');
  if (!button) return;
  await renderedRows[Number(button.dataset.jobIndex)].cancel();
  refreshJobs();
}

function escapeHtml(text) {
  if (text === null || text === undefined) return '';
  const replacements = { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' };
  return String(text).replace(/[&<>"]/g, (character) => replacements[character]);
}
