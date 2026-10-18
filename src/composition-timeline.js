import {
  clipAtFrame,
  clipStarts,
  embeddedSound,
  frameTimecode,
  layerEnd,
  splitReelAt,
  totalFrames,
  withReelStartsInsideTimeline,
} from './timeline-editor.js';

export { embeddedSound };

// postkit's TIMELINE_FORMAT_VERSION, which the wizards' submit_job checks
const TIMELINE_FORMAT_VERSION = 2;
const SECONDS_PER_MINUTE = 60;
const MINUTES_PER_HOUR = 60;
const SPLIT_TIMECODE_SEPARATOR = ', ';

export const DEFAULT_TIMELINE_FPS = [24, 1];

export const TIMELINE_KIND_BY_ASSET_TYPE = { video: 'picture', audio: 'sound', subtitle: 'subtitle' };

export function emptyTimeline(fps) {
  return { fps, clips: [], sound: null, subtitle: null, reelStarts: [] };
}

// 0 frames until the source is probed
function newClip(asset, start) {
  const frames = asset.frames ?? 0;
  return {
    picture: asset.id,
    frames,
    start,
    layer: 0,
    opacity: 1,
    inFrame: 0,
    outFrame: frames,
    sound: embeddedSound(),
    subtitle: null,
  };
}

export function clipHasOwnSound(clip) {
  return clip.sound.kind === 'file';
}

function clipHasOwnSubtitle(clip) {
  return clip.subtitle !== null;
}

export function assetPath(assetById, assetId) {
  return assetId === null ? null : assetById(assetId).path;
}

// a video follows the last layer 0 clip, a sound or subtitle covers the whole timeline while nothing else does
export function withImportedAsset(timeline, asset) {
  const placed = structuredClone(timeline);
  if (asset.type === 'video') placed.clips.push(newClip(asset, layerEnd(placed, 0)));
  if (asset.type === 'audio' && placed.sound === null && !placed.clips.some(clipHasOwnSound)) placed.sound = asset.id;
  if (asset.type === 'subtitle' && placed.subtitle === null && !placed.clips.some(clipHasOwnSubtitle)) {
    placed.subtitle = asset.id;
  }
  return placed;
}

function placedAssetIds(timeline) {
  const clipAssets = timeline.clips.flatMap((clip) => [clip.picture, clip.sound.asset, clip.subtitle]);
  return new Set([...clipAssets, timeline.sound, timeline.subtitle].filter((id) => id !== null));
}

// an empty timeline takes the first unplaced video, and every unplaced sound or subtitle gets the import rule
export function withUnplacedAssetsFilled(timeline, assets) {
  const placed = placedAssetIds(timeline);
  const unplaced = assets.filter((asset) => !placed.has(asset.id));
  const firstVideo = timeline.clips.length === 0 ? unplaced.find((asset) => asset.type === 'video') : undefined;
  const candidates = unplaced.filter((asset) => asset.type !== 'video');
  if (firstVideo) candidates.unshift(firstVideo);
  return candidates.reduce(withImportedAsset, timeline);
}

export function withProbedFrames(timeline, assetId, frames) {
  const probed = structuredClone(timeline);
  probed.clips.forEach((clip, index) => {
    if (clip.picture !== assetId || clip.frames !== 0) return;
    clip.frames = frames;
    if (clip.outFrame !== 0) return;
    clip.outFrame = frames;
    if (clip.layer === 0) moveBaseClipsAfter(probed, index, frames);
  });
  return probed;
}

// an unprobed clip is 0 frames long, so the layer 0 clips imported after it start at the same frame
function moveBaseClipsAfter(timeline, index, frames) {
  const { start } = timeline.clips[index];
  timeline.clips.forEach((other, otherIndex) => {
    const after = other.start > start || (other.start === start && otherIndex > index);
    if (other.layer === 0 && after) other.start += frames;
  });
}

// call once every probe has run
export function withReelStartingClipsPlaced(composition) {
  const { reelStartingClipIndices, ...placed } = composition;
  if (!reelStartingClipIndices) return composition;
  const starts = clipStarts(composition.timeline);
  placed.timeline = reelStartingClipIndices.map((index) => starts[index]).reduce(splitReelAt, composition.timeline);
  return placed;
}

export function withoutAsset(timeline, assetId) {
  const kept = structuredClone(timeline);
  kept.clips = kept.clips.filter((clip) => clip.picture !== assetId);
  for (const clip of kept.clips) {
    if (clip.sound.asset === assetId) clip.sound = embeddedSound();
    if (clip.subtitle === assetId) clip.subtitle = null;
  }
  if (kept.sound === assetId) kept.sound = null;
  if (kept.subtitle === assetId) kept.subtitle = null;
  return withReelStartsInsideTimeline(kept);
}

// an asset id the project file does not list is taken off the timeline and named
export function timelineWithKnownAssets(composition, assetsById, notRestored) {
  const timeline = structuredClone(composition.timeline);
  const known = (assetId, place) => {
    if (assetId === null || assetsById.has(assetId)) return true;
    notRestored.push(`${composition.name} ${place}`);
    return false;
  };
  timeline.clips = timeline.clips.filter((clip, index) => known(clip.picture, `clip ${index + 1} picture`));
  timeline.clips.forEach((clip, index) => {
    if (!known(clip.sound.asset, `clip ${index + 1} sound`)) clip.sound = embeddedSound();
    if (!known(clip.subtitle, `clip ${index + 1} subtitle`)) clip.subtitle = null;
  });
  if (!known(timeline.sound, 'timeline sound')) timeline.sound = null;
  if (!known(timeline.subtitle, 'timeline subtitle')) timeline.subtitle = null;
  return timeline;
}

export function parseFrameRate(rateText) {
  const [numerator, denominator = 1] = String(rateText ?? '').split('/').map(Number);
  if (!(numerator > 0) || !(denominator > 0)) return null;
  return [numerator, denominator];
}

// counted the way postkit's probe counts: the duration at the stream's rate, rounded
export function sourceFrameCount(durationSeconds, rateText) {
  const rate = parseFrameRate(rateText);
  if (!rate || !(durationSeconds > 0)) return 0;
  const [numerator, denominator] = rate;
  return Math.round((durationSeconds * numerator) / denominator);
}

// ffprobe's -show_streams -show_format JSON, null without a video stream
export function ffprobeVideoInfo(probe) {
  const stream = probe.streams?.find((each) => each.codec_type === 'video');
  if (!stream) return null;
  return {
    width: stream.width,
    height: stream.height,
    fps: stream.r_frame_rate,
    duration: parseFloat(probe.format?.duration || stream.duration || '0'),
    frames: sourceFrameCount(parseFloat(stream.duration ?? probe.format?.duration), stream.r_frame_rate),
    colorPrimaries: stream.color_primaries,
    colorTransfer: stream.color_transfer,
    colorSpace: stream.color_space,
    bitRate: parseInt(stream.bit_rate ?? probe.format?.bit_rate) || null,
  };
}

// every clip has to run at the timeline's rate, so the first clip's probed rate sets it
export function withFirstClipRate(timeline, assetById) {
  const firstClip = timeline.clips[0];
  const rate = firstClip ? parseFrameRate(assetById(firstClip.picture)?.fps) : null;
  if (!rate || (rate[0] === timeline.fps[0] && rate[1] === timeline.fps[1])) return timeline;
  return { ...timeline, fps: rate };
}

export function timelineFrameAtSourceFrame(timeline, clipIndex, sourceFrame) {
  const clip = timeline.clips[clipIndex];
  return clip.start + sourceFrame - clip.inFrame;
}

// the reel starts a split-at field holds, null while it does not parse
export function splitTimecodeFrames(spec, editRate) {
  const timecodes = spec
    .split(',')
    .map((timecode) => timecode.trim())
    .filter(Boolean);
  const frames = timecodes.map((timecode) => timecodeFrames(timecode, editRate));
  return frames.includes(null) ? null : frames;
}

function timecodeFrames(timecode, editRate) {
  const fields = timecode.split(':');
  if (fields.length !== 3 && fields.length !== 4) return null;
  if (!fields.every((field) => /^\d+$/.test(field))) return null;
  const [hours, minutes, seconds, frame = 0] = fields.map(Number);
  if (minutes >= SECONDS_PER_MINUTE || seconds >= SECONDS_PER_MINUTE || frame >= editRate) return null;
  return ((hours * MINUTES_PER_HOUR + minutes) * SECONDS_PER_MINUTE + seconds) * editRate + frame;
}

export function splitTimecodeSpec(frames, editRate) {
  return frames.map((frame) => frameTimecode(frame, [editRate, 1])).join(SPLIT_TIMECODE_SEPARATOR);
}

export function withSplitAtFrames(timeline, typedFrames) {
  if (timeline.clips.length > 1 || typedFrames === null) return timeline;
  const split = typedFrames.reduce(splitReelAt, { ...timeline, reelStarts: [] });
  // an unprobed clip has no end to measure against yet
  if ((timeline.clips[0]?.frames ?? 0) === 0) return split;
  return withReelStartsInsideTimeline(split);
}

// a clip can come twice from one file, so the clip already shown wins, then the earliest on the timeline
export function clipIndexShowing(timeline, picturePath, likelyIndex, assetById) {
  const shows = (clip) => clip !== undefined && assetPath(assetById, clip.picture) === picturePath;
  if (shows(timeline.clips[likelyIndex])) return likelyIndex;
  const showing = timeline.clips
    .map((clip, index) => ({ clip, index }))
    .filter(({ clip }) => shows(clip))
    .sort((first, second) => first.clip.start - second.clip.start);
  return showing[0]?.index ?? null;
}

// the timeline's own subtitle lines up with the source only where each source frame sits at the same timeline frame
export function clipPreviewSubtitle(timeline, clipIndex) {
  const clip = timeline.clips[clipIndex];
  const linesUp = clip.start === clip.inFrame;
  return clip.subtitle ?? (linesUp ? timeline.subtitle : null);
}

// what the first clip and the timeline give the sound panel and the package name
export function firstSoundAsset(timeline, assetById) {
  if (timeline.sound !== null) return assetById(timeline.sound);
  const clip = timeline.clips.find(clipHasOwnSound);
  return clip ? assetById(clip.sound.asset) : null;
}

export function firstSubtitleAsset(timeline, assetById) {
  const subtitle = timeline.subtitle ?? timeline.clips.find(clipHasOwnSubtitle)?.subtitle ?? null;
  return subtitle === null ? null : assetById(subtitle);
}

// one span per reel, with the assets of the layer 0 clip it starts in, a null picture where it starts in a gap
export function reelSpans(timeline, assetById) {
  if (timeline.clips.length === 0) return [];
  const total = totalFrames(timeline);
  const bounds = [0, ...timeline.reelStarts.filter((frame) => frame < total), total];
  return bounds.slice(0, -1).map((start, index) => {
    const place = clipAtFrame(timeline, start);
    const clips = place === null ? [] : [timeline.clips[place.index]];
    const subtitle = timeline.subtitle ?? clips[0]?.subtitle ?? null;
    return {
      frames: bounds[index + 1] - start,
      picture: place === null ? null : assetById(clips[0].picture),
      sound: firstSoundAsset({ ...timeline, clips }, assetById),
      subtitle: subtitle === null ? null : assetById(subtitle),
    };
  });
}

const CLIP_SOUND_WIRE = {
  embedded: () => 'Embedded',
  file: (sound, assetById) => ({ File: assetPath(assetById, sound.asset) }),
  none: () => 'None',
};

// postkit's Timeline JSON, with paths for asset ids
export function timelineWire(timeline, assetById) {
  const [numerator, denominator] = timeline.fps;
  return {
    version: TIMELINE_FORMAT_VERSION,
    fps: { numerator, denominator },
    clips: timeline.clips.map((clip) => ({
      picture: assetPath(assetById, clip.picture),
      start: clip.start,
      layer: clip.layer,
      opacity: clip.opacity,
      in_frame: clip.inFrame,
      out_frame: clip.outFrame,
      sound: CLIP_SOUND_WIRE[clip.sound.kind](clip.sound, assetById),
      subtitle: assetPath(assetById, clip.subtitle),
    })),
    sound: timeline.sound === null ? 'PerClip' : { Override: assetPath(assetById, timeline.sound) },
    subtitle: assetPath(assetById, timeline.subtitle),
    reel_starts: [...timeline.reelStarts],
  };
}
