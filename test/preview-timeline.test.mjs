import assert from 'node:assert/strict';
import { register } from 'node:module';
import test, { mock } from 'node:test';

register('./tauri-core-hooks.mjs', import.meta.url);

const POLL_MILLISECONDS = 250;
const TIMELINE = { version: 1, clips: [{ picture: '/media/first.mov', in_frame: 0, out_frame: 48 }] };
// opaque to guikit, the app's command reads it
const REQUEST = { container: 'flat' };
const COMPOSITION_NAME = 'Feature';
const RENDER_PROGRESS = 'timeline-render-progress';
const TIMELINE_SECONDS = 3.25;
// a grok timeline's poll reports the duration of the clip it plays
const PLAYING_CLIP_SECONDS = 2;

mock.timers.enable({ apis: ['setInterval'] });
const bridge = await import('./tauri-core-stub.mjs');
const events = await import('./tauri-event-stub.mjs');
const preview = await import('../src/preview.js');

const elements = new Map(['status-text', 'timeline-duration', 'timeline-position', 'preview-title'].map((id) => [id, { textContent: '', title: '', dataset: {} }]));
globalThis.document = { getElementById: (id) => elements.get(id) ?? null };

const positions = [];
preview.watchPreviewTimelinePosition((position) => positions.push(position));

function settled() {
  return new Promise((resolve) => setImmediate(resolve));
}

async function poll(metadata) {
  bridge.answerWith('preview_get_metadata', JSON.stringify(metadata));
  mock.timers.tick(POLL_MILLISECONDS);
  await new Promise((resolve) => setImmediate(resolve));
}

test('a timeline load hands the timeline and the app request over, resolves to its picture only flag and names the composition', async () => {
  bridge.forgetInvocations();
  bridge.answerWith('preview_load_timeline', { picture_only: true });
  bridge.answerWith('preview_get_duration', TIMELINE_SECONDS);
  assert.deepEqual(await preview.previewTimeline(TIMELINE, REQUEST, COMPOSITION_NAME), { pictureOnly: true });
  assert.deepEqual(bridge.invocations[0], ['preview_load_timeline', { timeline: TIMELINE, request: REQUEST }]);
  assert.equal(elements.get('preview-title').textContent, 'Preview: Feature timeline');
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
  assert.equal(await preview.previewTimeline(TIMELINE, REQUEST, COMPOSITION_NAME), null);
  assert.equal(elements.get('status-text').textContent, 'Preview failed: clip 2 plays on mpv and clip 1 on grok');
  bridge.forgetRefusals();
});

test('the status line counts the rendered frames and the time left, and clears once the timeline plays', async () => {
  const status = elements.get('status-text');
  const load = bridge.deferAnswer('preview_load_timeline');
  const loaded = preview.previewTimeline(TIMELINE, REQUEST, COMPOSITION_NAME);
  await settled();
  events.emit(RENDER_PROGRESS, { frames_rendered: 0, total_frames: 48, seconds_left: null });
  assert.equal(status.textContent, 'Rendering layers: 0 of 48 frames');
  events.emit(RENDER_PROGRESS, { frames_rendered: 12, total_frames: 48, seconds_left: 2.4 });
  assert.equal(status.textContent, 'Rendering layers: 12 of 48 frames, about 3 s left');
  assert.equal(status.title, status.textContent);

  load.resolve({ picture_only: false });
  assert.deepEqual(await loaded, { pictureOnly: false });
  assert.equal(status.textContent, '');
  assert.equal(events.listenerCount(RENDER_PROGRESS), 0);
});

test('a load a newer one replaced resolves null and leaves the status line to the newer one', async () => {
  const status = elements.get('status-text');
  const replaced = bridge.deferAnswer('preview_load_timeline');
  const replacedLoad = preview.previewTimeline(TIMELINE, REQUEST, COMPOSITION_NAME);
  await settled();
  const newer = bridge.deferAnswer('preview_load_timeline');
  const newerLoad = preview.previewTimeline(TIMELINE, REQUEST, COMPOSITION_NAME);
  await settled();
  events.emit(RENDER_PROGRESS, { frames_rendered: 1, total_frames: 2, seconds_left: 1 });

  replaced.resolve({ picture_only: false });
  assert.equal(await replacedLoad, null);
  assert.equal(status.textContent, 'Rendering layers: 1 of 2 frames, about 1 s left');
  newer.resolve({ picture_only: false });
  assert.deepEqual(await newerLoad, { pictureOnly: false });
});

for (const [ending, end, command] of [
  ['stopPreview', () => preview.stopPreview(), 'preview_stop'],
  ['cancelTimelineRender', () => preview.cancelTimelineRender(), 'preview_cancel_timeline_render'],
]) {
  test(`a render ${ending} cancels resolves null without a failure and clears its progress`, async () => {
    const status = elements.get('status-text');
    status.textContent = '';
    const load = bridge.deferAnswer('preview_load_timeline');
    const loaded = preview.previewTimeline(TIMELINE, REQUEST, COMPOSITION_NAME);
    await settled();
    events.emit(RENDER_PROGRESS, { frames_rendered: 3, total_frames: 48, seconds_left: 9 });
    bridge.forgetInvocations();

    end();
    assert.deepEqual(bridge.invocations[0], [command, undefined]);
    assert.equal(status.textContent, '');
    events.emit(RENDER_PROGRESS, { frames_rendered: 4, total_frames: 48, seconds_left: 8 });
    assert.equal(status.textContent, '');
    load.reject('the timeline preview load was cancelled');
    assert.equal(await loaded, null);
    assert.equal(status.textContent, '');
  });
}

test('play and pause invoke preview_play_pause and nothing else', async () => {
  bridge.forgetInvocations();
  preview.previewPlayPause();
  await settled();
  assert.deepEqual(bridge.invocations, [['preview_play_pause', undefined]]);
});

test('a load that keeps the transport plays or pauses the new picture the way the old one was', async () => {
  for (const [pausedBefore, pausedAfter, toggled] of [
    [true, false, true],
    [false, true, true],
    [false, false, false],
  ]) {
    bridge.forgetInvocations();
    bridge.answerWith('preview_get_metadata', JSON.stringify({ paused: pausedBefore }));
    await preview.loadKeepingTransport(async () => {
      bridge.answerWith('preview_get_metadata', JSON.stringify({ paused: pausedAfter }));
      return true;
    });
    assert.equal(bridge.invocations.some(([command]) => command === 'preview_play_pause'), toggled);
  }
});

test('a load that keeps the transport and fails leaves play and pause alone', async () => {
  bridge.forgetInvocations();
  bridge.answerWith('preview_get_metadata', JSON.stringify({ paused: true }));
  await preview.loadKeepingTransport(async () => {
    bridge.answerWith('preview_get_metadata', JSON.stringify({ paused: false }));
    return false;
  });
  assert.deepEqual(bridge.invocations.map(([command]) => command), ['preview_get_metadata']);
});
