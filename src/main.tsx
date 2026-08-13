import React, { lazy, Suspense } from "react";
import ReactDOM from "react-dom/client";
import "./index.css";

const DesktopApp = lazy(() => import("./apps/desktop/DesktopApp"));
const LandingPage = lazy(() => import("./apps/web/LandingPage"));

const requestedSurface = new URLSearchParams(window.location.search).get("surface");
const runningInTauri =
  requestedSurface !== "web" &&
  ("__TAURI_INTERNALS__" in window ||
    (import.meta.env.DEV && requestedSurface === "desktop"));

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <Suspense fallback={<div className="app-loading">Loading TrueNorth…</div>}>
      {runningInTauri ? <DesktopApp /> : <LandingPage />}
    </Suspense>
  </React.StrictMode>,
);
