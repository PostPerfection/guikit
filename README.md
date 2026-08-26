# GUI Kit

Shared code for the PostPerfection wizard GUIs, frontend and native.

- `src/preview.js`: mpv-backed preview player and scrubber
- `src/playlist.js`: the queue that plays packages one after another
- `src/shortcuts.js`: keyboard shortcut handling and the shortcuts dialog
- `src/base.css`: the shared stylesheet
- `rust/`: the `guikit` crate, holding the native side of the preview
- `test/`: the headless harness for the playlist queue and the transport bar

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

## Preview transport

The transport bar is the app's own markup, wired by element id and every button
optional: `timeline-scrubber` with `timeline-playhead` inside it,
`timeline-position` and `timeline-duration`, `timeline-play-btn`,
`timeline-start-btn`, `timeline-skip-back-btn`, `timeline-skip-forward-btn`,
`timeline-frame-back-btn` and `timeline-frame-forward-btn`. The skip buttons move
by the exported `PREVIEW_SEEK_SECONDS` and take their tooltip from it, so an app
binding the same jump to a key labels it from that constant rather than its own.

`previewPlayPause`, `previewSeek(seconds)`, `previewSeekAbsolute(seconds)`,
`previewFrameStepBack()` and `previewFrameStepForward()` are also exported for an
app's shortcuts. The two frame steps run mpv's `frame-back-step` and
`frame-step`, which pause playback, and need `preview_frame_back_step` and
`preview_frame_step` registered beside the other commands.

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

The overlays are one ASS overlay on mpv's OSD. `overlay_drawing` builds it from
a `PreviewOverlays` struct as one dialogue event per overlay, each a filled path
in the source picture's own pixels, and `preview_set_overlays` installs it
through postkit's `set_osd_overlay`. Nothing goes through a video filter any
more: libass composites the drawings, so no frame passes through the CPU for
them and switching one on is not a filter reconfiguration, which is what used to
clear mpv's `eof-reached` under the playlist and cost frame rate while playing.

mpv stretches an overlay's PlayRes canvas over the whole rendered surface, black
bars included. So the canvas is the surface's shape written in source pixels and
the drawing is shifted onto where the picture sits, both worked out from
`osd-dimensions`, which is what puts a 95% safe area on 95% of the picture
rather than 95% of the panel. The preview surface is a full-width box of a fixed
height, so that shift is the normal case, not an edge one.

The picture size everything is measured against is
`current-tracks/video/demux-w` and `demux-h`, what the container declares,
rather than the decoded frame: libavcodec's `lowres` shrinks only the decoders
that implement it, so at Half a JPEG 2000 frame comes back half size and an h264
frame comes back whole. A source that reports no demuxer size falls back to
`video-params/w` and `h` multiplied by the decode scale, which is that same
assumption. The player keeps the last size it read, because a reload leaves a
moment where mpv reports none and the page sends the overlays again inside it.
The crop is the one overlay the page gives in pixels, off each edge of the source
picture, and it is drawn in those same pixels.

`preview_get_metadata` installs the overlay again whenever the picture or the
surface has moved under it, since nothing tells the page when either happens: a
load, a resized window and a decode scale change all move it. The drawing
already on the player is remembered, so a poll that finds nothing moved sends
nothing.

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
supplies, and `addToPlaylist(directory, title)` queues a DCP or IMP directory as
the last row. The title is what to call the row, for an app queueing a package
the user picked by name; leave it out and the directory's name stands in until
the package plays.

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
marker. The title is whatever the row was queued under until the package plays,
and mpv's composition title after that, which the metadata poll reports as
`filename`.

Taking a row out that is not the one playing changes nothing else. Taking out the
row playing stops the player, and when that was the last row the panel goes too,
through `closePreview`, so no package keeps playing behind a hidden panel. A
queue that has let go of playback takes nothing with it: the rows are just rows
then, and removing them leaves whatever the app is previewing alone.

The queue rides on the scrubber's metadata poll rather than a timer of its own:
`watchPreviewMetadata` hands it every poll, and it loads the next row when the
poll reports `eof`. That decision is latched: the first poll reporting the end of
the row playing makes it, and no later poll can unmake it or make it twice, since
a property change elsewhere can clear mpv's `eof-reached` while the row still
sits on its last frame. mpv is paused on that frame and the pause outlives the
load, so the new row is started with one `preview_play_pause` once the poll shows
it loaded, unless it is playing already, which a loader of the app's own may have
seen to. A manual pause never advances the queue, because `eof-reached` is only
true at the end of the file. On the last row, or with nothing queued, the end of
a package is what it was before, and a row queued after that end waits to be
clicked.

`watchPreviewLoads` reports every load `previewFile` and `previewDcp` are asked
for. The queue compares each one against the directory it just handed over, and
anything else means the app loaded something of its own: the rows stay rendered,
the marker goes, and no end of file advances anything until a row is clicked
again.

`test/playlist.test.mjs` drives the queue headless, with `test/preview-stub.mjs`
standing in for the player, and `test/transport.test.mjs` clicks the transport
buttons with `test/tauri-core-stub.mjs` standing in for the tauri bridge:
`node --test 'test/*.test.mjs'`, no dependencies and nothing to build. That is the
whole JS test suite, and CI runs it beside the syntax check.

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
