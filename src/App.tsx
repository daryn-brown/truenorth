import Dashboard from "./pages/Dashboard";
import UpdatePrompt from "./components/UpdatePrompt";
import { useUpdater } from "./hooks/useUpdater";

export default function App() {
  const updater = useUpdater();
  return (
    <>
      <Dashboard
        onCheckForUpdates={() => updater.checkForUpdates(true)}
        checkingUpdate={updater.checking}
      />
      <UpdatePrompt {...updater} />
    </>
  );
}
