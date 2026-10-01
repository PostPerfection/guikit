import assert from 'node:assert/strict';
import { register } from 'node:module';
import test from 'node:test';

register('./tauri-plugins-hooks.mjs', import.meta.url);

const tauri = await import('./tauri-plugins-stub.mjs');
const { documentsOrHomeDir } = await import('../src/folders.js');

test('the documents folder is the default', async () => {
  tauri.folders.documents = `${tauri.HOME_FOLDER}/Documents`;
  assert.equal(await documentsOrHomeDir(), `${tauri.HOME_FOLDER}/Documents`);
});

test('an account without a documents folder gets its home folder', async () => {
  tauri.folders.documents = null;
  assert.equal(await documentsOrHomeDir(), tauri.HOME_FOLDER);
});
