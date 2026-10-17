use std::path::{Path, PathBuf};

use postkit::encode::FrameRate;
use postkit::timeline::{Clip, ClipSound, SoundTrack, Timeline, TIMELINE_FORMAT_VERSION};

use super::end_of_file_tests::{overlay_state, pump_until, write_clip};
use super::{load_mpv_timeline, poll_metadata, Backend, Player, PreviewOverlays};

const CLIP_FPS: u32 = 24;
// half a second of the first clip, then all of the second
const FIRST_CLIP_FRAMES: (u64, u64) = (6, 18);
const SECOND_CLIP_FRAMES: (u64, u64) = (0, 24);
const TIMELINE_SECONDS: f64 = 1.5;
const FRAME_SECONDS: f64 = 1.0 / CLIP_FPS as f64;
const INTO_THE_SECOND_CLIP_SECONDS: f64 = 0.6;
const PICTURE_AND_SOUND_TRACKS: i64 = 2;

fn clip(picture: PathBuf, (in_frame, out_frame): (u64, u64)) -> Clip {
    Clip {
        picture,
        in_frame,
        out_frame,
        sound: ClipSound::None,
        subtitle: None,
        source_colour: Default::default(),
        picture_processing: Default::default(),
    }
}

fn tone(directory: &Path) -> PathBuf {
    let sound = directory.join("tone.wav");
    let output = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg("sine=frequency=1000:sample_rate=48000:duration=2")
        .arg(&sound)
        .output()
        .expect("ffmpeg");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    sound
}

#[test]
fn an_mpv_timeline_lasts_as_long_as_its_clips_and_reports_the_clip_it_plays() {
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("first.mov");
    let second = directory.path().join("second.mov");
    write_clip(&first, 1);
    write_clip(&second, 1);
    let timeline = Timeline {
        version: TIMELINE_FORMAT_VERSION,
        fps: FrameRate::whole(CLIP_FPS),
        clips: vec![
            clip(first, FIRST_CLIP_FRAMES),
            clip(second, SECOND_CLIP_FRAMES),
        ],
        sound: SoundTrack::Override(tone(directory.path())),
        subtitle: None,
        reel_starts: Vec::new(),
    };
    let player = Player::new().unwrap();
    // never the real sound device
    player.mpv().set_property("ao", "null").unwrap();
    player.init_software().unwrap();
    let state = overlay_state(PreviewOverlays::default());

    load_mpv_timeline(&player, &state, &timeline, directory.path()).unwrap();
    assert_eq!(player.active(), Backend::Mpv);
    pump_until(player.mpv(), |_| player.mpv().get_duration().is_ok())
        .unwrap_or_else(|last| panic!("mpv never read the timeline, last read {last}"));
    let duration = player.mpv().get_duration().unwrap();
    assert!(
        (duration - TIMELINE_SECONDS).abs() < FRAME_SECONDS,
        "mpv plays the timeline for {duration}s"
    );
    assert_eq!(state.timeline_duration(), Some(TIMELINE_SECONDS));
    let tracks = player
        .mpv()
        .get_property_string("track-list/count")
        .unwrap();
    assert_eq!(tracks, PICTURE_AND_SOUND_TRACKS.to_string());

    pump_until(player.mpv(), |_| {
        player.mpv().get_position().unwrap_or(0.0) > INTO_THE_SECOND_CLIP_SECONDS
    })
    .unwrap_or_else(|last| panic!("the timeline never reached its second clip, last read {last}"));
    let metadata: serde_json::Value =
        serde_json::from_str(&poll_metadata(&player, &state).unwrap()).unwrap();
    assert_eq!(metadata["timeline_clip_index"], 1, "{metadata}");
    assert!(
        metadata["timeline_position"].as_f64().unwrap() > INTO_THE_SECOND_CLIP_SECONDS,
        "{metadata}"
    );
}
