import assert from 'node:assert/strict';
import test from 'node:test';

import {
  addClip,
  baseClipsInOrder,
  clipAtFrame,
  clipSourceFrameAt,
  clipStarts,
  frameAtSeconds,
  layersOf,
  insertClipAt,
  moveClipTo,
  removeClip,
  removeReelStart,
  secondsAtFrame,
  setClipOpacity,
  setClipRange,
  splitReelAt,
  totalFrames,
  withReelStartsInsideTimeline,
} from '../src/timeline-editor.js';

const FPS = [24, 1];
const POINTER_BOUNDS = { left: 0, width: 100 };
const ROW_HEIGHT = 40;
const NO_SOUND = { kind: 'none', asset: null };

const ASSETS = {
  1: { kind: 'picture', name: 'Reel One.mov', frames: 240 },
  2: { kind: 'picture', name: 'Reel Two.mov', frames: 120 },
  3: { kind: 'sound', name: 'Music.wav' },
  4: { kind: 'subtitle', name: 'English.srt' },
  5: { kind: 'picture', name: 'Title Card.png', still: true },
};

function clip(picture, inFrame, outFrame, start, layer = 0) {
  return {
    picture,
    frames: ASSETS[picture].frames,
    start,
    layer,
    opacity: 1,
    inFrame,
    outFrame,
    sound: layer === 0 ? { kind: 'embedded', asset: null } : NO_SOUND,
    subtitle: null,
  };
}

function stillClip(picture, outFrame, start, layer = 0) {
  return { ...clip(picture, 0, outFrame, start, layer), still: true, frames: null, sound: NO_SOUND };
}

// 240 kept frames from the first clip and 80 from the second right after it, 320 in all
function twoClipTimeline() {
  return { fps: FPS, clips: [clip(1, 0, 240, 0), clip(2, 20, 100, 240)], sound: null, subtitle: null, reelStarts: [] };
}

// the picture rows sit one under another in the order the markup lists them, each 40 pixels high
function fakeContainer() {
  const playhead = { style: {} };
  const rowContent = { getBoundingClientRect: () => POINTER_BOUNDS, append() {} };
  return {
    innerHTML: '',
    classList: { add() {} },
    handlers: {},
    playhead,
    addEventListener(name, handler) {
      this.handlers[name] = handler;
    },
    querySelector(selector) {
      assert.equal(selector, '.timeline-editor-playhead');
      return playhead;
    },
    querySelectorAll(selector) {
      assert.equal(selector, '.timeline-editor-track[data-layer]');
      return [...this.innerHTML.matchAll(/data-layer="(\d+)"/g)].map(([, layer], row) => ({
        dataset: { layer },
        getBoundingClientRect: () => ({ top: row * ROW_HEIGHT, bottom: (row + 1) * ROW_HEIGHT }),
        querySelector: () => rowContent,
      }));
    },
    capturedPointers: [],
    setPointerCapture(pointerId) {
      this.capturedPointers.push(pointerId);
    },
  };
}

// closest answers from what the scenario puts under the pointer
function fakeTarget({ action, frame, targetClip, checked, clipIndex, pictureClip, track, layer, trackContent, ruler } = {}) {
  const clipElement = clipIndex !== undefined && {
    dataset: { clipIndex: String(clipIndex) },
    getBoundingClientRect: () => POINTER_BOUNDS,
    style: {},
    classList: { add() {} },
  };
  const elements = {
    '[data-action]': action && { dataset: { action, frame: String(frame), targetClip: String(targetClip) }, checked },
    '[data-clip-index]': clipElement,
    '.timeline-editor-clip': pictureClip && clipElement,
    '[data-track]': track && { dataset: { track, layer: String(layer) } },
    '.timeline-editor-track-content': trackContent && { getBoundingClientRect: () => POINTER_BOUNDS },
    '.timeline-editor-ruler': ruler && { getBoundingClientRect: () => POINTER_BOUNDS },
  };
  return { closest: (selector) => elements[selector] || null };
}

// a press on a picture clip, one move and a release, then the click the browser sends after them
function dragClip(container, clipIndex, from, to) {
  const target = fakeTarget({ clipIndex, pictureClip: true });
  container.handlers.pointerdown({ target, button: 0, pointerId: 1, clientX: from.x, clientY: from.y });
  container.handlers.pointermove({ target, buttons: 1, clientX: to.x, clientY: to.y });
  container.handlers.pointerup({ target, clientX: to.x, clientY: to.y });
  container.handlers.click({ target, clientX: to.x, clientY: to.y });
}

// the rows of a timeline with clips on layer 0 only: layer 1 to drop onto, then the base picture
const LAYER_1_ROW_Y = 10;
const BASE_ROW_Y = 50;

let instances = 0;

async function freshEditor(model, reelWord = 'reel', timelinePlayback = { pictureOnly: false }) {
  const editor = await import(`../src/timeline-editor.js?instance=${++instances}`);
  const container = fakeContainer();
  const changes = [];
  const seeks = [];
  const playedTimelines = [];
  const timelineSeeks = [];
  const transportKeepingLoads = [];
  // what the fake preview shows besides the timeline, which the editor tracks itself
  const preview = { showsClipSource: false };
  editor.initTimelineEditor(container, {
    fps: FPS,
    reelWord,
    assetName: (assetId) => ASSETS[assetId].name,
    assetKind: (assetId) => ASSETS[assetId].kind,
    sourceLength: async (assetId) => (ASSETS[assetId].still ? { kind: 'still' } : { kind: 'counted', frames: ASSETS[assetId].frames }),
    onChange: (changed) => changes.push(changed),
    previewShowsClipSource: () => preview.showsClipSource,
    previewClipSource: async (frame) => {
      seeks.push(frame);
      preview.showsClipSource = true;
      return true;
    },
    playTimeline: async (played) => {
      playedTimelines.push(played);
      if (timelinePlayback !== null) preview.showsClipSource = false;
      return timelinePlayback;
    },
    seekTimeline: (seconds) => timelineSeeks.push(seconds),
    loadKeepingTransport: async (load) => transportKeepingLoads.push(await load()),
  });
  if (model) editor.setTimeline(model);
  return { editor, container, changes, seeks, playedTimelines, timelineSeeks, transportKeepingLoads, preview };
}

function settled() {
  return new Promise((resolve) => setImmediate(resolve));
}

async function setPlayWholeTimeline(container, checked) {
  click(container, { action: 'play-whole-timeline', checked });
  await settled();
}

// a clip's source on screen with the box unticked, as it starts, so a seek goes to that clip's source
function previewingClipSources({ preview }) {
  preview.showsClipSource = true;
}

function click(container, target, clientX = 0) {
  container.handlers.click({ target: fakeTarget(target), clientX });
}

function clickAction(container, action, details = {}) {
  click(container, { action, ...details });
}

async function drop(container, assetId, target, clientX = 0) {
  await container.handlers.drop({
    target: fakeTarget(target),
    clientX,
    dataTransfer: { getData: (format) => (format === 'text/plain' ? String(assetId) : '') },
    preventDefault() {},
  });
}

function typeInField(container, field, value) {
  container.handlers.change({ target: { dataset: { field }, value } });
}

// the second clip 40 frames after the first, and a layer 1 clip over the gap between them
function gappedTimeline() {
  return {
    fps: FPS,
    clips: [clip(2, 20, 100, 120), clip(1, 0, 80, 0), clip(2, 0, 60, 60, 1)],
    sound: null,
    subtitle: null,
    reelStarts: [],
  };
}

test('total frames is the furthest clip end and clip starts are the starts', () => {
  assert.equal(totalFrames(twoClipTimeline()), 320);
  assert.deepEqual(clipStarts(twoClipTimeline()), [0, 240]);
  assert.equal(totalFrames(gappedTimeline()), 200);
  assert.deepEqual(clipStarts(gappedTimeline()), [120, 0, 60]);
  assert.equal(totalFrames({ ...gappedTimeline(), clips: [] }), 0);
});

test('layersOf lists the layers holding a clip and layer 0 always, and the base clips run in start order', () => {
  assert.deepEqual(layersOf(gappedTimeline()), [0, 1]);
  assert.deepEqual(layersOf({ ...gappedTimeline(), clips: [clip(1, 0, 10, 0, 3)] }), [0, 3]);
  assert.deepEqual(
    baseClipsInOrder(gappedTimeline()).map(({ index, clip: base }) => [index, base.start]),
    [
      [1, 0],
      [0, 120],
    ],
  );
});

test('clipAtFrame finds the layer 0 clip and the offset into it', () => {
  const model = twoClipTimeline();
  assert.deepEqual(clipAtFrame(model, 0), { index: 0, frameInClip: 0 });
  assert.deepEqual(clipAtFrame(model, 239), { index: 0, frameInClip: 239 });
  assert.deepEqual(clipAtFrame(model, 240), { index: 1, frameInClip: 0 });
  assert.deepEqual(clipAtFrame(model, 319), { index: 1, frameInClip: 79 });
  assert.equal(clipAtFrame(model, 320), null);
  assert.equal(clipAtFrame(model, -1), null);
});

test('clipAtFrame is null in a gap of layer 0, whatever is above it', () => {
  const model = gappedTimeline();
  assert.deepEqual(clipAtFrame(model, 79), { index: 1, frameInClip: 79 });
  assert.equal(clipAtFrame(model, 80), null);
  assert.equal(clipAtFrame(model, 119), null);
  assert.deepEqual(clipSourceFrameAt(model, 120), { index: 0, sourceFrame: 20 });
});

test('addClip appends, refuses an overlap on its layer and leaves the model it was given alone', () => {
  const model = twoClipTimeline();
  assert.deepEqual(addClip(model, clip(2, 0, 120, 320)).clips[2], clip(2, 0, 120, 320));
  assert.equal(addClip(model, clip(2, 0, 120, 100, 1)).clips.length, 3);
  assert.equal(model.clips.length, 2);
  assert.throws(() => addClip(model, clip(2, 0, 120, 300)), {
    name: 'RangeError',
    message: /frames 300 to 420 on layer 0 overlap clip 1, which runs from 240 to 320/,
  });
});

test('moveClipTo moves a clip along its layer and onto another, and refuses an overlap', () => {
  const model = gappedTimeline();
  assert.equal(moveClipTo(model, 0, 90, 0).clips[0].start, 90);
  assert.equal(totalFrames(moveClipTo(model, 0, 80, 0)), 160);
  assert.throws(() => moveClipTo(model, 0, 70, 0), /frames 70 to 150 on layer 0 overlap clip 1, which runs from 0 to 80/);
  assert.throws(() => moveClipTo(model, 1, 100, 1), /overlap clip 2/);
  assert.throws(() => moveClipTo(model, 1, -1, 0), RangeError);
  assert.throws(() => moveClipTo(model, 3, 0, 0), /no clip 3/);
  assert.equal(model.clips[0].start, 120);
});

test('a clip moved above layer 0 loses its sound, and one moved back down takes its embedded sound at full opacity', () => {
  const model = { ...gappedTimeline(), clips: [{ ...clip(1, 0, 80, 0), sound: { kind: 'file', asset: 3 } }] };
  const raised = moveClipTo(model, 0, 0, 2);
  assert.deepEqual(raised.clips[0].sound, NO_SOUND);
  const faded = setClipOpacity(raised, 0, 0.25);
  assert.equal(moveClipTo(faded, 0, 0, 1).clips[0].opacity, 0.25);
  const lowered = moveClipTo(faded, 0, 0, 0);
  assert.deepEqual(lowered.clips[0].sound, { kind: 'embedded', asset: null });
  assert.equal(lowered.clips[0].opacity, 1);
});

test('a still moved down to layer 0 stays silent, since an image carries no sound', () => {
  const model = { ...gappedTimeline(), clips: [stillClip(1, 120, 0, 1)] };
  assert.deepEqual(moveClipTo(model, 0, 0, 0).clips[0].sound, NO_SOUND);
});

test('setClipOpacity refuses an opacity outside 0 to 1', () => {
  assert.equal(setClipOpacity(gappedTimeline(), 2, 0).clips[2].opacity, 0);
  assert.throws(() => setClipOpacity(gappedTimeline(), 2, 1.5), /clip 2: opacity 1.5 is outside 0 to 1/);
  assert.throws(() => setClipOpacity(gappedTimeline(), 2, Number.NaN), RangeError);
});

test('setClipRange refuses an out point that runs the clip into the next one on its layer', () => {
  assert.throws(() => setClipRange(gappedTimeline(), 1, 0, 121), /frames 0 to 121 on layer 0 overlap clip 0/);
  assert.equal(setClipRange(gappedTimeline(), 1, 0, 120).clips[1].outFrame, 120);
});

test('removeClip takes the clip out and refuses one that is not there', () => {
  const model = twoClipTimeline();
  assert.deepEqual(
    removeClip(model, 0).clips.map((each) => each.picture),
    [2],
  );
  assert.throws(() => removeClip(model, 2), /no clip 2/);
});

test('setClipRange sets the in and out points of that clip only', () => {
  const changed = setClipRange(twoClipTimeline(), 1, 10, 110);
  assert.equal(changed.clips[1].inFrame, 10);
  assert.equal(changed.clips[1].outFrame, 110);
  assert.equal(changed.clips[0].outFrame, 240);
});

test('setClipRange refuses an in point at or past the out point', () => {
  const model = twoClipTimeline();
  assert.throws(() => setClipRange(model, 1, 50, 50), { name: 'RangeError', message: /clip 1: in point 50 is at or past out point 50/ });
  assert.throws(() => setClipRange(model, 1, 60, 50), /clip 1: in point 60 is at or past out point 50/);
});

test('setClipRange refuses an out point past the source and an in point before it', () => {
  const model = twoClipTimeline();
  assert.throws(() => setClipRange(model, 1, 0, 121), /clip 1: out point 121 is past the source's 120 frames/);
  assert.throws(() => setClipRange(model, 1, -1, 50), /clip 1: in point -1 is before the first frame/);
  assert.equal(setClipRange(model, 1, 0, 120).clips[1].outFrame, 120);
});

test('trimming the last clip drops the reel starts now past the end and keeps the ones inside', () => {
  const model = { ...twoClipTimeline(), reelStarts: [100, 290, 310] };
  const trimmed = setClipRange(model, 1, 20, 60);
  assert.equal(totalFrames(trimmed), 280);
  assert.deepEqual(trimmed.reelStarts, [100]);
  assert.deepEqual(model.reelStarts, [100, 290, 310]);
});

test('removing a clip drops the reel starts now past the end and leaves a gap where it was', () => {
  const model = { ...twoClipTimeline(), reelStarts: [100, 250] };
  assert.deepEqual(removeClip(model, 1).reelStarts, [100]);
  assert.deepEqual(removeClip(model, 0).reelStarts, [100, 250]);
  assert.equal(clipAtFrame(removeClip(model, 0), 100), null);
});

test('a reel start exactly at the total is dropped, one a frame before it stays', () => {
  const model = { ...twoClipTimeline(), reelStarts: [319, 320] };
  assert.deepEqual(withReelStartsInsideTimeline(model).reelStarts, [319]);
});

test('a loaded timeline keeps reel starts past its end for clips not probed yet', async () => {
  const unprobed = { ...twoClipTimeline(), clips: [{ ...clip(1, 0, 0, 0), frames: 0 }], reelStarts: [100] };
  const { editor } = await freshEditor(unprobed);
  assert.deepEqual(editor.getTimeline().reelStarts, [100]);
});

test('splitReelAt keeps the reel starts sorted and ignores 0 and duplicates', () => {
  let model = twoClipTimeline();
  model = splitReelAt(model, 240);
  model = splitReelAt(model, 100);
  model = splitReelAt(model, 0);
  model = splitReelAt(model, 100);
  assert.deepEqual(model.reelStarts, [100, 240]);
  assert.deepEqual(removeReelStart(model, 100).reelStarts, [240]);
});

test('the markup labels the clips and their sound and rules the timeline in timecode', async () => {
  const { container } = await freshEditor(twoClipTimeline());
  assert.match(container.innerHTML, /Reel One\.mov/);
  assert.match(container.innerHTML, /Reel Two\.mov/);
  assert.match(container.innerHTML, />embedded</);
  // 320 frames at 24 a second ticks every 5 seconds
  for (const timecode of ['00:00:00', '00:00:05', '00:00:10']) assert.match(container.innerHTML, new RegExp(`>${timecode}<`));
  assert.doesNotMatch(container.innerHTML, />00:00:15</);
});

test('the markup draws one picture row per layer, the top one first, under an empty row to drop onto', async () => {
  const { container } = await freshEditor(gappedTimeline());
  const rows = [...container.innerHTML.matchAll(/data-layer="(\d+)">\s*<div class="timeline-editor-track-label">([^<]+)</g)];
  assert.deepEqual(
    rows.map(([, layer, label]) => [layer, label]),
    [
      ['2', 'Layer 2'],
      ['1', 'Layer 1'],
      ['0', 'Picture'],
    ],
  );
  // the layer 1 clip starts at frame 60 of 200
  assert.match(container.innerHTML, /data-clip-index="2" style="left: 30%; width: 30%"/);
});

test('the sound row has a span per layer 0 clip and the subtitle row one per clip, the upper ones in the lower half', async () => {
  const { container } = await freshEditor(gappedTimeline());
  const [, sound] = container.innerHTML.match(/data-track="sound">([\s\S]*?)data-track="subtitle"/);
  assert.deepEqual([...sound.matchAll(/data-clip-index="(\d)"/g)].map(([, index]) => index), ['0', '1']);
  assert.match(container.innerHTML, /data-track="subtitle" data-stacked/);
  assert.match(container.innerHTML, /timeline-editor-span-upper" data-clip-index="2"/);
});

test('a picture dropped on a clip lands after its end with the probed frame count', async () => {
  const { editor, container, changes } = await freshEditor(twoClipTimeline());
  await drop(container, 2, { track: 'picture', layer: 0, clipIndex: 1 });
  assert.deepEqual(changes.at(-1).clips[2], clip(2, 0, 120, 320));
  assert.deepEqual(editor.getTimeline(), changes.at(-1));
});

test('a picture dropped on empty track lands at the pointer frame on that row', async () => {
  const { container, changes } = await freshEditor(twoClipTimeline());
  // a quarter of the way along 320 frames
  await drop(container, 2, { track: 'picture', layer: 1, trackContent: true }, 25);
  assert.deepEqual(changes.at(-1).clips[2], clip(2, 0, 120, 80, 1));
});

test('a picture dropped where it would overlap a clip on its row changes nothing', async () => {
  const { container, changes } = await freshEditor(twoClipTimeline());
  await drop(container, 2, { track: 'picture', layer: 0, clipIndex: 0 });
  await drop(container, 2, { track: 'picture', layer: 0, trackContent: true }, 50);
  assert.deepEqual(changes, []);
});

test('an asset dropped on the wrong track changes nothing', async () => {
  const { container, changes } = await freshEditor(twoClipTimeline());
  await drop(container, 3, { track: 'picture', layer: 0, clipIndex: 0 });
  await drop(container, 1, { track: 'sound' });
  assert.deepEqual(changes, []);
});

test('a sound dropped over a clip is that clip sound, on the empty track the whole timeline', async () => {
  const { container, changes } = await freshEditor(twoClipTimeline());
  await drop(container, 3, { track: 'sound', clipIndex: 1 });
  assert.deepEqual(changes.at(-1).clips[1].sound, { kind: 'file', asset: 3 });
  assert.deepEqual(changes.at(-1).clips[0].sound, { kind: 'embedded', asset: null });
  assert.equal(changes.at(-1).sound, null);

  await drop(container, 3, { track: 'sound' });
  assert.equal(changes.at(-1).sound, 3);
  assert.deepEqual(changes.at(-1).clips[0].sound, { kind: 'embedded', asset: null });
  assert.match(container.innerHTML, /timeline-editor-span-whole/);
});

test('a subtitle dropped over a clip is that clip subtitle, on the empty track the whole timeline', async () => {
  const { container, changes } = await freshEditor(twoClipTimeline());
  await drop(container, 4, { track: 'subtitle', clipIndex: 0 });
  assert.equal(changes.at(-1).clips[0].subtitle, 4);
  assert.equal(changes.at(-1).subtitle, null);
  await drop(container, 4, { track: 'subtitle' });
  assert.equal(changes.at(-1).subtitle, 4);
});

test('Set in from playhead trims the selected clip at the playhead', async () => {
  const harness = await freshEditor(twoClipTimeline());
  const { editor, container, changes, seeks } = harness;
  previewingClipSources(harness);
  // the left edge of the second clip is timeline frame 240
  click(container, { clipIndex: 1 }, 0);
  assert.deepEqual(seeks, [240]);
  assert.match(container.innerHTML, /data-field="in" value="00:00:00:20"/);

  editor.setPlayhead(250);
  clickAction(container, 'set-in');
  assert.equal(changes.at(-1).clips[1].inFrame, 30);
  assert.equal(changes.at(-1).clips[1].outFrame, 100);
  assert.deepEqual(changes.at(-1).clips[0], clip(1, 0, 240, 0));
});

test('a range typed into a field is applied, an invalid one leaves the clip as it was', async () => {
  const { container, changes } = await freshEditor(twoClipTimeline());
  click(container, { clipIndex: 1 });

  typeInField(container, 'out', '00:00:04:12');
  assert.equal(changes.at(-1).clips[1].outFrame, 108);

  // 6 seconds is 144 frames, past the source's 120
  typeInField(container, 'out', '00:00:06:00');
  typeInField(container, 'in', 'not a timecode');
  assert.equal(changes.length, 1);
  assert.match(container.innerHTML, /data-field="out" value="00:00:04:12"/);
  assert.match(container.innerHTML, /data-field="in" value="00:00:00:20"/);
});

test('Remove takes the selected clip out', async () => {
  const { container, changes } = await freshEditor(twoClipTimeline());
  click(container, { clipIndex: 0 });
  clickAction(container, 'remove-clip');
  assert.deepEqual(changes.at(-1).clips, [clip(2, 20, 100, 240)]);
});

test('a Start typed into the field moves the clip, one that overlaps leaves it where it was', async () => {
  const { container, changes } = await freshEditor(twoClipTimeline());
  click(container, { clipIndex: 1 });
  assert.match(container.innerHTML, /data-field="start" value="00:00:10:00"/);

  typeInField(container, 'start', '00:00:12:00');
  assert.equal(changes.at(-1).clips[1].start, 288);

  typeInField(container, 'start', '00:00:09:00');
  assert.equal(changes.length, 1);
  assert.match(container.innerHTML, /data-field="start" value="00:00:12:00"/);
});

test('the opacity field shows for a clip above layer 0 only and sets its opacity in percent', async () => {
  const { container, changes } = await freshEditor(gappedTimeline());
  click(container, { clipIndex: 1 });
  assert.doesNotMatch(container.innerHTML, /data-field="opacity"/);

  click(container, { clipIndex: 2 });
  assert.match(container.innerHTML, /data-field="opacity" value="100"/);
  typeInField(container, 'opacity', '40');
  assert.equal(changes.at(-1).clips[2].opacity, 0.4);
  typeInField(container, 'opacity', '140');
  typeInField(container, 'opacity', '');
  assert.equal(changes.length, 1);
  assert.match(container.innerHTML, /data-field="opacity" value="40"/);
});

test('split reel here does nothing with the playhead at the end of the timeline', async () => {
  const { editor, container, changes } = await freshEditor(twoClipTimeline());
  editor.setPlayhead(320);
  click(container, { action: 'split-reel' });
  assert.deepEqual(changes, []);
  editor.setPlayhead(240);
  click(container, { action: 'split-reel' });
  assert.deepEqual(changes.at(-1).reelStarts, [240]);
});

test('playing from the playhead loads the whole timeline and seeks it to the playhead', async () => {
  const { editor, playedTimelines, timelineSeeks } = await freshEditor(twoClipTimeline());
  editor.setPlayhead(240);
  assert.equal(await editor.playTimelineFromPlayhead(), true);
  assert.equal(playedTimelines.length, 1);
  assert.deepEqual(timelineSeeks, [240 / 24]);
});

test('dragging a clip along its row moves its start and leaves the click after it alone', async () => {
  const { container, changes, seeks } = await freshEditor(twoClipTimeline());
  // 320 frames over 100 pixels, so 25 pixels is 80 frames
  dragClip(container, 1, { x: 80, y: BASE_ROW_Y }, { x: 105, y: BASE_ROW_Y });
  assert.equal(changes.at(-1).clips[1].start, 320);
  assert.deepEqual(seeks, []);
  assert.match(container.innerHTML, /timeline-editor-clip selected" data-clip-index="1"/);
});

test('a dragged clip snaps its start or its end to a clip edge or the playhead within a few pixels', async () => {
  const { editor, container, changes } = await freshEditor(gappedTimeline());
  // rows for layers 2, 1 and 0, and 200 frames over 100 pixels
  const layer1RowY = 60;
  editor.setPlayhead(30);
  // 14 pixels left of its start at 60 is frame 32, 2 frames past the playhead
  dragClip(container, 2, { x: 30, y: layer1RowY }, { x: 16, y: layer1RowY });
  assert.equal(changes.at(-1).clips[2].start, 30);

  // 53 pixels right is frame 136, which puts its end 4 frames short of the timeline's end at 200
  dragClip(container, 2, { x: 15, y: layer1RowY }, { x: 68, y: layer1RowY });
  assert.equal(changes.at(-1).clips[2].start, 140);
});

test('dragging a clip onto the row above puts it on that layer without sound', async () => {
  const { container, changes } = await freshEditor(twoClipTimeline());
  dragClip(container, 0, { x: 10, y: BASE_ROW_Y }, { x: 10, y: LAYER_1_ROW_Y });
  assert.deepEqual(changes.at(-1).clips[0], clip(1, 0, 240, 0, 1));
  assert.match(container.innerHTML, /data-layer="2"/);
});

test('dragging the last clip onto the start of the first puts it first and the first after it', async () => {
  const { container, changes } = await freshEditor(twoClipTimeline());
  // 320 frames over 100 pixels, so 75 pixels left takes the start at 240 to 0
  dragClip(container, 1, { x: 80, y: BASE_ROW_Y }, { x: 5, y: BASE_ROW_Y });
  assert.deepEqual(clipStarts(changes.at(-1)), [80, 0]);
  assert.equal(totalFrames(changes.at(-1)), 320);
});

test('a clip dropped on the second half of another goes in after it and its old place closes up', () => {
  const model = { ...twoClipTimeline(), clips: [clip(1, 0, 80, 0), clip(2, 0, 60, 80), clip(2, 0, 40, 140)] };
  assert.deepEqual(clipStarts(insertClipAt(model, 0, 170, 0)), [100, 0, 60]);
});

test('a clip dropped in a gap pushes the next clip on its row just far enough and leaves other rows alone', () => {
  assert.deepEqual(clipStarts(insertClipAt(gappedTimeline(), 1, 100, 0)), [180, 100, 60]);
});

// a pointer captured on the press sends the browser's click to the panel instead of the clip
test('a press and release without a move selects the clip and seeks without capturing the pointer', async () => {
  const harness = await freshEditor(twoClipTimeline());
  const { container, changes, seeks } = harness;
  previewingClipSources(harness);
  dragClip(container, 1, { x: 0, y: BASE_ROW_Y }, { x: 1, y: BASE_ROW_Y });
  assert.deepEqual(changes, []);
  assert.deepEqual(seeks, [240]);
  assert.deepEqual(container.capturedPointers, []);
});

test('a drag captures the pointer once it moves', async () => {
  const { container } = await freshEditor(twoClipTimeline());
  dragClip(container, 1, { x: 80, y: BASE_ROW_Y }, { x: 105, y: BASE_ROW_Y });
  assert.deepEqual(container.capturedPointers, [1]);
});

test('Split reel here adds a reel start at the playhead and clicking its marker removes it', async () => {
  const { editor, container, changes } = await freshEditor(twoClipTimeline());
  editor.setPlayhead(100);
  clickAction(container, 'split-reel');
  assert.deepEqual(changes.at(-1).reelStarts, [100]);
  assert.match(container.innerHTML, /timeline-editor-reel-marker" data-action="remove-reel-start" data-frame="100"/);

  clickAction(container, 'remove-reel-start', { frame: 100 });
  assert.deepEqual(changes.at(-1).reelStarts, []);
  assert.doesNotMatch(container.innerHTML, /timeline-editor-reel-marker/);
});

test('an app that splits into segments says segment on the button and the markers', async () => {
  const { container } = await freshEditor({ ...twoClipTimeline(), reelStarts: [100] }, 'segment');
  assert.match(container.innerHTML, />Split segment here</);
  assert.match(container.innerHTML, /title="Segment starts at 00:00:04:04, click to remove"/);
  assert.doesNotMatch(container.innerHTML, /[Rr]eel starts|Split reel/);
});

test('reel starts from the app are sorted and reported like any edit', async () => {
  const { editor, changes } = await freshEditor(twoClipTimeline());
  editor.setReelStarts([240, 0, 100, 100]);
  assert.deepEqual(changes.at(-1).reelStarts, [100, 240]);
});

test('a ruler click seeks to the frame under the pointer and moves the playhead', async () => {
  const harness = await freshEditor(twoClipTimeline());
  const { container, seeks } = harness;
  previewingClipSources(harness);
  click(container, { ruler: true }, 50);
  assert.deepEqual(seeks, [160]);
  assert.match(container.playhead.style.left, /\* 0\.5\)$/);
});

test('clipSourceFrameAt finds the clip under a frame and the source frame it shows there', () => {
  const model = twoClipTimeline();
  assert.deepEqual(clipSourceFrameAt(model, 0), { index: 0, sourceFrame: 0 });
  // the second clip keeps its source from frame 20
  assert.deepEqual(clipSourceFrameAt(model, 240), { index: 1, sourceFrame: 20 });
  assert.deepEqual(clipSourceFrameAt(model, 250), { index: 1, sourceFrame: 30 });
  assert.equal(clipSourceFrameAt(model, 320), null);
});

test('seconds and timeline frames convert at the timeline rate', () => {
  const ntsc = { ...twoClipTimeline(), fps: [24000, 1001] };
  assert.equal(frameAtSeconds(twoClipTimeline(), 10), 240);
  assert.equal(secondsAtFrame(twoClipTimeline(), 240), 10);
  assert.equal(frameAtSeconds(ntsc, secondsAtFrame(ntsc, 250)), 250);
  // a position a hair short of a frame's start still reads as that frame
  assert.equal(frameAtSeconds(twoClipTimeline(), 9.99999), 240);
});

test('the timeline load hands the whole timeline over and a picture only answer shows the hint', async () => {
  const { editor, container, playedTimelines } = await freshEditor(twoClipTimeline(), 'reel', { pictureOnly: true });
  assert.doesNotMatch(container.innerHTML, /previews picture only/);

  await editor.playTimelineFromPlayhead();
  assert.deepEqual(playedTimelines, [twoClipTimeline()]);
  assert.match(container.innerHTML, /The JPEG 2000 timeline previews picture only/);
});

test('the Play whole timeline box is unticked from the start, and disabled while the timeline has no clips', async () => {
  const empty = await freshEditor();
  assert.match(empty.container.innerHTML, /data-action="play-whole-timeline" disabled> Play whole timeline</);
  assert.equal(empty.editor.playsWholeTimeline(), false);

  const { editor, container } = await freshEditor(twoClipTimeline());
  assert.match(container.innerHTML, /data-action="play-whole-timeline"> Play whole timeline</);
  await setPlayWholeTimeline(container, true);
  assert.match(container.innerHTML, /data-action="play-whole-timeline" checked> Play whole timeline</);
  assert.equal(editor.playsWholeTimeline(), true);
  assert.doesNotMatch(container.innerHTML, /Play timeline</);
});

test('with the box ticked a seek over a clip source loads the whole timeline at the playhead, then seeks it', async () => {
  const { container, preview, seeks, playedTimelines, timelineSeeks } = await freshEditor(twoClipTimeline());
  await setPlayWholeTimeline(container, true);
  preview.showsClipSource = true;
  click(container, { ruler: true }, 50);
  await settled();
  assert.deepEqual(playedTimelines, [twoClipTimeline()]);
  assert.deepEqual(timelineSeeks, [160 / 24]);

  click(container, { ruler: true }, 25);
  assert.equal(playedTimelines.length, 1);
  assert.deepEqual(timelineSeeks, [160 / 24, 80 / 24]);
  assert.deepEqual(seeks, []);
});

test('with the box unticked a seek goes to the clip source under the playhead', async () => {
  const harness = await freshEditor(twoClipTimeline());
  const { editor, container, seeks, playedTimelines, timelineSeeks } = harness;
  previewingClipSources(harness);
  assert.equal(editor.playsWholeTimeline(), false);
  assert.match(container.innerHTML, /data-action="play-whole-timeline"> Play whole timeline</);
  click(container, { ruler: true }, 50);
  assert.deepEqual(seeks, [160]);
  assert.deepEqual(playedTimelines, []);
  assert.deepEqual(timelineSeeks, []);
});

test('a seek while the preview shows neither the timeline nor a clip source only moves the playhead', async () => {
  for (const checked of [true, false]) {
    const { container, seeks, playedTimelines, timelineSeeks } = await freshEditor(twoClipTimeline());
    await setPlayWholeTimeline(container, checked);
    click(container, { ruler: true }, 50);
    await settled();
    assert.match(container.playhead.style.left, /\* 0\.5\)$/);
    assert.deepEqual([seeks, playedTimelines, timelineSeeks], [[], [], []]);
  }
});

test('the box swaps a clip source for the timeline and back at the playhead, keeping the transport', async () => {
  const harness = await freshEditor(twoClipTimeline());
  const { editor, container, seeks, playedTimelines, timelineSeeks, transportKeepingLoads } = harness;
  previewingClipSources(harness);
  assert.deepEqual(transportKeepingLoads, []);
  editor.setPlayhead(100);

  await setPlayWholeTimeline(container, true);
  assert.deepEqual(playedTimelines, [twoClipTimeline()]);
  assert.deepEqual(timelineSeeks, [100 / 24]);
  assert.deepEqual(transportKeepingLoads, [true]);

  editor.followTimelinePosition({ seconds: 250 / 24, clipIndex: 1 });
  await setPlayWholeTimeline(container, false);
  assert.deepEqual(seeks, [250]);
  assert.deepEqual(transportKeepingLoads, [true, true]);
});

test('the box loads nothing while the preview shows something other than the timeline or a clip source', async () => {
  const { container, seeks, playedTimelines, transportKeepingLoads } = await freshEditor(twoClipTimeline());
  await setPlayWholeTimeline(container, true);
  await setPlayWholeTimeline(container, false);
  assert.deepEqual([seeks, playedTimelines, transportKeepingLoads], [[], [], []]);
});

test('the playhead follows the loaded timeline and lets go once the preview plays something else', async () => {
  const { editor, container, seeks, timelineSeeks } = await freshEditor(twoClipTimeline(), 'reel', { pictureOnly: true });
  await setPlayWholeTimeline(container, true);
  await editor.playTimelineFromPlayhead();
  timelineSeeks.length = 0;

  editor.followTimelinePosition({ seconds: 10, clipIndex: 1 });
  assert.match(container.playhead.style.left, /\* 0\.75\)$/);
  click(container, { ruler: true }, 50);
  assert.deepEqual(timelineSeeks, [160 / 24]);
  assert.deepEqual(seeks, []);

  editor.followTimelinePosition(null);
  assert.doesNotMatch(container.innerHTML, /previews picture only/);
  click(container, { ruler: true }, 50);
  assert.deepEqual(timelineSeeks, [160 / 24]);
});

test('a refused load does not count the timeline as loaded', async () => {
  const { editor, container, preview, playedTimelines, timelineSeeks } = await freshEditor(twoClipTimeline(), 'reel', null);
  await setPlayWholeTimeline(container, true);
  preview.showsClipSource = true;
  assert.equal(await editor.playTimelineFromPlayhead(), false);
  click(container, { ruler: true }, 50);
  await settled();
  assert.equal(playedTimelines.length, 2);
  assert.deepEqual(timelineSeeks, []);
});

test('a drag along the ruler seeks a loaded timeline and a hover does not', async () => {
  const { editor, container, timelineSeeks } = await freshEditor(twoClipTimeline());
  const move = (clientX, buttons) => container.handlers.pointermove({ target: fakeTarget({ ruler: true }), clientX, buttons });
  move(25, 1);
  assert.deepEqual(timelineSeeks, []);

  await setPlayWholeTimeline(container, true);
  await editor.playTimelineFromPlayhead();
  timelineSeeks.length = 0;
  move(25, 0);
  move(25, 1);
  move(50, 1);
  assert.deepEqual(timelineSeeks, [80 / 24, 160 / 24]);
});

test('with the timeline loaded Set in and Set out trim the clip under the playhead', async () => {
  const { editor, container, changes } = await freshEditor(twoClipTimeline());
  await setPlayWholeTimeline(container, true);
  await editor.playTimelineFromPlayhead();
  // the first clip is selected, the playhead plays on into the second
  click(container, { clipIndex: 0 }, 0);
  editor.followTimelinePosition({ seconds: 250 / 24, clipIndex: 1 });

  clickAction(container, 'set-in');
  assert.deepEqual(changes.at(-1).clips[1], clip(2, 30, 100, 240));
  assert.deepEqual(changes.at(-1).clips[0], clip(1, 0, 240, 0));
  assert.match(container.innerHTML, /data-field="in" value="00:00:01:06"/);

  // the second clip now starts at its source frame 30, so timeline frame 260 shows source frame 50
  editor.followTimelinePosition({ seconds: 260 / 24, clipIndex: 1 });
  clickAction(container, 'set-out');
  assert.deepEqual(changes.at(-1).clips[1], clip(2, 30, 50, 240));
});

test('Set in past the last clip or in a gap changes nothing', async () => {
  const { editor, container, changes } = await freshEditor(twoClipTimeline());
  await editor.playTimelineFromPlayhead();
  editor.followTimelinePosition({ seconds: 320 / 24, clipIndex: 1 });
  clickAction(container, 'set-in');
  assert.deepEqual(changes, []);

  editor.setTimeline(gappedTimeline());
  editor.followTimelinePosition({ seconds: 100 / 24, clipIndex: null });
  clickAction(container, 'set-in');
  assert.deepEqual(changes, []);
});

test('a still dropped on a clip is held for 5 seconds at the timeline rate, rounded to whole frames', async () => {
  for (const fps of [FPS, [24000, 1001]]) {
    const { container, changes } = await freshEditor({ ...twoClipTimeline(), fps });
    await drop(container, 5, { track: 'picture', layer: 1, clipIndex: 1 });
    // 5 seconds at 24000/1001 is 119.88 frames
    assert.deepEqual(changes.at(-1).clips[2], stillClip(5, 120, 320, 1));
  }
});

test('setClipRange refuses an in point other than 0 on a still and leaves its out point unbounded', () => {
  const model = { ...twoClipTimeline(), clips: [...twoClipTimeline().clips, stillClip(5, 120, 320)] };
  assert.throws(() => setClipRange(model, 2, 1, 120), { name: 'RangeError', message: /clip 2: in point 1 is not 0/ });
  assert.equal(setClipRange(model, 2, 0, 10000).clips[2].outFrame, 10000);
});

test('a selected still has its In field and Set in button disabled, and Set out from playhead sets its hold', async () => {
  const { editor, container, changes } = await freshEditor({ ...twoClipTimeline(), clips: [stillClip(5, 120, 0)] });
  click(container, { clipIndex: 0 });
  assert.match(container.innerHTML, /data-field="in" value="00:00:00:00" disabled>/);
  assert.match(container.innerHTML, /data-action="set-in" disabled>/);
  assert.doesNotMatch(container.innerHTML, /data-field="out"[^>]*disabled/);
  assert.doesNotMatch(container.innerHTML, /data-action="set-out"[^>]*disabled/);

  editor.setPlayhead(200);
  clickAction(container, 'set-out');
  assert.deepEqual(changes.at(-1).clips[0], stillClip(5, 200, 0));
});
