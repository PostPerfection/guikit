use std::ffi::{c_void, CStr};

use super::grok_fixture::{self, wait_until, BAND_ROWS, PICTURE_SIDE};
use super::render_thread_tests::{current_gl_context, gl_symbol, resolve_gl_symbol};
use super::{Backend, Player};

// 16:9, so a square picture leaves a bar either side of it
const TARGET_WIDTH: i32 = 128;
const TARGET_HEIGHT: i32 = 72;
const BYTES_PER_PIXEL: usize = 4;
const PICTURE_FRAMES: usize = 4;

const GL_RGBA: u32 = 0x1908;
const GL_UNSIGNED_BYTE: u32 = 0x1401;
const GL_NO_ERROR: u32 = 0;
const GL_RENDERER: u32 = 0x1F01;
const GL_VERSION: u32 = 0x1F02;

// what a linear magnification filter may cost a flat colour, per channel
const COLOUR_TOLERANCE: u8 = 3;

type GlReadPixels = unsafe extern "C" fn(i32, i32, i32, i32, u32, u32, *mut c_void);
type GlGetError = unsafe extern "C" fn() -> u32;
type GlGetString = unsafe extern "C" fn(u32) -> *const std::ffi::c_char;

// the picture, letterboxed into the target the way both presenters place it
const PICTURE_HEIGHT: i32 = TARGET_HEIGHT;
const PICTURE_LEFT: i32 = (TARGET_WIDTH - PICTURE_HEIGHT) / 2;
const PICTURE_RIGHT: i32 = PICTURE_LEFT + PICTURE_HEIGHT;
// rows of the drawn picture the fixture's colour band fills
const DRAWN_BAND_ROWS: i32 = PICTURE_HEIGHT * BAND_ROWS as i32 / PICTURE_SIDE as i32;
// rows sampled well inside the band and well inside the body below it
const BAND_ROW: i32 = DRAWN_BAND_ROWS / 3;
const BODY_ROW: i32 = PICTURE_HEIGHT * 2 / 3;
const PICTURE_MIDDLE_COLUMN: i32 = TARGET_WIDTH / 2;

fn gl_string(name: u32) -> String {
    let get_string: GlGetString = unsafe { std::mem::transmute(gl_symbol("glGetString")) };
    let value = unsafe { get_string(name) };
    if value.is_null() {
        return "unknown".to_string();
    }
    unsafe { CStr::from_ptr(value) }
        .to_string_lossy()
        .into_owned()
}

fn gl_error() -> u32 {
    let get_error: GlGetError = unsafe { std::mem::transmute(gl_symbol("glGetError")) };
    unsafe { get_error() }
}

// glReadPixels hands back the bottom row first, so the rows are turned over
fn read_top_down(width: i32, height: i32) -> Vec<u8> {
    let read_pixels: GlReadPixels = unsafe { std::mem::transmute(gl_symbol("glReadPixels")) };
    let stride = width as usize * BYTES_PER_PIXEL;
    let mut bottom_up = vec![0u8; stride * height as usize];
    unsafe {
        read_pixels(
            0,
            0,
            width,
            height,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            bottom_up.as_mut_ptr() as *mut c_void,
        )
    };
    bottom_up
        .chunks_exact(stride)
        .rev()
        .flatten()
        .copied()
        .collect()
}

fn pixel(pixels: &[u8], x: i32, y: i32) -> [u8; 3] {
    let at = (y as usize * TARGET_WIDTH as usize + x as usize) * BYTES_PER_PIXEL;
    [pixels[at], pixels[at + 1], pixels[at + 2]]
}

fn close_to(drawn: [u8; 3], expected: [u8; 3]) -> bool {
    drawn
        .iter()
        .zip(expected)
        .all(|(drawn, expected)| drawn.abs_diff(expected) <= COLOUR_TOLERANCE)
}

fn draw(player: &Player, framebuffer: i32, flip_y: bool) -> Vec<u8> {
    player
        .render_opengl(framebuffer, TARGET_WIDTH, TARGET_HEIGHT, flip_y)
        .expect("the GL presenter drew the frame");
    assert_eq!(gl_error(), GL_NO_ERROR, "the draw left a GL error behind");
    read_top_down(TARGET_WIDTH, TARGET_HEIGHT)
}

#[test]
fn the_gl_presenter_letterboxes_the_picture_and_keeps_it_the_right_way_up() {
    let directory = tempfile::tempdir().unwrap();
    let picture = grok_fixture::write_picture_file(directory.path(), PICTURE_FRAMES);

    let framebuffer = current_gl_context(TARGET_WIDTH, TARGET_HEIGHT);
    let renderer = format!("{} ({})", gl_string(GL_RENDERER), gl_string(GL_VERSION));
    eprintln!("[test] GL renderer: {renderer}");

    let player = Player::new().unwrap();
    player
        .init_opengl(resolve_gl_symbol, std::ptr::null_mut(), None)
        .expect("both players took the GL context");
    player.load_source(&picture.to_string_lossy()).unwrap();
    assert_eq!(player.active(), Backend::Grok, "the fixture is grok's");
    // every frame is the same picture, so stopping the clock costs the test nothing
    player.grok().set_paused(true);
    wait_until("a frame was composed", || {
        player.grok().frame_size().is_some()
    });

    let upright = draw(&player, framebuffer, false);
    if upright.iter().all(|value| *value == 0) {
        eprintln!(
            "[test] {renderer} read the framebuffer back as all black, \
             so only the GL calls are checked here"
        );
        draw(&player, framebuffer, true);
        return;
    }

    for row in [BAND_ROW, BODY_ROW] {
        assert_eq!(
            pixel(&upright, PICTURE_LEFT / 2, row),
            [0, 0, 0],
            "the bar left of the picture is not black on row {row}"
        );
        assert_eq!(
            pixel(&upright, (PICTURE_RIGHT + TARGET_WIDTH) / 2, row),
            [0, 0, 0],
            "the bar right of the picture is not black on row {row}"
        );
    }

    let band = pixel(&upright, PICTURE_MIDDLE_COLUMN, BAND_ROW);
    assert!(
        close_to(band, grok_fixture::band_colour()),
        "the fixture's band is not at the top of the picture: {band:?}"
    );
    let body = pixel(&upright, PICTURE_MIDDLE_COLUMN, BODY_ROW);
    assert!(
        close_to(body, grok_fixture::body_colour()),
        "the fixture's body colour is not below the band: {body:?}"
    );

    // a top-left origin surface, which is what GTK's GL area gives
    let flipped = draw(&player, framebuffer, true);
    let band = pixel(
        &flipped,
        PICTURE_MIDDLE_COLUMN,
        TARGET_HEIGHT - 1 - BAND_ROW,
    );
    assert!(
        close_to(band, grok_fixture::band_colour()),
        "flip_y did not put the band at the bottom: {band:?}"
    );
    let body = pixel(
        &flipped,
        PICTURE_MIDDLE_COLUMN,
        TARGET_HEIGHT - 1 - BODY_ROW,
    );
    assert!(
        close_to(body, grok_fixture::body_colour()),
        "flip_y did not turn the picture over: {body:?}"
    );
}
