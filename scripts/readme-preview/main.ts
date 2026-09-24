import { mockIPC } from "@tauri-apps/api/mocks";
import { responses, widgetSettings } from "./fixtures";

if (!import.meta.env.DEV) {
  throw new Error("The fictional README preview is development-only.");
}

localStorage.setItem("truenorth.advisor.open", "0");
mockIPC((command, payload) => {
  if (command === "set_mac_widget_enabled") {
    if (!payload || !("enabled" in payload) || typeof payload.enabled !== "boolean") {
      throw new Error("The demo widget switch requires a boolean.");
    }
    widgetSettings.enabled = payload.enabled;
    return structuredClone(widgetSettings);
  }
  if (Object.prototype.hasOwnProperty.call(responses, command)) {
    return structuredClone(responses[command]);
  }
  throw new Error(`The read-only README preview does not implement ${command}.`);
}, { shouldMockEvents: true });

void import("../../src/main");
