import type { GoalProgress } from "../types/finance";

interface Props {
  goal: GoalProgress | null;
  loading: boolean;
}

const usd = (value: number, maximumFractionDigits = 0) =>
  new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
    maximumFractionDigits,
  }).format(value);

const longDate = (iso: string) => {
  const d = new Date(`${iso}T00:00:00`);
  return Number.isNaN(d.getTime())
    ? iso
    : d.toLocaleDateString("en-US", { month: "long", day: "numeric", year: "numeric" });
};

/** "150 days" → a friendlier "5 months" / "1.4 years" once the horizon gets long. */
const humanizeDays = (days: number): string => {
  if (days <= 1) return "a day";
  if (days < 60) return `${days} days`;
  const months = Math.round(days / 30.44);
  if (months < 24) return `${months} month${months === 1 ? "" : "s"}`;
  return `${(days / 365.25).toFixed(1)} years`;
};

/**
 * The "$100k Countdown" — a progress bar toward the USD net-worth milestone plus the date the
 * user is projected to cross it at their rolling ~30-day pace.
 */
export default function GoalCountdownCard({ goal, loading }: Props) {
  if (loading && !goal) {
    return (
      <div className="rounded-2xl border border-slate-700 bg-slate-900/40 p-6">
        <div className="h-28 animate-pulse rounded-lg bg-slate-800" />
      </div>
    );
  }
  if (!goal) return null;

  const target = usd(goal.target_usd);
  const pct = Math.round(goal.progress * 100);
  const hasEta = goal.projected_date !== null && goal.days_to_goal !== null;
  const paceNegative = goal.daily_rate_usd !== null && goal.daily_rate_usd <= 0;

  return (
    <div className="flex h-full flex-col rounded-2xl border border-slate-700 bg-gradient-to-br from-slate-800 to-slate-900 p-6 shadow-xl">
      <div className="flex items-center justify-between">
        <p className="text-sm font-medium uppercase tracking-widest text-slate-400">
          Road to {target}
        </p>
        <span className="rounded-full border border-indigo-700/50 bg-indigo-900/40 px-2 py-0.5 text-[11px] font-semibold uppercase tracking-wider text-indigo-300">
          CoastFIRE
        </span>
      </div>

      {goal.already_met ? (
        <p className="mt-3 text-2xl font-bold text-emerald-400">
          🎉 Milestone reached — {usd(goal.current_usd)}
        </p>
      ) : (
        <>
          <div className="mt-3 flex items-baseline justify-between">
            <span className="text-3xl font-bold tracking-tight text-white">
              {usd(goal.current_usd)}
            </span>
            <span className="text-sm font-medium text-slate-400">
              {usd(goal.gap_usd)} to go
            </span>
          </div>

          <div className="mt-3 h-3 w-full overflow-hidden rounded-full bg-slate-800">
            <div
              className="h-full rounded-full bg-gradient-to-r from-indigo-500 to-emerald-400 transition-all duration-700"
              style={{ width: `${Math.max(2, Math.min(100, pct))}%` }}
            />
          </div>
          <div className="mt-1 flex justify-between text-xs text-slate-500">
            <span>{pct}%</span>
            <span>{target}</span>
          </div>

          <div className="mt-4 rounded-xl border border-slate-700/70 bg-slate-900/40 px-4 py-3">
            {hasEta ? (
              <>
                <p className="text-sm text-slate-300">
                  On pace to hit {target} by{" "}
                  <span className="font-semibold text-emerald-400">
                    {longDate(goal.projected_date as string)}
                  </span>
                </p>
                <p className="mt-1 text-xs text-slate-500">
                  ≈ {humanizeDays(goal.days_to_goal as number)} away
                  {goal.monthly_rate_usd !== null && (
                    <> · {usd(goal.monthly_rate_usd)}/mo pace</>
                  )}
                  {goal.window_days !== null && <> · last {goal.window_days}d</>}
                </p>
              </>
            ) : (
              <p className="text-sm text-slate-400">
                {paceNegative
                  ? "Net worth is flat or down over the last month, so there's no ETA yet — a positive month will project your hit-date. Zoom out: the milestone hasn't moved. 🧭"
                  : "Add a little more balance history and I'll project your hit-date from your 30-day pace."}
              </p>
            )}
          </div>
        </>
      )}
    </div>
  );
}
