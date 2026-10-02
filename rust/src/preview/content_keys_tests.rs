use std::path::{Path, PathBuf};

use postkit::content_keys::ContentKeys;

use super::end_of_file_tests::write_clip;
use super::grok_fixture::{write_encrypted_package, write_package, PictureEncryption};
use super::{preview_needs_content_keys, Backend, Player};

const PICTURE_FRAMES: usize = 4;
const CLIP_SECONDS: u32 = 1;
const CONTENT_KEY: [u8; 16] = [0x5a; 16];
const CONTENT_KEY_HEX: &str = "5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a";
const KEY_ID: [u8; 16] = [0x3e; 16];
const KEY_ID_TEXT: &str = "3e3e3e3e-3e3e-3e3e-3e3e-3e3e3e3e3e3e";

fn encrypted_package(directory: &Path) -> PathBuf {
    write_encrypted_package(
        directory,
        PICTURE_FRAMES,
        &PictureEncryption {
            content_key: CONTENT_KEY,
            key_id: KEY_ID,
        },
    )
}

fn keys_file(directory: &Path) -> ContentKeys {
    let path = directory.join("KEYS.json");
    std::fs::write(
        &path,
        format!(
            r#"{{"keys": [{{"key_type": "Mdik", "key_id": "{KEY_ID_TEXT}", "content_key_hex": "{CONTENT_KEY_HEX}"}}]}}"#
        ),
    )
    .unwrap();
    ContentKeys::from_keys_json(&path).unwrap()
}

fn needs_keys(path: &Path) -> Result<bool, String> {
    preview_needs_content_keys(path.to_string_lossy().into_owned())
}

#[test]
fn only_an_encrypted_package_asks_for_content_keys() {
    let directory = tempfile::tempdir().unwrap();
    let encrypted = encrypted_package(&directory.path().join("encrypted"));
    let plain = write_package(&directory.path().join("plain"), PICTURE_FRAMES);
    let clip = directory.path().join("clip.mp4");
    write_clip(&clip, CLIP_SECONDS);

    assert_eq!(needs_keys(&encrypted), Ok(true));
    assert_eq!(needs_keys(&encrypted.join("picture.mxf")), Ok(true));
    assert_eq!(needs_keys(&plain), Ok(false));
    assert_eq!(needs_keys(&clip), Ok(false), "a plain video is no package");
}

#[test]
fn an_encrypted_package_plays_on_grok_with_its_keys() {
    let directory = tempfile::tempdir().unwrap();
    let package = encrypted_package(directory.path());
    let package = package.to_string_lossy();

    let player = Player::new().unwrap();
    player.init_software().unwrap();

    let refusal = player.load_package_dir(&package, None).unwrap_err();
    assert!(refusal.contains("encrypted"), "{refusal}");

    player
        .load_package_dir(&package, Some(keys_file(directory.path())))
        .unwrap();
    assert_eq!(player.active(), Backend::Grok);
    assert!(player.grok().duration().is_some(), "nothing loaded");
}

#[test]
fn content_keys_for_an_mpv_source_are_refused() {
    let directory = tempfile::tempdir().unwrap();
    let clip = directory.path().join("clip.mp4");
    write_clip(&clip, CLIP_SECONDS);

    let player = Player::new().unwrap();
    player.init_software().unwrap();

    let error = player
        .load_source(&clip.to_string_lossy(), Some(keys_file(directory.path())))
        .unwrap_err();
    assert_eq!(
        error,
        format!(
            "{} is not JPEG 2000 MXF essence, so content keys do not apply to it",
            clip.display()
        )
    );
}
