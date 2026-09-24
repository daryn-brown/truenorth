import { useEffect, useRef, useState } from "react";
import { getMacWidgetSettings, setMacWidgetEnabled } from "../hooks/useFinanceApi";
import type { MacWidgetSettings } from "../types/finance";

interface Props {
  settings: MacWidgetSettings;
  onSettingsChanged: (settings: MacWidgetSettings) => void;
  onClose: () => void;
}

export default function MacWidgetModal({ settings, onSettingsChanged, onClose }: Props) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const element = dialog.current;
    element?.showModal();
    return () => element?.close();
  }, []);

  const changeSharing = async (enabled: boolean) => {
    setBusy(true);
    setError(null);
    try {
      onSettingsChanged(await setMacWidgetEnabled(enabled));
    } catch (err) {
      setError(String(err));
      // A failed cleanup still persists the opt-out. Reflect that rather than showing it enabled.
      try {
        onSettingsChanged(await getMacWidgetSettings());
      } catch (statusError) {
        setError(`${String(err)} Could not reload widget settings: ${String(statusError)}`);
      }
    } finally {
      setBusy(false);
    }
  };

  return (
    <dialog
      ref={dialog}
      className="tn-modal tn-widget-dialog"
      aria-labelledby="mac-widget-title"
      aria-describedby="mac-widget-description"
      onCancel={(event) => {
        event.preventDefault();
        if (!busy) onClose();
      }}
      onClick={(event) => {
        if (event.target === event.currentTarget && !busy) onClose();
      }}
    >
      <div className="tn-widget-dialog__content">
        <h2 id="mac-widget-title">Net worth on your Mac</h2>
        <p id="mac-widget-description">
          See your USD and CAD totals on your desktop or in Notification Center,
          in a small or medium native widget.
        </p>

        {!settings.available && (
          <div className="desktop-alert" role="status">
            {settings.unavailable_reason}
          </div>
        )}

        <label className="tn-widget-switch">
          <span>
            <strong>Share net worth with macOS</strong>
            <small>Off by default. Only this Mac receives the snapshot.</small>
          </span>
          <input
            type="checkbox"
            role="switch"
            checked={settings.enabled}
            disabled={busy || (!settings.available && !settings.enabled)}
            onChange={(event) => void changeSharing(event.target.checked)}
            aria-describedby="mac-widget-privacy"
          />
        </label>

        <p id="mac-widget-privacy" className="tn-widget-dialog__privacy">
          Enabling sharing saves an unencrypted local copy of your totals and data
          dates in a macOS app-group container. Account details, transactions, and
          credentials are never shared. Anyone who can see your desktop may see
          the widget.
        </p>

        <h3>Add the widget</h3>
        <ol className="tn-widget-dialog__steps">
          <li>Install and open the signed TrueNorth widget build on macOS 14 or later.</li>
          <li>Enable sharing above.</li>
          <li>Right-click your desktop, choose <strong>Edit Widgets</strong>, and search for TrueNorth.</li>
          <li>Add <strong>Net worth</strong> in your preferred size.</li>
        </ol>
        <p className="tn-widget-dialog__privacy">
          Totals refresh when you open TrueNorth or update, import, or sync your
          accounts. The widget does not connect to banks while the app is closed.
          Its data date reflects the oldest balance or exchange rate used.
        </p>
        <p className="tn-widget-dialog__privacy">
          Turning sharing off deletes the shared snapshot and requests a refresh.
          macOS controls refresh timing and may briefly retain the previous display;
          remove the widget from your desktop to hide it immediately.
        </p>

        {error && (
          <div className="desktop-alert desktop-alert--error" role="alert">
            {error}
            {!settings.enabled && (
              <button
                type="button"
                className="desktop-action"
                disabled={busy || !settings.available}
                onClick={() => void changeSharing(false)}
              >
                Retry clearing snapshot
              </button>
            )}
          </div>
        )}
        <div className="tn-widget-dialog__footer">
          <span role="status">{busy ? "Updating widget sharing..." : ""}</span>
          <button
            type="button"
            className="desktop-action desktop-action--primary"
            disabled={busy}
            onClick={onClose}
          >
            Done
          </button>
        </div>
      </div>
    </dialog>
  );
}
