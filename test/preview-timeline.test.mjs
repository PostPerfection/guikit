import assert from 'node:assert/strict';
import { register } from 'node:module';
import test, { mock } from 'node:test';

register('./tauri-core-hooks.mjs', import.meta.url);

const POLL_MILLISECONDS = 250;
const TIMELINE = { version: 1, clips: [{ picture: '/media/first.mov', in_frame: 0, out_frame: 48 }] };
const TIMELINE_SECONDS = 3.25;
// a grok timeline's poll reports the duration of the clip it plays
const PLAYING_CLIP_SECONDS = 2;

mock.timers.enable({ apis: ['setInterval'] });
const bridge = await import('./tauri-core-stub.mjs');
const preview = await import('../src/preview.js');

const elements = new Map(['status-text', 'timeline-duration', 'timeline-position'].map((id) => [id, { textContent: '', title: '', dataset: {} }]));
globalThis.document = { getElementById: (id) => elements.get(id) ?? null };

const positions = [];
preview.watchPreviewTimelinePosition((position) => positions.push(position));

async function poll(metadata) {
  bridge.answerWith('preview_get_metadata', JSON.stringify(metadata));
  mock.timers.tick(POLL_MILLISECONDS);
  await new Promise((resolve) => setImmediate(resolve));
}

test('a timeline load hands the timeline over and resolves to its picture only flag', async () => {
  bridge.forgetInvocations();
  bridge.answerWith('preview_load_timeline', { picture_only: true });
  bridge.answerWith('preview_get_duration', TIMELINE_SECONDS);
  assert.deepEqual(await preview.previewTimeline(TIMELINE), { pictureOnly: true });
  assert.deepEqual(bridge.invocations[0], ['preview_load_timeline', { timeline: TIMELINE }]);
});

test('the poll reports the timeline position and the scrubber spans the whole timeline', async () => {
  positions.length = 0;
  await poll({ position: 1, duration: PLAYING_CLIP_SECONDS, timeline_position: 3, timeline_clip_index: 1 });
  assert.deepEqual(positions, [{ seconds: 3, clipIndex: 1 }]);
  assert.equal(elements.get('timeline-duration').dataset.raw, String(TIMELINE_SECONDS));

  await poll({ position: 1, duration: PLAYING_CLIP_SECONDS });
  assert.deepEqual(positions.at(-1), null);
  assert.equal(elements.get('timeline-duration').dataset.raw, String(PLAYING_CLIP_SECONDS));
});

test('a refused timeline resolves null and says why', async () => {
  bridge.refuseWith('preview_load_timeline', 'clip 2 plays on mpv and clip 1 on grok');
  assert.equal(await preview.previewTimeline(TIMELINE), null);
  assert.equal(elements.get('status-text').textContent, 'Preview failed: clip 2 plays on mpv and clip 1 on grok');
  bridge.forgetRefusals();
});
