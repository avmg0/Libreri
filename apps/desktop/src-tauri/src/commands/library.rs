use super::blocking;
use crate::dto::{FolderKind, LibrarySummary};
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use libreri_library::relocate::MovePlan;
use libreri_library::{Library, OpenOptions};
use serde::Serialize;
use specta::Type;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Manager, State};
use tauri_specta::Event;

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Tells the Welcome screen what the chosen folder contains.
#[tauri::command]
#[specta::specta]
pub async fn inspect_folder(path: String) -> FolderKind {
    // A folder on a sleeping disk or a network share can take a while.
    tauri::async_runtime::spawn_blocking(move || folder_kind(Path::new(&path)))
        .await
        .unwrap_or(FolderKind::Missing)
}

fn folder_kind(path: &Path) -> FolderKind {
    if !path.exists() {
        FolderKind::Missing
    } else if Library::is_library(path) {
        FolderKind::Library
    } else if std::fs::read_dir(path)
        .map(|mut d| d.next().is_none())
        .unwrap_or(false)
    {
        FolderKind::Empty
    } else {
        FolderKind::OtherFiles
    }
}

/// Creates a new library in `path` and opens it.
#[tauri::command]
#[specta::specta]
pub async fn create_library(
    app: AppHandle,
    path: String,
    name: Option<String>,
) -> AppResult<LibrarySummary> {
    blocking(move || {
        let state = app.state::<AppState>();
        let _opening = state.opening();
        state.close_library();
        let library = Library::create(Path::new(&path), name.as_deref(), APP_VERSION)?;
        adopt(&state, library)
    })
    .await
}

/// Opens an existing library. `force` takes over a lock left by another
/// computer, after the user has confirmed.
#[tauri::command]
#[specta::specta]
pub async fn open_library(app: AppHandle, path: String, force: bool) -> AppResult<LibrarySummary> {
    blocking(move || {
        let state = app.state::<AppState>();
        let _opening = state.opening();
        state.close_library();
        let library = Library::open(Path::new(&path), OpenOptions { force })?;
        adopt(&state, library)
    })
    .await
}

/// Closes the library (writing out the database can take a moment).
#[tauri::command]
#[specta::specta]
pub async fn close_library(app: AppHandle) {
    let _ = blocking(move || {
        let state = app.state::<AppState>();
        let _opening = state.opening();
        state.close_library();
        Ok(())
    })
    .await;
}

#[tauri::command]
#[specta::specta]
pub async fn current_library(app: AppHandle) -> Option<LibrarySummary> {
    // At startup the library used last time may still be opening.
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        state.wait_for_reopen();
        state.library_if_open().map(|l| LibrarySummary::of(&l))
    })
    .await
    .ok()
    .flatten()
}

/// Checks the library folder for changes now (the watcher does this
/// automatically; this is the manual "Refresh").
#[tauri::command]
#[specta::specta]
pub fn rescan_library(state: State<'_, AppState>) -> AppResult<Option<String>> {
    state.library()?;
    Ok(state.schedule_scan().map(|id| id.to_string()))
}

/// Rebuilds the database from the book files and JSON sidecars.
#[tauri::command]
#[specta::specta]
pub fn rebuild_library_index(state: State<'_, AppState>) -> AppResult<String> {
    Ok(state.start_rebuild()?.to_string())
}

/// Stops a running import or scan at the next file.
#[tauri::command]
#[specta::specta]
pub fn cancel_job(state: State<'_, AppState>, id: String) -> bool {
    id.parse().is_ok_and(|id| state.jobs.cancel(id))
}

/// Makes `library` the open one and remembers it on the Welcome screen.
pub(crate) fn adopt(state: &AppState, library: Library) -> Result<LibrarySummary, AppError> {
    let summary = LibrarySummary::of(&library);
    let root: PathBuf = library.layout().root().to_path_buf();
    let name = library.info().name.clone();
    state.adopt(library);
    state.update_settings(|s| s.remember_library(name, root))?;
    Ok(summary)
}

/// What moving the library to a folder would do.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MovePlanDto {
    /// The library's new folder.
    pub target: String,
    /// Same disk: the move is a rename, done at once.
    pub same_disk: bool,
    pub files: u32,
    /// `f64`: sizes over 4 GB, as a plain TypeScript number.
    pub bytes: f64,
}

/// How a move ended.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MovedLibrary {
    pub library: LibrarySummary,
    /// The old folder could not go to the Trash; why.
    pub old_left_behind: Option<String>,
}

/// Progress of copying a library to another disk.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct LibraryMoveProgress {
    pub done: f64,
    pub total: f64,
    pub file: String,
}

/// Asked by the interface while a move copies; set by `cancel_library_move`.
#[derive(Default)]
pub struct MoveCancel(pub AtomicBool);

fn plan_for(state: &AppState, parent: &str) -> AppResult<(PathBuf, MovePlan)> {
    let library = state.library()?;
    library.require_owner_profile()?;
    let root = library.layout().root().to_path_buf();
    let plan = libreri_library::relocate::plan_move(&root, Path::new(parent))?;
    Ok((root, plan))
}

/// Checks that the library can move into `parent` and says how.
#[tauri::command]
#[specta::specta]
pub async fn plan_library_move(app: AppHandle, parent: String) -> AppResult<MovePlanDto> {
    blocking(move || {
        let state = app.state::<AppState>();
        let (_, plan) = plan_for(&state, &parent)?;
        Ok(MovePlanDto {
            target: plan.target.to_string_lossy().into_owned(),
            same_disk: plan.same_disk,
            files: plan.files.min(u32::MAX as u64) as u32,
            bytes: plan.bytes as f64,
        })
    })
    .await
}

struct MoveProgress<'a> {
    app: &'a AppHandle,
    cancel: &'a AtomicBool,
    last: std::cell::Cell<std::time::Instant>,
}

impl libreri_library::Progress for MoveProgress<'_> {
    fn report(&self, done: u64, total: u64, message: &str) {
        // A few times a second is plenty for a progress bar.
        if done < total && self.last.get().elapsed().as_millis() < 150 {
            return;
        }
        self.last.set(std::time::Instant::now());
        let _ = LibraryMoveProgress {
            done: done as f64,
            total: total as f64,
            file: message.to_owned(),
        }
        .emit(self.app);
    }
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// Moves the open library into `parent` (owner only): closes it, renames
/// or copies the folder, and opens it in its new place. If anything goes
/// wrong the library stays where it was and opens again there.
#[tauri::command]
#[specta::specta]
pub async fn move_library(app: AppHandle, parent: String) -> AppResult<MovedLibrary> {
    blocking(move || {
        let state = app.state::<AppState>();
        if state.jobs.pending() > 0 {
            return Err(AppError::invalid(
                "Libreri is still working on something (an import, scan or export). Try again when it has finished.",
            ));
        }
        let (root, plan) = plan_for(&state, &parent)?;
        let cancel = app.state::<MoveCancel>();
        cancel.0.store(false, Ordering::Relaxed);
        let _opening = state.opening();
        state.close_library();
        let progress = MoveProgress {
            app: &app,
            cancel: &cancel.0,
            last: std::cell::Cell::new(std::time::Instant::now()),
        };
        match libreri_library::relocate::move_closed(&root, &plan, &progress) {
            Ok(report) => {
                let library = Library::open(&plan.target, OpenOptions::default())?;
                state.update_settings(|s| s.recent_libraries.retain(|r| r.path != root))?;
                let summary = adopt(&state, library)?;
                Ok(MovedLibrary {
                    library: summary,
                    old_left_behind: report.old_left_behind,
                })
            }
            Err(err) => {
                // Back where it was.
                if let Ok(library) = Library::open(&root, OpenOptions::default()) {
                    let _ = adopt(&state, library);
                }
                Err(err.into())
            }
        }
    })
    .await
}

/// Stops a move that is copying; the library stays where it was.
#[tauri::command]
#[specta::specta]
pub fn cancel_library_move(cancel: State<'_, MoveCancel>) {
    cancel.0.store(true, Ordering::Relaxed);
}
