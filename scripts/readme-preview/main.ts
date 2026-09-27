import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { responses, widgetSettings } from "./fixtures";

if (!import.meta.env.DEV) {
  throw new Error("The fictional README preview is development-only.");
}

localStorage.setItem("truenorth.advisor.open", "0");
export const calls: string[] = [];
mockWindows("main");
Object.defineProperty(window, "isTauri", { value: true });
mockIPC(async (command, payload) => {
  calls.push(command);
  if (command === "plugin:shell|open") return;
  if (command.endsWith("_sync")) await new Promise((resolve) => setTimeout(resolve, 300));
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
