import { useEffect, useRef, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

type Phase = "idle" | "available" | "downloading" | "installing" | "error";

/**
 * Checks GitHub Releases for a newer signed build once at launch and, if one exists, shows a
 * dismissible prompt to download + install it. Runs only inside the Tauri shell — in a plain
 * browser (e.g. `vite dev` without Tauri) the updater plugin is absent and the check is skipped.
 */
export default function UpdatePrompt() {
  const [update, setUpdate] = useState<Update | null>(null);
  const [phase, setPhase] = useState<Phase>("idle");
  const [dismissed, setDismissed] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [progress, setProgress] = useState(0);
  const checkedRef = useRef(false);

  useEffect(() => {
    if (checkedRef.current) return;
    checkedRef.current = true;
    void (async () => {
      try {
        const found = await check();
        if (found) {
          setUpdate(found);
          setPhase("available");
        }
      } catch {
        // No updater (running outside Tauri) or the check failed — stay quiet.
      }
    })();
  }, []);

  if (!update || dismissed || phase === "idle") return null;

  const handleInstall = async () => {
    setError(null);
    setPhase("downloading");
    try {
      let total = 0;
      let downloaded = 0;
      await update.downloadAndInstall((event) => {
        switch (event.event) {
          case "Started":
            total = event.data.contentLength ?? 0;
            break;
          case "Progress":
            downloaded += event.data.chunkLength;
            setProgress(total > 0 ? Math.min(100, Math.round((downloaded / total) * 100)) : 0);
            break;
          case "Finished":
            setPhase("installing");
            break;
        }
      });
      await relaunch();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setPhase("error");
    }
  };

  const busy = phase === "downloading" || phase === "installing";

  return (
    <div className="fixed bottom-4 right-4 z-50 w-80 rounded-xl border border-indigo-700/60 bg-slate-900 p-4 shadow-2xl">
      <div className="flex items-start justify-between gap-3">
        <div>
          <h3 className="text-sm font-semibold text-white">Update available</h3>
          <p className="text-xs text-slate-400">
            TrueNorth {update.version}
            {update.currentVersion ? (
              <span className="text-slate-500"> · you have {update.currentVersion}</span>
            ) : null}
          </p>
        </div>
        {!busy && (
          <button
            type="button"
            onClick={() => setDismissed(true)}
            aria-label="Dismiss"
            className="shrink-0 text-slate-500 hover:text-slate-300"
          >
            ✕
          </button>
        )}
      </div>

      {update.body ? (
        <p className="mt-2 max-h-24 overflow-y-auto whitespace-pre-line text-xs text-slate-400">
          {update.body}
        </p>
      ) : null}

      {phase === "downloading" && (
        <div className="mt-3">
          <div className="h-1.5 w-full overflow-hidden rounded-full bg-slate-700">
            <div
              className="h-full bg-indigo-500 transition-all"
              style={{ width: `${progress}%` }}
            />
          </div>
          <p className="mt-1 text-[11px] text-slate-500">Downloading… {progress}%</p>
        </div>
      )}

      {phase === "installing" && (
        <p className="mt-3 text-[11px] text-slate-400">Installing… the app will restart.</p>
      )}

      {error && (
        <p className="mt-3 rounded-lg bg-red-900/20 px-2.5 py-1.5 text-xs text-red-400">{error}</p>
      )}

      <div className="mt-3 flex items-center justify-end gap-2">
        {!busy && (
          <button
            type="button"
            onClick={() => setDismissed(true)}
            className="rounded-lg border border-slate-600 px-3 py-1.5 text-xs font-medium text-slate-300 hover:bg-slate-700 transition-colors"
          >
            Later
          </button>
        )}
        <button
          type="button"
          onClick={handleInstall}
          disabled={busy}
          className="rounded-lg bg-indigo-600 px-3 py-1.5 text-xs font-semibold text-white hover:bg-indigo-500 disabled:opacity-50 transition-colors"
        >
          {phase === "downloading"
            ? "Downloading…"
            : phase === "installing"
              ? "Installing…"
              : phase === "error"
                ? "Retry"
                : "Update & restart"}
        </button>
      </div>
    </div>
  );
}
