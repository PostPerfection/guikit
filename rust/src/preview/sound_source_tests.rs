use std::path::{Path, PathBuf};

use postkit::mxf_wrap::{mxf_wrap, EssenceType, MxfStandard, MxfWrapOptions};
use postkit::packaging::{
    ns, AssetMap, AssetMapAsset, DcpAuxData, DcpCpl, DcpCplReel, ATMOS_DATA_TYPE_UL,
};

use super::end_of_file_tests::overlay_state;
use super::grok_fixture::{wait_until, write_picture_file, FRAMES_PER_SECOND, PICTURE_SIDE};
use super::player_controls::{set_sound_source, SoundSourceChoice};
use super::{poll_metadata, Backend, Player, PreviewOverlays};

const PICTURE_FRAMES: usize = 24;
// an IA bitstream frame with an empty IAFrame at 48 kHz, 24 bit, 24 fps (ST 2098-2), which renders as silence
const SILENT_IAB_FRAME: [u8; 16] = [
    0x01, 0, 0, 0, 0, 0x02, 0, 0, 0, 6, 0x08, 4, 0x01, 0x10, 0x01, 0x00,
];
const CPL_UUID: &str = "a7000000-0000-4000-8000-0000000c0000";
const PICTURE_UUID: &str = "a7000000-0000-4000-8000-000000000001";
const SOUND_UUID: &str = "a7000000-0000-4000-8000-000000000002";
const ATMOS_UUID: &str = "a7000000-0000-4000-8000-000000000003";
const REEL_UUID: &str = "a7000000-0000-4000-8000-000000000004";
const ASSET_MAP_UUID: &str = "a7000000-0000-4000-8000-000000000005";
const SOUND_NAME: &str = "sound.mxf";
const ATMOS_NAME: &str = "atmos.mxf";

fn wrap(input_files: Vec<PathBuf>, output: PathBuf, essence_type: EssenceType) {
    let track = mxf_wrap(&MxfWrapOptions {
        input_files,
        output,
        essence_type,
        standard: MxfStandard::AsDcp,
        fps_num: FRAMES_PER_SECOND,
        fps_den: 1,
        partition_size: 0,
        encryption: None,
        mca_config: None,
        resource_ids: Vec::new(),
        hdr: None,
        asset_uuid: None,
        timed_text_duration_frames: None,
    });
    assert!(track.success, "{}", track.error);
}

fn write_silent_wav(path: &Path) {
    let output = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg(format!(
            "anullsrc=r=48000:cl=stereo:d={}",
            PICTURE_FRAMES as f64 / f64::from(FRAMES_PER_SECOND)
        ))
        .args(["-c:a", "pcm_s24le"])
        .arg(path)
        .output()
        .expect("ffmpeg");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// one reel with a picture, a main sound and an Atmos track
fn write_atmos_package(directory: &Path) -> PathBuf {
    let package = directory.join("package");
    std::fs::create_dir_all(&package).unwrap();
    let picture = write_picture_file(&package, PICTURE_FRAMES);
    let scratch = tempfile::tempdir().unwrap();
    let wav = scratch.path().join("sound.wav");
    write_silent_wav(&wav);
    wrap(vec![wav], package.join(SOUND_NAME), EssenceType::Pcm);
    let iab_frames = (0..PICTURE_FRAMES)
        .map(|frame| {
            let path = scratch.path().join(format!("frame_{frame:05}.iab"));
            std::fs::write(&path, SILENT_IAB_FRAME).unwrap();
            path
        })
        .collect();
    wrap(iab_frames, package.join(ATMOS_NAME), EssenceType::Atmos);

    let asset = |id: &str, path: &str| AssetMapAsset {
        id: id.into(),
        path: path.into(),
        ..Default::default()
    };
    let picture_name = picture.file_name().unwrap().to_string_lossy();
    let asset_map = AssetMap {
        uuid: ASSET_MAP_UUID.into(),
        namespace: ns::AM_SMPTE.into(),
        assets: vec![
            asset(CPL_UUID, "CPL.xml"),
            asset(PICTURE_UUID, &picture_name),
            asset(SOUND_UUID, SOUND_NAME),
            asset(ATMOS_UUID, ATMOS_NAME),
        ],
        ..Default::default()
    };
    std::fs::write(package.join("ASSETMAP.xml"), asset_map.to_xml()).unwrap();
    let cpl = DcpCpl {
        uuid: CPL_UUID.into(),
        namespace: ns::CPL_SMPTE.into(),
        title: "Atmos and main sound".into(),
        reels: vec![DcpCplReel {
            reel_id: REEL_UUID.into(),
            picture_id: PICTURE_UUID.into(),
            picture_edit_rate_num: FRAMES_PER_SECOND,
            picture_edit_rate_den: 1,
            picture_duration: PICTURE_FRAMES as u64,
            picture_width: PICTURE_SIDE,
            picture_height: PICTURE_SIDE,
            sound_id: Some(SOUND_UUID.into()),
            sound_edit_rate_num: FRAMES_PER_SECOND,
            sound_edit_rate_den: 1,
            sound_duration: PICTURE_FRAMES as u64,
            aux_data: Some(DcpAuxData {
                id: ATMOS_UUID.into(),
                edit_rate_num: FRAMES_PER_SECOND,
                edit_rate_den: 1,
                duration: PICTURE_FRAMES as u64,
                entry_point: 0,
                key_id: None,
                data_type: ATMOS_DATA_TYPE_UL.into(),
            }),
            ..Default::default()
        }],
        ..Default::default()
    };
    std::fs::write(package.join("CPL.xml"), cpl.to_xml()).unwrap();
    package
}

// sound_source and atmos_available as the page's poll reads them
fn polled_sound(player: &Player) -> (serde_json::Value, serde_json::Value) {
    let state = overlay_state(PreviewOverlays::default());
    let metadata: serde_json::Value =
        serde_json::from_str(&poll_metadata(player, &state).unwrap()).unwrap();
    (
        metadata["sound_source"].clone(),
        metadata["atmos_available"].clone(),
    )
}

fn wait_for_sound(player: &Player, source: &str) {
    wait_until(&format!("the poll reported {source}"), || {
        polled_sound(player) == (source.into(), true.into())
    });
}

#[test]
fn the_sound_source_command_reaches_the_grok_player() {
    let directory = tempfile::tempdir().unwrap();
    let package = write_atmos_package(directory.path());
    let player = Player::new().unwrap();
    player.init_software().unwrap();
    player
        .load_package_dir(&package.to_string_lossy(), None)
        .unwrap();
    assert_eq!(player.active(), Backend::Grok);
    wait_for_sound(&player, "atmos");

    let outcome = set_sound_source(&player, SoundSourceChoice::Main);
    assert!(!outcome.mpv_file_unchanged);
    wait_for_sound(&player, "main");

    set_sound_source(&player, SoundSourceChoice::Atmos);
    wait_for_sound(&player, "atmos");
}

#[test]
fn the_page_names_read_as_the_sound_sources() {
    let read = |name: &str| serde_json::from_value::<SoundSourceChoice>(name.into());
    assert_eq!(read("atmos").unwrap(), SoundSourceChoice::Atmos);
    assert_eq!(read("main").unwrap(), SoundSourceChoice::Main);
    assert!(read("mainSound").is_err());
}
