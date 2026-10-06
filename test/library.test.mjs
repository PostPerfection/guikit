import assert from 'node:assert/strict';
import test from 'node:test';

const library = await import('../src/library.js');

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

function clickButton(tableBody, className, index) {
  const button = { dataset: { rowIndex: String(index) }, classList: { contains: (name) => name === className } };
  const selectorListsClass = (selector) => selector.split(',').map((part) => part.trim()).includes(`.${className}`);
  return tableBody.handlers.click({
    target: { closest: (selector) => (selectorListsClass(selector) ? button : null) },
  });
}

function rowsMarkup(tableBody) {
  return tableBody.innerHTML.split('</tr>').filter(Boolean);
}

function feature(verdictState = 'Valid') {
  return {
    directory: '/library/Feature',
    title: 'Feature <FTR>',
    standard: 'SMPTE',
    compositions: [
      { id: 'urn:uuid:1', title: 'Feature & Credits', durationFrames: 172800, editRate: [24, 1], encrypted: true },
      { id: 'urn:uuid:2', title: 'Trailer', durationFrames: 3000, editRate: [25, 1], encrypted: false },
    ],
    verdict: { state: verdictState, errorCount: 0, verifiedAt: null },
  };
}

async function panelWith(packages) {
  const calls = [];
  const loads = [];
  const actions = {
    play: async (libraryPackage, composition) => calls.push(['play', libraryPackage, composition]),
    verify: async (libraryPackage) => calls.push(['verify', libraryPackage]),
    remove: async (libraryPackage) => calls.push(['remove', libraryPackage]),
  };
  const load = async () => {
    loads.push(true);
    return { status: 'Watching 2 folders', packages };
  };
  const tableBody = fakeElement();
  const statusBadge = fakeElement();
  library.initLibraryPanel({ tableBody, statusBadge, load, actions });
  await library.refreshLibrary();
  return { tableBody, statusBadge, calls, loads };
}

test('each composition is a row with every column, escaped', async () => {
  const failed = { ...feature(), verdict: { state: 'Failed', errorCount: 3, verifiedAt: '2026-10-06T12:00:00Z' } };
  const { tableBody, statusBadge } = await panelWith([failed]);
  const [first, second] = rowsMarkup(tableBody);

  assert.ok(first.includes('<td>Feature &lt;FTR&gt;</td><td>Feature &amp; Credits</td><td>02:00:00</td><td>SMPTE</td>'));
  assert.ok(first.includes('<td title="Encrypted">🔒</td>'));
  assert.ok(first.includes('<td title="2026-10-06T12:00:00Z">failed (3)</td>'));
  assert.ok(second.includes('<td>Trailer</td><td>00:02:00</td><td>SMPTE</td><td></td><td title="2026-10-06T12:00:00Z">failed (3)</td>'));
  assert.equal(statusBadge.textContent, 'Watching 2 folders');
});

test('a verdict other than failed shows lowercase with no count', async () => {
  const { tableBody } = await panelWith([feature('Unverified')]);

  assert.ok(rowsMarkup(tableBody)[0].includes('<td>unverified</td>'));
});

test('no packages leaves the placeholder', async () => {
  const { tableBody } = await panelWith([]);

  assert.ok(tableBody.innerHTML.includes('colspan="7"'));
  assert.ok(tableBody.innerHTML.includes('No packages'));
});

test('play passes the package and the row composition, then refreshes', async () => {
  const listed = feature();
  const { tableBody, calls, loads } = await panelWith([listed]);

  await clickButton(tableBody, 'btn-play', 1);

  assert.deepEqual(calls, [['play', listed, listed.compositions[1]]]);
  assert.equal(loads.length, 2);
});

test('verify and remove pass the package, then refresh', async () => {
  const listed = feature();
  const { tableBody, calls, loads } = await panelWith([listed]);

  await clickButton(tableBody, 'btn-verify', 0);
  await clickButton(tableBody, 'btn-remove', 1);

  assert.deepEqual(calls, [['verify', listed], ['remove', listed]]);
  assert.equal(loads.length, 3);
});

test('a verifying row has play but no verify or remove', async () => {
  const { tableBody } = await panelWith([feature('Verifying')]);
  const [first] = rowsMarkup(tableBody);

  assert.ok(first.includes('btn-play'));
  assert.ok(first.includes('<td>verifying</td>'));
  assert.ok(!tableBody.innerHTML.includes('btn-verify'));
  assert.ok(!tableBody.innerHTML.includes('btn-remove'));
});
