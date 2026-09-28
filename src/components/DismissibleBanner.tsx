import { useEffect, useState, type ReactNode } from "react";
import DesktopIcon from "../apps/desktop/DesktopIcon";

interface Props {
  noticeKey: string | null;
  dismissLabel: string;
  className: string;
  children: ReactNode;
  role?: "alert" | "status";
  hidden?: boolean;
}

export default function DismissibleBanner({
  noticeKey,
  dismissLabel,
  className,
  children,
  role = "alert",
  hidden = false,
}: Props) {
  const [dismissedKey, setDismissedKey] = useState<string | null>(null);

  useEffect(() => {
    setDismissedKey(null);
  }, [noticeKey]);

  if (noticeKey === null || hidden || dismissedKey === noticeKey) return null;

  return (
    <div role={role} className={`desktop-dismissible-banner ${className}`}>
      <div className="desktop-dismissible-banner__content">{children}</div>
      <button
        type="button"
        className="desktop-dismissible-banner__close"
        onClick={() => setDismissedKey(noticeKey)}
        aria-label={dismissLabel}
        title={dismissLabel}
      >
        <DesktopIcon name="close" />
      </button>
    </div>
  );
}
