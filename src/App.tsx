import { getCurrentWindow } from "@tauri-apps/api/window";
import Dashboard from "./Dashboard";
import Popup from "./Popup";

export const POPUP_WINDOW = "popup";

/** The tray popup and the dashboard share one bundle; the window label picks the view. */
export function currentView(): "dashboard" | "popup" {
  if (new URLSearchParams(window.location.search).get("view") === POPUP_WINDOW) return "popup";
  try {
    return getCurrentWindow().label === POPUP_WINDOW ? "popup" : "dashboard";
  } catch {
    // Not running inside Tauri (browser preview, tests).
    return "dashboard";
  }
}

export default function App() {
  return currentView() === "popup" ? <Popup /> : <Dashboard />;
}
