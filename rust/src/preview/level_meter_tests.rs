use std::path::{Path, PathBuf};

use postkit::audio_levels::SILENCE_FLOOR_DBFS;

use super::end_of_file_tests::overlay_state;
use super::grok_fixture::wait_until;
use super::player_controls::set_level_meter;
use super::{poll_metadata, Backend, Player, PreviewOverlays};

const AUDIO_LEVELS_FIELD: &str = "audio_levels";
const HALF_SCALE_DBFS: f64 = -6.0206;
const TOLERANCE_DB: f64 = 0.01;

// a half scale tone on the left, silence on the right
fn stereo_tone(directory: &Path) -> PathBuf {
    let sound = directory.join("tone.wav");
    let output = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-f",
            "lavfi",
            "-i",
            "aevalsrc=0.5*sin(2*PI*1000*t)|0:s=48000:c=stereo:d=4",
        ])
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

fn audio_levels(player: &Player) -> serde_json::Value {
    let state = overlay_state(PreviewOverlays::default());
    let metadata: serde_json::Value =
        serde_json::from_str(&poll_metadata(player, &state).unwrap()).unwrap();
    metadata[AUDIO_LEVELS_FIELD].clone()
}

#[test]
fn the_poll_carries_each_channel_level_while_the_meter_is_on() {
    let directory = tempfile::tempdir().unwrap();
    let sound = stereo_tone(directory.path());
    let player = Player::new().unwrap();
    // never the real sound device
    player.mpv().set_property("ao", "null").unwrap();
    player.init_software().unwrap();
    player.mpv().set_property("aid", "auto").unwrap();
    assert!(audio_levels(&player).is_null(), "the meter starts off");

    set_level_meter(&player, true).unwrap();
    player.load_source(&sound.to_string_lossy(), None).unwrap();
    assert_eq!(player.active(), Backend::Mpv);
    wait_until("the left channel was measured", || {
        audio_levels(&player)[0]["peak_dbfs"]
            .as_f64()
            .is_some_and(|peak| peak > SILENCE_FLOOR_DBFS)
    });

    let levels = audio_levels(&player);
    assert_eq!(levels[0]["label"], "FL");
    let peak = levels[0]["peak_dbfs"].as_f64().unwrap();
    assert!((peak - HALF_SCALE_DBFS).abs() < TOLERANCE_DB, "{peak}");
    assert_eq!(levels[1]["label"], "FR");
    assert_eq!(levels[1]["rms_dbfs"], SILENCE_FLOOR_DBFS);

    set_level_meter(&player, false).unwrap();
    assert!(audio_levels(&player).is_null());
}
