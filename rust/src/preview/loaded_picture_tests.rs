use super::end_of_file_tests::write_clip;
use super::grok_fixture::{
    wait_until, write_as02_picture_file, write_package, write_stereo_picture_file,
};
use super::player_controls::{loaded_picture, LoadedPicture};
use super::Player;

const PICTURE_FRAMES: usize = 48;
const CLIP_SECONDS: u32 = 1;

fn wait_for(player: &Player, expected: LoadedPicture) {
    wait_until(&format!("the player reported {expected:?}"), || {
        loaded_picture(player).unwrap() == expected
    });
}

#[test]
fn the_loaded_picture_follows_what_the_player_has_loaded() {
    let directory = tempfile::tempdir().unwrap();
    let package = write_package(directory.path(), PICTURE_FRAMES);
    let stereo = write_stereo_picture_file(directory.path(), PICTURE_FRAMES);
    let as02 = write_as02_picture_file(directory.path(), PICTURE_FRAMES);
    let clip = directory.path().join("clip.mp4");
    write_clip(&clip, CLIP_SECONDS);

    let player = Player::new().unwrap();
    player.init_software().unwrap();
    assert_eq!(loaded_picture(&player).unwrap(), LoadedPicture::Nothing);

    player
        .load_package_dir(&package.to_string_lossy(), None)
        .unwrap();
    wait_for(
        &player,
        LoadedPicture::Dcp {
            stereoscopic: false,
        },
    );

    player.load_source(&stereo.to_string_lossy(), None).unwrap();
    wait_for(&player, LoadedPicture::Dcp { stereoscopic: true });

    player.load_source(&as02.to_string_lossy(), None).unwrap();
    wait_for(&player, LoadedPicture::Imf);

    player.load_source(&clip.to_string_lossy(), None).unwrap();
    wait_for(&player, LoadedPicture::MpvFile);

    player.stop().unwrap();
    wait_for(&player, LoadedPicture::Nothing);
}

#[test]
fn the_page_reads_the_kind_and_the_eyes() {
    assert_eq!(
        serde_json::to_value(LoadedPicture::Dcp { stereoscopic: true }).unwrap(),
        serde_json::json!({"kind": "dcp", "stereoscopic": true})
    );
    assert_eq!(
        serde_json::to_value(LoadedPicture::MpvFile).unwrap(),
        serde_json::json!({"kind": "mpvFile"})
    );
    assert_eq!(
        serde_json::to_value(LoadedPicture::Imf).unwrap(),
        serde_json::json!({"kind": "imf"})
    );
}
