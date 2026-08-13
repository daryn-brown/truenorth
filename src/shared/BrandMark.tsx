import { useId } from "react";

export default function BrandMark({
  className = "",
  title,
}: {
  className?: string;
  title?: string;
}) {
  const gradientId = useId().replace(/:/g, "");

  return (
    <svg
      className={className}
      viewBox="0 0 48 48"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      role={title ? "img" : undefined}
      aria-hidden={title ? undefined : true}
    >
      {title && <title>{title}</title>}
      <defs>
        <linearGradient id={gradientId} x1="8" y1="5" x2="40" y2="43">
          <stop stopColor="#8B7CFF" />
          <stop offset="0.5" stopColor="#D56BE4" />
          <stop offset="1" stopColor="#54E4C2" />
        </linearGradient>
      </defs>
      <rect x="1" y="1" width="46" height="46" rx="15" fill="#0C0A1A" />
      <rect x="1" y="1" width="46" height="46" rx="15" stroke="white" strokeOpacity="0.12" />
      <circle cx="24" cy="24" r="13" stroke="white" strokeOpacity="0.15" />
      <path
        d="M24 7.5L28 20L40.5 24L28 28L24 40.5L20 28L7.5 24L20 20L24 7.5Z"
        fill={`url(#${gradientId})`}
      />
      <circle cx="24" cy="24" r="3.75" fill="#080711" />
      <circle cx="24" cy="24" r="2" fill="white" fillOpacity="0.88" />
    </svg>
  );
}
