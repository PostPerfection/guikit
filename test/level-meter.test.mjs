import assert from 'node:assert/strict';
import test from 'node:test';

import {
  CLIP_HOLD_MS,
  PEAK_HOLD_MS,
  clipLit,
  levelMeterMarkup,
  nextLevelMeterState,
  scalePercent,
} from '../src/level-meter.js';

const SILENCE_DBFS = -100;
const POLL_MS = 250;

function levels(peakDbfs, rmsDbfs = peakDbfs - 3) {
  return [
    { label: 'L', peak_dbfs: peakDbfs, rms_dbfs: rmsDbfs },
    { label: 'R', peak_dbfs: SILENCE_DBFS, rms_dbfs: SILENCE_DBFS },
  ];
}

function peakAfter(polls) {
  let state = null;
  for (const [nowMs, peakDbfs] of polls) state = nextLevelMeterState(state, levels(peakDbfs), nowMs);
  return state.channels[0].peakMark.dbfs;
}

test('the peak mark holds for a second, then follows the current peak', () => {
  assert.equal(peakAfter([[0, -6], [POLL_MS, -20]]), -6, 'held');
  assert.equal(peakAfter([[0, -6], [PEAK_HOLD_MS - 1, -20]]), -6, 'still held');
  assert.equal(peakAfter([[0, -6], [PEAK_HOLD_MS, -20]]), -20, 'released');
  assert.equal(peakAfter([[0, -6], [PEAK_HOLD_MS, -20], [PEAK_HOLD_MS + POLL_MS, -30]]), -30, 'following');
  assert.equal(peakAfter([[0, -20], [POLL_MS, -6]]), -6, 'a higher peak takes over at once');
});

test('the clip mark is lit for two seconds after a peak reaches 0 dBFS', () => {
  let state = nextLevelMeterState(null, levels(0), 0);
  assert.ok(clipLit(state, 0));
  state = nextLevelMeterState(state, levels(-12), CLIP_HOLD_MS - 1);
  assert.ok(clipLit(state, CLIP_HOLD_MS - 1));
  state = nextLevelMeterState(state, levels(-12), CLIP_HOLD_MS);
  assert.ok(!clipLit(state, CLIP_HOLD_MS));
  assert.ok(!clipLit(nextLevelMeterState(null, levels(-0.5), 0), 0), 'half a dB under full scale is no clip');
});

test('a 24-bit positive full scale sample counts as a clip', () => {
  assert.ok(clipLit(nextLevelMeterState(null, levels(-0.000001), 0), 0));
});

test('no levels hide the meter, and other channels start the hold over', () => {
  assert.equal(nextLevelMeterState(nextLevelMeterState(null, levels(-6), 0), null, POLL_MS), null);
  const stereo = nextLevelMeterState(null, levels(-6), 0);
  const mono = nextLevelMeterState(stereo, [{ label: 'C', peak_dbfs: -30, rms_dbfs: -33 }], POLL_MS);
  assert.equal(mono.channels[0].peakMark.dbfs, -30);
});

test('the scale runs from -60 dBFS at the bottom to 0 at the top', () => {
  assert.equal(scalePercent(0), 100);
  assert.equal(scalePercent(-30), 50);
  assert.equal(scalePercent(-60), 0);
  assert.equal(scalePercent(SILENCE_DBFS), 0);
  assert.equal(scalePercent(3), 100);
});

test('the markup draws a bar with its label per channel and the peak only on the scale', () => {
  const state = nextLevelMeterState(null, levels(-30, -45), 0);
  const markup = levelMeterMarkup(state, 0);
  assert.equal(markup.match(/class="level-meter-channel"/g).length, 2);
  assert.match(markup, /class="level-meter-rms" style="height: 25%"/);
  assert.match(markup, /class="level-meter-peak" style="bottom: 50%"/);
  assert.equal(markup.match(/class="level-meter-peak"/g).length, 1, 'the silent channel has no peak mark');
  assert.match(markup, /<span class="level-meter-label">L<\/span>/);
  assert.match(markup, /class="level-meter-clip"/);
  assert.doesNotMatch(markup, /level-meter-clip-lit/);
  assert.equal(levelMeterMarkup(null, 0), '');
});

test('a channel label from the file is escaped', () => {
  const state = nextLevelMeterState(null, [{ label: '<b>', peak_dbfs: -6, rms_dbfs: -9 }], 0);
  assert.match(levelMeterMarkup(state, 0), /&lt;b&gt;/);
});
