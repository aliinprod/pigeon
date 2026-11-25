//! Pigeon desktop app (Tauri 2): one window, a tray / menu bar icon on
//! Windows and macOS, and one [`Backend`] holding the computer's one iroh
//! endpoint for both Share and Get.
//!
//! IPC: every command below is declared in `build.rs` and allowed one by one
//! in `capabilities/main.json`; the webview has no other access (no shell,
//! no fs, no plugin commands). Inputs are checked again in the backend. The
//! webview never sends a local path: folders come from the native picker,
//! and Open / Show in folder take a transfer ID, not a path.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{path::PathBuf, sync::Arc};

use pigeon_app::{Backend, Options};
use pigeon_core::config::Home;
use tauri::{Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

type Res<T> = Result<T, String>;

#[derive(Default)]
struct AppState {
    backend: std::sync::Mutex<Option<Arc<Backend>>>,
    error: std::sync::Mutex<Option<String>>,
}

fn be(state: &State<'_, AppState>) -> Res<Arc<Backend>> {
    if let Some(b) = state.backend.lock().expect("state poisoned").clone() {
        return Ok(b);
    }
    Err(state
        .error
        .lock()
        .expect("state poisoned")
        .clone()
        .unwrap_or_else(|| "Pigeon is still starting".into()))
}

fn err(e: anyhow::Error) -> String {
    pigeon_core::display_safe(&format!("{e:#}"))
}

#[tauri::command]
fn status(state: State<'_, AppState>) -> Res<pigeon_app::Status> {
    be(&state)?.status().map_err(err)
}

#[tauri::command]
fn wordlist() -> Vec<&'static str> {
    Backend::wordlist().to_vec()
}

/// Opens the native folder picker and shares the chosen folder.
#[tauri::command]
async fn add_folder(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Res<Option<pigeon_app::FolderView>> {
    let backend = be(&state)?;
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Share a folder (read-only)")
            .blocking_pick_folder()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(picked) = picked else {
        return Ok(None);
    };
    let path = picked.into_path().map_err(|e| e.to_string())?;
    backend.add_folder(&path).map(Some).map_err(err)
}

#[tauri::command]
fn remove_folder(state: State<'_, AppState>, name: String) -> Res<()> {
    be(&state)?.remove_folder(&name).map_err(err)
}

#[tauri::command]
fn create_invite(
    state: State<'_, AppState>,
    device_name: Option<String>,
) -> Res<pigeon_app::InviteView> {
    be(&state)?
        .create_invite(device_name.as_deref())
        .map_err(err)
}

#[tauri::command]
fn cancel_invite(state: State<'_, AppState>) -> Res<()> {
    be(&state)?.cancel_invite();
    Ok(())
}

#[tauri::command]
fn dismiss_invite(state: State<'_, AppState>) -> Res<()> {
    be(&state)?.dismiss_invite();
    Ok(())
}

#[tauri::command]
fn revoke_device(state: State<'_, AppState>, id: String) -> Res<()> {
    be(&state)?.revoke_device(&id).map_err(err)
}

#[tauri::command]
fn check_link(state: State<'_, AppState>, link: String) -> Res<pigeon_app::LinkView> {
    be(&state)?.check_link(&link).map_err(err)
}

/// The typed code goes from the webview straight into `pair::pair`; it is
/// not stored, logged or sent back.
#[tauri::command]
async fn pair_with(
    state: State<'_, AppState>,
    link: String,
    code: String,
    name: Option<String>,
) -> Res<pigeon_app::PairOutcome> {
    let code = zeroize::Zeroizing::new(code);
    let backend = be(&state)?;
    backend
        .pair(&link, &code, name.as_deref())
        .await
        .map_err(err)
}

#[tauri::command]
async fn list_dir(
    state: State<'_, AppState>,
    computer: String,
    path: String,
) -> Res<Vec<pigeon_app::EntryView>> {
    let backend = be(&state)?;
    backend.list(&computer, &path).await.map_err(err)
}

#[tauri::command]
fn get_file(state: State<'_, AppState>, computer: String, path: String) -> Res<u64> {
    be(&state)?.get(&computer, &path).map_err(err)
}

#[tauri::command]
fn resume_transfer(state: State<'_, AppState>, id: u64) -> Res<()> {
    be(&state)?.resume(id).map_err(err)
}

#[tauri::command]
fn clear_finished(state: State<'_, AppState>) -> Res<()> {
    be(&state)?.clear_finished();
    Ok(())
}

#[tauri::command]
fn open_file(app: tauri::AppHandle, state: State<'_, AppState>, id: u64) -> Res<()> {
    let path = be(&state)?.finished_file(id).map_err(err)?;
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn show_in_folder(app: tauri::AppHandle, state: State<'_, AppState>, id: u64) -> Res<()> {
    let path = be(&state)?.finished_file(id).map_err(err)?;
    app.opener()
        .reveal_item_in_dir(path)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn forget_computer(state: State<'_, AppState>, id: String) -> Res<()> {
    be(&state)?.forget_computer(&id).map_err(err)
}

#[tauri::command]
fn rename_computer(state: State<'_, AppState>, name: String) -> Res<String> {
    be(&state)?.rename(&name).map_err(err)
}

/// `Downloads/Pigeon`, or `$PIGEON_DOWNLOAD_DIR` (tests, a second instance).
fn download_dir(app: &tauri::App) -> anyhow::Result<PathBuf> {
    if let Some(dir) = std::env::var_os("PIGEON_DOWNLOAD_DIR").filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    Ok(app.path().download_dir()?.join("Pigeon"))
}

fn show_window(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn status_line(backend: Option<&Arc<Backend>>) -> String {
    let Some(b) = backend else {
        return "Pigeon · not running".into();
    };
    match b.status() {
        Ok(s) => {
            let active = s
                .transfers
                .iter()
                .filter(|t| t.state == pigeon_app::TransferState::Active)
                .count();
            let mut line = format!(
                "{} · {} folder{} shared",
                if s.online { "Online" } else { "Offline" },
                s.folders.len(),
                if s.folders.len() == 1 { "" } else { "s" }
            );
            if active > 0 {
                line.push_str(&format!(" · {active} getting"));
            }
            line
        }
        Err(_) => "Pigeon · error".into(),
    }
}

#[cfg(not(target_os = "linux"))]
fn tray(app: &tauri::App) -> tauri::Result<()> {
    use tauri::{
        image::Image,
        menu::{Menu, MenuItem, PredefinedMenuItem},
        tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    };
    let status = MenuItem::with_id(app, "status", "Pigeon · starting", false, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", "Open Pigeon", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Pigeon", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&status, &sep, &open, &quit])?;
    let tray = TrayIconBuilder::with_id("main")
        .icon(Image::from_bytes(include_bytes!("../icons/tray.png"))?)
        .tooltip("Pigeon")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_window(tray.app_handle());
            }
        })
        .build(app)?;
    let handle = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let backend = handle
                .state::<AppState>()
                .backend
                .lock()
                .expect("state poisoned")
                .clone();
            let line = status_line(backend.as_ref());
            let _ = status.set_text(&line);
            let _ = tray.set_tooltip(Some(format!("Pigeon\n{line}")));
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
    });
    Ok(())
}

fn main() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::default())
        .setup(|app| {
            let started = (|| -> anyhow::Result<Arc<Backend>> {
                let opts = Options {
                    home: Home::from_env()?,
                    download_dir: download_dir(app)?,
                    local_only: false,
                };
                tauri::async_runtime::block_on(Backend::start(opts))
            })();
            let state = app.state::<AppState>();
            match started {
                Ok(b) => *state.backend.lock().expect("state poisoned") = Some(b),
                Err(e) => {
                    eprintln!("Pigeon could not start: {e:#}");
                    *state.error.lock().expect("state poisoned") = Some(err(e));
                }
            }
            #[cfg(not(target_os = "linux"))]
            tray(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            // With a tray / menu bar icon, closing the window keeps sharing.
            #[cfg(not(target_os = "linux"))]
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
            #[cfg(target_os = "linux")]
            let _ = (window, event);
        })
        .invoke_handler(tauri::generate_handler![
            status,
            wordlist,
            add_folder,
            remove_folder,
            create_invite,
            cancel_invite,
            dismiss_invite,
            revoke_device,
            check_link,
            pair_with,
            list_dir,
            get_file,
            resume_transfer,
            clear_finished,
            open_file,
            show_in_folder,
            forget_computer,
            rename_computer,
        ])
        .build(tauri::generate_context!())
        .expect("could not start the Pigeon window");
    app.run(|handle, event| {
        if let tauri::RunEvent::Exit = event {
            let backend = handle
                .state::<AppState>()
                .backend
                .lock()
                .expect("state poisoned")
                .take();
            if let Some(b) = backend {
                tauri::async_runtime::block_on(b.shutdown());
            }
        }
    });
}
