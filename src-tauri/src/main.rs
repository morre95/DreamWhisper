#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
fn main() {
    // Apply before GTK/WebKit or any worker threads are initialized.
    // NVIDIA's DMA-BUF path can terminate WebKitGTK on Wayland with Gdk Error 71.
    // https://v2.tauri.app/develop/debug/linux-graphics/
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_some()
        && std::path::Path::new("/sys/module/nvidia").exists()
        && std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none()
    {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        eprintln!("DreamWhisper: aktiverar WebKitGTK-kompatibilitet för NVIDIA på Wayland");
    }
    dreamwhisper_lib::run();
}
