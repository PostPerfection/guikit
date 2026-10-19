use std::path::Path;

use postkit::timeline::{probe_source, SourceLength};
use serde::Serialize;

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SourceLengthWire {
    Counted { frames: u64 },
    Still,
}

#[tauri::command(async)]
pub fn picture_source_length(picture: String) -> Result<SourceLengthWire, String> {
    Ok(match probe_source(Path::new(&picture))? {
        SourceLength::Counted { frames, .. } => SourceLengthWire::Counted { frames },
        SourceLength::Still => SourceLengthWire::Still,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ffmpeg_testsrc(frames: u32, output: &Path) {
        let finished = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
            .arg("testsrc=size=64x48:rate=24")
            .args(["-frames:v", &frames.to_string()])
            .arg(output)
            .output()
            .expect("ffmpeg");
        assert!(
            finished.status.success(),
            "{}",
            String::from_utf8_lossy(&finished.stderr)
        );
    }

    fn wire_length(picture: &Path) -> serde_json::Value {
        let length = picture_source_length(picture.to_string_lossy().into_owned()).unwrap();
        serde_json::to_value(length).unwrap()
    }

    #[test]
    fn an_image_file_is_a_still_and_a_directory_or_video_counts_its_frames() {
        let directory = tempfile::tempdir().unwrap();
        let card = directory.path().join("card.png");
        ffmpeg_testsrc(1, &card);
        let sequence = directory.path().join("sequence");
        std::fs::create_dir(&sequence).unwrap();
        ffmpeg_testsrc(3, &sequence.join("frame%04d.png"));
        let video = directory.path().join("clip.mkv");
        ffmpeg_testsrc(12, &video);

        assert_eq!(wire_length(&card), serde_json::json!({"kind": "still"}));
        assert_eq!(
            wire_length(&sequence),
            serde_json::json!({"kind": "counted", "frames": 3})
        );
        assert_eq!(
            wire_length(&video),
            serde_json::json!({"kind": "counted", "frames": 12})
        );
    }
}
