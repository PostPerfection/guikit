# GUI Kit

Shared code for the PostPerfection wizard GUIs, frontend and native.

- `src/preview.js`: mpv-backed preview player and scrubber
- `src/playlist.js`: the queue that plays packages one after another
- `src/shortcuts.js`: keyboard shortcut handling and the shortcuts dialog
- `src/base.css`: the shared stylesheet
- `rust/`: the `guikit` crate, holding the native side of the preview

## The guikit crate

`rust/src/preview` hosts libmpv's video output inside the app window and
exposes the tauri commands the page drives it with. `attach` has a per-platform
implementation (`linux.rs`, `macos.rs`, `windows.rs`); only linux is written,
the others return an error saying so.

An app registers the commands from `guikit::preview` in its
`tauri::generate_handler!` list and manages the state in `setup`:

```rust
app.manage(guikit::preview::create_player(app, "main"));
```

Playback is the only path, so an app needs libmpv development files at build
time. When `attach` fails the app still starts, with `preview_is_embedded`
reporting false so the page hides the preview.

The crate depends on postkit by git url. Both wizards redirect that to their
own `extern/postkit` submodule with a `[patch]` in their gui `Cargo.toml`, so a
build compiles postkit once.

## Preview QC controls

`preview.js` builds a control strip into the preview panel header, so an app
needs no markup of its own for it: safe area (95% or 90%), aspect mask (1.85,
1.90, 2.39), centre cross, rule of thirds, crop, decode resolution, subtitles,
captions, and the counters. An app only has to register `preview_set_overlays`,
`preview_set_decode_scale`, `preview_set_subtitle_file` and
`preview_set_subtitle_visibility` beside the other commands.

Crop, Sub and CC start disabled, because they have nothing to show until the
page hands them one. `setPreviewCrop({ left, right, top, bottom })` gives the
crop overlay the pixels the job takes off each edge of the source picture, and
`setPreviewCrop(null)` takes it away. `setPreviewSubtitleFile(path)` and
`setPreviewCaptionFile(path)` load a file into mpv's primary and secondary
subtitle slots, the secondary one rendering at the top of the frame, and null
drops the track. Only what libass reads natively works: SRT, ASS or SSA and
WebVTT, so the wizards convert their subtitle XML to SRT first. The clip has to
be loaded before the track goes on it, and loading another clip drops both
tracks.

The overlays are one mpv filter chain. `overlay_filter_chain` builds it from a
`PreviewOverlays` struct and `preview_set_overlays` sets it on mpv's `vf`
property, so each change replaces the whole chain and an empty chain clears it.
Sizes are ffmpeg expressions over `iw` and `ih`, which keeps one chain right for
any frame size and any decode resolution. The aspect mask draws four boxes, two
per orientation, and switches off the pair that does not apply with `enable`,
because a drawbox sized zero covers the whole frame instead of nothing.

The crop is the one overlay the page gives in pixels, off each edge of the
source picture, so it is drawn as a fraction of the frame the source size turns
those pixels into. That size is `current-tracks/video/demux-w` and `demux-h`,
what the container declares, rather than the decoded frame: libavcodec's
`lowres` shrinks only the decoders that implement it, so at Half a JPEG 2000
frame comes back half size and an h264 frame comes back whole. A source that
reports no demuxer size falls back to `video-params/w` and `h` multiplied by the
decode scale, which is that same assumption. The player keeps the last size it
read, because a reload leaves a moment where mpv reports none and the page sends
the overlays again inside it.

`preview_set_decode_scale` takes `full`, `half` or `quarter` and sets
libavcodec's `lowres` to 0, 1 or 2 through mpv's `vd-lavc-o`. The decoder reads
lowres when it opens, so the command reloads the current file at the position
and pause state it had. JPEG 2000 reaches half and quarter by discarding DWT
levels, so a reduced scale costs a fraction of a full decode. Other codecs
honour lowres only where their decoder implements it, which h264, HEVC and
ProRes do not, and the control is offered for them all the same.

That reload takes the external subtitle tracks with it, and a `sub-add` sent
straight after a `loadfile` is refused because the file is not loaded yet. So
the reload carries the subtitle files as per-file options instead, `sub-files`
with `sid` and `secondary-sid`, which puts the same files back under the same
ids they had.

`preview_get_metadata` carries the counters beside position and duration:
`dropped_frames` (mpv `frame-drop-count`), `delayed_frames`
(`vo-delayed-frame-count`), `cache_seconds` (`demuxer-cache-duration`),
`decoder_fps` (`estimated-vf-fps`) and `container_fps` (`container-fps`). Each
is null until mpv has a value for it. The scrubber poll reads them, so the HUD
costs no second timer.

It also carries `eof`, mpv's `eof-reached`, which is what the playlist advances
on. mpv only holds that true, paused on the last frame, because postkit starts
the player with `keep-open` on; guikit sets nothing for it.

## Playlist

`playlist.js` plays queued packages one after another, for the session only:
nothing is written down and there is no file format.
`initPlaylist(container, options)` renders the panel into an element the app
supplies, and `addToPlaylist(directory)` queues a DCP or IMP directory as the
last row.

`options.loadPackage` is the app's own loader for a package directory, async or
not, and rows play through it instead of `previewDcp` when it is given. A wizard
has one already: it attaches the packaged subtitle and caption tracks and clears
the crop overlay, none of which a bare `previewDcp` knows about, so a queued row
would otherwise play with the crop of whatever came before it drawn over it and
no timed text. The loader has to pass the directory it is given through
unchanged, because that string is what tells the queue's own loads apart from the
app's. It may leave the package playing or paused, either way round.

A row shows its position, its title, and buttons to move it up or down or take
it out. Clicking a row plays the queue from there, and the row playing carries a
marker. The title is the directory's name until the package plays, and mpv's
composition title after that, which the metadata poll reports as `filename`.

The queue rides on the scrubber's metadata poll rather than a timer of its own:
`watchPreviewMetadata` hands it every poll, and it loads the next row when the
poll reports `eof`. mpv is paused on the last frame by then and that pause
outlives the load, so the new row is started with one `preview_play_pause` once
the poll shows it loaded, unless it is playing already, which a loader of the
app's own may have seen to. Waiting for the load is also what stops one end of
file advancing the queue twice. A manual
pause never advances it, because `eof-reached` is only true at the end of the
file. On the last row, or with nothing queued, the end of a package is what it
was before.

`watchPreviewLoads` reports every load `previewFile` and `previewDcp` are asked
for. The queue compares each one against the directory it just handed over, and
anything else means the app loaded something of its own: the rows stay rendered,
the marker goes, and no end of file advances anything until a row is clicked
again.

## Consumers

[dcpwizard](https://github.com/PostPerfection/dcpwizard) and
[imfwizard](https://github.com/PostPerfection/imfwizard) vendor this repo as an
`extern/guikit` submodule, import the JS from it with relative ES imports out of
their own `gui/src`, and depend on the crate by path at
`../../extern/guikit/rust`.

Each app keeps its own `style.css` for the rules that are genuinely app
specific, loaded after `base.css`.

`timeline.js` is deliberately not here. The DCP reel model and the IMF segment
model use disjoint field names from their Rust backends, so the two copies stay
in their own repos.

## Pin discipline

Every change here is immediately followed by a submodule pin bump commit in both
dcpwizard and imfwizard. Do not leave a guikit commit unpinned. The wizards must
never sit on a floating submodule reference.

## dcpdoctor

dcpdoctor is not a submodule consumer. It vendors `shortcuts.js` by plain `cp`.
When that file changes here, copy it across by hand.

## License

AGPL-3.0. See [LICENSE](LICENSE).
