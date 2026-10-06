use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use asdcplib::crypto::{AesEncContext, HmacContext};
use asdcplib::jp2k::{
    CodestreamHeader, MxfWriter, PictureDescriptor, StereoMxfWriter, StereoscopicPhase,
};
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
const STEREO_PICTURE_NAME: &str = "stereo.mxf";
const AS02_PICTURE_NAME: &str = "as02.mxf";
// bytes reserved for the MXF header partition
const MXF_HEADER_SIZE: u32 = 16_384;
const CPL_NAME: &str = "CPL_test.xml";
const ASSET_MAP_UUID: &str = "bbbbbbbb-0000-0000-0000-000000000000";
const CPL_UUID: &str = "cc10cc10-0000-0000-0000-000000000000";
const PICTURE_UUID: &str = "11111111-1111-1111-1111-111111111111";
const REEL_UUID: &str = "aaaaaaaa-0000-0000-0000-000000000000";

const ORIGINAL_VERSION_CPL_UUID: &str = "0e000000-0000-0000-0000-000000000000";
const VERSION_FILE_PICTURES: [(&str, &str); 3] = [
    ("21111111-1111-1111-1111-111111111111", "reel1.mxf"),
    ("22222222-2222-2222-2222-222222222222", "reel2.mxf"),
    ("23333333-3333-3333-3333-333333333333", "reel3.mxf"),
];
const ORIGINAL_VERSION_REEL: usize = 1;

const INITIALISATION_VECTOR: [u8; 16] = [0x9c; 16];

pub(super) const PATIENCE: Duration = Duration::from_secs(30);
pub(super) const POLL_INTERVAL: Duration = Duration::from_millis(5);

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

pub(super) struct PictureEncryption {
    pub(super) content_key: [u8; 16],
    pub(super) key_id: [u8; 16],
}

pub(super) fn write_picture_file(directory: &Path, frames: usize) -> PathBuf {
    let path = directory.join(PICTURE_NAME);
    write_mxf(&path, &codestreams(frames), None);
    path
}

// the IMF wrapping of the same codestreams
pub(super) fn write_as02_picture_file(directory: &Path, frames: usize) -> PathBuf {
    let path = directory.join(AS02_PICTURE_NAME);
    let frames = codestreams(frames);
    let info = WriterInfo {
        label_set: LabelSet::Smpte,
        ..Default::default()
    };
    let mut writer = asdcplib::as02::jp2k::MxfWriter::new();
    writer
        .open_write(
            &path.to_string_lossy(),
            &info,
            &picture_descriptor(&frames),
            MXF_HEADER_SIZE,
        )
        .unwrap();
    for frame in &frames {
        writer.write_frame(frame, None, None).unwrap();
    }
    writer.finalize().unwrap();
    path
}

pub(super) fn write_package(directory: &Path, frames: usize) -> PathBuf {
    write_package_with(directory, frames, None)
}

pub(super) fn write_encrypted_package(
    directory: &Path,
    frames: usize,
    encryption: &PictureEncryption,
) -> PathBuf {
    write_package_with(directory, frames, Some(encryption))
}

fn write_package_with(
    directory: &Path,
    frames: usize,
    encryption: Option<&PictureEncryption>,
) -> PathBuf {
    let package = directory.join("package");
    std::fs::create_dir_all(&package).unwrap();
    write_mxf(
        &package.join(PICTURE_NAME),
        &codestreams(frames),
        encryption,
    );

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

// the version file's CPL and the original version package, which alone holds reel 2
pub(super) fn write_version_file_and_original_version(
    directory: &Path,
    frames_per_reel: usize,
) -> (PathBuf, PathBuf) {
    let version_file = directory.join("version_file");
    let original_version = directory.join("original_version");
    let frames = codestreams(frames_per_reel * VERSION_FILE_PICTURES.len());
    let mut reels = Vec::new();
    for (index, (picture_id, name)) in VERSION_FILE_PICTURES.iter().enumerate() {
        let package = match index {
            ORIGINAL_VERSION_REEL => &original_version,
            _ => &version_file,
        };
        std::fs::create_dir_all(package).unwrap();
        let reel_frames = &frames[index * frames_per_reel..(index + 1) * frames_per_reel];
        write_mxf(&package.join(name), reel_frames, None);
        reels.push(DcpCplReel {
            reel_id: format!("aaaaaaaa-0000-0000-0000-00000000000{index}"),
            picture_id: (*picture_id).into(),
            picture_edit_rate_num: FRAMES_PER_SECOND,
            picture_edit_rate_den: 1,
            picture_duration: frames_per_reel as u64,
            picture_width: PICTURE_SIDE,
            picture_height: PICTURE_SIDE,
            ..Default::default()
        });
    }
    let original_version_pictures = [VERSION_FILE_PICTURES[ORIGINAL_VERSION_REEL]];
    let version_file_pictures: Vec<(&str, &str)> = VERSION_FILE_PICTURES
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != ORIGINAL_VERSION_REEL)
        .map(|(_, picture)| *picture)
        .collect();
    write_composition(
        &original_version,
        ORIGINAL_VERSION_CPL_UUID,
        &original_version_pictures,
        vec![reels[ORIGINAL_VERSION_REEL].clone()],
    );
    write_composition(&version_file, CPL_UUID, &version_file_pictures, reels);
    (version_file.join(CPL_NAME), original_version)
}

fn write_composition(
    package: &Path,
    cpl_id: &str,
    pictures: &[(&str, &str)],
    reels: Vec<DcpCplReel>,
) {
    let assets = std::iter::once((cpl_id, CPL_NAME))
        .chain(pictures.iter().copied())
        .map(|(id, path)| AssetMapAsset {
            id: id.into(),
            path: path.into(),
            ..Default::default()
        })
        .collect();
    let asset_map = AssetMap {
        uuid: ASSET_MAP_UUID.into(),
        namespace: ns::AM_SMPTE.into(),
        assets,
        ..Default::default()
    };
    std::fs::write(package.join("ASSETMAP.xml"), asset_map.to_xml()).unwrap();
    let cpl = DcpCpl {
        uuid: cpl_id.into(),
        namespace: ns::CPL_SMPTE.into(),
        title: "Grok Version File Test".into(),
        reels,
        ..Default::default()
    };
    std::fs::write(package.join(CPL_NAME), cpl.to_xml()).unwrap();
}

// the left eye flat body colour, the right eye flat band colour
pub(super) fn write_stereo_picture_file(directory: &Path, frames: usize) -> PathBuf {
    let path = directory.join(STEREO_PICTURE_NAME);
    let left = codestreams_of(&flat_plane(BODY_CODE), frames);
    let right = codestreams_of(&flat_plane(BAND_CODE), frames);
    let info = WriterInfo {
        asset_uuid: [9; 16],
        label_set: LabelSet::Smpte,
        ..Default::default()
    };
    let mut writer = StereoMxfWriter::new();
    writer
        .open_write(
            &path.to_string_lossy(),
            &info,
            &picture_descriptor(&left),
            MXF_HEADER_SIZE,
        )
        .unwrap();
    for (left, right) in left.iter().zip(&right) {
        writer
            .write_frame(left, StereoscopicPhase::Left, None, None)
            .unwrap();
        writer
            .write_frame(right, StereoscopicPhase::Right, None, None)
            .unwrap();
    }
    writer.finalize().unwrap();
    path
}

pub(super) fn left_eye_colour() -> [u8; 3] {
    code_colour(BODY_CODE)
}

pub(super) fn right_eye_colour() -> [u8; 3] {
    code_colour(BAND_CODE)
}

fn codestreams(count: usize) -> Vec<Vec<u8>> {
    codestreams_of(&banded_plane(), count)
}

fn codestreams_of(plane: &[i32], count: usize) -> Vec<Vec<u8>> {
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
    let mut next = 0usize;
    let result = postkit::grok_encoder::encode_pipeline(
        directory.path(),
        &params,
        count as u64,
        &Arc::new(AtomicBool::new(false)),
        &Arc::new(PhaseClocks::default()),
        |_: &postkit::grok_encoder::FrameBufferPool| {
            if next >= count {
                return None;
            }
            let frame = RawFrame::Planar {
                components: [plane.to_vec(), plane.to_vec(), plane.to_vec()],
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

fn flat_plane(code: i32) -> Vec<i32> {
    vec![code; (PICTURE_SIDE * PICTURE_SIDE) as usize]
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

fn picture_descriptor(frames: &[Vec<u8>]) -> PictureDescriptor {
    PictureDescriptor {
        edit_rate: Rational::new(FRAMES_PER_SECOND as i32, 1),
        sample_rate: Rational::new(FRAMES_PER_SECOND as i32, 1),
        stored_width: PICTURE_SIDE,
        stored_height: PICTURE_SIDE,
        aspect_ratio: Rational::new(PICTURE_SIDE as i32, PICTURE_SIDE as i32),
        container_duration: frames.len() as u32,
        codestream: CodestreamHeader::parse(&frames[0]).expect("the fixture is a codestream"),
    }
}

fn write_mxf(path: &Path, frames: &[Vec<u8>], encryption: Option<&PictureEncryption>) {
    let info = WriterInfo {
        asset_uuid: [8; 16],
        context_id: [0xc7; 16],
        cryptographic_key_id: encryption.map_or([0xd4; 16], |encryption| encryption.key_id),
        encrypted_essence: encryption.is_some(),
        uses_hmac: encryption.is_some(),
        label_set: LabelSet::Smpte,
        ..Default::default()
    };
    let mut writer = MxfWriter::new();
    writer
        .open_write(
            &path.to_string_lossy(),
            &info,
            &picture_descriptor(frames),
            MXF_HEADER_SIZE,
        )
        .unwrap();
    let mut crypto = encryption.map(|encryption| {
        let mut encryptor = AesEncContext::new();
        encryptor.init_key(&encryption.content_key).unwrap();
        encryptor.set_ivec(&INITIALISATION_VECTOR).unwrap();
        let mut hmac = HmacContext::new();
        hmac.init_key(&encryption.content_key, LabelSet::Smpte)
            .unwrap();
        (encryptor, hmac)
    });
    for frame in frames {
        match crypto.as_mut() {
            Some((encryptor, hmac)) => writer
                .write_frame(frame, Some(encryptor), Some(hmac))
                .unwrap(),
            None => writer.write_frame(frame, None, None).unwrap(),
        }
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
