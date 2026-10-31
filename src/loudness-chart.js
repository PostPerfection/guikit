export const CHART_WIDTH = 250;
const CHART_HEIGHT = 140;
const PLOT_LEFT = 26;
const PLOT_RIGHT = 234;
const PLOT_TOP = 14;
const PLOT_BOTTOM = 122;
const LOUDEST_LUFS = 0;
const QUIETEST_LUFS = -60;

const LUFS_GRID_STEP = 10;
const TIME_LABEL_BASELINE = CHART_HEIGHT - 4;
const LINE_LABEL_OFFSET = 3;
const MAXIMUM_TIME_TICKS = 5;
const TIME_TICK_STEPS_SECONDS = [1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600];
const SECONDS_PER_MINUTE = 60;

function lufsToY(lufs) {
  const clamped = Math.min(LOUDEST_LUFS, Math.max(QUIETEST_LUFS, lufs));
  const fraction = (LOUDEST_LUFS - clamped) / (LOUDEST_LUFS - QUIETEST_LUFS);
  return round(PLOT_TOP + fraction * (PLOT_BOTTOM - PLOT_TOP));
}

function secondsToX(seconds, durationSeconds) {
  return round(PLOT_LEFT + (seconds / durationSeconds) * (PLOT_RIGHT - PLOT_LEFT));
}

function formatReadoutTime(totalSeconds) {
  const minutes = Math.floor(totalSeconds / SECONDS_PER_MINUTE);
  const seconds = (totalSeconds % SECONDS_PER_MINUTE).toFixed(1);
  return `${String(minutes).padStart(2, "0")}:${seconds.padStart(4, "0")}`;
}

function formatMinutesSeconds(totalSeconds) {
  const minutes = Math.floor(totalSeconds / SECONDS_PER_MINUTE);
  const seconds = Math.round(totalSeconds % SECONDS_PER_MINUTE);
  return `${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
}

function round(value) {
  return Math.round(value * 10) / 10;
}

function timeTickStep(durationSeconds) {
  return TIME_TICK_STEPS_SECONDS.find((step) => durationSeconds / step <= MAXIMUM_TIME_TICKS)
    ?? TIME_TICK_STEPS_SECONDS.at(-1);
}

function lastValueSeconds(levels) {
  const count = levels.shortTermLufs.length;
  if (count === 0) return levels.shortTermFirstSeconds;
  return levels.shortTermFirstSeconds + (count - 1) * levels.shortTermStepSeconds;
}

function chartDurationSeconds(levels) {
  return Math.max(lastValueSeconds(levels), levels.truePeakAtSeconds);
}

// one "M" per run of values between silent windows
function seriesPath(values, firstSeconds, stepSeconds, durationSeconds) {
  let path = "";
  let drawing = false;
  values.forEach((level, index) => {
    if (level === null) {
      drawing = false;
      return;
    }
    const seconds = firstSeconds + index * stepSeconds;
    path += `${drawing ? "L" : "M"}${secondsToX(seconds, durationSeconds)} ${lufsToY(level)} `;
    drawing = true;
  });
  return path.trim();
}

function shortTermPath(values, levels, durationSeconds) {
  return seriesPath(values, levels.shortTermFirstSeconds, levels.shortTermStepSeconds, durationSeconds);
}

// a channel's true peak series starts at 0 s, one value per short term step
function truePeakPath(values, levels, durationSeconds) {
  return seriesPath(values, 0, levels.shortTermStepSeconds, durationSeconds);
}

// undefined outside the series, null for silence
function shortTermAt(values, levels, seconds) {
  return values[Math.round((seconds - levels.shortTermFirstSeconds) / levels.shortTermStepSeconds)];
}

function truePeakAt(values, levels, seconds) {
  return values[Math.floor(seconds / levels.shortTermStepSeconds)];
}

function readoutPart(name, level, unit) {
  if (level === undefined) return [];
  return [`${name} ${level === null ? "silent" : `${level.toFixed(1)} ${unit}`}`];
}

// chartX is in the viewBox's units, null outside the plot
export function secondsAtChartX(levels, chartX) {
  if (chartX < PLOT_LEFT || chartX > PLOT_RIGHT) return null;
  return ((chartX - PLOT_LEFT) / (PLOT_RIGHT - PLOT_LEFT)) * chartDurationSeconds(levels);
}

export function chartXAtSeconds(levels, seconds) {
  return secondsToX(seconds, chartDurationSeconds(levels));
}

// the value of every drawn curve at one moment, as the hover line under the chart shows it
export function chartReadout(levels, seconds, { showMix, channelCurves, showPeaks }) {
  const parts = [];
  if (showMix) parts.push(...readoutPart("Mix", shortTermAt(levels.shortTermLufs, levels, seconds), "LUFS"));
  for (const { channel, label } of channelCurves) {
    parts.push(...readoutPart(label, shortTermAt(levels.channelShortTermLufs[channel], levels, seconds), "LUFS"));
    if (showPeaks) parts.push(...readoutPart(`${label} peak`, truePeakAt(levels.channelTruePeakDbtp[channel], levels, seconds), "dBTP"));
  }
  return `${formatReadoutTime(seconds)}: ${parts.join(", ")}`;
}

function lufsGrid() {
  let markup = "";
  for (let lufs = QUIETEST_LUFS; lufs <= LOUDEST_LUFS; lufs += LUFS_GRID_STEP) {
    const y = lufsToY(lufs);
    markup += `<line class="loudness-chart-grid" x1="${PLOT_LEFT}" y1="${y}" x2="${PLOT_RIGHT}" y2="${y}"/>`;
    markup += `<text class="loudness-chart-label" x="${PLOT_LEFT - LINE_LABEL_OFFSET}" y="${y}" text-anchor="end" dominant-baseline="middle">${lufs}</text>`;
  }
  return markup;
}

function timeTicks(durationSeconds) {
  const step = timeTickStep(durationSeconds);
  let markup = "";
  for (let seconds = 0; seconds <= durationSeconds; seconds += step) {
    const x = secondsToX(seconds, durationSeconds);
    markup += `<line class="loudness-chart-grid" x1="${x}" y1="${PLOT_TOP}" x2="${x}" y2="${PLOT_BOTTOM}"/>`;
    markup += `<text class="loudness-chart-label" x="${x}" y="${TIME_LABEL_BASELINE}" text-anchor="middle">${formatMinutesSeconds(seconds)}</text>`;
  }
  return markup;
}

function levelLine(className, lufs, label, { labelAtRight, labelAbove }) {
  const y = lufsToY(lufs);
  const labelX = labelAtRight ? PLOT_RIGHT - LINE_LABEL_OFFSET : PLOT_LEFT + LINE_LABEL_OFFSET;
  const labelY = labelAbove ? y - LINE_LABEL_OFFSET : y + LINE_LABEL_OFFSET;
  return `<line class="${className}" x1="${PLOT_LEFT}" y1="${y}" x2="${PLOT_RIGHT}" y2="${y}"/>`
    + `<text class="loudness-chart-label" x="${labelX}" y="${labelY}" text-anchor="${labelAtRight ? "end" : "start"}" dominant-baseline="${labelAbove ? "auto" : "hanging"}">${label}</text>`;
}

function truePeakMark(levels, durationSeconds) {
  const x = secondsToX(levels.truePeakAtSeconds, durationSeconds);
  const labelOnLeft = x > (PLOT_LEFT + PLOT_RIGHT) / 2;
  const labelX = labelOnLeft ? x - LINE_LABEL_OFFSET : x + LINE_LABEL_OFFSET;
  return `<line class="loudness-chart-peak" x1="${x}" y1="${PLOT_TOP}" x2="${x}" y2="${PLOT_BOTTOM}"/>`
    + `<text class="loudness-chart-label loudness-chart-peak-label" x="${labelX}" y="${PLOT_TOP - LINE_LABEL_OFFSET}" text-anchor="${labelOnLeft ? "end" : "start"}">${levels.truePeakDbtp.toFixed(1)} dBTP</text>`;
}

// target is parseLoudnessTarget's result, a Leq(m) target has no place on a LUFS axis
// channelCurves holds { channel, colourSlot } for each channel curve drawn over the mix
export function loudnessChartSvg(levels, target, { showMix, channelCurves, showPeaks }) {
  const durationSeconds = chartDurationSeconds(levels);
  const lufsTarget = target?.metric === "lufs" ? target.value : null;
  // the two labels sit at opposite ends and on the far sides of their lines
  const integratedAboveTarget = lufsTarget === null || levels.integratedLufs > lufsTarget;
  let markup = lufsGrid() + timeTicks(durationSeconds);
  for (const { channel, colourSlot } of channelCurves) {
    markup += `<path class="loudness-chart-channel" data-channel="${channel}" style="stroke: var(--channel-colour-${colourSlot})" d="${shortTermPath(levels.channelShortTermLufs[channel], levels, durationSeconds)}"/>`;
    if (showPeaks) {
      markup += `<path class="loudness-chart-channel-peak" data-channel="${channel}" style="stroke: var(--channel-colour-${colourSlot})" d="${truePeakPath(levels.channelTruePeakDbtp[channel], levels, durationSeconds)}"/>`;
    }
  }
  if (showMix) {
    markup += `<path class="loudness-chart-curve" d="${shortTermPath(levels.shortTermLufs, levels, durationSeconds)}"/>`;
  }
  if (Number.isFinite(levels.integratedLufs)) {
    markup += levelLine("loudness-chart-integrated", levels.integratedLufs, `Integrated ${levels.integratedLufs.toFixed(1)} LUFS`, { labelAtRight: true, labelAbove: integratedAboveTarget });
  }
  if (lufsTarget !== null) {
    markup += levelLine("loudness-chart-target", lufsTarget, `Target ${lufsTarget} LUFS`, { labelAtRight: false, labelAbove: !integratedAboveTarget });
  }
  if (Number.isFinite(levels.truePeakDbtp)) {
    markup += truePeakMark(levels, durationSeconds);
  }
  markup += `<line class="loudness-chart-cursor" x1="${PLOT_LEFT}" y1="${PLOT_TOP}" x2="${PLOT_LEFT}" y2="${PLOT_BOTTOM}" visibility="hidden"/>`;
  return `<svg class="loudness-chart" viewBox="0 0 ${CHART_WIDTH} ${CHART_HEIGHT}" role="img" aria-label="Short-term loudness over time">${markup}</svg>`;
}
