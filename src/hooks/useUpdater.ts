import { useCallback, useEffect, useRef, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

export type UpdaterPhase = "available" | "downloading" | "installing" | "error";

/** Result of a *manual* check when there is nothing to install (drives a small toast). */
export type CheckOutcome = "uptodate" | "error";

export interface Updater {
  /** The available update, if any. */
  update: Update | null;
  /** Install lifecycle while an update is in hand. */
  phase: UpdaterPhase | null;
  /** A check is currently in flight. */
  checking: boolean;
  /** Feedback for a manual check that found nothing to install. */
  outcome: CheckOutcome | null;
  error: string | null;
  progress: number;
  dismissed: boolean;
  /** Check GitHub Releases. Pass `true` for a user-initiated check (surfaces "up to date"/errors). */
  checkForUpdates: (manual?: boolean) => Promise<void>;
  install: () => Promise<void>;
  dismiss: () => void;
}

/**
 * Shared updater state: a silent check at launch plus an on-demand "Check for updates" path, both
 * feeding the same prompt. Runs only inside the Tauri shell — in a plain browser the updater plugin
 * is absent, so the launch check stays quiet and a manual check reports the error.
 */
export function useUpdater(): Updater {
  const [update, setUpdate] = useState<Update | null>(null);
  const [phase, setPhase] = useState<UpdaterPhase | null>(null);
  const [checking, setChecking] = useState(false);
  const [outcome, setOutcome] = useState<CheckOutcome | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [progress, setProgress] = useState(0);
  const [dismissed, setDismissed] = useState(false);
  const launchedRef = useRef(false);
  const checkingRef = useRef(false);
  const toastTimer = useRef<number | null>(null);

  const showToast = useCallback((kind: CheckOutcome) => {
    setOutcome(kind);
    if (toastTimer.current) window.clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => setOutcome(null), 6000);
  }, []);

  const checkForUpdates = useCallback(
    async (manual = false) => {
      if (checkingRef.current) return;
      checkingRef.current = true;
      setChecking(true);
      setOutcome(null);
      if (manual) {
        setError(null);
        setDismissed(false);
      }
      try {
        const found = await check();
        if (found) {
          setUpdate(found);
          setPhase("available");
          setDismissed(false);
        } else if (manual) {
          showToast("uptodate");
        }
      } catch (err) {
        // Outside the Tauri shell the plugin is absent — stay silent unless the user asked.
        if (manual) {
          setError(err instanceof Error ? err.message : String(err));
          showToast("error");
        }
      } finally {
        checkingRef.current = false;
        setChecking(false);
      }
    },
    [showToast],
  );

  useEffect(() => {
    if (launchedRef.current) return;
    launchedRef.current = true;
    void checkForUpdates(false);
  }, [checkForUpdates]);

  const install = useCallback(async () => {
    if (!update) return;
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
            setProgress(
              total > 0 ? Math.min(100, Math.round((downloaded / total) * 100)) : 0,
            );
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
  }, [update]);

  const dismiss = useCallback(() => {
    setDismissed(true);
    setOutcome(null);
  }, []);

  return {
    update,
    phase,
    checking,
    outcome,
    error,
    progress,
    dismissed,
    checkForUpdates,
    install,
    dismiss,
  };
}
