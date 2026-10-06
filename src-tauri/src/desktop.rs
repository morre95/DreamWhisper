use crate::{
    service::{export_text, Service},
    types::*,
};
use std::{path::PathBuf, sync::Arc};
use tauri::{Manager, State};
type AppState = Arc<Service>;
type ApiResult<T> = Result<T, String>;
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[tauri::command]
fn image_snapshot(state: State<'_, AppState>) -> ApiResult<crate::images::ImageSnapshot> {
    state.db.image_snapshot().map_err(err)
}
#[tauri::command]
fn save_comfy_settings(
    state: State<'_, AppState>,
    settings: crate::images::ComfySettings,
) -> ApiResult<()> {
    state.db.save_comfy_settings(&settings).map_err(err)
}
#[tauri::command]
fn save_workflow(
    state: State<'_, AppState>,
    workflow: crate::images::SavedWorkflow,
) -> ApiResult<String> {
    state.db.save_workflow(workflow).map_err(err)
}
#[tauri::command]
fn select_workflow(state: State<'_, AppState>, id: String) -> ApiResult<()> {
    state.db.select_workflow(&id).map_err(err)
}
#[tauri::command]
fn save_comfy_connection(
    state: State<'_, AppState>,
    settings: crate::images::ComfyConnection,
) -> ApiResult<()> {
    state.db.save_comfy_connection(settings).map_err(err)
}
#[tauri::command]
fn comfy_server_status(state: State<'_, AppState>) -> crate::comfy_server::ServerStatus {
    state.comfy_server.lock().unwrap().snapshot()
}
#[tauri::command]
async fn start_comfy_server(
    state: State<'_, AppState>,
) -> ApiResult<crate::comfy_server::ServerStatus> {
    let service = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let settings = service.db.comfy_settings().map_err(err)?;
        crate::comfy_server::ComfyServer::start(&service.comfy_server, &settings, &service.root)
            .map_err(err)
    })
    .await
    .map_err(err)?
}
#[tauri::command]
fn save_image_draft(state: State<'_, AppState>, draft: crate::images::ImageDraft) -> ApiResult<()> {
    state.db.save_image_draft(&draft).map_err(err)
}
#[tauri::command]
fn create_image(
    state: State<'_, AppState>,
    draft: crate::images::ImageDraft,
    workflow_id: Option<String>,
) -> ApiResult<String> {
    state
        .db
        .enqueue_image_with_workflow(draft, workflow_id.as_deref())
        .map_err(err)
}
#[tauri::command]
async fn dismiss_image_job(state: State<'_, AppState>, id: String) -> ApiResult<()> {
    let service = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = service.image_operations.lock().unwrap();
        service.db.dismiss_image_job(&id).map_err(err)
    })
    .await
    .map_err(err)?
}
#[tauri::command]
async fn follow_image_job(state: State<'_, AppState>, id: String) -> ApiResult<()> {
    let service = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = service.image_operations.lock().unwrap();
        service.db.retry_image_download(&id).map_err(err)
    })
    .await
    .map_err(err)?
}
#[tauri::command]
async fn test_comfy_connection(url: String) -> ApiResult<()> {
    tauri::async_runtime::spawn_blocking(move || crate::images::test_connection(&url).map_err(err))
        .await
        .map_err(err)?
}
#[tauri::command]
fn snapshot(state: State<'_, AppState>) -> ApiResult<Snapshot> {
    state.snapshot().map_err(err)
}
#[tauri::command]
async fn import_folder(state: State<'_, AppState>, path: String) -> ApiResult<ImportReport> {
    let service = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        service.import(&PathBuf::from(path), None).map_err(err)
    })
    .await
    .map_err(err)?
}
#[tauri::command]
async fn import_device(state: State<'_, AppState>, id: String) -> ApiResult<ImportReport> {
    let service = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || service.import_device(&id).map_err(err))
        .await
        .map_err(err)?
}
#[tauri::command]
async fn register_device(state: State<'_, AppState>, id: String) -> ApiResult<()> {
    let service = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || service.register(&id).map_err(err))
        .await
        .map_err(err)?
}
#[tauri::command]
fn unregister_device(state: State<'_, AppState>, id: String) -> ApiResult<()> {
    state.unregister(&id).map_err(err)
}
#[tauri::command]
fn save_settings(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    settings: Settings,
) -> ApiResult<()> {
    if settings.transcription_enabled
        && !PathBuf::from(&settings.model_path)
            .join("model.bin")
            .is_file()
    {
        return Err("Modellmappen saknar model.bin. Hämta modellen enligt README först.".into());
    }
    let previous = state.db.settings(&state.root).map_err(err)?;
    if settings.start_at_login != previous.start_at_login {
        if settings.start_at_login && cfg!(debug_assertions) && !cfg!(feature = "custom-protocol") {
            return Err("Start vid inloggning kräver en byggd app. Bygg och installera appen först enligt README.".into());
        }
        let executable = std::env::var_os("APPIMAGE")
            .map(PathBuf::from)
            .map_or_else(std::env::current_exe, Ok)
            .map_err(err)?;
        let config = app.path().config_dir().map_err(err)?;
        crate::autostart::configure(&config, &executable, settings.start_at_login).map_err(err)?;
        if let Err(e) = state.db.save_settings(&settings) {
            let _ = crate::autostart::configure(&config, &executable, previous.start_at_login);
            return Err(err(e));
        }
        return Ok(());
    }
    state.db.save_settings(&settings).map_err(err)
}
#[tauri::command]
fn control_queue(state: State<'_, AppState>, enabled: bool) -> ApiResult<()> {
    state.control_queue(enabled).map_err(err)
}
#[tauri::command]
fn retry_recording(state: State<'_, AppState>, id: String) -> ApiResult<()> {
    state.db.retry(&id).map_err(err)
}
#[tauri::command]
fn transcript(
    state: State<'_, AppState>,
    id: String,
    run_id: Option<String>,
) -> ApiResult<Transcript> {
    state.db.transcript(&id, run_id.as_deref()).map_err(err)
}
#[tauri::command]
fn transcription_runs(state: State<'_, AppState>, id: String) -> ApiResult<Vec<serde_json::Value>> {
    state.db.runs(&id).map_err(err)
}
#[tauri::command]
fn save_segment(state: State<'_, AppState>, id: i64, text: String) -> ApiResult<()> {
    state.db.edit(id, &text).map_err(err)
}
#[tauri::command]
fn search_recordings(state: State<'_, AppState>, query: String) -> ApiResult<Vec<String>> {
    state.db.search(&query).map_err(err)
}
#[tauri::command]
fn export_transcript(
    state: State<'_, AppState>,
    id: String,
    run_id: Option<String>,
    format: String,
) -> ApiResult<String> {
    let transcript = state.db.transcript(&id, run_id.as_deref()).map_err(err)?;
    let run = transcript
        .run_id
        .as_ref()
        .ok_or("Inspelningen har inget transkript ännu")?;
    let text = export_text(&transcript, &format).map_err(err)?;
    let export = state.root.join("exports");
    std::fs::create_dir_all(&export).map_err(err)?;
    // Run IDs come from SQLite, formats are validated by export_text.
    let path = export.join(format!("{run}-{}.{}", uuid::Uuid::new_v4(), format));
    std::fs::write(&path, text).map_err(err)?;
    Ok(path.to_string_lossy().into())
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let root = app.path().app_data_dir()?;
            let service = Service::open(root.clone())?;
            app.asset_protocol_scope()
                .allow_directory(root.join("archive"), true)?;
            app.asset_protocol_scope()
                .allow_directory(root.join("images"), true)?;
            let development = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../worker/worker.py");
            let script = if cfg!(debug_assertions) && development.is_file() {
                development
            } else {
                app.path()
                    .resolve("worker/worker.py", tauri::path::BaseDirectory::Resource)?
            };
            service.start(script);
            app.manage(service);
            let show = tauri::menu::MenuItem::with_id(
                app,
                "show",
                "Öppna DreamWhisper",
                true,
                None::<&str>,
            )?;
            let quit = tauri::menu::MenuItem::with_id(app, "quit", "Avsluta", true, None::<&str>)?;
            let menu = tauri::menu::Menu::with_items(app, &[&show, &quit])?;
            let mut tray = tauri::tray::TrayIconBuilder::with_id("dreamwhisper")
                .menu(&menu)
                .tooltip("DreamWhisper · lokalt ljudarkiv")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;
            if std::env::args().any(|a| a == "--background") {
                if let Some(window) = app.get_webview_window("main") {
                    window.hide()?;
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            save_workflow,
            select_workflow,
            save_comfy_connection,
            comfy_server_status,
            start_comfy_server,
            image_snapshot,
            save_comfy_settings,
            save_image_draft,
            create_image,
            follow_image_job,
            dismiss_image_job,
            test_comfy_connection,
            snapshot,
            import_folder,
            import_device,
            register_device,
            unregister_device,
            save_settings,
            retry_recording,
            control_queue,
            transcript,
            transcription_runs,
            save_segment,
            search_recordings,
            export_transcript
        ])
        .build(tauri::generate_context!())
        .expect("DreamWhisper kunde inte startas")
        .run(|app, event| {
            if matches!(
                event,
                tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
            ) {
                if let Some(service) = app.try_state::<AppState>() {
                    service.shutdown();
                }
            }
        });
}
