// The Jobs panel in src/jobs.js, driven headless: the real module is imported,
// the table and the badge are fake elements, and the stubbed bridge answers
// `list_jobs` and records the cancels and moves. `node --test test/`.
import assert from 'node:assert/strict';
import { register } from 'node:module';
import test from 'node:test';

register('./tauri-core-hooks.mjs', import.meta.url);

const bridge = await import('./tauri-core-stub.mjs');

function fakeElement() {
  return {
    innerHTML: '',
    textContent: '',
    handlers: {},
    addEventListener(name, handler) {
      this.handlers[name] = handler;
    },
  };
}

let instances = 0;

/// A panel of its own per scenario, since the module holds the rendered rows.
async function freshPanel(backendJobs, options = {}) {
  bridge.forgetInvocations();
  bridge.answerWith('list_jobs', backendJobs);
  const tableBody = fakeElement();
  const statusBadge = fakeElement();
  const panel = await import(`../src/jobs.js?instance=${++instances}`);
  panel.initJobsPanel({ tableBody, statusBadge, ...options });
  await panel.refreshJobs();
  return { panel, tableBody, statusBadge };
}

/// A button click, the way the table delivers one: the button the event started
/// on matches the selector listing its class and carries the index of its row.
function clickButton(tableBody, className, index) {
  const button = { dataset: { jobIndex: String(index) }, classList: { contains: (name) => name === className } };
  const selectorListsClass = (selector) => selector.split(',').map((part) => part.trim()).includes(`.${className}`);
  return tableBody.handlers.click({
    target: { closest: (selector) => (selectorListsClass(selector) ? button : null) },
  });
}

function cancels() {
  return bridge.invocations.filter(([command]) => command === 'cancel_job');
}

function moves() {
  return bridge.invocations.filter(([command]) => command === 'move_job');
}

function rowsMarkup(tableBody) {
  return tableBody.innerHTML.split('</tr>').filter(Boolean);
}

function queuedJob(id) {
  return { id, title: `Build ${id}`, state: 'Queued', percent: 0, message: '' };
}

function queuedHookRow(id, moved) {
  return { id, label: 'transcode', state: 'queued', progress: '', message: '', cancel: async () => {}, move: async (beforeId) => moved.push([id, beforeId]) };
}

const MOVE_BUTTON_CLASSES = ['btn-move-top', 'btn-move-up', 'btn-move-down'];

test('a backend job becomes a row, escaped, with its message as the row title', async () => {
  const { tableBody, statusBadge } = await freshPanel([
    { id: 7, title: 'Build <DCP>', state: 'Running', percent: 41.4, message: 'reel 1 & 2' },
  ]);

  assert.ok(tableBody.innerHTML.includes('<td>7</td><td>gui</td><td>Build &lt;DCP&gt;</td>'));
  assert.ok(tableBody.innerHTML.includes('<td>running</td><td>41%</td>'));
  assert.ok(tableBody.innerHTML.includes('title="reel 1 &amp; 2"'));
  assert.equal(statusBadge.textContent, 'Ready');
});

test('no jobs anywhere leaves the placeholder', async () => {
  const { tableBody } = await freshPanel([]);

  assert.ok(tableBody.innerHTML.includes('colspan="6"'));
  assert.ok(tableBody.innerHTML.includes('No jobs'));
});

test('only running and queued rows get a cancel button', async () => {
  const { tableBody } = await freshPanel([
    { id: 1, title: 'Done', state: 'Completed', percent: 100, message: '' },
    { id: 2, title: 'Waiting', state: 'Queued', percent: 0, message: '' },
  ]);

  const buttons = tableBody.innerHTML.match(/btn-cancel/g);
  assert.equal(buttons.length, 1);
  assert.ok(tableBody.innerHTML.includes('data-job-index="1"'));
});

test('cancelling a backend row asks the backend for that job id', async () => {
  const { tableBody } = await freshPanel([
    { id: 12, title: 'Build', state: 'Running', percent: 5, message: '' },
  ]);

  await clickButton(tableBody, 'btn-cancel', 0);

  assert.deepEqual(cancels(), [['cancel_job', { jobId: 12 }]]);
});

test('hook rows carry the hook source, its status and its own cancel', async () => {
  const cancelled = [];
  const extraRows = async () => ({
    source: 'daemon',
    status: 'Online',
    rows: [{ id: 'a7', label: 'transcode', state: 'queued', progress: '10%', message: '', cancel: async () => cancelled.push('a7') }],
  });
  const { tableBody, statusBadge } = await freshPanel(
    [{ id: 3, title: 'Build', state: 'Completed', percent: 100, message: '' }],
    { extraRows },
  );

  assert.ok(tableBody.innerHTML.includes('<td>a7</td><td>daemon</td><td>transcode</td>'));
  assert.equal(statusBadge.textContent, 'Online');

  await clickButton(tableBody, 'btn-cancel', 1);

  assert.deepEqual(cancelled, ['a7']);
  assert.deepEqual(cancels(), []);
});

test('a capitalised hook row state shows lowercase and keeps its cancel', async () => {
  const extraRows = async () => ({
    source: 'daemon',
    status: 'Online',
    rows: [{ id: 'b2', label: 'package', state: 'Queued', progress: '', message: '', cancel: async () => {} }],
  });
  const { tableBody } = await freshPanel([], { extraRows });

  assert.ok(tableBody.innerHTML.includes('<td>queued</td>'));
  assert.ok(tableBody.innerHTML.includes('data-job-index="0"'));
});

test('a queued backend row gets the reorder buttons, running and completed rows none', async () => {
  const { tableBody } = await freshPanel([
    { id: 1, title: 'Build', state: 'Running', percent: 5, message: '' },
    queuedJob(2),
    queuedJob(3),
    queuedJob(4),
    { id: 5, title: 'Done', state: 'Completed', percent: 100, message: '' },
  ]);
  const [running, , middle, , completed] = rowsMarkup(tableBody);

  assert.ok(middle.includes('<button class="btn-sm btn-move-top" data-job-index="2" title="Run next">⤒</button>'));
  assert.ok(middle.includes('<button class="btn-sm btn-move-up" data-job-index="2" title="Earlier">↑</button>'));
  assert.ok(middle.includes('<button class="btn-sm btn-move-down" data-job-index="2" title="Later">↓</button>'));
  assert.ok(!running.includes('btn-move'));
  assert.ok(!completed.includes('btn-move'));
});

test('the first queued row has no top or up, the last no down, a lone one none', async () => {
  const { tableBody } = await freshPanel([queuedJob(1), queuedJob(2)]);
  const [first, last] = rowsMarkup(tableBody);

  assert.ok(!first.includes('btn-move-top'));
  assert.ok(!first.includes('btn-move-up'));
  assert.ok(first.includes('btn-move-down'));
  assert.ok(last.includes('btn-move-top'));
  assert.ok(last.includes('btn-move-up'));
  assert.ok(!last.includes('btn-move-down'));

  const { tableBody: loneTableBody } = await freshPanel([queuedJob(9)]);
  assert.ok(!loneTableBody.innerHTML.includes('btn-move'));
  assert.ok(loneTableBody.innerHTML.includes('btn-cancel'));
});

test('run next on a backend row moves it to the front', async () => {
  const { tableBody } = await freshPanel([{ id: 1, title: 'Build', state: 'Running', percent: 5, message: '' }, queuedJob(2), queuedJob(3)]);

  await clickButton(tableBody, 'btn-move-top', 2);

  assert.deepEqual(moves(), [['move_job', { jobId: 3, beforeJobId: null }]]);
});

test('earlier on the second queued backend row moves it before the first', async () => {
  const { tableBody } = await freshPanel([{ id: 1, title: 'Build', state: 'Running', percent: 5, message: '' }, queuedJob(2), queuedJob(3)]);

  await clickButton(tableBody, 'btn-move-up', 2);

  assert.deepEqual(moves(), [['move_job', { jobId: 3, beforeJobId: 2 }]]);
});

test('later on the first queued backend row moves the second before it', async () => {
  const { tableBody } = await freshPanel([{ id: 1, title: 'Build', state: 'Running', percent: 5, message: '' }, queuedJob(2), queuedJob(3)]);

  await clickButton(tableBody, 'btn-move-down', 1);

  assert.deepEqual(moves(), [['move_job', { jobId: 3, beforeJobId: 2 }]]);
});

test('a hook row with move gets the buttons and earlier calls its own move', async () => {
  const moved = [];
  const extraRows = async () => ({ source: 'daemon', status: 'Online', rows: [queuedHookRow('h1', moved), queuedHookRow('h2', moved)] });
  const { tableBody } = await freshPanel([], { extraRows });

  assert.ok(rowsMarkup(tableBody)[1].includes('btn-move-up'));

  await clickButton(tableBody, 'btn-move-up', 1);

  assert.deepEqual(moved, [['h2', 'h1']]);
  assert.deepEqual(moves(), []);
});

test('a queued hook row without move keeps its cancel and gets no reorder buttons', async () => {
  const rowWithoutMove = (id) => ({ id, label: 'package', state: 'queued', progress: '', message: '', cancel: async () => {} });
  const extraRows = async () => ({ source: 'daemon', status: 'Online', rows: [rowWithoutMove('c1'), rowWithoutMove('c2')] });
  const { tableBody } = await freshPanel([], { extraRows });

  assert.equal(tableBody.innerHTML.match(/btn-cancel/g).length, 2);
  for (const className of MOVE_BUTTON_CLASSES) assert.ok(!tableBody.innerHTML.includes(className));
});

test('reorder buttons and moves stay within the row source', async () => {
  const moved = [];
  const extraRows = async () => ({ source: 'daemon', status: 'Online', rows: [queuedHookRow('d1', moved), queuedHookRow('d2', moved)] });
  const { tableBody } = await freshPanel([queuedJob(1), queuedJob(2)], { extraRows });
  const [, lastBackend, firstDaemon] = rowsMarkup(tableBody);

  assert.ok(!lastBackend.includes('btn-move-down'));
  assert.ok(!firstDaemon.includes('btn-move-up'));
  assert.ok(!firstDaemon.includes('btn-move-top'));

  await clickButton(tableBody, 'btn-move-up', 1);
  await clickButton(tableBody, 'btn-move-up', 3);

  assert.deepEqual(moves(), [['move_job', { jobId: 2, beforeJobId: 1 }]]);
  assert.deepEqual(moved, [['d2', 'd1']]);
});
