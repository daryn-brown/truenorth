import { useEffect, useState } from "react";
import AdvisorPanel from "../../components/AdvisorPanel";
import UpdatePrompt from "../../components/UpdatePrompt";
import { useUpdater } from "../../hooks/useUpdater";
import Dashboard from "../../pages/Dashboard";
import "./desktop.css";

const ADVISOR_OPEN_KEY = "truenorth.advisor.open";

export default function DesktopApp() {
  const updater = useUpdater();
  const [advisorOpen, setAdvisorOpen] = useState<boolean>(
    () => localStorage.getItem(ADVISOR_OPEN_KEY) === "1",
  );

  useEffect(() => {
    localStorage.setItem(ADVISOR_OPEN_KEY, advisorOpen ? "1" : "0");
  }, [advisorOpen]);

  return (
    <div className={`desktop-app ${advisorOpen ? "desktop-app--advisor-open" : ""}`}>
      <div className="desktop-aurora desktop-aurora--top" aria-hidden="true" />
      <div className="desktop-aurora desktop-aurora--bottom" aria-hidden="true" />
      <Dashboard
        onCheckForUpdates={() => updater.checkForUpdates(true)}
        checkingUpdate={updater.checking}
        onToggleAdvisor={() => setAdvisorOpen((open) => !open)}
      />
      <AdvisorPanel
        open={advisorOpen}
        onOpen={() => setAdvisorOpen(true)}
        onClose={() => setAdvisorOpen(false)}
      />
      <UpdatePrompt {...updater} />
    </div>
  );
}
