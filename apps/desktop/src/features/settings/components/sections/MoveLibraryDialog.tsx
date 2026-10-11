import { useEffect, useState } from "react";
import { FolderInput, Loader2 } from "lucide-react";
import { toast } from "sonner";
import { open } from "@tauri-apps/plugin-dialog";
import { useQueryClient } from "@tanstack/react-query";
import { Button } from "@/components/ui/button";
import { Dialog } from "@/components/ui/dialog";
import { currentLibraryKey, libKey } from "@/features/library";
import { commands, events, type LibrarySummary, type MovePlanDto } from "@/lib/ipc";

function size(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = bytes / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${units[i]}`;
}

/**
 * Settings › Library & storage › Move…: choose a place, see what will
 * happen, then move. Same disk: a rename. Another disk: a copy with
 * progress, after which the old folder goes to the Trash.
 */
export function MoveLibraryDialog({
  library,
  onClose,
}: {
  library: LibrarySummary;
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const [parent, setParent] = useState<string | null>(null);
  const [plan, setPlan] = useState<MovePlanDto | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [moving, setMoving] = useState(false);
  const [progress, setProgress] = useState<{ done: number; total: number; file: string } | null>(
    null,
  );

  useEffect(() => {
    const off = events.libraryMoveProgress.listen(({ payload }) =>
      setProgress({ done: payload.done ?? 0, total: payload.total ?? 0, file: payload.file }),
    );
    return () => void off.then((f) => f());
  }, []);

  const choose = async () => {
    const dir = await open({ directory: true, title: "Choose where the library goes" });
    if (typeof dir !== "string") return;
    setParent(dir);
    setPlan(null);
    setProblem(null);
    const r = await commands.planLibraryMove(dir);
    if (r.status === "ok") setPlan(r.data);
    else setProblem(r.error.message);
  };

  const move = async () => {
    if (!parent) return;
    setMoving(true);
    setProgress(null);
    const r = await commands.moveLibrary(parent);
    setMoving(false);
    if (r.status === "error") {
      if (r.error.message !== "cancelled") setProblem(r.error.message);
      else toast("Move cancelled. The library is where it was.");
      void qc.invalidateQueries();
      return;
    }
    qc.removeQueries({ queryKey: libKey });
    qc.setQueryData(currentLibraryKey, r.data.library);
    void qc.invalidateQueries({ queryKey: ["settings"] });
    if (r.data.oldLeftBehind)
      toast.warning("The library moved, but its old folder could not go to the Trash", {
        description: `Delete ${library.path} yourself. (${r.data.oldLeftBehind})`,
        duration: 20_000,
      });
    else toast.success(`The library is now in ${r.data.library.path}`);
    onClose();
  };

  const fraction = progress && progress.total > 0 ? progress.done / progress.total : 0;

  return (
    <Dialog
      open
      onOpenChange={(o) => !o && !moving && onClose()}
      title="Move this library"
      description="Your books, notes and everything Libreri keeps move together. Links keep working, because Libreri only stores paths inside the library."
    >
      <dl className="grid grid-cols-[64px_1fr] gap-x-3 gap-y-2 text-[13px]">
        <dt className="text-muted-foreground">From</dt>
        <dd className="font-mono text-[11.5px] break-all">{library.path}</dd>
        <dt className="self-center text-muted-foreground">To</dt>
        <dd className="flex items-center gap-2">
          <span className="min-w-0 flex-1 font-mono text-[11.5px] break-all">
            {plan?.target ?? parent ?? (
              <span className="font-sans text-muted-foreground">Choose a place…</span>
            )}
          </span>
          <Button variant="outline" size="sm" disabled={moving} onClick={() => void choose()}>
            <FolderInput /> Choose…
          </Button>
        </dd>
      </dl>

      {plan && !moving && (
        <p className="rounded-md border bg-muted/40 px-3 py-2 text-[12.5px]">
          {plan.sameDisk ? (
            <>
              Same disk: the folder is moved at once ({plan.files.toLocaleString()} files,{" "}
              {size(plan.bytes ?? 0)}).
            </>
          ) : (
            <>
              Another disk: {size(plan.bytes ?? 0)} in {plan.files.toLocaleString()} files are
              copied and checked, then the old folder goes to the Trash.
            </>
          )}
        </p>
      )}

      {moving && (
        <div className="flex flex-col gap-1.5">
          <div
            className="h-1.5 overflow-hidden rounded-full bg-muted"
            role="progressbar"
            aria-label="Moving the library"
            aria-valuenow={Math.round(fraction * 100)}
          >
            <div
              className="h-full bg-primary transition-[width]"
              style={{ width: `${fraction * 100}%` }}
            />
          </div>
          <p className="truncate text-[12px] text-muted-foreground">
            {progress
              ? `${size(progress.done)} of ${size(progress.total)} · ${progress.file}`
              : "Closing the library…"}
          </p>
        </div>
      )}

      {problem && <p className="text-[12.5px] text-destructive">{problem}</p>}

      <p className="text-[12px] text-muted-foreground">
        The library closes while it moves. If it is in a synced folder used on other computers, open
        it there from its new place.
      </p>

      <div className="flex justify-end gap-2">
        {moving ? (
          plan?.sameDisk ? null : (
            <Button variant="ghost" onClick={() => void commands.cancelLibraryMove()}>
              Cancel
            </Button>
          )
        ) : (
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
        )}
        <Button disabled={!plan || moving} onClick={() => void move()}>
          {moving && <Loader2 className="animate-spin" />}
          {moving ? "Moving…" : "Move"}
        </Button>
      </div>
    </Dialog>
  );
}
