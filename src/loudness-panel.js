import { loudnessChartSvg } from "./loudness-chart.js";
import { escapeHtml } from "./html.js";

const DECIBEL_STEPS = 10;
const STALE_NOTE = ". Measure again to confirm";

// the same spellings the build's loudness target accepts, null when it would refuse
export function parseLoudnessTarget(spec) {
  const text = String(spec ?? "");
  const separator = text.indexOf("=");
  if (separator < 0) return null;
  const metric = text.slice(0, separator).trim().toLowerCase().replace("(m)", "");
  const valueText = text.slice(separator + 1).trim();
  const value = Number(valueText);
  if (valueText === "" || !Number.isFinite(value)) return null;
  if (metric === "leqm" || metric === "leq") return { metric: "leqm", value };
  if (metric === "lufs") return { metric: "lufs", value };
  return null;
}

// the gain that takes the sound before its level step to the target, or null to keep the button disabled
export function gainToReachTarget(targetSpec, measurement) {
  const target = parseLoudnessTarget(targetSpec);
  if (!target || !measurement) return null;
  const before = target.metric === "leqm" ? measurement.before.leqMDb : measurement.before.integratedLufs;
  // adding zero turns a rounded -0 into 0
  return Math.round((target.value - before) * DECIBEL_STEPS) / DECIBEL_STEPS + 0;
}

// a typed 0 is a gain, so only an empty field is no gain
export function optionalNumber(text) {
  const trimmed = String(text ?? "").trim();
  if (trimmed === "") return null;
  const value = Number(trimmed);
  return Number.isFinite(value) ? value : null;
}

function levelsSummary(levels) {
  return `Integrated ${levels.integratedLufs.toFixed(1)} LUFS, `
    + `Leq(m) ${levels.leqMDb.toFixed(1)} dB, `
    + `true peak ${levels.truePeakDbtp.toFixed(2)} dBTP, `
    + `range ${levels.rangeLu.toFixed(1)} LU`;
}

export function sourceLine(measurement) {
  return `Source: ${levelsSummary(measurement.before)}`;
}

export function deliveredLine(measurement, stale = false) {
  return `Delivered: ${levelsSummary(measurement.delivered)}${stale ? STALE_NOTE : ""}`;
}

export function measurementSteps(measurement) {
  return measurement.steps.length ? measurement.steps.join(", ") : "Source track as is";
}

// the categorical colours base.css defines as --channel-colour-1 onwards
const CHANNEL_COLOURS = 8;

// channels that are not silent all through, each with a colour, null past the eighth
export function channelChoices(measurement) {
  const choices = [];
  measurement.delivered.channelShortTermLufs.forEach((series, channel) => {
    if (series.every((lufs) => lufs === null)) return;
    const colourSlot = choices.length < CHANNEL_COLOURS ? choices.length + 1 : null;
    choices.push({ channel, label: measurement.channelLabels[channel], colourSlot });
  });
  return choices;
}

function curveCheckbox({ value, label, checked, swatch }) {
  return `<label class="loudness-curve-choice"><input type="checkbox" value="${escapeHtml(value)}"${checked ? " checked" : ""}>`
    + `${swatch}${escapeHtml(label)}</label>`;
}

// shownCurves is { mix, labels }, labels holding the channel names ticked
export function curvePickerHtml(choices, shownCurves) {
  const mix = curveCheckbox({ value: "mix", label: "Mix", checked: shownCurves.mix, swatch: '<span class="loudness-curve-swatch loudness-curve-swatch-mix"></span>' });
  const channels = choices.filter(({ colourSlot }) => colourSlot !== null).map(({ label, colourSlot }) => curveCheckbox({
    value: label,
    label,
    checked: shownCurves.labels.has(label),
    swatch: `<span class="loudness-curve-swatch" style="background: var(--channel-colour-${colourSlot})"></span>`,
  }));
  const withoutCurve = choices.filter(({ colourSlot }) => colourSlot === null).map(({ label }) => label);
  const note = withoutCurve.length
    ? `<span class="loudness-curve-note">No curve for ${escapeHtml(withoutCurve.join(", "))}: eight channels at most.</span>`
    : "";
  return [mix, ...channels].join("") + note;
}

export function deliveredChart(measurement, targetSpec, shownCurves) {
  const channelCurves = channelChoices(measurement)
    .filter(({ label, colourSlot }) => colourSlot !== null && shownCurves.labels.has(label));
  return loudnessChartSvg(measurement.delivered, parseLoudnessTarget(targetSpec), { showMix: shownCurves.mix, channelCurves });
}

function loudnessPanelFields() {
  return {
    target: document.getElementById("prop-loudness"),
    measure: document.getElementById("prop-measure-sound"),
    setGain: document.getElementById("prop-gain-to-target"),
    gain: document.getElementById("prop-audio-gain"),
    source: document.getElementById("prop-loudness-source"),
    delivered: document.getElementById("prop-loudness-delivered"),
    steps: document.getElementById("prop-loudness-steps"),
    chart: document.getElementById("prop-loudness-chart"),
    curves: document.getElementById("prop-loudness-curves"),
  };
}

// measureSound resolves to a postkit SoundMeasurement, soundMissing returns why there is nothing to measure or null
export function initLoudnessPanel({ measureSound, soundMissing }) {
  const fields = loudnessPanelFields();
  let lastMeasurement = null;
  const shownCurves = { mix: true, labels: new Set() };

  function refreshGainToTarget() {
    fields.setGain.disabled = gainToReachTarget(fields.target.value, lastMeasurement) === null;
  }

  function markDeliveredStale() {
    if (!lastMeasurement) return;
    fields.delivered.textContent = deliveredLine(lastMeasurement, true);
  }

  function drawChart() {
    fields.chart.innerHTML = deliveredChart(lastMeasurement, fields.target.value, shownCurves);
  }

  fields.measure.addEventListener("click", async () => {
    const missing = soundMissing();
    fields.source.textContent = missing ?? "Measuring…";
    fields.delivered.textContent = "";
    fields.steps.textContent = "";
    fields.chart.innerHTML = "";
    fields.curves.innerHTML = "";
    lastMeasurement = null;
    refreshGainToTarget();
    if (missing) return;
    fields.measure.disabled = true;
    try {
      lastMeasurement = await measureSound();
      fields.source.textContent = sourceLine(lastMeasurement);
      fields.delivered.textContent = deliveredLine(lastMeasurement);
      fields.steps.textContent = measurementSteps(lastMeasurement);
      fields.curves.innerHTML = curvePickerHtml(channelChoices(lastMeasurement), shownCurves);
      drawChart();
    } catch (e) {
      fields.source.textContent = String(e);
    } finally {
      fields.measure.disabled = false;
      refreshGainToTarget();
    }
  });

  fields.curves.addEventListener("change", (event) => {
    const box = event.target;
    if (box.value === "mix") shownCurves.mix = box.checked;
    else if (box.checked) shownCurves.labels.add(box.value);
    else shownCurves.labels.delete(box.value);
    drawChart();
  });

  fields.setGain.addEventListener("click", () => {
    const gain = gainToReachTarget(fields.target.value, lastMeasurement);
    if (gain === null) return;
    fields.gain.value = gain.toFixed(1);
    markDeliveredStale();
  });

  fields.target.addEventListener("input", refreshGainToTarget);
  fields.gain.addEventListener("input", markDeliveredStale);
}
