import assert from 'node:assert/strict';
import test from 'node:test';

import {
  addClip,
  clipAtFrame,
  clipSourceFrameAt,
  clipStarts,
  frameAtSeconds,
  moveClip,
  removeClip,
  removeReelStart,
  secondsAtFrame,
  setClipRange,
  splitReelAt,
  totalFrames,
  withReelStartsInsideTimeline,
} from '../src/timeline-editor.js';

const FPS = [24, 1];
const POINTER_BOUNDS = { left: 0, width: 100 };

const ASSETS = {
  1: { kind: 'picture', name: 'Reel One.mov', frames: 240 },
  2: { kind: 'picture', name: 'Reel Two.mov', frames: 120 },
  3: { kind: 'sound', name: 'Music.wav' },
  4: { kind: 'subtitle', name: 'English.srt' },
};

function clip(picture, inFrame, outFrame) {
  return {
    picture,
    frames: ASSETS[picture].frames,
    inFrame,
    outFrame,
    sound: { kind: 'embedded', asset: null },
    subtitle: null,
  };
}

// 240 kept frames from the first clip and 80 from the second, 320 in all
function twoClipTimeline() {
  return { fps: FPS, clips: [clip(1, 0, 240), clip(2, 20, 100)], sound: null, subtitle: null, reelStarts: [] };
}

function fakeContainer() {
  const playhead = { style: {} };
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
  };
}

// closest answers from what the scenario puts under the pointer
function fakeTarget({ action, frame, targetClip, clipIndex, track, ruler } = {}) {
  const elements = {
    '[data-action]': action && { dataset: { action, frame: String(frame), targetClip: String(targetClip) } },
    '[data-clip-index]': clipIndex !== undefined && {
      dataset: { clipIndex: String(clipIndex) },
      getBoundingClientRect: () => POINTER_BOUNDS,
    },
    '[data-track]': track && { dataset: { track } },
    '.timeline-editor-ruler': ruler && { getBoundingClientRect: () => POINTER_BOUNDS },
  };
  return { closest: (selector) => elements[selector] || null };
}

let instances = 0;

async function freshEditor(model, reelWord = 'reel', timelinePlayback = { pictureOnly: false }) {
  const editor = await import(`../src/timeline-editor.js?instance=${++instances}`);
  const container = fakeContainer();
  const changes = [];
  const seeks = [];
  const playedTimelines = [];
  const timelineSeeks = [];
  editor.initTimelineEditor(container, {
    fps: FPS,
    reelWord,
    assetName: (assetId) => ASSETS[assetId].name,
    assetKind: (assetId) => ASSETS[assetId].kind,
    sourceFrames: async (assetId) => ASSETS[assetId].frames,
    onChange: (changed) => changes.push(changed),
    onSeek: (frame) => seeks.push(frame),
    playTimeline: async (played) => {
      playedTimelines.push(played);
      return timelinePlayback;
    },
    seekTimeline: (seconds) => timelineSeeks.push(seconds),
  });
  if (model) editor.setTimeline(model);
  return { editor, container, changes, seeks, playedTimelines, timelineSeeks };
}

async function playTimeline(container) {
  await container.handlers.click({ target: fakeTarget({ action: 'play-timeline' }), clientX: 0 });
  // the editor takes the load's answer on the next turn
  await new Promise((resolve) => setImmediate(resolve));
}

function click(container, target, clientX = 0) {
  container.handlers.click({ target: fakeTarget(target), clientX });
}

function clickAction(container, action, details = {}) {
  click(container, { action, ...details });
}

async function drop(container, assetId, target) {
  await container.handlers.drop({
    target: fakeTarget(target),
    dataTransfer: { getData: (format) => (format === 'text/plain' ? String(assetId) : '') },
    preventDefault() {},
  });
}

function typeInField(container, field, value) {
  container.handlers.change({ target: { dataset: { field }, value } });
}

test('total frames and clip starts count the kept frames only', () => {
  const model = twoClipTimeline();
  assert.equal(totalFrames(model), 320);
  assert.deepEqual(clipStarts(model), [0, 240]);
});

test('clipAtFrame finds the clip and the offset into it', () => {
  const model = twoClipTimeline();
  assert.deepEqual(clipAtFrame(model, 0), { index: 0, frameInClip: 0 });
  assert.deepEqual(clipAtFrame(model, 239), { index: 0, frameInClip: 239 });
  assert.deepEqual(clipAtFrame(model, 240), { index: 1, frameInClip: 0 });
  assert.deepEqual(clipAtFrame(model, 319), { index: 1, frameInClip: 79 });
  assert.equal(clipAtFrame(model, 320), null);
  assert.equal(clipAtFrame(model, -1), null);
});

test('addClip inserts at the index, appends by default and leaves the model it was given alone', () => {
  const model = twoClipTimeline();
  const inserted = addClip(model, clip(2, 0, 120), 1);
  assert.deepEqual(
    inserted.clips.map((each) => [each.picture, each.inFrame]),
    [
      [1, 0],
      [2, 0],
      [2, 20],
    ],
  );
  assert.equal(addClip(model, clip(2, 0, 120)).clips[2].inFrame, 0);
  assert.equal(model.clips.length, 2);
  assert.throws(() => addClip(model, clip(2, 0, 120), 3), RangeError);
});

test('removeClip takes the clip out and refuses one that is not there', () => {
  const model = twoClipTimeline();
  assert.deepEqual(
    removeClip(model, 0).clips.map((each) => each.picture),
    [2],
  );
  assert.throws(() => removeClip(model, 2), /no clip 2/);
});

test('moveClip reorders and refuses a place off either end', () => {
  const model = twoClipTimeline();
  assert.deepEqual(
    moveClip(model, 0, 1).clips.map((each) => each.picture),
    [2, 1],
  );
  assert.throws(() => moveClip(model, 0, 2), RangeError);
  assert.throws(() => moveClip(model, -1, 0), RangeError);
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

test('removing a clip drops the reel starts now past the end and keeps the ones inside', () => {
  const model = { ...twoClipTimeline(), reelStarts: [100, 250] };
  assert.deepEqual(removeClip(model, 1).reelStarts, [100]);
  assert.deepEqual(removeClip(model, 0).reelStarts, []);
});

test('a reel start exactly at the total is dropped, one a frame before it stays', () => {
  const model = { ...twoClipTimeline(), reelStarts: [319, 320] };
  assert.deepEqual(withReelStartsInsideTimeline(model).reelStarts, [319]);
});

test('a loaded timeline keeps reel starts past its end for clips not probed yet', async () => {
  const unprobed = { ...twoClipTimeline(), clips: [{ ...clip(1, 0, 0), frames: 0 }], reelStarts: [100] };
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

test('a picture dropped on a clip becomes a clip after it with the probed frame count', async () => {
  const { editor, container, changes } = await freshEditor(twoClipTimeline());
  await drop(container, 2, { track: 'picture', clipIndex: 0 });
  const added = changes.at(-1).clips[1];
  assert.deepEqual(added, clip(2, 0, 120));
  assert.equal(changes.at(-1).clips[2].inFrame, 20);
  assert.deepEqual(editor.getTimeline(), changes.at(-1));
});

test('a picture dropped past the clips is appended last', async () => {
  const { container, changes } = await freshEditor(twoClipTimeline());
  await drop(container, 2, { track: 'picture' });
  assert.equal(changes.at(-1).clips.length, 3);
  assert.deepEqual(changes.at(-1).clips[2], clip(2, 0, 120));
});

test('an asset dropped on the wrong track changes nothing', async () => {
  const { container, changes } = await freshEditor(twoClipTimeline());
  await drop(container, 3, { track: 'picture', clipIndex: 0 });
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
  const { editor, container, changes, seeks } = await freshEditor(twoClipTimeline());
  // the left edge of the second clip is timeline frame 240
  click(container, { clipIndex: 1 }, 0);
  assert.deepEqual(seeks, [240]);
  assert.match(container.innerHTML, /data-field="in" value="00:00:00:20"/);

  editor.setPlayhead(250);
  clickAction(container, 'set-in');
  assert.equal(changes.at(-1).clips[1].inFrame, 30);
  assert.equal(changes.at(-1).clips[1].outFrame, 100);
  assert.deepEqual(changes.at(-1).clips[0], clip(1, 0, 240));
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

test('the arrows move the selected clip and Remove takes it out', async () => {
  const { container, changes } = await freshEditor(twoClipTimeline());
  click(container, { clipIndex: 1 });
  clickAction(container, 'move-left');
  assert.deepEqual(
    changes.at(-1).clips.map((each) => each.picture),
    [2, 1],
  );
  clickAction(container, 'remove-clip');
  assert.deepEqual(
    changes.at(-1).clips.map((each) => each.picture),
    [1],
  );
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
  const { container, seeks } = await freshEditor(twoClipTimeline());
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

test('Play timeline hands the whole timeline over and a picture only answer shows the hint', async () => {
  const { container, playedTimelines } = await freshEditor(twoClipTimeline(), 'reel', { pictureOnly: true });
  assert.match(container.innerHTML, /data-action="play-timeline">Play timeline</);
  assert.doesNotMatch(container.innerHTML, /previews picture only/);

  await playTimeline(container);
  assert.deepEqual(playedTimelines, [twoClipTimeline()]);
  assert.match(container.innerHTML, /The JPEG 2000 timeline previews picture only/);
});

test('an empty timeline cannot be played', async () => {
  const { container } = await freshEditor();
  assert.match(container.innerHTML, /data-action="play-timeline" disabled>/);
});

test('the playhead follows the loaded timeline and lets go once the preview plays something else', async () => {
  const { editor, container, seeks, timelineSeeks } = await freshEditor(twoClipTimeline(), 'reel', { pictureOnly: true });
  await playTimeline(container);

  editor.followTimelinePosition({ seconds: 10, clipIndex: 1 });
  assert.match(container.playhead.style.left, /\* 0\.75\)$/);
  click(container, { ruler: true }, 50);
  assert.deepEqual(timelineSeeks, [160 / 24]);
  assert.deepEqual(seeks, []);

  editor.followTimelinePosition(null);
  assert.doesNotMatch(container.innerHTML, /previews picture only/);
  click(container, { ruler: true }, 50);
  assert.deepEqual(seeks, [160]);
});

test('a refused load leaves the playhead on the clip preview', async () => {
  const { container, seeks, timelineSeeks } = await freshEditor(twoClipTimeline(), 'reel', null);
  await playTimeline(container);
  click(container, { ruler: true }, 50);
  assert.deepEqual(seeks, [160]);
  assert.deepEqual(timelineSeeks, []);
});

test('a drag along the ruler seeks a loaded timeline and a hover does not', async () => {
  const { container, timelineSeeks } = await freshEditor(twoClipTimeline());
  const move = (clientX, buttons) => container.handlers.pointermove({ target: fakeTarget({ ruler: true }), clientX, buttons });
  move(25, 1);
  assert.deepEqual(timelineSeeks, []);

  await playTimeline(container);
  move(25, 0);
  move(25, 1);
  move(50, 1);
  assert.deepEqual(timelineSeeks, [80 / 24, 160 / 24]);
});

test('with the timeline loaded Set in and Set out trim the clip under the playhead', async () => {
  const { editor, container, changes } = await freshEditor(twoClipTimeline());
  await playTimeline(container);
  // the first clip is selected, the playhead plays on into the second
  click(container, { clipIndex: 0 }, 0);
  editor.followTimelinePosition({ seconds: 250 / 24, clipIndex: 1 });

  clickAction(container, 'set-in');
  assert.deepEqual(changes.at(-1).clips[1], clip(2, 30, 100));
  assert.deepEqual(changes.at(-1).clips[0], clip(1, 0, 240));
  assert.match(container.innerHTML, /data-field="in" value="00:00:01:06"/);

  // the second clip now starts at its source frame 30, so timeline frame 260 shows source frame 50
  editor.followTimelinePosition({ seconds: 260 / 24, clipIndex: 1 });
  clickAction(container, 'set-out');
  assert.deepEqual(changes.at(-1).clips[1], clip(2, 30, 50));
});

test('Set in past the last clip changes nothing', async () => {
  const { editor, container, changes } = await freshEditor(twoClipTimeline());
  await playTimeline(container);
  editor.followTimelinePosition({ seconds: 320 / 24, clipIndex: 1 });
  clickAction(container, 'set-in');
  assert.deepEqual(changes, []);
});
