import test from 'node:test';
import assert from 'node:assert/strict';
import { chartReadout, loudnessChartSvg, secondsAtChartX } from '../src/loudness-chart.js';
import { parseLoudnessTarget } from '../src/loudness-panel.js';

// 31 values from 3.0 s to 6.0 s, so 3.0 s sits half way along the time axis
function levels(overrides = {}) {
  return {
    shortTermLufs: Array(31).fill(-30),
    shortTermFirstSeconds: 3,
    shortTermStepSeconds: 0.1,
    integratedLufs: -30,
    truePeakDbtp: -1.234,
    truePeakAtSeconds: 1.5,
    channelShortTermLufs: [Array(31).fill(-20), Array(31).fill(-40)],
    channelTruePeakDbtp: [Array(60).fill(-6), Array(60).fill(null)],
    ...overrides,
  };
}

const MIX_ONLY = { showMix: true, channelCurves: [] };

function curvePath(svg) {
  return svg.match(/class="loudness-chart-curve" d="([^"]*)"/)[1];
}

function texts(svg) {
  return [...svg.matchAll(/<text[^>]*>([^<]*)<\/text>/g)].map((match) => match[1]);
}

// the plot runs x 26 to 234 and y 14 (0 LUFS) to 122 (-60 LUFS)
test('a value maps to its time and level', () => {
  const svg = loudnessChartSvg(levels(), null, MIX_ONLY);
  assert.match(curvePath(svg), /^M130 68 L/);
  assert.match(curvePath(svg), / L234 68$/);
  assert.match(svg, /class="loudness-chart-integrated" x1="26" y1="68" x2="234" y2="68"/);
  assert.match(svg, /class="loudness-chart-peak" x1="78" y1="14" x2="78" y2="122"/);
  assert.match(svg, /viewBox="0 0 250 140"/);
});

test('levels past the axis ends sit on the axis', () => {
  const svg = loudnessChartSvg(levels({ shortTermLufs: [-80, 3], shortTermStepSeconds: 3 }), null, MIX_ONLY);
  assert.equal(curvePath(svg), 'M130 122 L234 14');
});

test('a silent window splits the curve', () => {
  const svg = loudnessChartSvg(levels({ shortTermLufs: [-20, -20, null, -20, -20] }), null, MIX_ONLY);
  const path = curvePath(svg);
  assert.equal(path.match(/M/g).length, 2);
  assert.equal(path.match(/L/g).length, 2);
});

test('a lufs target draws its line and a Leq(m) target draws none', () => {
  const lufs = loudnessChartSvg(levels(), parseLoudnessTarget('lufs=-20'), MIX_ONLY);
  assert.match(lufs, /class="loudness-chart-target" x1="26" y1="50" x2="234" y2="50"/);
  assert.ok(texts(lufs).includes('Target -20 LUFS'));
  const leqM = loudnessChartSvg(levels(), parseLoudnessTarget('leqm=85'), MIX_ONLY);
  assert.doesNotMatch(leqM, /loudness-chart-target/);
  assert.doesNotMatch(loudnessChartSvg(levels(), null, MIX_ONLY), /loudness-chart-target/);
});

test('the axes read in LUFS and mm:ss and the peak in dBTP', () => {
  const short = texts(loudnessChartSvg(levels(), null, MIX_ONLY));
  for (const label of ['-60', '-50', '-40', '-30', '-20', '-10', '0', '00:00', '00:02', '00:04', '00:06', 'Integrated -30.0 LUFS', '-1.2 dBTP']) {
    assert.ok(short.includes(label), `no ${label} in ${short}`);
  }
  const tenMinutes = levels({ shortTermLufs: Array(5971).fill(-30) });
  const long = texts(loudnessChartSvg(tenMinutes, null, MIX_ONLY));
  assert.deepEqual(long.filter((label) => label.includes(':')), ['00:00', '02:00', '04:00', '06:00', '08:00', '10:00']);
});

test('a silent track draws no integrated line and no peak mark', () => {
  const svg = loudnessChartSvg(levels({ shortTermLufs: [null, null], integratedLufs: null, truePeakDbtp: null }), null, MIX_ONLY);
  assert.equal(curvePath(svg), '');
  assert.doesNotMatch(svg, /loudness-chart-integrated|loudness-chart-peak/);
});

test('a channel curve draws in its colour slot and the mix can be hidden', () => {
  const svg = loudnessChartSvg(levels(), null, { showMix: false, channelCurves: [{ channel: 1, colourSlot: 3 }] });
  assert.doesNotMatch(svg, /loudness-chart-curve/);
  const channel = svg.match(/<path class="loudness-chart-channel" data-channel="1" style="stroke: var\(--channel-colour-3\)" d="([^"]*)"/);
  assert.ok(channel, svg);
  // -40 LUFS sits two thirds of the way down the plot
  assert.match(channel[1], /^M130 86 L/);
});

test('channel peaks are drawn from 0 s only while ticked', () => {
  const channelCurves = [{ channel: 0, colourSlot: 1, label: 'L' }];
  const without = loudnessChartSvg(levels(), null, { showMix: true, channelCurves, showPeaks: false });
  assert.doesNotMatch(without, /loudness-chart-channel-peak/);
  const peaks = loudnessChartSvg(levels(), null, { showMix: true, channelCurves, showPeaks: true });
  assert.match(peaks, /class="loudness-chart-channel-peak" data-channel="0" style="stroke: var\(--channel-colour-1\)" d="M26 24\.8 L/);
});

test('the pointer position maps to a time only inside the plot', () => {
  assert.equal(secondsAtChartX(levels(), 130), 3);
  assert.equal(secondsAtChartX(levels(), 20), null);
  assert.equal(secondsAtChartX(levels(), 240), null);
});

test('the readout lists every drawn curve at that time', () => {
  const channelCurves = [{ channel: 0, colourSlot: 1, label: 'L' }, { channel: 1, colourSlot: 2, label: 'R' }];
  assert.equal(
    chartReadout(levels(), 3.04, { showMix: true, channelCurves, showPeaks: true }),
    '00:03.0: Mix -30.0 LUFS, L -20.0 LUFS, L peak -6.0 dBTP, R -40.0 LUFS, R peak silent',
  );
  assert.equal(
    chartReadout(levels(), 3.04, { showMix: false, channelCurves: channelCurves.slice(0, 1), showPeaks: false }),
    '00:03.0: L -20.0 LUFS',
  );
});

test('before the first short term window the readout has the peaks only', () => {
  const channelCurves = [{ channel: 0, colourSlot: 1, label: 'L' }];
  assert.equal(
    chartReadout(levels(), 1.25, { showMix: true, channelCurves, showPeaks: true }),
    '00:01.3: L peak -6.0 dBTP',
  );
});
