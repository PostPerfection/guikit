// What previewFile and previewDcp ask the backend for, with the real module and
// the stubbed bridge. `node --test test/`.
import assert from 'node:assert/strict';
import { register } from 'node:module';
import test from 'node:test';

register('./tauri-core-hooks.mjs', import.meta.url);

const bridge = await import('./tauri-core-stub.mjs');
const preview = await import('../src/preview.js');

const PACKAGE = '/films/Film_FTR';
const PICTURE = '/films/Film_FTR/picture.mxf';
const CONTENT_KEYS = { kdm: '/keys/film.kdm.xml', recipient_key: '/keys/recipient.pem', keys: null };

const status = { textContent: '', title: '' };
globalThis.document = { getElementById: (id) => (id === 'status-text' ? status : null) };

function loadsAskedFor(load) {
  bridge.forgetInvocations();
  const loaded = load();
  preview.stopScrubberPolling();
  return { loaded, invocations: bridge.invocations.filter(([command]) => command.startsWith('preview_load')) };
}

test('a package load carries the content keys it is given', async () => {
  const { loaded, invocations } = loadsAskedFor(() => preview.previewDcp(PACKAGE, CONTENT_KEYS));
  assert.deepEqual(invocations, [['preview_load_dcp', { dirPath: PACKAGE, contentKeys: CONTENT_KEYS }]]);
  assert.equal(await loaded, true);
});

test('a package load given no keys sends null', () => {
  const { invocations } = loadsAskedFor(() => preview.previewDcp(PACKAGE));
  assert.deepEqual(invocations, [['preview_load_dcp', { dirPath: PACKAGE, contentKeys: null }]]);
});

test('a file load carries its content keys or null', () => {
  assert.deepEqual(loadsAskedFor(() => preview.previewFile(PICTURE, CONTENT_KEYS)).invocations, [
    ['preview_load', { filePath: PICTURE, contentKeys: CONTENT_KEYS }],
  ]);
  assert.deepEqual(loadsAskedFor(() => preview.previewFile(PICTURE)).invocations, [
    ['preview_load', { filePath: PICTURE, contentKeys: null }],
  ]);
});

test('a refused load resolves false and says why', async () => {
  bridge.refuseWith('preview_load_dcp', 'KDM/keys do not cover picture KeyId 1234');
  const { loaded } = loadsAskedFor(() => preview.previewDcp(PACKAGE, CONTENT_KEYS));
  assert.equal(await loaded, false);
  assert.equal(status.textContent, 'Preview failed: KDM/keys do not cover picture KeyId 1234');
  bridge.forgetRefusals();
});

test('asking whether a path needs keys invokes its own command', async () => {
  bridge.forgetInvocations();
  bridge.answerWith('preview_needs_content_keys', true);
  assert.equal(await preview.previewNeedsContentKeys(PACKAGE), true);
  assert.deepEqual(bridge.invocations, [['preview_needs_content_keys', { path: PACKAGE }]]);
});
