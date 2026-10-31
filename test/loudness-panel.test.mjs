import test from 'node:test';
import assert from 'node:assert/strict';
import {
  parseLoudnessTarget,
  gainToReachTarget,
  optionalNumber,
  sourceLine,
  deliveredLine,
  measurementSteps,
  channelChoices,
  curvePickerHtml,
  deliveredChart,
} from '../src/loudness-panel.js';

const levels = {
  integratedLufs: -27.34,
  leqMDb: 71.42,
  truePeakDbtp: -1.956,
  rangeLu: 19.21,
  shortTermMaxLufs: -15.2,
};

// the delivered levels carry a -3 dB gain the before levels exclude
const measurement = {
  before: levels,
  delivered: { ...levels, integratedLufs: levels.integratedLufs - 3, leqMDb: levels.leqMDb - 3 },
  steps: ['Routed the channel set by filename', 'Applied gain/fades'],
};

test('a lufs target sets the gain from the integrated loudness before the level step', () => {
  assert.equal(gainToReachTarget('lufs=-20', measurement), 7.3);
  assert.equal(gainToReachTarget('LUFS = -30', measurement), -2.7);
});

test('a leqm target sets the gain from the Leq(m) before the level step', () => {
  assert.equal(gainToReachTarget('leqm=85', measurement), 13.6);
  assert.equal(gainToReachTarget('Leq(m)=82', measurement), 10.6);
});

test('no usable target or no measurement leaves the button disabled', () => {
  assert.equal(gainToReachTarget('', measurement), null);
  assert.equal(gainToReachTarget('lufs=', measurement), null);
  assert.equal(gainToReachTarget('loud=3', measurement), null);
  assert.equal(gainToReachTarget('lufs=-20', null), null);
});

test('one press reaches the target and a second press after measuring again keeps it', () => {
  const gain = gainToReachTarget('lufs=-20', measurement);
  const remeasured = {
    before: levels,
    delivered: { ...levels, integratedLufs: levels.integratedLufs + gain },
    steps: [],
  };
  assert.ok(Math.abs(remeasured.delivered.integratedLufs + 20) < 0.05);
  assert.equal(gainToReachTarget('lufs=-20', remeasured), gain);
  assert.ok(Object.is(gainToReachTarget('lufs=-20', { ...measurement, before: { ...levels, integratedLufs: -20.04 } }), 0));
});

test('the target parses the way the build parses it', () => {
  assert.deepEqual(parseLoudnessTarget('leq=85'), { metric: 'leqm', value: 85 });
  assert.equal(parseLoudnessTarget('85'), null);
  assert.equal(parseLoudnessTarget('lufs=abc'), null);
});

test('a typed zero gain is a gain and an empty field is none', () => {
  assert.equal(optionalNumber('0'), 0);
  assert.equal(optionalNumber(' -6.5 '), -6.5);
  assert.equal(optionalNumber(''), null);
  assert.equal(optionalNumber(undefined), null);
});

test('the results lines read as the panel shows them', () => {
  assert.equal(
    sourceLine(measurement),
    'Source: Integrated -27.3 LUFS, Leq(m) 71.4 dB, true peak -1.96 dBTP, range 19.2 LU',
  );
  assert.equal(
    deliveredLine(measurement, true),
    'Delivered: Integrated -30.3 LUFS, Leq(m) 68.4 dB, true peak -1.96 dBTP, range 19.2 LU. Measure again to confirm',
  );
  assert.equal(measurementSteps(measurement), 'Routed the channel set by filename, Applied gain/fades');
  assert.equal(measurementSteps({ ...measurement, steps: [] }), 'Source track as is');
});

const LANES = ['L', 'R', 'C', 'LFE', 'Ls', 'Rs', 'Lc', 'Rc', 'BsL', 'BsR', 'HI', 'VI'];

function measuredChannels(silent) {
  const series = LANES.map((_, channel) => silent.includes(channel) ? [null, null] : [-30, -29]);
  const delivered = { ...levels, shortTermLufs: [-30, -29], shortTermFirstSeconds: 3, shortTermStepSeconds: 0.1, truePeakAtSeconds: 1, channelShortTermLufs: series };
  return { ...measurement, delivered, channelLabels: LANES };
}

test('a silent channel gets no choice and the colours follow the channels with sound', () => {
  const choices = channelChoices(measuredChannels([3, 6, 7, 10, 11]));
  assert.deepEqual(choices.map(({ label, colourSlot }) => [label, colourSlot]), [
    ['L', 1], ['R', 2], ['C', 3], ['Ls', 4], ['Rs', 5], ['BsL', 6], ['BsR', 7],
  ]);
});

test('channels past the eighth with sound are named as having no curve', () => {
  const choices = channelChoices(measuredChannels([]));
  assert.deepEqual(choices.slice(8).map(({ label, colourSlot }) => [label, colourSlot]), [['BsL', null], ['BsR', null], ['HI', null], ['VI', null]]);
  const html = curvePickerHtml(choices, { mix: true, labels: new Set(['C']) });
  assert.equal((html.match(/type="checkbox"/g) ?? []).length, 9);
  assert.match(html, /value="mix" checked/);
  assert.match(html, /value="C" checked/);
  assert.doesNotMatch(html, /value="L" checked/);
  assert.match(html, /No curve for BsL, BsR, HI, VI: eight channels at most\./);
});

test('the chart draws the ticked channels by their colour', () => {
  const svg = deliveredChart(measuredChannels([3]), 'lufs=-20', { mix: false, labels: new Set(['Ls', 'R']) });
  const drawn = [...svg.matchAll(/data-channel="(\d+)" style="stroke: var\(--channel-colour-(\d)\)"/g)].map((match) => [match[1], match[2]]);
  assert.deepEqual(drawn, [['1', '2'], ['4', '4']]);
  assert.doesNotMatch(svg, /loudness-chart-curve/);
});
