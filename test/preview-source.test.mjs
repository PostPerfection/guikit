import assert from 'node:assert/strict';
import { register } from 'node:module';
import test, { mock } from 'node:test';

register('./tauri-core-hooks.mjs', import.meta.url);

const POLL_MILLISECONDS = 250;
const PACKAGE = '/packages/feature';
const CLIP = 'clip.mp4';

mock.timers.enable({ apis: ['setInterval'] });
const bridge = await import('./tauri-core-stub.mjs');
const preview = await import('../src/preview.js');

globalThis.document = { getElementById: () => null };

const reported = [];
preview.watchPreviewSource((source) => reported.push(source));
preview.enablePreviewTransport();

async function poll(metadata) {
  bridge.answerWith('preview_get_metadata', JSON.stringify(metadata));
  mock.timers.tick(POLL_MILLISECONDS);
  await new Promise((resolve) => setImmediate(resolve));
}

test('each change of what is loaded is reported once', async () => {
  await poll({ source: PACKAGE, filename: 'picture.mxf' });
  await poll({ source: PACKAGE, filename: 'picture.mxf' });
  await poll({ filename: CLIP });
  await poll({ source: null, filename: null });
  assert.deepEqual(reported, [PACKAGE, CLIP, null]);
});
