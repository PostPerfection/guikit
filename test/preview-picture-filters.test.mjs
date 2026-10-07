import assert from 'node:assert/strict';
import { register } from 'node:module';
import test from 'node:test';

register('./tauri-core-hooks.mjs', import.meta.url);

const bridge = await import('./tauri-core-stub.mjs');
const preview = await import('../src/preview.js');

const BUILD_FILTERS = ['crop=w=1998:h=1080:x=0:y=0', 'pad=w=2048:h=1080:x=25:y=0:color=black'];
const SOURCE = '/media/film.mov';

function invocationsOf(command) {
  return bridge.invocations.filter(([name]) => name === command);
}

test('the filters reach the backend as given, and null takes them off', async () => {
  bridge.forgetInvocations();
  await preview.setPreviewPictureFilters(BUILD_FILTERS);
  await preview.setPreviewPictureFilters(null);
  assert.deepEqual(invocationsOf('preview_set_picture_filters'), [
    ['preview_set_picture_filters', { filters: BUILD_FILTERS }],
    ['preview_set_picture_filters', { filters: null }],
  ]);
});

test('the backend is asked whether a source takes filters by its path', async () => {
  bridge.forgetInvocations();
  bridge.answerWith('preview_takes_picture_filters', false);
  assert.equal(await preview.previewTakesPictureFilters(SOURCE), false);
  assert.deepEqual(invocationsOf('preview_takes_picture_filters'), [['preview_takes_picture_filters', { path: SOURCE }]]);
});
