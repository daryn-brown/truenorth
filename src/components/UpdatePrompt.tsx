import type { Updater } from "../hooks/useUpdater";

/**
 * Presentational updater surface, driven by {@link useUpdater}. Shows either the available-update
 * card (with a download/install progress flow) or, after a manual check, a small toast confirming
 * the app is up to date or reporting an error.
 */
export default function UpdatePrompt({
  update,
  phase,
  outcome,
  error,
  progress,
  dismissed,
  install,
  dismiss,
}: Updater) {
  // An update is in hand — offer to download + install it.
  if (update && phase && !dismissed) {
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
              onClick={dismiss}
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
          <p className="mt-3 rounded-lg bg-red-900/20 px-2.5 py-1.5 text-xs text-red-400">
            {error}
          </p>
        )}

        <div className="mt-3 flex items-center justify-end gap-2">
          {!busy && (
            <button
              type="button"
              onClick={dismiss}
              className="rounded-lg border border-slate-600 px-3 py-1.5 text-xs font-medium text-slate-300 hover:bg-slate-700 transition-colors"
            >
              Later
            </button>
          )}
          <button
            type="button"
            onClick={install}
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

  // Feedback from a manual "Check for updates" when there's nothing to install.
  if (outcome && !dismissed) {
    return (
      <div className="fixed bottom-4 right-4 z-50 w-72 rounded-xl border border-slate-700 bg-slate-900 p-3 shadow-2xl">
        <div className="flex items-start justify-between gap-3">
          <p className="text-sm text-slate-200">
            {outcome === "uptodate"
              ? "You're on the latest version."
              : `Update check failed: ${error ?? "unknown error"}`}
          </p>
          <button
            type="button"
            onClick={dismiss}
            aria-label="Dismiss"
            className="shrink-0 text-slate-500 hover:text-slate-300"
          >
            ✕
          </button>
        </div>
      </div>
    );
  }

  return null;
}
