/**
 * Files opened with Libreri from the computer (double-click, *Open With*,
 * or Libreri as the default app). A file that is already a book in the
 * library opens straight away; others are offered to be added first, and
 * open when the import finishes.
 */
import { useEffect } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { commands, events, type FileType } from "@/lib/ipc";
import { useTabs } from "@/lib/tabs";
import { useImport } from "./import";

async function openBook(bookId: string, title?: string | null, fileType?: string | null) {
  let t = title;
  let f = fileType;
  if (!t || !f) {
    const r = await commands.getBook(bookId);
    if (r.status !== "ok") return;
    t = r.data.metadata.title;
    f = r.data.fileType;
  }
  useTabs.getState().open({ bookId, title: t ?? "Book", fileType: f as FileType });
}

async function takeOpened() {
  const r = await commands.takeOpenedFiles();
  if (r.status !== "ok" || r.data.length === 0) return;
  const unknown: string[] = [];
  for (const file of r.data) {
    if (file.bookId) void openBook(file.bookId, file.title, file.fileType);
    else unknown.push(file.path);
  }
  if (unknown.length) useImport.getState().askToOpen(unknown);
}

/** Opens the books of an import started by the "Open" dialog. */
export function openImported(jobId: string, ids: string[]) {
  const { openAfterJob, setOpenAfterJob } = useImport.getState();
  if (openAfterJob !== jobId) return;
  setOpenAfterJob(null);
  for (const id of ids) void openBook(id);
}

/** Takes files opened from the computer (the main window only). */
export function useOpenedFiles() {
  useEffect(() => {
    if (getCurrentWindow().label !== "main") return;
    void takeOpened();
    const off = events.filesOpened.listen(() => void takeOpened());
    return () => void off.then((f) => f());
  }, []);
}
