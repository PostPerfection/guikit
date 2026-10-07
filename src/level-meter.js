import { escapeHtml } from './html.js';

const SCALE_BOTTOM_DBFS = -60;
const SCALE_TOP_DBFS = 0;
const SCALE_TICKS_DBFS = [0, -10, -20, -40, -60];
const PERCENT = 100;
export const PEAK_HOLD_MS = 1000;
export const CLIP_HOLD_MS = 2000;
// a 24-bit sample at positive full scale reads -0.000001 dBFS
export const CLIP_DBFS = -0.0001;

// held at its highest for PEAK_HOLD_MS, then it follows the current peak down
function nextPeakMark(mark, peakDbfs, nowMs) {
  if (!mark || peakDbfs >= mark.dbfs) return { dbfs: peakDbfs, sinceMs: nowMs };
  if (nowMs - mark.sinceMs >= PEAK_HOLD_MS) return { dbfs: peakDbfs, sinceMs: mark.sinceMs };
  return mark;
}

function sameChannels(state, levels) {
  return (
    state?.channels.length === levels.length &&
    state.channels.every((channel, index) => channel.label === levels[index].label)
  );
}

// levels is the poll's audio_levels, null hides the meter
export function nextLevelMeterState(state, levels, nowMs) {
  if (!levels) return null;
  const previous = sameChannels(state, levels) ? state : null;
  const clipped = levels.some((level) => level.peak_dbfs >= CLIP_DBFS);
  return {
    channels: levels.map((level, index) => ({
      label: level.label,
      rmsDbfs: level.rms_dbfs,
      peakMark: nextPeakMark(previous?.channels[index].peakMark, level.peak_dbfs, nowMs),
    })),
    lastClipMs: clipped ? nowMs : (previous?.lastClipMs ?? null),
  };
}

export function clipLit(state, nowMs) {
  return state.lastClipMs !== null && nowMs - state.lastClipMs < CLIP_HOLD_MS;
}

// height on the bar in percent, 0 at the bottom of the scale and below
export function scalePercent(dbfs) {
  const fraction = (dbfs - SCALE_BOTTOM_DBFS) / (SCALE_TOP_DBFS - SCALE_BOTTOM_DBFS);
  return Math.min(1, Math.max(0, fraction)) * PERCENT;
}

function channelMarkup(channel) {
  const peakShown = channel.peakMark.dbfs > SCALE_BOTTOM_DBFS;
  const peak = peakShown
    ? `<div class="level-meter-peak" style="bottom: ${scalePercent(channel.peakMark.dbfs)}%"></div>`
    : '';
  return (
    '<div class="level-meter-channel">' +
    `<div class="level-meter-bar"><div class="level-meter-rms" style="height: ${scalePercent(channel.rmsDbfs)}%"></div>${peak}</div>` +
    `<span class="level-meter-label">${escapeHtml(channel.label)}</span>` +
    '</div>'
  );
}

export function levelMeterMarkup(state, nowMs) {
  if (!state) return '';
  const clipClass = clipLit(state, nowMs) ? 'level-meter-clip level-meter-clip-lit' : 'level-meter-clip';
  const ticks = SCALE_TICKS_DBFS.map(
    (dbfs) => `<span class="level-meter-tick" style="bottom: ${scalePercent(dbfs)}%">${dbfs}</span>`,
  ).join('');
  return (
    `<div class="${clipClass}" title="Lit while a peak in the last two seconds reached 0 dBFS">Clip</div>` +
    `<div class="level-meter-body"><div class="level-meter-scale">${ticks}</div>${state.channels.map(channelMarkup).join('')}</div>`
  );
}

// keeps the hold and clip state between polls and draws each poll into element
export function createLevelMeter(element) {
  let state = null;
  return (levels) => {
    const nowMs = performance.now();
    state = nextLevelMeterState(state, levels, nowMs);
    element.hidden = !state;
    element.innerHTML = levelMeterMarkup(state, nowMs);
  };
}
