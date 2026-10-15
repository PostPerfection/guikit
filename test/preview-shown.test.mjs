import assert from 'node:assert/strict';
import { register } from 'node:module';
import test, { mock } from 'node:test';

register('./tauri-core-hooks.mjs', import.meta.url);

const POLL_MILLISECONDS = 250;
const PACKAGE = '/films/Film_FTR';

mock.timers.enable({ apis: ['setInterval'] });
const bridge = await import('./tauri-core-stub.mjs');
const preview = await import('../src/preview.js');

globalThis.document = { getElementById: () => null };

const shown = [];
preview.watchPreviewShown((path, ended) => shown.push([path, ended]));

async function poll(metadata) {
  bridge.answerWith('preview_get_metadata', JSON.stringify(metadata));
  mock.timers.tick(POLL_MILLISECONDS);
  await new Promise((resolve) => setImmediate(resolve));
}

test('the shown path stays loaded at the end and through a seek back', async () => {
  preview.previewDcp(PACKAGE);
  assert.deepEqual(shown, [[PACKAGE, false]]);

  await poll({ eof: true, position: 10, duration: 10 });
  assert.deepEqual(shown.at(-1), [PACKAGE, true]);

  await poll({ eof: false, position: 5, duration: 10 });
  assert.deepEqual(shown.at(-1), [PACKAGE, false]);

  await poll({ eof: true, position: 10, duration: 10 });
  await poll({ eof: true, position: 10, duration: 10 });
  assert.deepEqual(shown, [
    [PACKAGE, false],
    [PACKAGE, true],
    [PACKAGE, false],
    [PACKAGE, true],
  ]);

  preview.stopPreview();
  await poll({ eof: null, position: null, duration: null });
  preview.stopScrubberPolling();
  assert.deepEqual(shown.slice(4), [[null, false]], 'a stop at the end stays stopped');
});
