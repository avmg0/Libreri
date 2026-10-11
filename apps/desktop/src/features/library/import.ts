/**
 * Importing: the file / folder pickers, files dropped from the desktop, and
 * the small dialog that confirms where they go and whether to move or copy.
 */
import { useEffect } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { create } from "zustand";
import type { ImportModeDto } from "@/lib/ipc";
import { useLibraryView } from "./store";

/** Every extension Libreri reads, for the file picker. */
export const BOOK_EXTENSIONS = [
  "pdf",
  "epub",
  "mobi",
  "prc",
  "azw",
  "azw3",
  "kf8",
  "fb2",
  "txt",
  "md",
  "markdown",
  "rtf",
  "djvu",
  "djv",
  "cbz",
  "cbr",
  "cb7",
  "cba",
  "cbt",
  "mp3",
  "m4b",
  "m4a",
  "aac",
  "ogg",
  "oga",
  "opus",
  "flac",
];

interface ImportState {
  /** `open`: files opened from the computer, opened once they are added. */
  pending: { paths: string[]; folder: string; open?: boolean } | null;
  /** The import whose books open when it finishes. */
  openAfterJob: string | null;
  /** The choice of move or copy to go back to after opening files. */
  modeBefore: ImportModeDto;
  mode: ImportModeDto;
  /** A file from the desktop is being dragged over the window. */
  hovering: boolean;
  ask: (paths: string[], folder: string) => void;
  /** Asks to add files opened from the computer, then opens them. */
  askToOpen: (paths: string[]) => void;
  setOpenAfterJob: (job: string | null) => void;
  close: () => void;
  setMode: (mode: ImportModeDto) => void;
  setFolder: (folder: string) => void;
}

export const useImport = create<ImportState>((set) => ({
  pending: null,
  openAfterJob: null,
  modeBefore: "move",
  mode: "move",
  hovering: false,
  ask: (paths, folder) => paths.length && set({ pending: { paths, folder } }),
  // Files opened from the computer are copied by default: someone who
  // double-clicks a PDF does not expect it to leave its folder.
  askToOpen: (paths) =>
    paths.length &&
    set((s) => ({
      pending: {
        paths: [...new Set([...(s.pending?.open ? s.pending.paths : []), ...paths])],
        folder: s.pending?.open ? s.pending.folder : currentFolder(),
        open: true,
      },
      modeBefore: s.pending?.open ? s.modeBefore : s.mode,
      mode: "copy",
    })),
  setOpenAfterJob: (openAfterJob) => set({ openAfterJob }),
  close: () => set((s) => ({ pending: null, mode: s.pending?.open ? s.modeBefore : s.mode })),
  setMode: (mode) => set({ mode }),
  setFolder: (folder) => set((s) => (s.pending ? { pending: { ...s.pending, folder } } : {})),
}));

/** The folder the user is looking at, as the default destination. */
function currentFolder() {
  const { nav } = useLibraryView.getState();
  return nav.kind === "folder" ? nav.path : "";
}

export async function pickFilesToImport() {
  const picked = await open({
    multiple: true,
    title: "Import books",
    filters: [{ name: "Books and audiobooks", extensions: BOOK_EXTENSIONS }],
  });
  if (picked) useImport.getState().ask(Array.isArray(picked) ? picked : [picked], currentFolder());
}

export async function pickFolderToImport() {
  const picked = await open({ directory: true, multiple: true, title: "Import a folder of books" });
  if (picked) useImport.getState().ask(Array.isArray(picked) ? picked : [picked], currentFolder());
}

/** Accepts files and folders dropped from the desktop onto the window. */
/** Files dropped on the window are imported (when the profile may import). */
export function useDesktopDrop(enabled = true) {
  useEffect(() => {
    if (!enabled) return;
    const unlisten = getCurrentWebview().onDragDropEvent(({ payload }) => {
      if (payload.type === "enter" || payload.type === "over") {
        useImport.setState({ hovering: true });
      } else if (payload.type === "leave") {
        useImport.setState({ hovering: false });
      } else if (payload.type === "drop") {
        useImport.setState({ hovering: false });
        const ratio = window.devicePixelRatio || 1;
        const el = document
          .elementFromPoint(payload.position.x / ratio, payload.position.y / ratio)
          ?.closest<HTMLElement>("[data-drop-folder]");
        const folder = el?.dataset.dropFolder ?? currentFolder();
        useImport.getState().ask(payload.paths, folder);
      }
    });
    return () => void unlisten.then((off) => off());
  }, [enabled]);
}
