import assert from 'node:assert/strict';
import { register } from 'node:module';
import test from 'node:test';

register('./tauri-core-hooks.mjs', import.meta.url);

const bridge = await import('./tauri-core-stub.mjs');
const preview = await import('../src/preview.js');

const FLAT_IN_SCOPE = { x: 262, y: 0, width: 1524, height: 858 };

function sentContentOutline() {
  const sent = bridge.invocations.filter(([name]) => name === 'preview_set_overlays');
  return sent.at(-1)[1].overlays.content_outline;
}

test('the content rectangle reaches the backend with the overlays, and null takes it off', () => {
  bridge.forgetInvocations();
  preview.setPreviewContentOutline(FLAT_IN_SCOPE);
  assert.deepEqual(sentContentOutline(), FLAT_IN_SCOPE);
  preview.setPreviewContentOutline(null);
  assert.equal(sentContentOutline(), null);
});
