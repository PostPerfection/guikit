# GUI Kit

Shared code for the PostPerfection wizard GUIs, frontend and native.

- `src/preview.js`: mpv-backed preview player and scrubber
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
