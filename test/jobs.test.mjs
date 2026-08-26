// The Jobs panel in src/jobs.js, driven headless: the real module is imported,
// the table and the badge are fake elements, and the stubbed bridge answers
// `list_jobs` and records the cancels. `node --test test/`.
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

/// A ✕ click, the way the table delivers one: the button the event started on
/// carries the index of its row.
function clickCancel(tableBody, index) {
  return tableBody.handlers.click({
    target: {
      closest: (selector) => (selector === '.btn-cancel' ? { dataset: { jobIndex: String(index) } } : null),
    },
  });
}

function cancels() {
  return bridge.invocations.filter(([command]) => command === 'cancel_job');
}

test('a backend job becomes a row, escaped, with its message as the row title', async () => {
  const { tableBody, statusBadge } = await freshPanel([
    { id: 7, title: 'Build <DCP>', status: 'running', percent: 41.4, message: 'reel 1 & 2' },
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
    { id: 1, title: 'Done', status: 'completed', percent: 100, message: '' },
    { id: 2, title: 'Waiting', status: 'queued', percent: 0, message: '' },
  ]);

  const buttons = tableBody.innerHTML.match(/btn-cancel/g);
  assert.equal(buttons.length, 1);
  assert.ok(tableBody.innerHTML.includes('data-job-index="1"'));
});

test('cancelling a backend row asks the backend for that job id', async () => {
  const { tableBody } = await freshPanel([
    { id: 12, title: 'Build', status: 'running', percent: 5, message: '' },
  ]);

  await clickCancel(tableBody, 0);

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
    [{ id: 3, title: 'Build', status: 'completed', percent: 100, message: '' }],
    { extraRows },
  );

  assert.ok(tableBody.innerHTML.includes('<td>a7</td><td>daemon</td><td>transcode</td>'));
  assert.equal(statusBadge.textContent, 'Online');

  await clickCancel(tableBody, 1);

  assert.deepEqual(cancelled, ['a7']);
  assert.deepEqual(cancels(), []);
});
