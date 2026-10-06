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
    let webview = webview_builder(window, MAIN_PAGE).background_color(window.background);
    create_window(app, window, webview, true)
}

// starts hidden, its page sits over the video of create_player_under_page
#[cfg(target_os = "linux")]
pub fn create_player_window(
    app: &tauri::App,
    window: &MainWindow,
    page: &str,
) -> tauri::Result<()> {
    let webview = webview_builder(window, page).transparent(true);
    create_window(app, window, webview, false)
}

#[cfg(target_os = "linux")]
fn webview_builder(window: &MainWindow, page: &str) -> tauri::webview::WebviewBuilder<tauri::Wry> {
    tauri::webview::WebviewBuilder::new(window.webview_label, tauri::WebviewUrl::App(page.into()))
}

#[cfg(target_os = "linux")]
fn create_window(
    app: &tauri::App,
    window: &MainWindow,
    webview: tauri::webview::WebviewBuilder<tauri::Wry>,
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
    built.add_child(webview, tauri::LogicalPosition::new(0, 0), size)?;
    Ok(())
}
