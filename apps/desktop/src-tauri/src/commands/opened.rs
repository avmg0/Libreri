//! Files opened with Libreri from the computer's file manager (double-click,
//! *Open With*, or Libreri set as the default app for a type).
//!
//! macOS hands them over as an event, Windows and Linux as program
//! arguments (to a second copy of Libreri that passes them on and quits).
//! They wait here until the interface asks for them, because a file can
//! arrive before the window has loaded or a library is open.

use super::blocking;
use crate::error::AppResult;
use crate::events::FilesOpened;
use crate::state::{lock, AppState};
use serde::Serialize;
use specta::Type;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_specta::Event;

/// Files waiting for the interface.
#[derive(Default)]
pub struct OpenedFiles(Mutex<Vec<PathBuf>>);

/// Keeps `paths` (files only) for the interface and brings Libreri forward.
pub fn queue(app: &AppHandle, paths: impl IntoIterator<Item = PathBuf>) {
    let files: Vec<PathBuf> = paths.into_iter().filter(|p| p.is_file()).collect();
    if files.is_empty() {
        return;
    }
    if let Some(queue) = app.try_state::<OpenedFiles>() {
        let mut q = lock(&queue.0);
        for f in files {
            if !q.contains(&f) {
                q.push(f);
            }
        }
    }
    // Emitted by name: on macOS a file can arrive before the typed events
    // are mounted.
    let _ = app.emit(FilesOpened::NAME, FilesOpened {});
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// The files among a program's arguments (the first is the program).
pub fn from_args<I: IntoIterator<Item = String>>(args: I) -> Vec<PathBuf> {
    args.into_iter()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .collect()
}

/// A file opened from the computer, and the book it already is, if any.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OpenedFileDto {
    pub path: String,
    /// The book in the library with the same content.
    pub book_id: Option<String>,
    pub title: Option<String>,
    pub file_type: Option<String>,
}

/// Hands over the files opened from the computer, each with the library
/// book it already is. Waits (returns nothing) while no library is open.
#[tauri::command]
#[specta::specta]
pub async fn take_opened_files(
    state: State<'_, AppState>,
    queue: State<'_, OpenedFiles>,
) -> AppResult<Vec<OpenedFileDto>> {
    let Some(library) = state.library_if_open() else {
        return Ok(Vec::new());
    };
    let files: Vec<PathBuf> = std::mem::take(&mut *lock(&queue.0));
    if files.is_empty() {
        return Ok(Vec::new());
    }
    blocking(move || {
        Ok(files
            .into_iter()
            .map(|path| {
                let book = library.book_for_file(&path).ok().flatten();
                OpenedFileDto {
                    path: path.to_string_lossy().into_owned(),
                    book_id: book.as_ref().map(|b| b.id.to_string()),
                    title: book.as_ref().map(|b| b.metadata.title.clone()),
                    file_type: book.as_ref().map(|b| b.file_type.as_str().to_owned()),
                }
            })
            .collect())
    })
    .await
}
