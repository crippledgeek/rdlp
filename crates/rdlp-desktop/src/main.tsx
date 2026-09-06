import React from "react";
import ReactDOM from "react-dom/client";
import { attachConsole } from "@tauri-apps/plugin-log";
import App from "./App";
import { queryClient } from "./query/queryClient";
import { registerDownloadEvents } from "./events/registerDownloadEvents";
import { registerLogEvents } from "./events/registerLogEvents";
import "./index.css";

// Every Tauri event subscription in the app, in one place.
//
// `attachConsole` and `registerLogEvents` both subscribe to tauri-plugin-log's
// `log://log` event — the first mirrors the Rust side's `log` records into the
// devtools console, the second feeds them to the in-app Log Viewer.
// `registerDownloadEvents` subscribes to the per-job download events and
// writes them into the query cache.
//
// All three are attached at module scope rather than in a component effect,
// because none of them is scoped to a component: their lifetime is the
// window's. Module scope also means StrictMode's double-invoke cannot register
// any of them twice, and that records emitted while the shell is still
// mounting are not lost. Their detach handles are unused by design — nothing
// outlives the window that would need to call them.
//
// `queryClient` is the same module singleton `App` hands to
// `QueryClientProvider`, so writing to the cache from out here reaches exactly
// the queries the tree reads.
void attachConsole();
void registerLogEvents();
void registerDownloadEvents(queryClient);

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
