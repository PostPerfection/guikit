use super::end_of_file_tests::write_clip;
use super::grok_fixture::{wait_until, write_package, write_picture_file};
use super::{Backend, Player};

// long enough that the fixture is still playing when the assert reads it
const PICTURE_FRAMES: usize = 48;
const CLIP_SECONDS: u32 = 1;

#[test]
fn the_backend_follows_what_the_source_is() {
    let directory = tempfile::tempdir().unwrap();
    let picture = write_picture_file(directory.path(), PICTURE_FRAMES);
    let package = write_package(directory.path(), PICTURE_FRAMES);
    let clip = directory.path().join("clip.mp4");
    write_clip(&clip, CLIP_SECONDS);

    let player = Player::new().unwrap();
    player.init_software().unwrap();

    player.load_source(&clip.to_string_lossy()).unwrap();
    assert_eq!(player.active(), Backend::Mpv, "an mp4 is libmpv's");

    player.load_source(&picture.to_string_lossy()).unwrap();
    assert_eq!(
        player.active(),
        Backend::Grok,
        "a JPEG 2000 picture track file is grok's"
    );
    // the page sends no play after a load, so the backend has to start itself
    wait_until("the picture file started playing", || {
        !player.grok().paused()
    });

    player.load_package_dir(&package.to_string_lossy()).unwrap();
    assert_eq!(
        player.active(),
        Backend::Grok,
        "a package around one is grok's too"
    );

    player.load_source(&clip.to_string_lossy()).unwrap();
    assert_eq!(
        player.active(),
        Backend::Mpv,
        "the mp4 hands mpv the surface"
    );
    wait_until("the grok player let the package go", || {
        player.grok().duration().is_none()
    });
}
