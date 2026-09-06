use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use asdcplib::jp2k::{CodestreamHeader, MxfWriter, PictureDescriptor};
use asdcplib::{LabelSet, Rational, WriterInfo};
use postkit::colour::XyzToSrgb;
use postkit::grok_encoder::{CompressParams, PhaseClocks, RawFrame};
use postkit::packaging::{ns, AssetMap, AssetMapAsset, DcpCpl, DcpCplReel};

pub(super) const FRAMES_PER_SECOND: u32 = 24;
// square, so a 16:9 surface leaves bars either side of the picture
pub(super) const PICTURE_SIDE: u32 = 64;
// rows of the source picture the band fills, counted from its top
pub(super) const BAND_ROWS: u32 = 16;

const CINEMA_2K_PROFILE: u16 = 0x0003;
// 12-bit X'Y'Z' code values, the body dark enough for the red crop band to read as red over it
const BODY_CODE: i32 = 300;
const BAND_CODE: i32 = 2600;
const PICTURE_PRECISION: u8 = 12;
// enough for the quarter decode scale, which drops two levels
const RESOLUTIONS: u8 = 3;

const PICTURE_NAME: &str = "picture.mxf";
const CPL_NAME: &str = "CPL_test.xml";
const ASSET_MAP_UUID: &str = "bbbbbbbb-0000-0000-0000-000000000000";
const CPL_UUID: &str = "cc10cc10-0000-0000-0000-000000000000";
const PICTURE_UUID: &str = "11111111-1111-1111-1111-111111111111";
const REEL_UUID: &str = "aaaaaaaa-0000-0000-0000-000000000000";

pub(super) const PATIENCE: Duration = Duration::from_secs(30);
pub(super) const POLL_INTERVAL: Duration = Duration::from_millis(5);

// only the gl presenter test, which is linux only, reads the band
#[cfg(target_os = "linux")]
pub(super) fn band_colour() -> [u8; 3] {
    code_colour(BAND_CODE)
}

pub(super) fn body_colour() -> [u8; 3] {
    code_colour(BODY_CODE)
}

fn code_colour(code: i32) -> [u8; 3] {
    let code = code as u16;
    XyzToSrgb::new().pixel(code, code, code)
}

pub(super) fn write_picture_file(directory: &Path, frames: usize) -> PathBuf {
    let path = directory.join(PICTURE_NAME);
    write_mxf(&path, &codestreams(frames));
    path
}

pub(super) fn write_package(directory: &Path, frames: usize) -> PathBuf {
    let package = directory.join("package");
    std::fs::create_dir_all(&package).unwrap();
    write_picture_file(&package, frames);

    let asset_map = AssetMap {
        uuid: ASSET_MAP_UUID.into(),
        namespace: ns::AM_SMPTE.into(),
        assets: vec![
            AssetMapAsset {
                id: CPL_UUID.into(),
                path: CPL_NAME.into(),
                ..Default::default()
            },
            AssetMapAsset {
                id: PICTURE_UUID.into(),
                path: PICTURE_NAME.into(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    std::fs::write(package.join("ASSETMAP.xml"), asset_map.to_xml()).unwrap();

    let cpl = DcpCpl {
        uuid: CPL_UUID.into(),
        namespace: ns::CPL_SMPTE.into(),
        title: "Grok Backend Test".into(),
        reels: vec![DcpCplReel {
            reel_id: REEL_UUID.into(),
            picture_id: PICTURE_UUID.into(),
            picture_edit_rate_num: FRAMES_PER_SECOND,
            picture_edit_rate_den: 1,
            picture_duration: frames as u64,
            picture_width: PICTURE_SIDE,
            picture_height: PICTURE_SIDE,
            ..Default::default()
        }],
        ..Default::default()
    };
    std::fs::write(package.join(CPL_NAME), cpl.to_xml()).unwrap();
    package
}

fn codestreams(count: usize) -> Vec<Vec<u8>> {
    let params = CompressParams {
        irreversible: false,
        compression_ratio: 1.0,
        mct: false,
        apply_xyz_transform: false,
        profile: CINEMA_2K_PROFILE,
        num_resolutions: RESOLUTIONS,
        ..CompressParams::default()
    };
    postkit::grok_encoder::initialize(0);
    let directory = tempfile::tempdir().unwrap();
    let plane = banded_plane();
    let mut next = 0usize;
    let result = postkit::grok_encoder::encode_pipeline(
        directory.path(),
        &params,
        count as u64,
        &Arc::new(AtomicBool::new(false)),
        &Arc::new(PhaseClocks::default()),
        || {
            if next >= count {
                return None;
            }
            let frame = RawFrame::Planar {
                components: [plane.clone(), plane.clone(), plane.clone()],
                width: PICTURE_SIDE,
                height: PICTURE_SIDE,
                precision: PICTURE_PRECISION,
                index: next as u64,
            };
            next += 1;
            Some(frame)
        },
        |_| {},
    );
    assert!(result.success, "fixture encode failed: {}", result.error);
    (0..count)
        .map(|index| {
            let path = directory.path().join(format!("frame_{index:08}.j2c"));
            std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        })
        .collect()
}

fn banded_plane() -> Vec<i32> {
    let mut plane = Vec::with_capacity((PICTURE_SIDE * PICTURE_SIDE) as usize);
    for row in 0..PICTURE_SIDE {
        let code = if row < BAND_ROWS {
            BAND_CODE
        } else {
            BODY_CODE
        };
        plane.extend(std::iter::repeat_n(code, PICTURE_SIDE as usize));
    }
    plane
}

fn write_mxf(path: &Path, frames: &[Vec<u8>]) {
    let info = WriterInfo {
        asset_uuid: [8; 16],
        context_id: [0xc7; 16],
        cryptographic_key_id: [0xd4; 16],
        label_set: LabelSet::Smpte,
        ..Default::default()
    };
    let descriptor = PictureDescriptor {
        edit_rate: Rational::new(FRAMES_PER_SECOND as i32, 1),
        sample_rate: Rational::new(FRAMES_PER_SECOND as i32, 1),
        stored_width: PICTURE_SIDE,
        stored_height: PICTURE_SIDE,
        aspect_ratio: Rational::new(PICTURE_SIDE as i32, PICTURE_SIDE as i32),
        container_duration: frames.len() as u32,
        codestream: CodestreamHeader::parse(&frames[0]).expect("the fixture is a codestream"),
    };
    let mut writer = MxfWriter::new();
    writer
        .open_write(&path.to_string_lossy(), &info, &descriptor, 16_384)
        .unwrap();
    for frame in frames {
        writer.write_frame(frame, None, None).unwrap();
    }
    writer.finalize().unwrap();
}

pub(super) fn wait_until(what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline {
        if ready() {
            return;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    panic!("{what} did not happen within {PATIENCE:?}");
}
