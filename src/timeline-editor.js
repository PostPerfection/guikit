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
const PICTURE_ONLY_HINT = 'The JPEG 2000 timeline previews picture only';
const PRIMARY_BUTTON_PRESSED = 1;
const MAIN_BUTTON = 0;
const DRAG_START_PIXELS = 3;
const SNAP_PIXELS = 6;

let panel = null;
let editorOptions = null;
let timeline = null;
let selectedIndex = null;
let playheadFrame = 0;
let zoom = MINIMUM_ZOOM;
// set while the preview plays the whole timeline
let timelineLoaded = false;
let loadedTimelinePictureOnly = false;
// set from a clip's pointerdown to its pointerup
let clipDrag = null;
// the click that ends a clip drag must not select or seek
let clickEndsClipDrag = false;

export function initTimelineEditor(container, options) {
  panel = container;
  editorOptions = options;
  timeline = { fps: options.fps, clips: [], sound: null, subtitle: null, reelStarts: [] };
  selectedIndex = null;
  playheadFrame = 0;
  zoom = MINIMUM_ZOOM;
  timelineLoaded = false;
  loadedTimelinePictureOnly = false;
  clipDrag = null;
  clickEndsClipDrag = false;
  panel.classList.add('timeline-editor');
  panel.addEventListener('click', handleClick);
  panel.addEventListener('pointerdown', handlePointerDown);
  panel.addEventListener('pointermove', handlePointerMove);
  panel.addEventListener('pointerup', handlePointerUp);
  panel.addEventListener('pointercancel', handlePointerCancel);
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

// the preview's place in a loaded timeline, null while the preview plays anything else
export function followTimelinePosition(position) {
  const loaded = position !== null;
  if (loaded !== timelineLoaded) {
    timelineLoaded = loaded;
    render();
  }
  if (loaded && position.seconds !== null) setPlayhead(frameAtSeconds(timeline, position.seconds));
}

export function setReelStarts(frames) {
  const withoutReelStarts = editedCopy(timeline, (copy) => {
    copy.reelStarts = [];
  });
  applyEdit(frames.reduce((model, frame) => splitReelAt(model, frame), withoutReelStarts));
}

export function totalFrames(model) {
  return Math.max(0, ...model.clips.map(clipEnd));
}

export function clipStarts(model) {
  return model.clips.map((clip) => clip.start);
}

export function clipEnd(clip) {
  return clip.start + keptFrames(clip);
}

export function layerEnd(model, layer) {
  return Math.max(0, ...model.clips.filter((clip) => clip.layer === layer).map(clipEnd));
}

// the layers holding a clip, and layer 0 always, bottom first
export function layersOf(model) {
  return [...new Set([0, ...model.clips.map((clip) => clip.layer)])].sort((first, second) => first - second);
}

export function baseClipsInOrder(model) {
  return model.clips
    .map((clip, index) => ({ index, clip }))
    .filter(({ clip }) => clip.layer === 0)
    .sort((first, second) => first.clip.start - second.clip.start);
}

// the layer 0 clip under a timeline frame, null in a gap and past either end
export function clipAtFrame(model, frame) {
  const base = baseClipsInOrder(model).find(({ clip }) => frame >= clip.start && frame < clipEnd(clip));
  if (!base) return null;
  return { index: base.index, frameInClip: frame - base.clip.start };
}

export function clipSourceFrameAt(model, frame) {
  const place = clipAtFrame(model, frame);
  if (!place) return null;
  return { index: place.index, sourceFrame: model.clips[place.index].inFrame + place.frameInClip };
}

export function frameAtSeconds(model, seconds) {
  const [numerator, denominator] = model.fps;
  return Math.round((seconds * numerator) / denominator);
}

export function secondsAtFrame(model, frame) {
  const [numerator, denominator] = model.fps;
  return (frame * denominator) / numerator;
}

export function embeddedSound() {
  return { kind: 'embedded', asset: null };
}

function noSound() {
  return { kind: 'none', asset: null };
}

export function addClip(model, clip) {
  refuseOverlap(model, null, clip);
  return editedCopy(model, (copy) => {
    copy.clips.push(structuredClone(clip));
  });
}

export function removeClip(model, index) {
  checkClipIndex(model, index);
  const edited = editedCopy(model, (copy) => {
    copy.clips.splice(index, 1);
  });
  return withReelStartsInsideTimeline(edited);
}

// postkit plays the sound of layer 0 clips only, and the editor shows no opacity on layer 0
export function moveClipTo(model, index, start, layer) {
  checkClipIndex(model, index);
  if (!Number.isInteger(start) || start < 0) throw new RangeError(`clip ${index}: start ${start} is not a timeline frame`);
  if (!Number.isInteger(layer) || layer < 0) throw new RangeError(`clip ${index}: layer ${layer} is not a layer`);
  const clip = model.clips[index];
  const moved = { ...structuredClone(clip), start, layer };
  if (layer === 0 && clip.layer !== 0) {
    moved.sound = embeddedSound();
    moved.opacity = 1;
  }
  if (layer !== 0 && clip.layer === 0) moved.sound = noSound();
  refuseOverlap(model, index, moved);
  const edited = editedCopy(model, (copy) => {
    copy.clips[index] = moved;
  });
  return withReelStartsInsideTimeline(edited);
}

export function setClipOpacity(model, index, opacity) {
  checkClipIndex(model, index);
  if (!(opacity >= 0 && opacity <= 1)) throw new RangeError(`clip ${index}: opacity ${opacity} is outside 0 to 1`);
  return editedCopy(model, (copy) => {
    copy.clips[index].opacity = opacity;
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
  refuseOverlap(model, index, { ...model.clips[index], inFrame, outFrame });
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

// placedIndex is the clip being placed, null for a new one
function refuseOverlap(model, placedIndex, placed) {
  const end = clipEnd(placed);
  const other = model.clips.findIndex(
    (clip, index) => index !== placedIndex && clip.layer === placed.layer && clip.start < end && placed.start < clipEnd(clip),
  );
  if (other === -1) return;
  const { start: otherStart } = model.clips[other];
  throw new RangeError(
    `frames ${placed.start} to ${end} on layer ${placed.layer} overlap clip ${other}, which runs from ${otherStart} to ${clipEnd(model.clips[other])}`,
  );
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
export function frameTimecode(frames, fps) {
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
  panel.innerHTML = `
    <div class="timeline-editor-toolbar">
      <button class="btn-sm" type="button" data-action="zoom-out" title="Zoom out"${zoom <= MINIMUM_ZOOM ? ' disabled' : ''}>−</button>
      <button class="btn-sm" type="button" data-action="zoom-in" title="Zoom in"${zoom >= MAXIMUM_ZOOM ? ' disabled' : ''}>+</button>
      <button class="btn-sm" type="button" data-action="split-reel">Split ${editorOptions.reelWord} here</button>
      <button class="btn-sm" type="button" data-action="play-timeline"${timeline.clips.length === 0 ? ' disabled' : ''}>Play timeline</button>
      ${timelineLoaded && loadedTimelinePictureOnly ? `<span class="timeline-editor-hint">${PICTURE_ONLY_HINT}</span>` : ''}
    </div>
    <div class="timeline-editor-scroll">
      <div class="timeline-editor-body" style="width: ${zoom * PERCENT}%">
        ${renderRuler(total)}
        ${pictureRows(total)}
        ${renderTrack('data-track="sound"', 'Sound', WHOLE_TIMELINE_DROP_HINT, soundTrackContent(total))}
        ${renderTrack(subtitleTrackAttributes(), 'Subtitle', WHOLE_TIMELINE_DROP_HINT, subtitleTrackContent(total))}
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
        style="left: ${(frame / total) * PERCENT}%" title="${capitalized(editorOptions.reelWord)} starts at ${frameTimecode(frame, timeline.fps)}, click to remove"></div>`,
    );
  return `<div class="timeline-editor-ruler">${ticks.join('')}${markers.join('')}</div>`;
}

function capitalized(word) {
  return word.charAt(0).toUpperCase() + word.slice(1);
}

function renderTrack(attributes, label, labelTitle, content) {
  return `
    <div class="timeline-editor-track" ${attributes}>
      <div class="timeline-editor-track-label"${labelTitle ? ` title="${labelTitle}"` : ''}>${label}</div>
      <div class="timeline-editor-track-content">${content}</div>
    </div>`;
}

// the top layer first, with an empty row above it to drag or drop a clip onto
function pictureRows(total) {
  const layers = layersOf(timeline);
  const rows = timeline.clips.length === 0 ? layers : [...layers, layers.at(-1) + 1];
  return rows
    .reverse()
    .map((layer) => renderTrack(`data-track="picture" data-layer="${layer}"`, layerLabel(layer), '', pictureRowContent(layer, total)))
    .join('');
}

function layerLabel(layer) {
  return layer === 0 ? 'Picture' : `Layer ${layer}`;
}

function pictureRowContent(layer, total) {
  if (timeline.clips.length === 0) return '<span class="timeline-editor-empty">Drop a picture asset here</span>';
  return timeline.clips
    .map((clip, index) => {
      if (clip.layer !== layer) return '';
      const name = escapeHtml(editorOptions.assetName(clip.picture));
      const range = `${frameTimecode(clip.inFrame, timeline.fps)} to ${frameTimecode(clip.outFrame, timeline.fps)}`;
      const selected = index === selectedIndex ? ' selected' : '';
      return `<div class="timeline-editor-clip${selected}" data-clip-index="${index}" style="${spanStyle(clip.start, keptFrames(clip), total)}" title="${name}, ${range}">${name}</div>`;
    })
    .join('');
}

const clipSoundLabels = {
  embedded: () => 'embedded',
  file: (sound) => editorOptions.assetName(sound.asset),
  none: () => 'no sound',
};

function soundTrackContent(total) {
  if (timeline.sound !== null) return wholeTimelineSpan(timeline.sound, 'clear-sound');
  return timeline.clips
    .map((clip, index) => {
      if (clip.layer !== 0) return '';
      const clearButton = clip.sound.kind === 'file' ? clearSpanButton('clear-clip-sound', index) : '';
      return clipSpan(index, total, clipSoundLabels[clip.sound.kind](clip.sound), clearButton);
    })
    .join('');
}

// a clip above layer 0 has its subtitle span in the lower half of the row
function subtitleTrackAttributes() {
  const stacked = timeline.subtitle === null && timeline.clips.some((clip) => clip.layer !== 0);
  return `data-track="subtitle"${stacked ? ' data-stacked' : ''}`;
}

function subtitleTrackContent(total) {
  if (timeline.subtitle !== null) return wholeTimelineSpan(timeline.subtitle, 'clear-subtitle');
  return timeline.clips
    .map((clip, index) => {
      if (clip.subtitle === null) return clipSpan(index, total, '', '');
      const clearButton = clearSpanButton('clear-clip-subtitle', index);
      return clipSpan(index, total, editorOptions.assetName(clip.subtitle), clearButton);
    })
    .join('');
}

function clipSpan(index, total, label, clearButton) {
  const clip = timeline.clips[index];
  const empty = label === '' ? ' timeline-editor-span-empty' : '';
  const upper = clip.layer === 0 ? '' : ' timeline-editor-span-upper';
  return `<div class="timeline-editor-span${empty}${upper}" data-clip-index="${index}" style="${spanStyle(clip.start, keptFrames(clip), total)}">
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
  const opacityField =
    clip.layer === 0
      ? ''
      : `<label>Opacity <input class="timeline-editor-opacity" type="number" min="0" max="100" data-field="opacity" value="${Math.round(clip.opacity * PERCENT)}"> %</label>`;
  return `
    <div class="timeline-editor-clip-panel">
      <span class="timeline-editor-clip-name">${escapeHtml(editorOptions.assetName(clip.picture))}</span>
      <label>In <input class="timeline-editor-timecode" type="text" spellcheck="false" data-field="in" value="${frameTimecode(clip.inFrame, timeline.fps)}"></label>
      <label>Out <input class="timeline-editor-timecode" type="text" spellcheck="false" data-field="out" value="${frameTimecode(clip.outFrame, timeline.fps)}"></label>
      <button class="btn-sm" type="button" data-action="set-in">Set in from playhead</button>
      <button class="btn-sm" type="button" data-action="set-out">Set out from playhead</button>
      <label>Start <input class="timeline-editor-timecode" type="text" spellcheck="false" data-field="start" value="${frameTimecode(clip.start, timeline.fps)}"></label>
      ${opacityField}
      <button class="btn-sm" type="button" data-action="remove-clip">Remove</button>
    </div>`;
}

const clickActions = {
  'zoom-in': () => setZoom(zoom * ZOOM_STEP),
  'zoom-out': () => setZoom(zoom / ZOOM_STEP),
  'split-reel': () => applyEdit(splitReelAt(timeline, playheadFrame)),
  'remove-reel-start': (element) => applyEdit(removeReelStart(timeline, Number(element.dataset.frame))),
  'play-timeline': () => playWholeTimeline(),
  'set-in': () => setPointFromPlayhead('in'),
  'set-out': () => setPointFromPlayhead('out'),
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
        copy.clips[Number(element.dataset.targetClip)].sound = embeddedSound();
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
  if (clickEndsClipDrag) {
    clickEndsClipDrag = false;
    return;
  }
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
  if (ruler) seekAtRuler(ruler, event);
}

function handlePointerDown(event) {
  clickEndsClipDrag = false;
  const clipElement = event.target.closest('.timeline-editor-clip');
  if (!clipElement || event.button !== MAIN_BUTTON) return;
  const index = Number(clipElement.dataset.clipIndex);
  const clip = timeline.clips[index];
  const rows = [...panel.querySelectorAll('.timeline-editor-track[data-layer]')].map((row) => ({
    layer: Number(row.dataset.layer),
    bounds: row.getBoundingClientRect(),
    content: row.querySelector('.timeline-editor-track-content'),
  }));
  const rowWidth = rows.find((row) => row.layer === clip.layer).content.getBoundingClientRect().width;
  clipDrag = {
    index,
    element: clipElement,
    rows,
    pointerStartX: event.clientX,
    framesPerPixel: totalFrames(timeline) / rowWidth,
    start: clip.start,
    layer: clip.layer,
    pointerId: event.pointerId,
    moved: false,
  };
}

// a clip preview would reload its file at every step of a drag
function handlePointerMove(event) {
  if (clipDrag) {
    dragClip(event);
    return;
  }
  if (!timelineLoaded || (event.buttons & PRIMARY_BUTTON_PRESSED) === 0) return;
  const ruler = event.target.closest('.timeline-editor-ruler');
  if (ruler) seekAtRuler(ruler, event);
}

function dragClip(event) {
  const clip = timeline.clips[clipDrag.index];
  const offsetPixels = event.clientX - clipDrag.pointerStartX;
  const row = clipDrag.rows.find(({ bounds }) => event.clientY >= bounds.top && event.clientY < bounds.bottom);
  const layer = row ? row.layer : clipDrag.layer;
  if (!clipDrag.moved && Math.abs(offsetPixels) < DRAG_START_PIXELS && layer === clip.layer) return;
  // capturing on the press would send the click to the panel, and a plain click would never select a clip
  if (!clipDrag.moved) panel.setPointerCapture(clipDrag.pointerId);
  clipDrag.moved = true;
  const proposedStart = Math.max(0, clip.start + Math.round(offsetPixels * clipDrag.framesPerPixel));
  clipDrag.start = snappedStart(clipDrag.index, proposedStart, SNAP_PIXELS * clipDrag.framesPerPixel);
  if (layer !== clipDrag.layer) row.content.append(clipDrag.element);
  clipDrag.layer = layer;
  clipDrag.element.classList.add('dragging');
  clipDrag.element.style.left = `${(clipDrag.start / totalFrames(timeline)) * PERCENT}%`;
}

// the start that puts the clip's start or end on another clip's edge or the playhead within the tolerance
function snappedStart(index, proposedStart, toleranceFrames) {
  const length = keptFrames(timeline.clips[index]);
  const otherEdges = timeline.clips.flatMap((clip, other) => (other === index ? [] : [clip.start, clipEnd(clip)]));
  const distance = (start) => Math.abs(start - proposedStart);
  const snaps = [playheadFrame, ...otherEdges]
    .flatMap((edge) => [edge, edge - length])
    .filter((start) => start >= 0 && distance(start) <= toleranceFrames)
    .sort((first, second) => distance(first) - distance(second));
  return snaps[0] ?? proposedStart;
}

function handlePointerUp() {
  const drag = clipDrag;
  clipDrag = null;
  if (!drag?.moved) return;
  clickEndsClipDrag = true;
  selectedIndex = drag.index;
  const clip = timeline.clips[drag.index];
  if (drag.start === clip.start && drag.layer === clip.layer) {
    render();
    return;
  }
  tryEdit(() => moveClipTo(timeline, drag.index, drag.start, drag.layer));
}

function handlePointerCancel() {
  if (!clipDrag) return;
  clipDrag = null;
  render();
}

function seekAtRuler(ruler, event) {
  const total = totalFrames(timeline);
  if (total === 0) return;
  const frame = frameAtPointer(ruler, event, 0, total);
  setPlayhead(frame);
  seekTo(frame);
}

function seekTo(frame) {
  if (timelineLoaded) editorOptions.seekTimeline(secondsAtFrame(timeline, frame));
  else editorOptions.onSeek(frame);
}

function selectClip(clipElement, event) {
  selectedIndex = Number(clipElement.dataset.clipIndex);
  const clip = timeline.clips[selectedIndex];
  playheadFrame = frameAtPointer(clipElement, event, clip.start, keptFrames(clip));
  render();
  seekTo(playheadFrame);
}

async function playWholeTimeline() {
  const playback = await editorOptions.playTimeline(structuredClone(timeline));
  if (playback === null) return;
  timelineLoaded = true;
  loadedTimelinePictureOnly = playback.pictureOnly;
  render();
}

function frameAtPointer(element, event, firstFrame, frameCount) {
  return firstFrame + Math.min(frameCount - 1, Math.floor(pointerFraction(element, event) * frameCount));
}

function pointerFraction(element, event) {
  const bounds = element.getBoundingClientRect();
  return Math.min(1, Math.max(0, (event.clientX - bounds.left) / bounds.width));
}

function setZoom(level) {
  zoom = Math.max(MINIMUM_ZOOM, Math.min(MAXIMUM_ZOOM, level));
  render();
}

function sourceFrameAtPlayhead() {
  const clip = timeline.clips[selectedIndex];
  return clip.inFrame + playheadFrame - clip.start;
}

const PLAYHEAD_POINT_RANGES = {
  in: (clip, sourceFrame) => [sourceFrame, clip.outFrame],
  out: (clip, sourceFrame) => [clip.inFrame, sourceFrame],
};

// a loaded timeline trims the clip under the playhead, not the selected one
function setPointFromPlayhead(point) {
  const target = timelineLoaded
    ? clipSourceFrameAt(timeline, playheadFrame)
    : { index: selectedIndex, sourceFrame: sourceFrameAtPlayhead() };
  if (target === null) return;
  selectedIndex = target.index;
  const [inFrame, outFrame] = PLAYHEAD_POINT_RANGES[point](timeline.clips[target.index], target.sourceFrame);
  tryEdit(() => setClipRange(timeline, target.index, inFrame, outFrame));
}

// the re-render puts the clip's own values back in the fields
function tryEdit(edit) {
  try {
    applyEdit(edit());
  } catch (error) {
    if (!(error instanceof RangeError)) throw error;
    render();
  }
}

function parseOpacityPercent(text) {
  if (text.trim() === '') return null;
  const percent = Number(text);
  return Number.isFinite(percent) ? percent / PERCENT : null;
}

const FIELD_PARSERS = {
  in: parseFrameTimecode,
  out: parseFrameTimecode,
  start: parseFrameTimecode,
  opacity: parseOpacityPercent,
};

const FIELD_EDITS = {
  in: (clip, frame) => setClipRange(timeline, selectedIndex, frame, clip.outFrame),
  out: (clip, frame) => setClipRange(timeline, selectedIndex, clip.inFrame, frame),
  start: (clip, frame) => moveClipTo(timeline, selectedIndex, frame, clip.layer),
  opacity: (clip, opacity) => setClipOpacity(timeline, selectedIndex, opacity),
};

function handleFieldChange(event) {
  const field = event.target.dataset.field;
  if (!field) return;
  const value = FIELD_PARSERS[field](event.target.value, timeline.fps);
  if (value === null) {
    render();
    return;
  }
  tryEdit(() => FIELD_EDITS[field](timeline.clips[selectedIndex], value));
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
  if (track !== 'picture') {
    applyEdit(dropOnTrack[track](assetId, clipIndex));
    return;
  }
  const layer = Number(trackElement.dataset.layer);
  await dropPicture(assetId, layer, pictureDropStart(event, clipIndex, layer));
}

// after the clip under the pointer, at the pointer on empty track, after the row's last clip on its label
function pictureDropStart(event, clipIndex, layer) {
  if (clipIndex !== null) return clipEnd(timeline.clips[clipIndex]);
  const content = event.target.closest('.timeline-editor-track-content');
  if (content) return Math.round(pointerFraction(content, event) * totalFrames(timeline));
  return layerEnd(timeline, layer);
}

async function dropPicture(assetId, layer, start) {
  const frames = await editorOptions.sourceFrames(assetId);
  const clip = {
    picture: assetId,
    frames,
    start,
    layer,
    opacity: 1,
    inFrame: 0,
    outFrame: frames,
    sound: layer === 0 ? embeddedSound() : noSound(),
    subtitle: null,
  };
  tryEdit(() => addClip(timeline, clip));
}

const dropOnTrack = {
  sound: (assetId, clipIndex) =>
    editedCopy(timeline, (copy) => {
      if (clipIndex === null) copy.sound = assetId;
      else copy.clips[clipIndex].sound = { kind: 'file', asset: assetId };
    }),
  subtitle: (assetId, clipIndex) =>
    editedCopy(timeline, (copy) => {
      if (clipIndex === null) copy.subtitle = assetId;
      else copy.clips[clipIndex].subtitle = assetId;
    }),
};
