import type { ReactNode } from "react";

export type DesktopIconName =
  | "accounts"
  | "chart"
  | "connect"
  | "download"
  | "edit"
  | "exchange"
  | "history"
  | "menu"
  | "overview"
  | "plus"
  | "refresh"
  | "send"
  | "settings"
  | "shield"
  | "sparkles"
  | "target"
  | "trash"
  | "upload"
  | "wallet";

const paths: Record<DesktopIconName, ReactNode> = {
  overview: (
    <>
      <path d="M4 13.5A8.5 8.5 0 1 0 12.5 5v8.5H4Z" />
      <path d="M15.5 3.5a5 5 0 0 1 5 5h-5v-5Z" />
    </>
  ),
  wallet: (
    <>
      <path d="M4 7.5h14.5A1.5 1.5 0 0 1 20 9v9.5a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 2 18.5v-13A1.5 1.5 0 0 1 3.5 4H17" />
      <path d="M15 12h5v4h-5a2 2 0 1 1 0-4Z" />
    </>
  ),
  chart: (
    <>
      <path d="M4 19V10M10 19V5M16 19v-7M22 19V2" />
      <path d="M2 22h20" />
    </>
  ),
  accounts: (
    <>
      <rect x="3" y="5" width="18" height="14" rx="3" />
      <path d="M3 10h18M7 15h4" />
    </>
  ),
  target: (
    <>
      <circle cx="12" cy="12" r="8.5" />
      <circle cx="12" cy="12" r="4.5" />
      <path d="m12 12 7-7M16 5h3v3" />
    </>
  ),
  connect: (
    <>
      <path d="M9.5 14.5 14.5 9" />
      <path d="M7.2 17.3 5.7 18.8a3.5 3.5 0 0 1-5-5l3.1-3.1a3.5 3.5 0 0 1 5 0" transform="translate(2 0)" />
      <path d="m16.8 6.7 1.5-1.5a3.5 3.5 0 1 1 5 5l-3.1 3.1a3.5 3.5 0 0 1-5 0" transform="translate(-2 0)" />
    </>
  ),
  upload: (
    <>
      <path d="M12 16V3M7 8l5-5 5 5" />
      <path d="M4 14v5a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-5" />
    </>
  ),
  download: (
    <>
      <path d="M12 3v13M7 11l5 5 5-5" />
      <path d="M4 19v2h16v-2" />
    </>
  ),
  refresh: (
    <>
      <path d="M20 7V3l-2.2 2.2A8 8 0 1 0 20 13" />
      <path d="M16 3h4v4" />
    </>
  ),
  sparkles: (
    <>
      <path d="m12 2 1.7 4.3L18 8l-4.3 1.7L12 14l-1.7-4.3L6 8l4.3-1.7L12 2Z" />
      <path d="m19 14 .9 2.1L22 17l-2.1.9L19 20l-.9-2.1L16 17l2.1-.9L19 14ZM5 14l.7 1.8 1.8.7-1.8.7L5 19l-.7-1.8-1.8-.7 1.8-.7L5 14Z" />
    </>
  ),
  shield: (
    <>
      <path d="M12 2 20 5v6c0 5-3.4 8.8-8 11-4.6-2.2-8-6-8-11V5l8-3Z" />
      <path d="m8.5 12 2.2 2.2 4.8-5" />
    </>
  ),
  plus: <path d="M12 5v14M5 12h14" />,
  exchange: (
    <>
      <path d="M5 7h13l-3-3M19 17H6l3 3" />
    </>
  ),
  edit: (
    <>
      <path d="m4 16-.8 4.8L8 20l10.5-10.5-4-4L4 16Z" />
      <path d="m12.8 7.2 4 4" />
    </>
  ),
  trash: (
    <>
      <path d="M4 7h16M9 7V4h6v3M7 7l1 14h8l1-14" />
      <path d="M10 11v6M14 11v6" />
    </>
  ),
  settings: (
    <>
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.7 1.7 0 0 0 .3 1.9l.1.1-2.8 2.8-.1-.1a1.7 1.7 0 0 0-1.9-.3 1.7 1.7 0 0 0-1 1.6v.2h-4V21a1.7 1.7 0 0 0-1-1.6 1.7 1.7 0 0 0-1.9.3l-.1.1L4.2 17l.1-.1a1.7 1.7 0 0 0 .3-1.9A1.7 1.7 0 0 0 3 14H2.8v-4H3a1.7 1.7 0 0 0 1.6-1 1.7 1.7 0 0 0-.3-1.9L4.2 7 7 4.2l.1.1a1.7 1.7 0 0 0 1.9.3A1.7 1.7 0 0 0 10 3V2.8h4V3a1.7 1.7 0 0 0 1 1.6 1.7 1.7 0 0 0 1.9-.3l.1-.1L19.8 7l-.1.1a1.7 1.7 0 0 0-.3 1.9 1.7 1.7 0 0 0 1.6 1h.2v4H21a1.7 1.7 0 0 0-1.6 1Z" />
    </>
  ),
  menu: <path d="M4 7h16M4 12h16M4 17h16" />,
  history: (
    <>
      <path d="M4 7V3m0 0h4M4 3l3 3a8 8 0 1 1-2 8" />
      <path d="M12 7v5l3 2" />
    </>
  ),
  send: (
    <>
      <path d="m3 11 18-8-8 18-2-8-8-2Z" />
      <path d="m11 13 4-4" />
    </>
  ),
};

export default function DesktopIcon({
  name,
  className = "",
}: {
  name: DesktopIconName;
  className?: string;
}) {
  return (
    <svg
      className={className}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {paths[name]}
    </svg>
  );
}
