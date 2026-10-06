import assert from 'node:assert/strict';
import test from 'node:test';

process.env.TZ = 'UTC';

const keys = await import('../src/keys.js');

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

function clickRemove(tableBody, index) {
  const button = { dataset: { kdmIndex: String(index) } };
  return tableBody.handlers.click({
    target: { closest: (selector) => (selector === '.btn-remove' ? button : null) },
  });
}

function rowsMarkup(tableBody) {
  return tableBody.innerHTML.split('</tr>').filter(Boolean);
}

function kdm(fit, path = '/keys/feature_kdm.xml') {
  return {
    path,
    cplId: 'urn:uuid:7f3a',
    contentTitle: 'Feature <FTR> & Co',
    notValidBefore: '2026-10-01T09:00:30+02:00',
    notValidAfter: '2026-10-08T23:59:59Z',
    fit,
  };
}

async function panelWith(kdms) {
  const calls = [];
  const loads = [];
  const actions = {
    ingest: async () => calls.push(['ingest']),
    remove: async (removed) => calls.push(['remove', removed]),
  };
  const load = async () => {
    loads.push(true);
    return { status: '3 KDMs', kdms };
  };
  const tableBody = fakeElement();
  const statusBadge = fakeElement();
  const ingestButton = fakeElement();
  keys.initKeysPanel({ tableBody, statusBadge, ingestButton, load, actions });
  await keys.refreshKeys();
  return { tableBody, statusBadge, ingestButton, calls, loads };
}

test('a kdm is a row with its title, cpl id, window, fit and file name, escaped', async () => {
  const { tableBody, statusBadge } = await panelWith([kdm('Valid', 'C:\\Keys\\a&b.xml')]);
  const [row] = rowsMarkup(tableBody);

  assert.ok(row.includes('<td>Feature &lt;FTR&gt; &amp; Co</td><td>urn:uuid:7f3a</td>'));
  assert.ok(row.includes('<td>2026-10-01 07:00 to 2026-10-08 23:59</td>'));
  assert.ok(row.includes('<td>valid</td><td title="C:\\Keys\\a&amp;b.xml">a&amp;b.xml</td>'));
  assert.ok(row.includes('btn-remove'));
  assert.equal(statusBadge.textContent, '3 KDMs');
});

test('every fit reads as words', async () => {
  const { tableBody } = await panelWith([kdm('Valid'), kdm('NotYetValid'), kdm('Expired'), kdm('WrongRecipient')]);
  const fits = rowsMarkup(tableBody).map((row) => row.match(/<td>(valid|not yet valid|expired|wrong recipient)<\/td>/)[1]);

  assert.deepEqual(fits, ['valid', 'not yet valid', 'expired', 'wrong recipient']);
});

test('no kdms leaves the placeholder', async () => {
  const { tableBody } = await panelWith([]);

  assert.ok(tableBody.innerHTML.includes('colspan="6"'));
  assert.ok(tableBody.innerHTML.includes('No KDMs'));
});

test('remove passes the row kdm, then refreshes', async () => {
  const listed = [kdm('Valid', '/keys/one.xml'), kdm('Expired', '/keys/two.xml')];
  const { tableBody, calls, loads } = await panelWith(listed);

  await clickRemove(tableBody, 1);

  assert.deepEqual(calls, [['remove', listed[1]]]);
  assert.equal(loads.length, 2);
});

test('the ingest button calls ingest, then refreshes', async () => {
  const { ingestButton, calls, loads } = await panelWith([]);

  await ingestButton.handlers.click();

  assert.deepEqual(calls, [['ingest']]);
  assert.equal(loads.length, 2);
});
