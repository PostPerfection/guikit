/// Fork a parent process that waits for the app to exit, then unconditionally
/// restores terminal settings. WebKitGTK child processes corrupt the terminal
/// after the main process exits. A no-op on other Unix platforms: WKWebView in
/// a forked process never commits a page load, so the window stays blank.
#[cfg(all(unix, not(target_os = "linux")))]
pub fn fork_terminal_guard() {}

#[cfg(target_os = "linux")]
pub fn fork_terminal_guard() {
    unsafe {
        if libc::isatty(libc::STDIN_FILENO) == 0 {
            return;
        }

        let mut saved: libc::termios = std::mem::zeroed();
        libc::tcgetattr(libc::STDIN_FILENO, &mut saved);

        let pid = libc::fork();
        if pid < 0 {
            return;
        }
        if pid > 0 {
            let mut status: libc::c_int = 0;
            libc::waitpid(pid, &mut status, 0);
            // let orphaned webkitgtk processes settle before the reset
            libc::usleep(100_000);
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSAFLUSH, &saved);
            libc::system(c"stty sane 2>/dev/null".as_ptr());
            let exit_code = if libc::WIFEXITED(status) {
                libc::WEXITSTATUS(status)
            } else {
                1
            };
            std::process::exit(exit_code);
        }
        // the child runs the app; stdin goes to /dev/null so webkitgtk
        // subprocesses cannot touch the terminal
        let devnull = libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY);
        if devnull >= 0 {
            libc::dup2(devnull, libc::STDIN_FILENO);
            libc::close(devnull);
        }
    }
}

#[cfg(target_os = "linux")]
const DRM_DEVICES_DIRECTORY: &str = "/sys/class/drm";
#[cfg(target_os = "linux")]
const DRM_NODE_VENDOR_FILE: &str = "device/vendor";
#[cfg(target_os = "linux")]
const NVIDIA_PCI_VENDOR_ID: &str = "0x10de";
#[cfg(target_os = "linux")]
const WEBKIT_DMABUF_RENDERER_FORCE_SHM: &str = "WEBKIT_DMABUF_RENDERER_FORCE_SHM";
#[cfg(target_os = "linux")]
const WEBKIT_DISABLE_DMABUF_RENDERER: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";
#[cfg(target_os = "linux")]
const WEBKIT_DISABLE_COMPOSITING_MODE: &str = "WEBKIT_DISABLE_COMPOSITING_MODE";
#[cfg(target_os = "linux")]
const WEBKIT_RENDERER_VARIABLES: [&str; 3] = [
    WEBKIT_DMABUF_RENDERER_FORCE_SHM,
    WEBKIT_DISABLE_DMABUF_RENDERER,
    WEBKIT_DISABLE_COMPOSITING_MODE,
];

/// With an NVIDIA GPU present, make WebKitGTK hand its frames to the
/// compositor in shared memory instead of DMA-BUF, which froze the app on the
/// NVIDIA driver under Wayland. Rendering stays on the GPU. Does nothing when
/// any of WebKit's renderer variables is already set. WebKit reads them once,
/// so call this before anything touches GTK.
#[cfg(target_os = "linux")]
pub fn prefer_shared_memory_webkit_frames_on_nvidia() {
    let nvidia_present = nvidia_gpu_present(std::path::Path::new(DRM_DEVICES_DIRECTORY));
    if should_force_shared_memory_frames(nvidia_present, |name| std::env::var_os(name)) {
        std::env::set_var(WEBKIT_DMABUF_RENDERER_FORCE_SHM, "1");
    }
}

#[cfg(target_os = "linux")]
fn nvidia_gpu_present(drm_directory: &std::path::Path) -> bool {
    let Ok(nodes) = std::fs::read_dir(drm_directory) else {
        return false;
    };
    nodes.flatten().any(|node| {
        std::fs::read_to_string(node.path().join(DRM_NODE_VENDOR_FILE))
            .is_ok_and(|vendor| vendor.trim() == NVIDIA_PCI_VENDOR_ID)
    })
}

#[cfg(target_os = "linux")]
fn should_force_shared_memory_frames(
    nvidia_present: bool,
    environment_variable: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> bool {
    nvidia_present
        && WEBKIT_RENDERER_VARIABLES
            .iter()
            .all(|name| environment_variable(name).is_none())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::path::Path;

    const AMD_PCI_VENDOR_ID: &str = "0x1002";

    fn add_drm_node(drm_directory: &Path, node: &str, vendor: Option<&str>) {
        let device = drm_directory.join(node).join("device");
        std::fs::create_dir_all(&device).unwrap();
        if let Some(vendor) = vendor {
            std::fs::write(device.join("vendor"), format!("{vendor}\n")).unwrap();
        }
    }

    fn no_environment(_name: &str) -> Option<OsString> {
        None
    }

    #[test]
    fn an_nvidia_node_forces_shared_memory_frames() {
        let drm_directory = tempfile::tempdir().unwrap();
        add_drm_node(drm_directory.path(), "card0", Some(NVIDIA_PCI_VENDOR_ID));
        add_drm_node(drm_directory.path(), "card1", Some(AMD_PCI_VENDOR_ID));

        let nvidia_present = nvidia_gpu_present(drm_directory.path());

        assert!(nvidia_present);
        assert!(should_force_shared_memory_frames(
            nvidia_present,
            no_environment
        ));
    }

    #[test]
    fn only_amd_nodes_leave_webkit_alone() {
        let drm_directory = tempfile::tempdir().unwrap();
        add_drm_node(drm_directory.path(), "card1", Some(AMD_PCI_VENDOR_ID));
        add_drm_node(drm_directory.path(), "renderD128", Some(AMD_PCI_VENDOR_ID));

        let nvidia_present = nvidia_gpu_present(drm_directory.path());

        assert!(!nvidia_present);
        assert!(!should_force_shared_memory_frames(
            nvidia_present,
            no_environment
        ));
    }

    #[test]
    fn a_renderer_variable_the_user_set_wins_even_when_empty() {
        for name in WEBKIT_RENDERER_VARIABLES {
            let environment = HashMap::from([(name, OsString::new())]);
            assert!(
                !should_force_shared_memory_frames(true, |lookup| environment.get(lookup).cloned()),
                "{name} was set but shared memory frames were still forced"
            );
        }
    }

    #[test]
    fn a_node_without_a_vendor_file_is_skipped() {
        let drm_directory = tempfile::tempdir().unwrap();
        add_drm_node(drm_directory.path(), "card0-DP-1", None);
        assert!(!nvidia_gpu_present(drm_directory.path()));

        add_drm_node(
            drm_directory.path(),
            "renderD129",
            Some(NVIDIA_PCI_VENDOR_ID),
        );
        assert!(nvidia_gpu_present(drm_directory.path()));
    }
}
