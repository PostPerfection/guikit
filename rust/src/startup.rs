//! App startup boilerplate every wizard repeats: keeping the terminal usable
//! after the app exits, and building the main window with an embedded webview.

/// Fork a parent process that waits for the app to exit, then unconditionally
/// restores terminal settings. WebKitGTK child processes corrupt the terminal
/// after the main process exits.
#[cfg(unix)]
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

/// What the app's main window looks like before any content loads.
#[cfg(target_os = "linux")]
pub struct MainWindow<'a> {
    pub label: &'a str,
    pub webview_label: &'a str,
    pub title: &'a str,
    pub width: f64,
    pub height: f64,
    pub minimum_width: f64,
    pub minimum_height: f64,
    pub background: tauri::window::Color,
}

/// Build the main window with the webview as an explicit child, which is what
/// lets the preview render underneath it.
#[cfg(target_os = "linux")]
pub fn create_main_window(app: &tauri::App, window: &MainWindow) -> tauri::Result<()> {
    let built = tauri::window::WindowBuilder::new(app, window.label)
        .title(window.title)
        .inner_size(window.width, window.height)
        .min_inner_size(window.minimum_width, window.minimum_height)
        .background_color(window.background)
        .build()?;
    let size = built.inner_size()?;
    let webview = tauri::webview::WebviewBuilder::new(
        window.webview_label,
        tauri::WebviewUrl::App("index.html".into()),
    )
    .background_color(window.background);
    built.add_child(webview, tauri::LogicalPosition::new(0, 0), size)?;
    Ok(())
}
