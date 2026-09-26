use postkit::component_versions::ComponentVersion;

use crate::preview::PreviewPlayer;

pub fn installed_components(
    application_name: &str,
    application_version: &str,
    preview_player: &PreviewPlayer,
) -> Vec<ComponentVersion> {
    let mut versions =
        postkit::component_versions::installed_components(application_name, application_version);
    versions.push(ComponentVersion::new("mpv", preview_player.mpv_version()));
    versions
}
