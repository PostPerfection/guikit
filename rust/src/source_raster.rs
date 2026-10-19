use std::path::Path;

use serde::Serialize;

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct PictureRaster {
    pub width: u32,
    pub height: u32,
}

#[tauri::command(async)]
pub fn picture_source_raster(picture: String) -> Result<PictureRaster, String> {
    let (width, height) = postkit::encode::source_raster(Path::new(&picture))?;
    Ok(PictureRaster { width, height })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_directory_of_images_measures_its_first_frame() {
        let directory = tempfile::tempdir().unwrap();
        let finished = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
            .arg("testsrc=size=64x48:rate=24")
            .args(["-frames:v", "3"])
            .arg(directory.path().join("frame%04d.png"))
            .output()
            .expect("ffmpeg");
        assert!(
            finished.status.success(),
            "{}",
            String::from_utf8_lossy(&finished.stderr)
        );

        let raster =
            picture_source_raster(directory.path().to_string_lossy().into_owned()).unwrap();
        assert_eq!(
            serde_json::to_value(raster).unwrap(),
            serde_json::json!({"width": 64, "height": 48})
        );
    }
}
