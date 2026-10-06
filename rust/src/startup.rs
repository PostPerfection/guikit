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
