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
1.90, 2.39), centre cross, rule of thirds, decode resolution, and the counters.
An app only has to register `preview_set_overlays` and `preview_set_decode_scale`
beside the other commands.

The overlays are one mpv filter chain. `overlay_filter_chain` builds it from a
`PreviewOverlays` struct and `preview_set_overlays` sets it on mpv's `vf`
property, so each change replaces the whole chain and an empty chain clears it.
Sizes are ffmpeg expressions over `iw` and `ih`, which keeps one chain right for
any frame size and any decode resolution. The aspect mask draws four boxes, two
per orientation, and switches off the pair that does not apply with `enable`,
because a drawbox sized zero covers the whole frame instead of nothing.

`preview_set_decode_scale` takes `full`, `half` or `quarter` and sets
libavcodec's `lowres` to 0, 1 or 2 through mpv's `vd-lavc-o`. The decoder reads
lowres when it opens, so the command reloads the current file at the position
and pause state it had. JPEG 2000 reaches half and quarter by discarding DWT
levels, so a reduced scale costs a fraction of a full decode. Other codecs
honour lowres only where their decoder implements it, h264 among them, and the
control is offered for them all the same.

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
