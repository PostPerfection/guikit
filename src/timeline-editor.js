import { escapeHtml } from './html.js';
import { framesToTimecode } from './time-format.js';

const PERCENT = 100;
const ZOOM_STEP = 1.5;
const MINIMUM_ZOOM = 1;
const MAXIMUM_ZOOM = 20;
const RULER_TICK_SECONDS = [1, 5, 10, 30, 60, 300, 600, 1800, 3600];
const RULER_TICKS_PER_VIEW = 10;
const SECONDS_PER_MINUTE = 60;
const SECONDS_PER_HOUR = 3600;
const TIMECODE_FIELD_DIGITS = 2;
const FRAME_TIMECODE_PATTERN = /^(\d+):(\d{1,2}):(\d{1,2}):(\d{1,2})$/;
const ASSET_ID_RADIX = 10;
const WHOLE_TIMELINE_DROP_HINT = 'Drop a file on this label to cover the whole timeline';

let panel = null;
let editorOptions = null;
let timeline = null;
let selectedIndex = null;
let playheadFrame = 0;
let zoom = MINIMUM_ZOOM;

export function initTimelineEditor(container, options) {
  panel = container;
  editorOptions = options;
  timeline = { fps: options.fps, clips: [], sound: null, subtitle: null, reelStarts: [] };
  selectedIndex = null;
  playheadFrame = 0;
  zoom = MINIMUM_ZOOM;
  panel.classList.add('timeline-editor');
  panel.addEventListener('click', handleClick);
  panel.addEventListener('change', handleFieldChange);
  panel.addEventListener('dragover', handleDragOver);
  panel.addEventListener('drop', handleDrop);
  render();
}

export function setTimeline(model) {
  timeline = structuredClone(model);
  selectedIndex = null;
  render();
}

export function getTimeline() {
  return structuredClone(timeline);
}

export function setPlayhead(frame) {
  playheadFrame = frame;
  panel.querySelector('.timeline-editor-playhead').style.left = playheadLeft();
}

export function setReelStarts(frames) {
  const withoutReelStarts = editedCopy(timeline, (copy) => {
    copy.reelStarts = [];
  });
  applyEdit(frames.reduce((model, frame) => splitReelAt(model, frame), withoutReelStarts));
}

export function totalFrames(model) {
  return model.clips.reduce((sum, clip) => sum + keptFrames(clip), 0);
}

export function clipStarts(model) {
  let start = 0;
  return model.clips.map((clip) => {
    const clipStart = start;
    start += keptFrames(clip);
    return clipStart;
  });
}

export function clipAtFrame(model, frame) {
  const starts = clipStarts(model);
  const index = model.clips.findIndex(
    (clip, clipIndex) => frame >= starts[clipIndex] && frame < starts[clipIndex] + keptFrames(clip),
  );
  if (index === -1) return null;
  return { index, frameInClip: frame - starts[index] };
}

export function addClip(model, clip, index = model.clips.length) {
  if (!Number.isInteger(index) || index < 0 || index > model.clips.length) {
    throw new RangeError(`cannot add a clip at ${index}, the timeline has ${model.clips.length}`);
  }
  return editedCopy(model, (copy) => {
    copy.clips.splice(index, 0, structuredClone(clip));
  });
}

export function removeClip(model, index) {
  checkClipIndex(model, index);
  const edited = editedCopy(model, (copy) => {
    copy.clips.splice(index, 1);
  });
  return withReelStartsInsideTimeline(edited);
}

export function moveClip(model, from, to) {
  checkClipIndex(model, from);
  checkClipIndex(model, to);
  return editedCopy(model, (copy) => {
    const [clip] = copy.clips.splice(from, 1);
    copy.clips.splice(to, 0, clip);
  });
}

export function setClipRange(model, index, inFrame, outFrame) {
  checkClipIndex(model, index);
  const { frames } = model.clips[index];
  if (inFrame < 0) throw new RangeError(`clip ${index}: in point ${inFrame} is before the first frame`);
  if (inFrame >= outFrame) {
    throw new RangeError(`clip ${index}: in point ${inFrame} is at or past out point ${outFrame}`);
  }
  if (outFrame > frames) {
    throw new RangeError(`clip ${index}: out point ${outFrame} is past the source's ${frames} frames`);
  }
  const edited = editedCopy(model, (copy) => {
    copy.clips[index].inFrame = inFrame;
    copy.clips[index].outFrame = outFrame;
  });
  return withReelStartsInsideTimeline(edited);
}

export function splitReelAt(model, frame) {
  return editedCopy(model, (copy) => {
    if (frame <= 0 || copy.reelStarts.includes(frame)) return;
    copy.reelStarts.push(frame);
    copy.reelStarts.sort((first, second) => first - second);
  });
}

// a reel start at the total would begin an empty reel
export function withReelStartsInsideTimeline(model) {
  const total = totalFrames(model);
  return editedCopy(model, (copy) => {
    copy.reelStarts = copy.reelStarts.filter((reelStart) => reelStart < total);
  });
}

export function removeReelStart(model, frame) {
  return editedCopy(model, (copy) => {
    copy.reelStarts = copy.reelStarts.filter((reelStart) => reelStart !== frame);
  });
}

function keptFrames(clip) {
  return clip.outFrame - clip.inFrame;
}

function checkClipIndex(model, index) {
  if (Number.isInteger(index) && index >= 0 && index < model.clips.length) return;
  throw new RangeError(`there is no clip ${index}, the timeline has ${model.clips.length}`);
}

function editedCopy(model, edit) {
  const copy = structuredClone(model);
  edit(copy);
  return copy;
}

function applyEdit(nextTimeline) {
  timeline = nextTimeline;
  render();
  editorOptions.onChange(structuredClone(timeline));
}

// non-drop timecode at the rate rounded to whole frames
function frameTimecode(frames, fps) {
  const framesPerSecond = nominalFramesPerSecond(fps);
  const frameInSecond = String(frames % framesPerSecond).padStart(TIMECODE_FIELD_DIGITS, '0');
  return `${framesToTimecode(frames, [framesPerSecond, 1])}:${frameInSecond}`;
}

function parseFrameTimecode(text, fps) {
  const match = FRAME_TIMECODE_PATTERN.exec(text.trim());
  if (!match) return null;
  const [hours, minutes, seconds, frameInSecond] = match.slice(1).map(Number);
  const framesPerSecond = nominalFramesPerSecond(fps);
  if (minutes >= SECONDS_PER_MINUTE || seconds >= SECONDS_PER_MINUTE || frameInSecond >= framesPerSecond) return null;
  const totalSeconds = hours * SECONDS_PER_HOUR + minutes * SECONDS_PER_MINUTE + seconds;
  return totalSeconds * framesPerSecond + frameInSecond;
}

function nominalFramesPerSecond([numerator, denominator]) {
  return Math.round(numerator / denominator);
}

function render() {
  const total = totalFrames(timeline);
  const starts = clipStarts(timeline);
  panel.innerHTML = `
    <div class="timeline-editor-toolbar">
      <button class="btn-sm" type="button" data-action="zoom-out" title="Zoom out"${zoom <= MINIMUM_ZOOM ? ' disabled' : ''}>−</button>
      <button class="btn-sm" type="button" data-action="zoom-in" title="Zoom in"${zoom >= MAXIMUM_ZOOM ? ' disabled' : ''}>+</button>
      <button class="btn-sm" type="button" data-action="split-reel">Split reel here</button>
    </div>
    <div class="timeline-editor-scroll">
      <div class="timeline-editor-body" style="width: ${zoom * PERCENT}%">
        ${renderRuler(total)}
        ${renderTrack('picture', 'Picture', '', pictureTrackContent(total, starts))}
        ${renderTrack('sound', 'Sound', WHOLE_TIMELINE_DROP_HINT, soundTrackContent(total, starts))}
        ${renderTrack('subtitle', 'Subtitle', WHOLE_TIMELINE_DROP_HINT, subtitleTrackContent(total, starts))}
        <div class="timeline-editor-playhead" style="left: ${playheadLeft()}"></div>
      </div>
    </div>
    ${renderClipPanel()}`;
}

function renderRuler(total) {
  if (total === 0) return '<div class="timeline-editor-ruler"></div>';
  const [numerator, denominator] = timeline.fps;
  const visibleSeconds = (total * denominator) / numerator / zoom;
  const tickSeconds =
    RULER_TICK_SECONDS.find((seconds) => visibleSeconds / seconds <= RULER_TICKS_PER_VIEW) ?? RULER_TICK_SECONDS.at(-1);
  const framesPerTick = (tickSeconds * numerator) / denominator;
  const ticks = [];
  for (let frame = 0; frame < total; frame += framesPerTick) {
    ticks.push(`
      <div class="timeline-editor-tick" style="left: ${(frame / total) * PERCENT}%">
        <span class="timeline-editor-tick-label">${framesToTimecode(Math.round(frame), timeline.fps)}</span>
      </div>`);
  }
  const markers = timeline.reelStarts
    .filter((frame) => frame < total)
    .map(
      (frame) => `
      <div class="timeline-editor-reel-marker" data-action="remove-reel-start" data-frame="${frame}"
        style="left: ${(frame / total) * PERCENT}%" title="Reel starts at ${frameTimecode(frame, timeline.fps)}, click to remove"></div>`,
    );
  return `<div class="timeline-editor-ruler">${ticks.join('')}${markers.join('')}</div>`;
}

function renderTrack(kind, label, labelTitle, content) {
  return `
    <div class="timeline-editor-track" data-track="${kind}">
      <div class="timeline-editor-track-label"${labelTitle ? ` title="${labelTitle}"` : ''}>${label}</div>
      <div class="timeline-editor-track-content">${content}</div>
    </div>`;
}

function pictureTrackContent(total, starts) {
  if (timeline.clips.length === 0) return '<span class="timeline-editor-empty">Drop a picture asset here</span>';
  return timeline.clips
    .map((clip, index) => {
      const name = escapeHtml(editorOptions.assetName(clip.picture));
      const range = `${frameTimecode(clip.inFrame, timeline.fps)} to ${frameTimecode(clip.outFrame, timeline.fps)}`;
      const selected = index === selectedIndex ? ' selected' : '';
      return `<div class="timeline-editor-clip${selected}" data-clip-index="${index}" style="${spanStyle(starts[index], keptFrames(clip), total)}" title="${name}, ${range}">${name}</div>`;
    })
    .join('');
}

const clipSoundLabels = {
  embedded: () => 'embedded',
  file: (sound) => editorOptions.assetName(sound.asset),
  none: () => 'no sound',
};

function soundTrackContent(total, starts) {
  if (timeline.sound !== null) return wholeTimelineSpan(timeline.sound, 'clear-sound');
  return timeline.clips
    .map((clip, index) => {
      const clearButton = clip.sound.kind === 'file' ? clearSpanButton('clear-clip-sound', index) : '';
      return clipSpan(index, starts[index], keptFrames(clip), total, clipSoundLabels[clip.sound.kind](clip.sound), clearButton);
    })
    .join('');
}

function subtitleTrackContent(total, starts) {
  if (timeline.subtitle !== null) return wholeTimelineSpan(timeline.subtitle, 'clear-subtitle');
  return timeline.clips
    .map((clip, index) => {
      if (clip.subtitle === null) return clipSpan(index, starts[index], keptFrames(clip), total, '', '');
      const clearButton = clearSpanButton('clear-clip-subtitle', index);
      return clipSpan(index, starts[index], keptFrames(clip), total, editorOptions.assetName(clip.subtitle), clearButton);
    })
    .join('');
}

function clipSpan(index, start, length, total, label, clearButton) {
  const empty = label === '' ? ' timeline-editor-span-empty' : '';
  return `<div class="timeline-editor-span${empty}" data-clip-index="${index}" style="${spanStyle(start, length, total)}">
    <span class="timeline-editor-span-label">${escapeHtml(label)}</span>${clearButton}</div>`;
}

function wholeTimelineSpan(assetId, clearAction) {
  return `<div class="timeline-editor-span timeline-editor-span-whole" style="left: 0%; width: ${PERCENT}%">
    <span class="timeline-editor-span-label">${escapeHtml(editorOptions.assetName(assetId))}</span>${clearSpanButton(clearAction, null)}</div>`;
}

function clearSpanButton(action, clipIndex) {
  const clipAttribute = clipIndex === null ? '' : ` data-target-clip="${clipIndex}"`;
  return `<button class="timeline-editor-span-clear" type="button" data-action="${action}"${clipAttribute} title="Take off">✕</button>`;
}

function spanStyle(start, length, total) {
  return `left: ${(start / total) * PERCENT}%; width: ${(length / total) * PERCENT}%`;
}

function playheadLeft() {
  const total = totalFrames(timeline);
  const fraction = total === 0 ? 0 : playheadFrame / total;
  return `calc(var(--timeline-track-label-width) + (100% - var(--timeline-track-label-width)) * ${fraction})`;
}

function renderClipPanel() {
  if (selectedIndex === null) return '';
  const clip = timeline.clips[selectedIndex];
  const isFirst = selectedIndex === 0;
  const isLast = selectedIndex === timeline.clips.length - 1;
  return `
    <div class="timeline-editor-clip-panel">
      <span class="timeline-editor-clip-name">${escapeHtml(editorOptions.assetName(clip.picture))}</span>
      <label>In <input class="timeline-editor-timecode" type="text" spellcheck="false" data-field="in" value="${frameTimecode(clip.inFrame, timeline.fps)}"></label>
      <label>Out <input class="timeline-editor-timecode" type="text" spellcheck="false" data-field="out" value="${frameTimecode(clip.outFrame, timeline.fps)}"></label>
      <button class="btn-sm" type="button" data-action="set-in">Set in from playhead</button>
      <button class="btn-sm" type="button" data-action="set-out">Set out from playhead</button>
      <button class="btn-sm" type="button" data-action="move-left" title="Move left"${isFirst ? ' disabled' : ''}>←</button>
      <button class="btn-sm" type="button" data-action="move-right" title="Move right"${isLast ? ' disabled' : ''}>→</button>
      <button class="btn-sm" type="button" data-action="remove-clip">Remove</button>
    </div>`;
}

const clickActions = {
  'zoom-in': () => setZoom(zoom * ZOOM_STEP),
  'zoom-out': () => setZoom(zoom / ZOOM_STEP),
  'split-reel': () => applyEdit(splitReelAt(timeline, playheadFrame)),
  'remove-reel-start': (element) => applyEdit(removeReelStart(timeline, Number(element.dataset.frame))),
  'set-in': () => trySetClipRange(sourceFrameAtPlayhead(), timeline.clips[selectedIndex].outFrame),
  'set-out': () => trySetClipRange(timeline.clips[selectedIndex].inFrame, sourceFrameAtPlayhead()),
  'move-left': () => moveSelectedClip(selectedIndex - 1),
  'move-right': () => moveSelectedClip(selectedIndex + 1),
  'remove-clip': () => {
    const index = selectedIndex;
    selectedIndex = null;
    applyEdit(removeClip(timeline, index));
  },
  'clear-sound': () =>
    applyEdit(
      editedCopy(timeline, (copy) => {
        copy.sound = null;
      }),
    ),
  'clear-subtitle': () =>
    applyEdit(
      editedCopy(timeline, (copy) => {
        copy.subtitle = null;
      }),
    ),
  'clear-clip-sound': (element) =>
    applyEdit(
      editedCopy(timeline, (copy) => {
        copy.clips[Number(element.dataset.targetClip)].sound = { kind: 'embedded', asset: null };
      }),
    ),
  'clear-clip-subtitle': (element) =>
    applyEdit(
      editedCopy(timeline, (copy) => {
        copy.clips[Number(element.dataset.targetClip)].subtitle = null;
      }),
    ),
};

function handleClick(event) {
  const actionElement = event.target.closest('[data-action]');
  if (actionElement) {
    clickActions[actionElement.dataset.action](actionElement);
    return;
  }
  const clipElement = event.target.closest('[data-clip-index]');
  if (clipElement) {
    selectClip(clipElement, event);
    return;
  }
  const ruler = event.target.closest('.timeline-editor-ruler');
  const total = totalFrames(timeline);
  if (!ruler || total === 0) return;
  const frame = frameAtPointer(ruler, event, 0, total);
  setPlayhead(frame);
  editorOptions.onSeek(frame);
}

function selectClip(clipElement, event) {
  selectedIndex = Number(clipElement.dataset.clipIndex);
  const start = clipStarts(timeline)[selectedIndex];
  playheadFrame = frameAtPointer(clipElement, event, start, keptFrames(timeline.clips[selectedIndex]));
  render();
  editorOptions.onSeek(playheadFrame);
}

function frameAtPointer(element, event, firstFrame, frameCount) {
  const bounds = element.getBoundingClientRect();
  const fraction = Math.min(1, Math.max(0, (event.clientX - bounds.left) / bounds.width));
  return firstFrame + Math.min(frameCount - 1, Math.floor(fraction * frameCount));
}

function setZoom(level) {
  zoom = Math.max(MINIMUM_ZOOM, Math.min(MAXIMUM_ZOOM, level));
  render();
}

function sourceFrameAtPlayhead() {
  return timeline.clips[selectedIndex].inFrame + playheadFrame - clipStarts(timeline)[selectedIndex];
}

function moveSelectedClip(target) {
  const from = selectedIndex;
  selectedIndex = target;
  applyEdit(moveClip(timeline, from, target));
}

// the re-render puts the clip's own values back in the fields
function trySetClipRange(inFrame, outFrame) {
  try {
    applyEdit(setClipRange(timeline, selectedIndex, inFrame, outFrame));
  } catch (error) {
    if (!(error instanceof RangeError)) throw error;
    render();
  }
}

function handleFieldChange(event) {
  const field = event.target.dataset.field;
  if (!field) return;
  const frame = parseFrameTimecode(event.target.value, timeline.fps);
  if (frame === null) {
    render();
    return;
  }
  const clip = timeline.clips[selectedIndex];
  if (field === 'in') trySetClipRange(frame, clip.outFrame);
  else trySetClipRange(clip.inFrame, frame);
}

function handleDragOver(event) {
  if (event.target.closest('[data-track]')) event.preventDefault();
}

async function handleDrop(event) {
  const trackElement = event.target.closest('[data-track]');
  if (!trackElement) return;
  event.preventDefault();
  const assetId = Number.parseInt(event.dataTransfer.getData('text/plain'), ASSET_ID_RADIX);
  // text dragged in from outside the assets list
  if (Number.isNaN(assetId)) return;
  const track = trackElement.dataset.track;
  if (editorOptions.assetKind(assetId) !== track) return;
  const clipElement = event.target.closest('[data-clip-index]');
  const clipIndex = clipElement ? Number(clipElement.dataset.clipIndex) : null;
  await dropOnTrack[track](assetId, clipIndex);
}

const dropOnTrack = {
  picture: async (assetId, clipIndexUnderDrop) => {
    const frames = await editorOptions.sourceFrames(assetId);
    const clipCount = timeline.clips.length;
    const index = clipIndexUnderDrop === null ? clipCount : Math.min(clipIndexUnderDrop + 1, clipCount);
    if (selectedIndex !== null && index <= selectedIndex) selectedIndex += 1;
    const clip = {
      picture: assetId,
      frames,
      inFrame: 0,
      outFrame: frames,
      sound: { kind: 'embedded', asset: null },
      subtitle: null,
    };
    applyEdit(addClip(timeline, clip, index));
  },
  sound: (assetId, clipIndex) =>
    applyEdit(
      editedCopy(timeline, (copy) => {
        if (clipIndex === null) copy.sound = assetId;
        else copy.clips[clipIndex].sound = { kind: 'file', asset: assetId };
      }),
    ),
  subtitle: (assetId, clipIndex) =>
    applyEdit(
      editedCopy(timeline, (copy) => {
        if (clipIndex === null) copy.subtitle = assetId;
        else copy.clips[clipIndex].subtitle = assetId;
      }),
    ),
};
