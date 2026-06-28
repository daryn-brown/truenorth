import { useEffect, useState } from "react";
import Dashboard from "./pages/Dashboard";
import UpdatePrompt from "./components/UpdatePrompt";
import AdvisorPanel from "./components/AdvisorPanel";
import { useUpdater } from "./hooks/useUpdater";

// Remember whether the advisor panel was expanded between launches.
const ADVISOR_OPEN_KEY = "truenorth.advisor.open";

export default function App() {
  const updater = useUpdater();
  const [advisorOpen, setAdvisorOpen] = useState<boolean>(
    () => localStorage.getItem(ADVISOR_OPEN_KEY) === "1",
  );

  useEffect(() => {
    localStorage.setItem(ADVISOR_OPEN_KEY, advisorOpen ? "1" : "0");
  }, [advisorOpen]);

  return (
    <div className="flex h-screen overflow-hidden bg-slate-950">
      <div className="min-w-0 flex-1 overflow-y-auto">
        <Dashboard
          onCheckForUpdates={() => updater.checkForUpdates(true)}
          checkingUpdate={updater.checking}
          onToggleAdvisor={() => setAdvisorOpen((v) => !v)}
        />
      </div>
      <AdvisorPanel
        open={advisorOpen}
        onOpen={() => setAdvisorOpen(true)}
        onClose={() => setAdvisorOpen(false)}
      />
      <UpdatePrompt {...updater} />
    </div>
  );
}
