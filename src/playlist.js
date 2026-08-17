// Sequential playback of queued packages: the panel a wizard renders, and the
// queue that starts the next package when the player reaches the end of the one
// before it. The queue lasts as long as the session and nothing writes it down.
import {
  previewDcp,
  previewPlayPause,
  watchPreviewLoads,
  watchPreviewMetadata,
} from './preview.js';

// The queue in play order: { directory, title }, the title being the directory's
// name until the package plays and mpv reports its composition title.
const playlist = [];

let panel = null;
let playingIndex = -1;
// Set from the moment a row is handed to the player until the player reports it
// loaded, which is both the guard against advancing twice over one end of file
// and what tells the queue to start the row playing.
let startingRow = false;
// The directory the queue has just handed to the player, which is how a load it
// made is told apart from one the app made.
let expectedDirectory = null;

/// Render the playlist panel into a container element of the app's choosing and
/// take over end-of-file handling from there.
export function initPlaylist(container) {
  panel = container;
  panel.classList.add('playlist');
  panel.addEventListener('click', handlePanelClick);
  watchPreviewMetadata(handleMetadata);
  watchPreviewLoads(handleLoad);
  renderPlaylist();
}

/// Queue a DCP or IMP directory as the last row. Nothing starts playing: a row
/// plays when it is clicked, or when the queue reaches it.
export function addToPlaylist(directory) {
  playlist.push({ directory, title: directoryName(directory) });
  renderPlaylist();
}

function renderPlaylist() {
  if (!panel) return;
  panel.innerHTML = playlist
    .map(
      (entry, index) => `
    <div class="playlist-row${index === playingIndex ? ' playing' : ''}" data-index="${index}">
      <span class="playlist-marker">${index === playingIndex ? '▶' : ''}</span>
      <span class="playlist-order">${index + 1}</span>
      <span class="playlist-title" title="${escapeText(entry.directory)}">${escapeText(entry.title)}</span>
      <button class="btn-sm playlist-up" type="button" title="Move up"${index === 0 ? ' disabled' : ''}>↑</button>
      <button class="btn-sm playlist-down" type="button" title="Move down"${index === playlist.length - 1 ? ' disabled' : ''}>↓</button>
      <button class="btn-sm playlist-remove" type="button" title="Remove">✕</button>
    </div>`,
    )
    .join('');
}

// Rows are re-rendered on every change, so the controls go through delegation.
// Anything in a row that is not one of them plays that row.
function handlePanelClick(e) {
  const row = e.target.closest('.playlist-row');
  if (!row) return;
  const index = Number(row.dataset.index);
  if (e.target.classList.contains('playlist-remove')) removeRow(index);
  else if (e.target.classList.contains('playlist-up')) moveRow(index, index - 1);
  else if (e.target.classList.contains('playlist-down')) moveRow(index, index + 1);
  else playRow(index);
}

function removeRow(index) {
  playlist.splice(index, 1);
  // the marker stays on the package it was on, and the queue lets go of the
  // playing package when that is the row taken out
  if (index === playingIndex) playingIndex = -1;
  else if (index < playingIndex) playingIndex -= 1;
  renderPlaylist();
}

function moveRow(index, target) {
  if (target < 0 || target >= playlist.length) return;
  const [entry] = playlist.splice(index, 1);
  playlist.splice(target, 0, entry);
  if (playingIndex === index) playingIndex = target;
  else if (playingIndex === target) playingIndex = index;
  renderPlaylist();
}

function playRow(index) {
  playingIndex = index;
  startingRow = true;
  expectedDirectory = playlist[index].directory;
  previewDcp(expectedDirectory);
  renderPlaylist();
}

// The app previewing something of its own means the user has gone elsewhere, so
// the queue lets go: the rows stay, and nothing advances until one is clicked.
function handleLoad(path) {
  const ours = path === expectedDirectory;
  expectedDirectory = null;
  if (ours) return;
  playingIndex = -1;
  startingRow = false;
  renderPlaylist();
}

function handleMetadata(meta) {
  if (playingIndex < 0) return;
  if (startingRow) {
    // the flag still set means mpv is on the last frame of the row before this
    // one, so nothing here applies to the row being started yet
    if (meta.eof) return;
    startingRow = false;
    // mpv pauses on the last frame of the row before this one, and that pause
    // outlives the load, so the row needs one play/pause to get going
    if (meta.paused) previewPlayPause();
  }
  useReportedTitle(meta.filename);
  if (meta.eof) playNextRow();
}

// The composition title, which is only known once the package is loaded. A
// package stating no title leaves mpv reporting the source it opened, and for a
// multi-reel composition that is a long edl: uri rather than a name.
function useReportedTitle(title) {
  const entry = playlist[playingIndex];
  if (!title || !entry || entry.title === title || title.includes('://')) return;
  entry.title = title;
  renderPlaylist();
}

function playNextRow() {
  const next = playingIndex + 1;
  // the last row ends the way a single package does, with the player holding
  // its last frame
  if (next >= playlist.length) return;
  playRow(next);
}

function directoryName(directory) {
  const parts = directory.split(/[\\/]+/).filter(Boolean);
  return parts[parts.length - 1] || directory;
}

// Titles come from a filesystem path or from mpv, so they cannot go into the row
// markup as they are.
function escapeText(text) {
  return String(text).replace(/[&<>"]/g, (character) => `&#${character.charCodeAt(0)};`);
}
