/// Switch grok's accelerator plugin on or off for the whole process. The
/// `initialize` call loads the plugin and may run here first.
#[tauri::command]
pub fn set_gpu(enabled: bool) -> Result<bool, String> {
    postkit::grok_encoder::initialize(0);
    if enabled {
        postkit::grok_encoder::use_gpu()?;
    } else {
        postkit::grok_encoder::use_cpu();
    }
    Ok(postkit::grok_encoder::gpu_active())
}
