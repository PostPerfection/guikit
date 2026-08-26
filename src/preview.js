// Preview player - uses mpv via IPC for high-performance video playback
import { invoke } from '@tauri-apps/api/core';

let scrubberInterval = null;
let isSeeking = false;
let isEmbedded = false;
let reportSurface = () => {};
let qcControls = null;
let metadataWatcher = () => {};
let loadWatcher = () => {};

const OVERLAY_CONTROLS_ID = 'preview-controls';

// What the backend draws over the picture, sent whole on every change.
const overlays = {
  safe_area_percent: null,
  aspect_mask: null,
  centre_cross: false,
  thirds_grid: false,
  crop: null,
  crop_visible: false,
};

// The two subtitle slots mpv renders, keyed by the name the backend takes.
const SUBTITLE_TRACKS = ['subtitle', 'caption'];

export function initPreview() {
  initQcControls();
  initEmbeddedSurface();

  // Initialize scrubber
  initScrubber();
}

export function previewPlayPause() {
  invoke('preview_play_pause').catch(() => {});
}

export function previewSeek(seconds) {
  invoke('preview_seek', { seconds }).catch(() => {});
}

export function previewSeekAbsolute(seconds) {
  invoke('preview_seek_absolute', { seconds }).catch(() => {});
}

/// How far the skip buttons and an app's skip shortcuts move, so the two cannot
/// disagree.
export const PREVIEW_SEEK_SECONDS = 5;

/// Both frame steps pause playback, which is mpv's behaviour.
export function previewFrameStepForward() {
  invoke('preview_frame_step').catch(() => {});
}

export function previewFrameStepBack() {
  invoke('preview_frame_back_step').catch(() => {});
}

/// Read every metadata poll, one watcher at a time. The playlist takes the
/// end-of-file flag and the composition title from it, so it needs no timer.
export function watchPreviewMetadata(watcher) {
  metadataWatcher = watcher;
}

/// Read every load the page asks for, one watcher at a time, as the file or
/// directory handed over. The playlist lets go of its queue on a load it did not
/// make itself.
export function watchPreviewLoads(watcher) {
  loadWatcher = watcher;
}

export function isPreviewVisible() {
  const panel = document.getElementById('preview-panel');
  return !!panel && !panel.hidden;
}

// The QC strip lives in the panel header, built here so an app needs no markup
// of its own for it.
function initQcControls() {
  const panel = document.getElementById('preview-panel');
  const header = panel?.querySelector('.preview-panel-header');
  if (!header || document.getElementById(OVERLAY_CONTROLS_ID)) return;

  const strip = document.createElement('div');
  strip.id = OVERLAY_CONTROLS_ID;
  strip.className = 'preview-controls';
  strip.innerHTML = `
    <label>Safe
      <select id="preview-safe-area">
        <option value="">off</option>
        <option value="95">95%</option>
        <option value="90">90%</option>
      </select>
    </label>
    <label>Aspect
      <select id="preview-aspect-mask">
        <option value="">off</option>
        <option value="1.85">1.85</option>
        <option value="1.9">1.90</option>
        <option value="2.39">2.39</option>
      </select>
    </label>
    <button id="preview-centre-cross" class="btn-sm" title="Centre cross">Cross</button>
    <button id="preview-thirds-grid" class="btn-sm" title="Rule of thirds grid">Thirds</button>
    <button id="preview-crop" class="btn-sm" title="Crop the job applies" disabled>Crop</button>
    <label>Decode
      <select id="preview-decode-scale">
        <option value="full">Full</option>
        <option value="half">Half</option>
        <option value="quarter">Quarter</option>
      </select>
    </label>
    <button id="preview-subtitles" class="btn-sm" title="Subtitles" disabled>Sub</button>
    <button id="preview-captions" class="btn-sm" title="Closed captions" disabled>CC</button>
    <span id="preview-hud" class="preview-hud"></span>`;
  header.insertBefore(strip, document.getElementById('preview-close'));

  qcControls = {
    safeArea: strip.querySelector('#preview-safe-area'),
    aspectMask: strip.querySelector('#preview-aspect-mask'),
    centreCross: strip.querySelector('#preview-centre-cross'),
    thirdsGrid: strip.querySelector('#preview-thirds-grid'),
    crop: strip.querySelector('#preview-crop'),
    decodeScale: strip.querySelector('#preview-decode-scale'),
    trackToggles: {
      subtitle: strip.querySelector('#preview-subtitles'),
      caption: strip.querySelector('#preview-captions'),
    },
    hud: strip.querySelector('#preview-hud'),
  };

  qcControls.safeArea.addEventListener('change', () => {
    overlays.safe_area_percent = qcControls.safeArea.value ? Number(qcControls.safeArea.value) : null;
    applyOverlays();
  });
  qcControls.aspectMask.addEventListener('change', () => {
    overlays.aspect_mask = qcControls.aspectMask.value ? Number(qcControls.aspectMask.value) : null;
    applyOverlays();
  });
  bindOverlayToggle(qcControls.centreCross, 'centre_cross');
  bindOverlayToggle(qcControls.thirdsGrid, 'thirds_grid');
  bindOverlayToggle(qcControls.crop, 'crop_visible');
  for (const track of SUBTITLE_TRACKS) bindTrackToggle(track);
  qcControls.decodeScale.addEventListener('change', async () => {
    try {
      await invoke('preview_set_decode_scale', { scale: qcControls.decodeScale.value });
    } catch (e) {
      console.error('[preview] Failed to set decode scale:', e);
      return;
    }
    // a source that declares no picture size has its crop measured from the
    // decoded frame, so the drawing is measured again for the scale now in force
    applyOverlays();
  });
}

function bindOverlayToggle(button, field) {
  button.addEventListener('click', () => {
    overlays[field] = !overlays[field];
    button.classList.toggle('primary', overlays[field]);
    applyOverlays();
  });
}

function bindTrackToggle(track) {
  const button = qcControls.trackToggles[track];
  button.addEventListener('click', () => {
    const visible = !button.classList.contains('primary');
    invoke('preview_set_subtitle_visibility', { track, visible })
      .then(() => button.classList.toggle('primary', visible))
      .catch((e) => console.error('[preview] Failed to set subtitle visibility:', e));
  });
}

function applyOverlays() {
  invoke('preview_set_overlays', { overlays }).catch((e) => {
    console.error('[preview] Failed to set overlays:', e);
  });
}

/// Show the crop the job will apply, as pixels off each edge of the source
/// picture, or null for none. Setting the first crop switches the overlay on,
/// clearing it switches it off, and in between the Crop button rules.
export function setPreviewCrop(crop) {
  if (!crop) {
    overlays.crop_visible = false;
  } else if (!overlays.crop) {
    overlays.crop_visible = true;
  }
  overlays.crop = crop;
  if (qcControls) {
    qcControls.crop.disabled = !crop;
    qcControls.crop.classList.toggle('primary', overlays.crop_visible);
  }
  applyOverlays();
}

/// Render a subtitle file over playback as the bottom track, or null to drop it.
/// Only what libass reads natively: SRT, ASS or SSA and WebVTT, so a wizard
/// converts its subtitle XML to SRT first. The clip has to be loaded already.
export function setPreviewSubtitleFile(filePath) {
  setTrackFile('subtitle', filePath);
}

/// The same for a caption file, which mpv renders at the top of the frame.
export function setPreviewCaptionFile(filePath) {
  setTrackFile('caption', filePath);
}

function setTrackFile(track, filePath) {
  invoke('preview_set_subtitle_file', { track, filePath })
    .then(() => {
      if (!qcControls) return;
      const button = qcControls.trackToggles[track];
      button.disabled = !filePath;
      button.classList.toggle('primary', !!filePath);
    })
    .catch((e) => console.error('[preview] Failed to set subtitle file:', e));
}

function resetOverlays() {
  if (!qcControls) return;
  overlays.safe_area_percent = null;
  overlays.aspect_mask = null;
  overlays.centre_cross = false;
  overlays.thirds_grid = false;
  overlays.crop = null;
  overlays.crop_visible = false;
  qcControls.safeArea.value = '';
  qcControls.aspectMask.value = '';
  qcControls.centreCross.classList.remove('primary');
  qcControls.thirdsGrid.classList.remove('primary');
  qcControls.crop.classList.remove('primary');
  qcControls.crop.disabled = true;
  applyOverlays();
}

// The backend drops the subtitle tracks with the file they were added to.
function resetTrackToggles() {
  if (!qcControls) return;
  for (const track of SUBTITLE_TRACKS) {
    qcControls.trackToggles[track].classList.remove('primary');
    qcControls.trackToggles[track].disabled = true;
  }
}

// The video is a native surface the app draws over #preview-surface, so the
// page's only job is telling the backend where that element ended up.
async function initEmbeddedSurface() {
  const panel = document.getElementById('preview-panel');
  const surface = document.getElementById('preview-surface');
  if (!panel || !surface) return;

  isEmbedded = await invoke('preview_is_embedded').catch(() => false);
  if (!isEmbedded) return;

  const report = () => {
    const visible = !panel.hidden;
    const rect = surface.getBoundingClientRect();
    invoke('preview_set_surface', {
      x: Math.round(rect.left),
      y: Math.round(rect.top),
      width: Math.round(rect.width),
      height: Math.round(rect.height),
      visible,
    }).catch(() => {});
  };

  new ResizeObserver(report).observe(surface);
  window.addEventListener('resize', report);
  document.addEventListener('scroll', report, true);

  document.getElementById('preview-close')?.addEventListener('click', closePreview);

  reportSurface = report;
  report();
}

/// Stop the player, leaving the panel where it is. The tracks go with the file
/// the backend drops.
export function stopPreview() {
  invoke('preview_stop').catch(() => {});
  resetTrackToggles();
}

/// Stop the player and take the panel off the page, which is what the panel's own
/// ✕ does. The playlist calls it when the last of its rows goes, so nothing keeps
/// playing behind a hidden panel.
export function closePreview() {
  stopPreview();
  resetOverlays();
  const panel = document.getElementById('preview-panel');
  if (panel) panel.hidden = true;
  reportSurface();
}

export function showEmbeddedPanel() {
  if (!isEmbedded) return;
  const panel = document.getElementById('preview-panel');
  if (panel) panel.hidden = false;
  reportSurface();
}

// The transport buttons an app puts in its own markup, wired by id, each one
// optional. The skip buttons take their tooltip from PREVIEW_SEEK_SECONDS.
const TRANSPORT_BUTTONS = [
  { id: 'timeline-start-btn', onClick: () => previewSeekAbsolute(0) },
  {
    id: 'timeline-skip-back-btn',
    onClick: () => previewSeek(-PREVIEW_SEEK_SECONDS),
    title: `Back ${PREVIEW_SEEK_SECONDS} seconds`,
  },
  { id: 'timeline-frame-back-btn', onClick: previewFrameStepBack },
  { id: 'timeline-play-btn', onClick: previewPlayPause },
  { id: 'timeline-frame-forward-btn', onClick: previewFrameStepForward },
  {
    id: 'timeline-skip-forward-btn',
    onClick: () => previewSeek(PREVIEW_SEEK_SECONDS),
    title: `Forward ${PREVIEW_SEEK_SECONDS} seconds`,
  },
];

function initScrubber() {
  const scrubber = document.getElementById('timeline-scrubber');
  const durLabel = document.getElementById('timeline-duration');

  if (!scrubber) return;

  // Click to seek. The pointer is captured for the drag: the video under the
  // scrubber is a native widget, not part of this page, so a plain mouseup
  // released over it never arrives and the seek would stay latched, freezing
  // every poll-driven control until some later click landed in the page.
  scrubber.addEventListener('pointerdown', (e) => {
    scrubber.setPointerCapture(e.pointerId);
    isSeeking = true;
    seekToMouse(e);
  });
  scrubber.addEventListener('pointermove', (e) => {
    if (isSeeking) seekToMouse(e);
  });
  const endSeek = () => {
    isSeeking = false;
  };
  scrubber.addEventListener('pointerup', endSeek);
  scrubber.addEventListener('pointercancel', endSeek);
  scrubber.addEventListener('lostpointercapture', endSeek);

  function seekToMouse(e) {
    const rect = scrubber.getBoundingClientRect();
    const pct = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
    const dur = parseFloat(durLabel?.dataset.raw || '0');
    if (dur > 0) {
      invoke('preview_seek_absolute', { seconds: pct * dur }).catch(() => {});
      updatePlayhead(pct);
    }
  }

  for (const { id, onClick, title } of TRANSPORT_BUTTONS) {
    const button = document.getElementById(id);
    if (!button) continue;
    button.addEventListener('click', onClick);
    if (title) button.title = title;
  }

  // Start position polling
  startScrubberPolling();
}

// the last poll failure, so a repeating one logs once instead of four times a second
let lastPollError = '';

function startScrubberPolling() {
  if (scrubberInterval) return;
  scrubberInterval = setInterval(async () => {
    if (isSeeking) return;
    try {
      const resp = await invoke('preview_get_metadata');
      const meta = JSON.parse(resp);
      lastPollError = '';
      updateHud(meta);
      metadataWatcher(meta);
      if (meta.position != null && meta.duration != null && meta.duration > 0) {
        const pct = meta.position / meta.duration;
        updatePlayhead(pct);
        updateTimecode(meta.position, meta.duration, meta.container_fps);
      }
      updatePlayBtn(meta.paused);
    } catch (error) {
      const text = String(error);
      if (text !== lastPollError) {
        lastPollError = text;
        console.error('metadata poll:', error);
      }
    }
  }, 250);
}

export function stopScrubberPolling() {
  if (scrubberInterval) {
    clearInterval(scrubberInterval);
    scrubberInterval = null;
  }
}

function updatePlayhead(pct) {
  const playhead = document.getElementById('timeline-playhead');
  if (playhead) {
    playhead.style.left = `${(pct * 100).toFixed(2)}%`;
  }
}

function updateTimecode(pos, dur, fps) {
  const posLabel = document.getElementById('timeline-position');
  const durLabel = document.getElementById('timeline-duration');
  if (posLabel) posLabel.textContent = formatTimecode(pos, fps);
  if (durLabel) {
    durLabel.textContent = formatTimecode(dur, fps);
    durLabel.dataset.raw = String(dur);
  }
}

function updateHud(meta) {
  if (!qcControls) return;
  const parts = [];
  if (meta.position != null && meta.container_fps > 0) {
    const counted = Math.floor(meta.position * meta.container_fps) + 1;
    const total = meta.duration > 0 ? Math.round(meta.duration * meta.container_fps) : null;
    // at the end mpv reports the whole duration, which counts as the frame after the last one
    const frame = total ? Math.min(counted, total) : counted;
    parts.push(total ? `frame ${frame}/${total}` : `frame ${frame}`);
  }
  if (meta.decoder_fps != null) parts.push(`${meta.decoder_fps.toFixed(2)} fps`);
  if (meta.cache_seconds != null) parts.push(`buffer ${meta.cache_seconds.toFixed(1)}s`);
  if (meta.dropped_frames != null) parts.push(`dropped ${meta.dropped_frames}`);
  if (meta.delayed_frames) parts.push(`delayed ${meta.delayed_frames}`);
  qcControls.hud.textContent = parts.join('  ');
}

function updatePlayBtn(paused) {
  const playBtn = document.getElementById('timeline-play-btn');
  if (playBtn) {
    playBtn.textContent = paused ? '▶' : '⏸';
    playBtn.title = paused ? 'Play' : 'Pause';
  }
}

const FALLBACK_TIMECODE_FPS = 24;

// the frame field counts at the container's rate, or 24 until mpv reports one
function formatTimecode(seconds, fps) {
  if (!seconds || seconds < 0) return '00:00:00:00';
  const rate = fps > 0 ? fps : FALLBACK_TIMECODE_FPS;
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const s = Math.floor(seconds % 60);
  const f = Math.floor((seconds % 1) * rate);
  return `${String(h).padStart(2, '0')}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}:${String(f).padStart(2, '0')}`;
}

/// Load a file into the preview player
export function previewFile(filePath) {
  showEmbeddedPanel();
  loadWatcher(filePath);
  invoke('preview_load', { filePath }).catch((e) => {
    console.error('[preview] Failed to load:', e);
  });
  resetTrackToggles();
  startScrubberPolling();
}

/// Load a DCP directory into the preview player
export function previewDcp(dirPath) {
  showEmbeddedPanel();
  loadWatcher(dirPath);
  invoke('preview_load_dcp', { dirPath }).catch((e) => {
    console.error('[preview] Failed to load DCP:', e);
  });
  resetTrackToggles();
  startScrubberPolling();
}
