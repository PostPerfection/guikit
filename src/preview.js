// Preview player - uses mpv via IPC for high-performance video playback
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { FULLSCREEN_HUD_IDLE_TIMEOUT_MS, fullscreenHudStaysShown, pointIsInsideAnyRectangle } from './fullscreen-hud.js';

let scrubberInterval = null;
let isSeeking = false;
let isEmbedded = false;
let reportSurface = () => {};
let qcControls = null;
let metadataWatcher = () => {};
let loadWatcher = () => {};
let shownWatcher = () => {};
let sourceWatcher = () => {};
let audioLevelsWatcher = () => {};
let polledSource = null;
let endReported = false;
// set while the page draws something the native surface would otherwise cover
let surfaceCovered = false;
let previewPaused = false;
// in page coordinates, null once a resize has moved the page under the pointer
let lastPointer = null;
let fullscreenHudTimer = null;

const OVERLAY_CONTROLS_ID = 'preview-controls';
const FULLSCREEN_CLASS = 'preview-fullscreen';
const LEAVE_FULLSCREEN_KEY = 'Escape';
// on body, hides the cursor along with the controls
const FULLSCREEN_HUD_HIDDEN_CLASS = 'preview-fullscreen-hud-hidden';
// must match the elements the stylesheet hides for FULLSCREEN_HUD_HIDDEN_CLASS
const FULLSCREEN_HUD_SELECTOR =
  '.preview-panel.preview-fullscreen > :not(.preview-surface), .main-area:has(> .preview-panel.preview-fullscreen) > .timeline-transport';

const PREVIEW_PANEL_DEFAULT_TITLE = 'Preview';
const SCRUBBER_IDLE_CLASS = 'timeline-scrubber-idle';

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

/// Read what the panel is showing, one watcher at a time, as the file or
/// directory loaded into it, or null once the player is stopped.
export function watchPreviewShown(watcher) {
  shownWatcher = watcher;
}

// what the player has loaded, one watcher at a time, each time the polled source or file name changes
export function watchPreviewSource(watcher) {
  sourceWatcher = watcher;
}

// the poll's audio_levels, one watcher at a time, null while the level meter is off or nothing sounds
export function watchPreviewAudioLevels(watcher) {
  audioLevelsWatcher = watcher;
}

export function isPreviewVisible() {
  const panel = document.getElementById('preview-panel');
  return !!panel && !panel.hidden;
}

// The QC strip sits under the picture, not in the header. The native GL view
// is a sibling over #preview-surface and covers anything in that rectangle,
// including a header row that wrapped into it.
function initQcControls() {
  const panel = document.getElementById('preview-panel');
  const surface = panel?.querySelector('.preview-surface');
  if (!panel || !surface) return;

  let strip = document.getElementById(OVERLAY_CONTROLS_ID);
  if (!strip) {
    strip = document.createElement('div');
    strip.id = OVERLAY_CONTROLS_ID;
    strip.className = 'preview-controls';
    surface.insertAdjacentElement('afterend', strip);
  }
  if (strip.dataset.ready) return;
  strip.className = 'preview-controls';
  strip.innerHTML = `
    <span class="preview-controls-label">QC</span>
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
  strip.dataset.ready = '1';

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

// ffmpeg filter strings run on the picture mpv plays, null plays the source as it is
export function setPreviewPictureFilters(filters) {
  return invoke('preview_set_picture_filters', { filters }).catch((e) => {
    console.error('[preview] Failed to set picture filters:', e);
  });
}

// false for a source the grok player plays
export function previewTakesPictureFilters(path) {
  return invoke('preview_takes_picture_filters', { path });
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

  document.getElementById('preview-close')?.addEventListener('click', closePreview);
  document
    .getElementById('preview-fullscreen')
    ?.addEventListener('click', () => setPreviewFullscreen(!isPreviewFullscreen()));
  // on window so the shortcuts overlay sees Escape first and can claim it
  window.addEventListener('keydown', (event) => {
    if (event.key !== LEAVE_FULLSCREEN_KEY || event.defaultPrevented || !isPreviewFullscreen()) return;
    event.preventDefault();
    setPreviewFullscreen(false);
  });

  reportSurfacePlacement(surface, () => !panel.hidden);
}

// the video fills the whole page
export function initFullPageSurface() {
  reportSurfacePlacement(document.getElementById('preview-surface'), () => true);
}

// for a load the app made without the page, such as a playlist it runs
export function enablePreviewTransport() {
  setTransportEnabled(true);
  startScrubberPolling();
}

// for a window shown only while something plays
export function initLiveTransport() {
  initScrubber();
  setTransportEnabled(true);
}

function reportSurfacePlacement(surface, isSurfaceShown) {
  let sentPlacement = null;
  const sendPlacement = (onlyIfChanged) => {
    const rect = surface.getBoundingClientRect();
    const placement = {
      x: Math.round(rect.left),
      y: Math.round(rect.top),
      width: Math.round(rect.width),
      height: Math.round(rect.height),
      visible: isSurfaceShown() && !surfaceCovered,
    };
    const unchanged =
      sentPlacement !== null && Object.keys(placement).every((key) => placement[key] === sentPlacement[key]);
    if (onlyIfChanged && unchanged) return;
    sentPlacement = placement;
    invoke('preview_set_surface', placement).catch(() => {});
  };
  const report = () => sendPlacement(false);
  // hiding the surface already sent the hidden placement
  const reportIfMoved = () => {
    if (isSurfaceShown()) sendPlacement(true);
  };

  // the panel growing under the surface moves it without resizing it
  const observer = new ResizeObserver(reportIfMoved);
  observer.observe(surface);
  observer.observe(surface.parentElement);
  window.addEventListener('resize', reportIfMoved);
  document.addEventListener('scroll', reportIfMoved, true);

  reportSurface = report;
  report();
}

function isPreviewFullscreen() {
  return !!document.getElementById('preview-panel')?.classList.contains(FULLSCREEN_CLASS);
}

function setPreviewFullscreen(fullscreen) {
  document.getElementById('preview-panel')?.classList.toggle(FULLSCREEN_CLASS, fullscreen);
  if (fullscreen) startFullscreenHud();
  else stopFullscreenHud();
  getCurrentWindow()
    .setFullscreen(fullscreen)
    .catch((e) => console.error('[preview] Failed to set full screen:', e));
  reportSurface();
}

function setFullscreenHudHidden(hidden) {
  document.body.classList.toggle(FULLSCREEN_HUD_HIDDEN_CLASS, hidden);
  reportSurface();
}

function showFullscreenHud() {
  clearTimeout(fullscreenHudTimer);
  fullscreenHudTimer = setTimeout(hideFullscreenHudUnlessHeld, FULLSCREEN_HUD_IDLE_TIMEOUT_MS);
  if (document.body.classList.contains(FULLSCREEN_HUD_HIDDEN_CLASS)) setFullscreenHudHidden(false);
}

function pointerIsOverFullscreenHud() {
  if (!lastPointer) return false;
  const rectangles = [...document.querySelectorAll(FULLSCREEN_HUD_SELECTOR)].map((element) =>
    element.getBoundingClientRect(),
  );
  return pointIsInsideAnyRectangle(lastPointer, rectangles);
}

function hideFullscreenHudUnlessHeld() {
  if (fullscreenHudStaysShown({ paused: previewPaused, pointerOverHud: pointerIsOverFullscreenHud() })) {
    fullscreenHudTimer = setTimeout(hideFullscreenHudUnlessHeld, FULLSCREEN_HUD_IDLE_TIMEOUT_MS);
    return;
  }
  setFullscreenHudHidden(true);
}

function showFullscreenHudAtPointer(event) {
  lastPointer = { x: event.clientX, y: event.clientY };
  showFullscreenHud();
}

function showFullscreenHudOnKey(event) {
  if (event.key !== LEAVE_FULLSCREEN_KEY) showFullscreenHud();
}

function forgetPointer() {
  lastPointer = null;
}

const FULLSCREEN_HUD_LISTENERS = [
  ['mousemove', showFullscreenHudAtPointer],
  ['keydown', showFullscreenHudOnKey],
  ['resize', forgetPointer],
];

function startFullscreenHud() {
  for (const [name, handler] of FULLSCREEN_HUD_LISTENERS) window.addEventListener(name, handler);
  showFullscreenHud();
}

function stopFullscreenHud() {
  for (const [name, handler] of FULLSCREEN_HUD_LISTENERS) window.removeEventListener(name, handler);
  clearTimeout(fullscreenHudTimer);
  lastPointer = null;
  document.body.classList.remove(FULLSCREEN_HUD_HIDDEN_CLASS);
}

/// Take the picture off the screen while the page draws over it, playback carries
/// on underneath.
export function coverPreviewSurface() {
  surfaceCovered = true;
  reportSurface();
}

/// Put the picture back where the panel is showing.
export function uncoverPreviewSurface() {
  surfaceCovered = false;
  reportSurface();
}

/// Stop the player, leaving the panel where it is. The tracks go with the file
/// the backend drops.
export function stopPreview() {
  invoke('preview_stop').catch(() => {});
  shownWatcher(null);
  resetTrackToggles();
  setTransportEnabled(false);
  showPreviewTitle(null);
}

/// Stop the player and take the panel off the page, which is what the panel's own
/// ✕ does. The playlist calls it when the last of its rows goes, so nothing keeps
/// playing behind a hidden panel.
export function closePreview() {
  if (isPreviewFullscreen()) setPreviewFullscreen(false);
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

function setTransportEnabled(enabled) {
  for (const { id } of TRANSPORT_BUTTONS) {
    const button = document.getElementById(id);
    if (button) button.disabled = !enabled;
  }
  const scrubber = document.getElementById('timeline-scrubber');
  if (!scrubber) return;
  if (enabled) scrubber.classList.remove(SCRUBBER_IDLE_CLASS);
  else scrubber.classList.add(SCRUBBER_IDLE_CLASS);
}

// the last segment of a path, with either separator, and a trailing one ignored
function lastPathSegment(path) {
  return path.replace(/[/\\]+$/, '').split(/[/\\]/).pop();
}

function showPreviewTitle(path) {
  const title = document.getElementById('preview-title');
  if (!title) return;
  title.textContent = path ? `Preview: ${lastPathSegment(path)}` : PREVIEW_PANEL_DEFAULT_TITLE;
  title.title = path || '';
}

function initScrubber() {
  const scrubber = document.getElementById('timeline-scrubber');
  const durLabel = document.getElementById('timeline-duration');

  if (!scrubber) return;

  // Click to seek. The pointer is captured for the drag: the video under the
  // scrubber is a native widget, not part of this page, so a plain mouseup
  // released over it never arrives and the seek would stay latched, freezing
  // every poll-driven control until some later click landed in the page.
  scrubber.addEventListener('pointerdown', (e) => {
    if (scrubber.classList.contains(SCRUBBER_IDLE_CLASS)) return;
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

  setTransportEnabled(false);
  showPreviewTitle(null);

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
      audioLevelsWatcher(meta.audio_levels ?? null);
      reportPolledSource(meta.source ?? meta.filename ?? null);
      // at the end nothing is previewing any more, so the Preview button comes back
      if (meta.eof && !endReported) {
        endReported = true;
        shownWatcher(null);
      }
      if (meta.position != null && meta.duration != null && meta.duration > 0) {
        const pct = meta.position / meta.duration;
        updatePlayhead(pct);
        updateTimecode(meta.position, meta.duration, meta.container_fps);
      }
      updatePlayBtn(meta.paused);
      previewPaused = !!meta.paused;
    } catch (error) {
      const text = String(error);
      if (text !== lastPollError) {
        lastPollError = text;
        console.error('metadata poll:', error);
      }
    }
  }, 250);
}

function reportPolledSource(source) {
  if (source === polledSource) return;
  polledSource = source;
  sourceWatcher(source);
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

/// Load a file into the preview player. `otherPackages` are the package
/// directories a version file CPL takes its original version's assets from.
export function previewFile(filePath, contentKeys = null, otherPackages = []) {
  showEmbeddedPanel();
  shownWatcher(filePath);
  endReported = false;
  loadWatcher(filePath);
  const loaded = invoke('preview_load', { filePath, contentKeys, otherPackages }).then(() => true, reportPreviewLoadFailure);
  resetTrackToggles();
  setTransportEnabled(true);
  showPreviewTitle(filePath);
  startScrubberPolling();
  return loaded;
}

/// Load a DCP directory into the preview player
export function previewDcp(dirPath, contentKeys = null) {
  showEmbeddedPanel();
  shownWatcher(dirPath);
  endReported = false;
  loadWatcher(dirPath);
  const loaded = invoke('preview_load_dcp', { dirPath, contentKeys }).then(() => true, reportPreviewLoadFailure);
  resetTrackToggles();
  setTransportEnabled(true);
  showPreviewTitle(dirPath);
  startScrubberPolling();
  return loaded;
}

// true for an encrypted DCP directory, CPL or picture MXF
export function previewNeedsContentKeys(path) {
  return invoke('preview_needs_content_keys', { path });
}

function reportPreviewLoadFailure(error) {
  const message = `Preview failed: ${error}`;
  console.error('[preview] Failed to load:', error);
  const status = document.getElementById('status-text');
  if (status) {
    status.textContent = message;
    status.title = message;
  }
  return false;
}
