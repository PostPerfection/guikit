use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use tauri::Manager;

#[cfg(target_os = "macos")]
const OPEN_PROJECT_FILE_EVENT: &str = "open-project-file";

#[derive(Default)]
pub struct LaunchProject {
    launch_project: Mutex<Option<String>>,
    frontend_took_launch_path: AtomicBool,
}

impl LaunchProject {
    fn launch_project(&self) -> MutexGuard<'_, Option<String>> {
        self.launch_project
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn take(&self) -> Option<String> {
        let mut launch_project = self.launch_project();
        self.frontend_took_launch_path.store(true, Ordering::SeqCst);
        launch_project.take()
    }

    // Some when the frontend already took the launch path
    #[cfg(any(target_os = "macos", test))]
    fn store_or_hand_over(&self, path: String) -> Option<String> {
        let mut launch_project = self.launch_project();
        if self.frontend_took_launch_path.load(Ordering::SeqCst) {
            return Some(path);
        }
        *launch_project = Some(path);
        None
    }
}

fn is_project_file(path: &str, extension: &str) -> bool {
    path.strip_suffix(extension)
        .is_some_and(|stem| stem.ends_with('.'))
}

pub fn store_from_iter(
    launch_project: &LaunchProject,
    arguments: impl Iterator<Item = String>,
    extension: &str,
) {
    let project_file = arguments
        .skip(1)
        .find(|argument| is_project_file(argument, extension));
    if let Some(project_file) = project_file {
        *launch_project.launch_project() = Some(project_file);
    }
}

pub fn store_from_args(app: &tauri::AppHandle, extension: &str) {
    let arguments = std::env::args_os().map(|argument| argument.to_string_lossy().into_owned());
    store_from_iter(&app.state::<LaunchProject>(), arguments, extension);
}

#[cfg(any(target_os = "macos", test))]
fn project_paths_in_urls(urls: &[tauri::Url], extension: &str) -> Vec<String> {
    urls.iter()
        .filter_map(|url| url.to_file_path().ok())
        .filter_map(|path| path.into_os_string().into_string().ok())
        .filter(|path| is_project_file(path, extension))
        .collect()
}

pub fn handle_run_event(app: &tauri::AppHandle, event: &tauri::RunEvent, extension: &str) {
    #[cfg(target_os = "macos")]
    if let tauri::RunEvent::Opened { urls } = event {
        use tauri::Emitter;
        let launch_project = app.state::<LaunchProject>();
        for path in project_paths_in_urls(urls, extension) {
            if let Some(path) = launch_project.store_or_hand_over(path) {
                let _ = app.emit(OPEN_PROJECT_FILE_EVENT, path);
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, event, extension);
}

#[tauri::command]
pub fn take_launch_project_path(state: tauri::State<'_, LaunchProject>) -> Option<String> {
    state.take()
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXTENSION: &str = "dcpwizard";

    fn stored_from(arguments: &[&str]) -> LaunchProject {
        let launch_project = LaunchProject::default();
        store_from_iter(
            &launch_project,
            arguments.iter().map(|argument| argument.to_string()),
            EXTENSION,
        );
        launch_project
    }

    #[test]
    fn the_first_project_file_after_the_executable_is_the_launch_project() {
        let launch_project = stored_from(&[
            "/usr/bin/dcpwizard-gui.dcpwizard",
            "--verbose",
            "/home/user/.films/a.dcpwizard",
            "/home/user/b.dcpwizard",
        ]);
        assert_eq!(
            launch_project.take().as_deref(),
            Some("/home/user/.films/a.dcpwizard")
        );
        assert_eq!(launch_project.take(), None);
    }

    #[test]
    fn a_launch_with_no_project_file_has_no_launch_project() {
        let launch_project = stored_from(&[
            "dcpwizard-gui",
            "/home/user/film.mov",
            "/home/user/notdcpwizard",
        ]);
        assert_eq!(launch_project.take(), None);
    }

    #[test]
    fn a_file_opened_before_the_frontend_asked_waits_and_one_opened_after_is_handed_over() {
        let launch_project = LaunchProject::default();
        assert_eq!(
            launch_project.store_or_hand_over("/a.dcpwizard".into()),
            None
        );
        assert_eq!(launch_project.take().as_deref(), Some("/a.dcpwizard"));
        assert_eq!(
            launch_project
                .store_or_hand_over("/b.dcpwizard".into())
                .as_deref(),
            Some("/b.dcpwizard")
        );
        assert_eq!(launch_project.take(), None);
    }

    #[test]
    fn only_file_urls_naming_a_project_file_are_opened() {
        // a file url round trips to the platform's own path form
        let project = std::env::temp_dir().join("My Film.dcpwizard");
        let film = std::env::temp_dir().join("film.mov");
        let urls = [
            tauri::Url::from_file_path(&project).unwrap(),
            tauri::Url::from_file_path(&film).unwrap(),
            tauri::Url::parse("https://example.com/film.dcpwizard").unwrap(),
        ];
        assert_eq!(
            project_paths_in_urls(&urls, EXTENSION),
            [project.to_string_lossy().into_owned()]
        );
    }
}
