import assert from 'node:assert/strict';
import test from 'node:test';

import {
  emptyTimeline,
  ffprobeVideoInfo,
  reelSpans,
  sourceFrameCount,
  splitTimecodeFrames,
  splitTimecodeSpec,
  timelineFrameAtSourceFrame,
  timelineWire,
  timelineWithKnownAssets,
  withFirstClipRate,
  withImportedAsset,
  withProbedFrames,
  withReelStartingClipsPlaced,
  withSplitAtFrames,
  withUnplacedAssetsFilled,
  withoutAsset,
} from '../src/composition-timeline.js';

const ASSETS = [
  { id: 1, type: 'video', path: '/media/a.mov', name: 'a.mov', fps: '24/1', frames: 240 },
  { id: 2, type: 'video', path: '/media/b.mov', name: 'b.mov', fps: '24/1', frames: 96 },
  { id: 3, type: 'video', path: '/media/c', name: 'c', frames: 24 },
  { id: 4, type: 'audio', path: '/media/b.wav', name: 'b.wav' },
  { id: 5, type: 'subtitle', path: '/media/b.srt', name: 'b.srt' },
  { id: 6, type: 'audio', path: '/media/mix.wav', name: 'mix.wav' },
  { id: 8, type: 'subtitle', path: '/media/feature.srt', name: 'feature.srt' },
];

function assetById(assetId) {
  return ASSETS.find((asset) => asset.id === assetId);
}

function clip(picture, frames, inFrame, outFrame, sound = { kind: 'embedded', asset: null }, subtitle = null) {
  return { picture, frames, inFrame, outFrame, sound, subtitle };
}

test("the wire timeline is postkit's JSON with paths for asset ids", () => {
  const timeline = {
    fps: [24, 1],
    clips: [
      clip(1, 240, 0, 48),
      clip(2, 96, 10, 58, { kind: 'file', asset: 4 }, 5),
      clip(3, 24, 0, 24, { kind: 'none', asset: null }),
    ],
    sound: null,
    subtitle: null,
    reelStarts: [48],
  };

  assert.deepEqual(timelineWire(timeline, assetById), {
    version: 1,
    fps: { numerator: 24, denominator: 1 },
    clips: [
      { picture: '/media/a.mov', in_frame: 0, out_frame: 48, sound: 'Embedded', subtitle: null },
      { picture: '/media/b.mov', in_frame: 10, out_frame: 58, sound: { File: '/media/b.wav' }, subtitle: '/media/b.srt' },
      { picture: '/media/c', in_frame: 0, out_frame: 24, sound: 'None', subtitle: null },
    ],
    sound: 'PerClip',
    subtitle: null,
    reel_starts: [48],
  });
  const covered = timelineWire({ ...timeline, sound: 6, subtitle: 8 }, assetById);
  assert.deepEqual(covered.sound, { Override: '/media/mix.wav' });
  assert.equal(covered.subtitle, '/media/feature.srt');
});

test('an imported video becomes the last clip and the first sound and subtitle cover the timeline', () => {
  let timeline = emptyTimeline([24, 1]);
  for (const assetId of [1, 6, 8, 2, 4, 5]) timeline = withImportedAsset(timeline, assetById(assetId));

  assert.deepEqual(timeline, {
    fps: [24, 1],
    clips: [clip(1, 240, 0, 240), clip(2, 96, 0, 96)],
    sound: 6,
    subtitle: 8,
    reelStarts: [],
  });
});

test('an imported sound stays off the timeline while a clip has a sound file of its own', () => {
  const timeline = { ...emptyTimeline([24, 1]), clips: [clip(1, 240, 0, 240, { kind: 'file', asset: 4 }, 5)] };

  assert.deepEqual(withImportedAsset(withImportedAsset(timeline, assetById(6)), assetById(8)), timeline);
});

test('an unprobed clip takes its frame count, and the first clip its rate', () => {
  const unprobed = withImportedAsset(emptyTimeline([24, 1]), { ...assetById(2), frames: undefined });
  const probed = withProbedFrames(unprobed, 2, 96);

  assert.deepEqual(unprobed.clips[0], clip(2, 0, 0, 0));
  assert.deepEqual(probed.clips[0], clip(2, 96, 0, 96));
  const ntsc = (assetId) => ({ ...assetById(assetId), fps: '24000/1001' });
  assert.deepEqual(withFirstClipRate(probed, ntsc).fps, [24000, 1001]);
  assert.equal(withFirstClipRate(probed, assetById), probed);
  assert.equal(sourceFrameCount(4.004, '24000/1001'), 96);
});

test("ffprobe's video stream gives the size, rate and frame count", () => {
  const probe = {
    streams: [
      { codec_type: 'audio', duration: '9.0' },
      { codec_type: 'video', width: 1920, height: 1080, r_frame_rate: '24000/1001', duration: '4.004', bit_rate: '8000' },
    ],
    format: { duration: '4.1' },
  };

  const info = ffprobeVideoInfo(probe);

  assert.equal(info.width, 1920);
  assert.equal(info.fps, '24000/1001');
  assert.equal(info.frames, 96);
  assert.equal(info.duration, 4.1);
  assert.equal(info.bitRate, 8000);
  assert.equal(ffprobeVideoInfo({ streams: [{ codec_type: 'audio' }] }), null);
});

test('filling the timeline places the first unplaced video only on an empty timeline', () => {
  const filled = withUnplacedAssetsFilled(emptyTimeline([24, 1]), ASSETS);

  assert.deepEqual(
    filled.clips.map((placed) => placed.picture),
    [1],
  );
  assert.equal(filled.sound, 4);
  assert.equal(filled.subtitle, 5);
  assert.deepEqual(withUnplacedAssetsFilled(filled, ASSETS), filled);
});

test('a reel start lands at each listed clip once every clip is probed', () => {
  const unprobed = {
    id: 1,
    timeline: { ...emptyTimeline([24, 1]), clips: [clip(1, 240, 0, 240), clip(2, 0, 0, 0), clip(3, 0, 0, 0)] },
    reelStartingClipIndices: [1, 2],
  };

  assert.equal(withReelStartingClipsPlaced(unprobed), unprobed);
  const probed = { ...unprobed, timeline: withProbedFrames(withProbedFrames(unprobed.timeline, 2, 96), 3, 24) };
  const placed = withReelStartingClipsPlaced(probed);
  assert.deepEqual(placed.timeline.reelStarts, [240, 336]);
  assert.equal('reelStartingClipIndices' in placed, false);
});

test('split timecodes and reel starts read back as the same frames', () => {
  assert.deepEqual(splitTimecodeFrames('00:00:02, 00:01:00:12', 24), [48, 1452]);
  assert.deepEqual(splitTimecodeFrames('', 24), []);
  assert.equal(splitTimecodeFrames('00:00', 24), null);
  assert.equal(splitTimecodeFrames('00:00:01:24', 24), null);
  assert.equal(splitTimecodeSpec([48, 1452], 24), '00:00:02:00, 00:01:00:12');
});

test('a typed split past the end of a one-clip timeline is left out of its reel starts', () => {
  const timeline = { ...emptyTimeline([24, 1]), clips: [clip(1, 48, 0, 48)] };

  assert.deepEqual(withSplitAtFrames(timeline, [24, 48, 120]).reelStarts, [24]);
});

test('a typed split stays while the one clip is unprobed, and a multi-clip timeline ignores the field', () => {
  const unprobed = { ...emptyTimeline([24, 1]), clips: [clip(1, 0, 0, 0)] };
  const twoClips = { ...emptyTimeline([24, 1]), clips: [clip(1, 48, 0, 48), clip(2, 48, 0, 48)], reelStarts: [48] };

  assert.deepEqual(withSplitAtFrames(unprobed, [24, 120]).reelStarts, [24, 120]);
  assert.equal(withSplitAtFrames(twoClips, [24]), twoClips);
  assert.equal(withSplitAtFrames(unprobed, null), unprobed);
});

test('each reel span carries the assets of the clip it starts in', () => {
  const timeline = {
    fps: [24, 1],
    clips: [clip(1, 240, 0, 96), clip(2, 96, 12, 84, { kind: 'file', asset: 4 }, 5)],
    sound: null,
    subtitle: null,
    reelStarts: [96, 500],
  };

  assert.deepEqual(reelSpans(timeline, assetById), [
    { frames: 96, picture: assetById(1), sound: null, subtitle: null },
    { frames: 72, picture: assetById(2), sound: assetById(4), subtitle: assetById(5) },
  ]);
  assert.deepEqual(reelSpans(emptyTimeline([24, 1]), assetById), []);
});

test('a source frame of the second clip lands after the first clip on the timeline', () => {
  const timeline = { ...emptyTimeline([24, 1]), clips: [clip(1, 240, 0, 96), clip(2, 96, 12, 84)] };

  assert.equal(timelineFrameAtSourceFrame(timeline, 1, 12), 96);
  assert.equal(timelineFrameAtSourceFrame(timeline, 1, 50), 134);
});

test('removing an asset drops its clips and the reel starts now past the end', () => {
  const timeline = {
    ...emptyTimeline([24, 1]),
    clips: [clip(1, 240, 0, 96), clip(2, 96, 12, 84, { kind: 'file', asset: 4 }, 5)],
    sound: 6,
    reelStarts: [48, 96, 120],
  };

  assert.deepEqual(withoutAsset(timeline, 2).clips, [clip(1, 240, 0, 96)]);
  assert.deepEqual(withoutAsset(timeline, 2).reelStarts, [48]);
  const withoutSound = withoutAsset(withoutAsset(timeline, 4), 6);
  assert.deepEqual(withoutSound.clips[1].sound, { kind: 'embedded', asset: null });
  assert.equal(withoutSound.sound, null);
});

test('an asset the project file no longer lists is taken off the timeline and named', () => {
  const composition = {
    name: 'Main',
    timeline: {
      ...emptyTimeline([24, 1]),
      clips: [clip(1, 240, 0, 96), clip(9, 96, 0, 96), clip(2, 96, 0, 96, { kind: 'file', asset: 9 }, 9)],
      sound: 9,
      subtitle: 8,
    },
  };
  const notRestored = [];

  const kept = timelineWithKnownAssets(composition, new Map(ASSETS.map((asset) => [asset.id, asset])), notRestored);

  assert.deepEqual(kept.clips, [clip(1, 240, 0, 96), clip(2, 96, 0, 96)]);
  assert.equal(kept.sound, null);
  assert.equal(kept.subtitle, 8);
  assert.deepEqual(notRestored, ['Main clip 2 picture', 'Main clip 2 sound', 'Main clip 2 subtitle', 'Main timeline sound']);
});
