import assert from 'node:assert/strict';
import test from 'node:test';

import { previewButtonEnabled, previewTarget, sourcePreviewTarget, PREVIEW_KIND_PACKAGE, PREVIEW_KIND_SOURCE, PREVIEW_KIND_TIMELINE } from '../src/preview-target.js';

const PICKED_ASSET = { kind: PREVIEW_KIND_SOURCE, path: '/assets/picked.mov' };
const PICKED_PACKAGE = { kind: PREVIEW_KIND_PACKAGE, path: '/packages/Picked' };
const BUILT = '/packages/Built';

test('a picked asset wins over the built package', () => {
  assert.deepEqual(previewTarget({ selectedPreview: PICKED_ASSET, builtPackage: BUILT }), PICKED_ASSET);
});

test('a picked package wins over the built package', () => {
  assert.deepEqual(previewTarget({ selectedPreview: PICKED_PACKAGE, builtPackage: BUILT }), PICKED_PACKAGE);
});

test('with nothing picked Preview plays the built package', () => {
  assert.deepEqual(previewTarget({ selectedPreview: null, builtPackage: BUILT }), { kind: PREVIEW_KIND_PACKAGE, path: BUILT });
});

test('with nothing picked and nothing built there is nothing to preview', () => {
  assert.equal(previewTarget({ selectedPreview: null, builtPackage: null }), null);
});

const CLIP_AT_FRAME_240 = { picture: 2, start: 240, layer: 0, inFrame: 20, outFrame: 100 };

test('with Play whole timeline ticked Preview on a clip plays the whole timeline from the clip start', () => {
  assert.deepEqual(sourcePreviewTarget('/assets/second.mov', CLIP_AT_FRAME_240, true), { kind: PREVIEW_KIND_TIMELINE, startFrame: 240 });
});

test('with Play whole timeline unticked Preview on a clip plays the clip source', () => {
  assert.deepEqual(sourcePreviewTarget('/assets/second.mov', CLIP_AT_FRAME_240, false), { kind: PREVIEW_KIND_SOURCE, path: '/assets/second.mov' });
});

test('Preview on a picture no clip shows plays it alone whatever the box says', () => {
  for (const playsWholeTimeline of [true, false]) {
    assert.deepEqual(sourcePreviewTarget('/assets/unplaced.mov', null, playsWholeTimeline), { kind: PREVIEW_KIND_SOURCE, path: '/assets/unplaced.mov' });
  }
});

test('the button is on when the target differs from what the panel shows', () => {
  const target = { kind: PREVIEW_KIND_SOURCE, path: '/assets/first.mov' };

  assert.equal(previewButtonEnabled(target, '/assets/other.mov', false), true);
});

test('the button is off when the target is already shown', () => {
  const target = { kind: PREVIEW_KIND_SOURCE, path: '/assets/first.mov' };

  assert.equal(previewButtonEnabled(target, '/assets/first.mov', false), false);
});

test('the button comes back on once the panel shows nothing', () => {
  const target = { kind: PREVIEW_KIND_SOURCE, path: '/assets/first.mov' };

  assert.equal(previewButtonEnabled(target, null, false), true);
});

test('the button comes back on when playback reaches the end', () => {
  const target = { kind: PREVIEW_KIND_SOURCE, path: '/assets/first.mov' };

  assert.equal(previewButtonEnabled(target, '/assets/first.mov', true), true);
});

test('the button is off when there is nothing to open', () => {
  assert.equal(previewButtonEnabled(null, null, false), false);
});
