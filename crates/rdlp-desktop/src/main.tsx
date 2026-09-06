import React from "react";
import ReactDOM from "react-dom/client";
import { attachConsole } from "@tauri-apps/plugin-log";
import App from "./App";
import { registerLogEvents } from "./events/registerLogEvents";
import "./index.css";

// Both subscribe to tauri-plugin-log's `log://log` event: `attachConsole`
// forwards the Rust side's `log` records to the devtools console, and
// `registerLogEvents` feeds them to the in-app Log Viewer.
//
// Attached at module scope rather than in an effect so StrictMode's
// double-invoke cannot register two listeners, and so records emitted while
// the shell is still mounting are not lost. Their detach handles are unused:
// these listeners should live as long as the window.
void attachConsole();
void registerLogEvents();

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
