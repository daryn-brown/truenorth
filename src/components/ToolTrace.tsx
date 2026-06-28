import type { ToolStep } from "../types/ai";

// Pretty-print a JSON string when possible; otherwise show it as-is. Tool args/results are JSON
// strings from the backend, but a result may be plain text (e.g. an error), so we fall back safely.
function pretty(value: string): string {
  try {
    return JSON.stringify(JSON.parse(value), null, 2);
  } catch {
    return value;
  }
}

function humanize(name: string): string {
  return name.replace(/_/g, " ");
}

/** Collapsible "Used N tools" trace shown under an assistant answer. */
export default function ToolTrace({ steps }: { steps: ToolStep[] }) {
  if (steps.length === 0) return null;
  const label = `Used ${steps.length} ${steps.length === 1 ? "tool" : "tools"}`;
  return (
    <details className="mt-2 rounded-lg border border-slate-700/70 bg-slate-900/40 text-xs">
      <summary className="cursor-pointer select-none px-3 py-1.5 text-slate-400 hover:text-slate-200">
        🔧 {label}
      </summary>
      <ol className="space-y-2 px-3 pb-3 pt-1">
        {steps.map((s, i) => (
          <li key={i} className="space-y-1">
            <div className="font-medium text-slate-300">{humanize(s.name)}</div>
            {s.arguments && s.arguments !== "{}" && (
              <pre className="overflow-x-auto rounded bg-slate-950/60 p-2 font-mono text-[11px] text-slate-400">
                {pretty(s.arguments)}
              </pre>
            )}
            <pre className="max-h-40 overflow-auto rounded bg-slate-950/60 p-2 font-mono text-[11px] text-slate-400">
              {pretty(s.result)}
            </pre>
          </li>
        ))}
      </ol>
    </details>
  );
}
