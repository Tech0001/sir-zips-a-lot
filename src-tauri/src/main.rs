#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use sir_zips_a_lot::watcher::{Config, Watcher, save_json};
use tauri::{
    AppHandle, Manager, State, WindowEvent,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LogEntry {
    id: u64,
    timestamp: u64,
    kind: String,
    message: String,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    running: bool,
    stopping: bool,
    pending: usize,
    delivered: usize,
    config: Option<Config>,
    activity: Vec<LogEntry>,
    next_id: u64,
}

#[derive(Clone)]
struct AppState {
    status: Arc<Mutex<Status>>,
    active: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    quitting: Arc<AtomicBool>,
    data_dir: PathBuf,
}

impl AppState {
    fn log(&self, kind: &str, message: String) {
        let mut status = self.status.lock().expect("Status lock poisoned");
        status.next_id += 1;
        let id = status.next_id;
        status.activity.push(LogEntry {
            id,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            kind: kind.into(),
            message,
        });
        if status.activity.len() > 100 {
            status.activity.remove(0);
        }
    }
}

#[tauri::command]
fn get_status(state: State<'_, AppState>) -> Result<Status, String> {
    state
        .status
        .lock()
        .map(|status| status.clone())
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn start_watch(config: Config, state: State<'_, AppState>) -> Result<(), String> {
    let state = state.inner().clone();
    state
        .active
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .map_err(|_| "The watcher is already running or stopping".to_string())?;
    state.stop.store(false, Ordering::SeqCst);
    if state.quitting.load(Ordering::SeqCst) {
        state.active.store(false, Ordering::SeqCst);
        return Err("The app is quitting".into());
    }
    let setup_state = state.clone();
    let setup_config = config.clone();
    let setup = tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<Watcher> {
        let watcher = Watcher::new(
            setup_config.clone(),
            setup_state.data_dir.join("history.json"),
        )?;
        save_json(&setup_state.data_dir.join("settings.json"), &setup_config)?;
        Ok(watcher)
    })
    .await;
    let mut watcher = match setup {
        Ok(Ok(watcher)) => watcher,
        error => {
            state.active.store(false, Ordering::SeqCst);
            return Err(match error {
                Ok(Err(error)) => format!("{error:#}"),
                Err(error) => error.to_string(),
                _ => unreachable!(),
            });
        }
    };
    {
        let mut status = state.status.lock().expect("Status lock poisoned");
        status.running = true;
        status.stopping = false;
        status.pending = 0;
        status.config = Some(config);
    }
    state.log("info", "On duty. Watching for new order folders.".into());
    tauri::async_runtime::spawn_blocking(move || {
        while !state.stop.load(Ordering::SeqCst) && !state.quitting.load(Ordering::SeqCst) {
            match watcher.poll_while(|| {
                !state.stop.load(Ordering::SeqCst) && !state.quitting.load(Ordering::SeqCst)
            }) {
                Ok(activity) => {
                    for item in activity {
                        if item.kind == "success" {
                            state.status.lock().expect("Status lock poisoned").delivered += 1;
                        }
                        state.log(&item.kind, item.message);
                    }
                    state.status.lock().expect("Status lock poisoned").pending = watcher.pending();
                }
                Err(error) => {
                    state.log("error", format!("Watching stopped: {error:#}"));
                    break;
                }
            }
            // Respond promptly to Stop between scans.
            for _ in 0..20 {
                if state.stop.load(Ordering::SeqCst) {
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
        }
        state.log("info", "Off duty. Watching stopped.".into());
        {
            let mut status = state.status.lock().expect("Status lock poisoned");
            status.running = false;
            status.stopping = false;
            status.pending = 0;
        }
        state.active.store(false, Ordering::SeqCst);
    });
    Ok(())
}

#[tauri::command]
fn stop_watch(state: State<'_, AppState>) {
    state.stop.store(true, Ordering::SeqCst);
    let mut status = state.status.lock().expect("Status lock poisoned");
    status.stopping = status.running;
}

fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn quit_after_current_order(app: &AppHandle) {
    let state = app.state::<AppState>().inner().clone();
    if state.quitting.swap(true, Ordering::SeqCst) {
        return;
    }
    state.stop.store(true, Ordering::SeqCst);
    {
        let mut status = state.status.lock().expect("Status lock poisoned");
        status.stopping = status.running;
    }
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        while state.active.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(100));
        }
        app.exit(0);
    });
}

fn main() {
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        // Avoid WebKitGTK's blank-window/GBM failures on NVIDIA desktops.
        // SAFETY: This runs before Tauri or any application threads are started.
        unsafe {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            show_window(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let initial_status = Status {
                config: app.path().desktop_dir().ok().map(|desktop| Config {
                    source: PathBuf::new(),
                    destination: desktop.join("Zipped Orders"),
                    quiet_seconds: 30,
                }),
                ..Status::default()
            };
            let state = AppState {
                status: Arc::new(Mutex::new(initial_status)),
                active: Arc::new(AtomicBool::new(false)),
                stop: Arc::new(AtomicBool::new(false)),
                quitting: Arc::new(AtomicBool::new(false)),
                data_dir,
            };
            match fs::read(state.data_dir.join("settings.json")) {
                Ok(bytes) => match serde_json::from_slice::<Config>(&bytes) {
                    Ok(config) => {
                        state.status.lock().expect("Status lock poisoned").config = Some(config)
                    }
                    Err(error) => state.log(
                        "error",
                        format!("Saved settings could not be read: {error}"),
                    ),
                },
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => state.log(
                    "error",
                    format!("Saved settings could not be opened: {error}"),
                ),
            }
            app.manage(state);
            let open = MenuItem::with_id(app, "open", "Open Sir Zips-a-Lot", true, None::<&str>)?;
            let separator = PredefinedMenuItem::separator(app)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &separator, &quit])?;
            TrayIconBuilder::with_id("main-tray")
                .icon(app.default_window_icon().ok_or("Missing app icon")?.clone())
                .tooltip("Sir Zips-a-Lot — your order courier")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show_window(app),
                    "quit" => quit_after_current_order(app),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if matches!(
                        event,
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        }
                    ) {
                        show_window(tray.app_handle());
                    }
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                if let Err(error) = window.hide() {
                    window
                        .app_handle()
                        .state::<AppState>()
                        .log("error", format!("Cannot hide window: {error}"));
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_status,
            start_watch,
            stop_watch
        ])
        .run(tauri::generate_context!())
        .expect("Could not launch Sir Zips-a-Lot");
}
