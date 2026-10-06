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

#[cfg(target_os = "linux")]
const MAIN_PAGE: &str = "index.html";

/// Build the main window with the webview as an explicit child, which is what
/// lets the preview render underneath it.
#[cfg(target_os = "linux")]
pub fn create_main_window(app: &tauri::App, window: &MainWindow) -> tauri::Result<()> {
    create_window(app, window, MAIN_PAGE, true)
}

// a second window built the same way, which the app shows when it needs it
#[cfg(target_os = "linux")]
pub fn create_hidden_window(
    app: &tauri::App,
    window: &MainWindow,
    page: &str,
) -> tauri::Result<()> {
    create_window(app, window, page, false)
}

#[cfg(target_os = "linux")]
fn create_window(
    app: &tauri::App,
    window: &MainWindow,
    page: &str,
    visible: bool,
) -> tauri::Result<()> {
    let built = tauri::window::WindowBuilder::new(app, window.label)
        .title(window.title)
        .inner_size(window.width, window.height)
        .min_inner_size(window.minimum_width, window.minimum_height)
        .background_color(window.background)
        .visible(visible)
        .build()?;
    let size = built.inner_size()?;
    let webview = tauri::webview::WebviewBuilder::new(
        window.webview_label,
        tauri::WebviewUrl::App(page.into()),
    )
    .background_color(window.background);
    built.add_child(webview, tauri::LogicalPosition::new(0, 0), size)?;
    Ok(())
}
