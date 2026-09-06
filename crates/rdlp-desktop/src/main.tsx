import React from "react";
import ReactDOM from "react-dom/client";
import { attachConsole } from "@tauri-apps/plugin-log";
import App from "./App";
import { withDevtools } from "./devtools/withDevtools";
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
// any of them twice. Their detach handles are unused by design — nothing
// outlives the window that would need to call them.
//
// Registering before first render additionally keeps log records emitted
// during startup, which a mount-time effect would have missed. That benefit is
// specific to the two log listeners: they write to a module-level ring buffer
// that exists already. It does NOT extend to the download events, whose
// `setQueryData` updaters are all `old?.map(...)` and so no-op until the
// downloads query has data — harmless, since a download event cannot precede
// mount when the user has to start the download from the UI.
//
// `queryClient` is the same module singleton `App` hands to
// `QueryClientProvider`, so writing to the cache from out here reaches exactly
// the queries the tree reads.
void attachConsole();
void registerLogEvents();
void registerDownloadEvents(queryClient);

// Devtools are attached here rather than inside `App`, so the app component
// carries no reference to them.
//
// This file is NOT rewritten by the devtools Vite plugin — the transform only
// visits files that name a devtools package, and this one names none. So in a
// production build `withDevtools` is still imported and still called here; what
// changes is that the wrapper it returns has had its devtools JSX stripped and
// renders only `<App />`.
const Root = withDevtools(App);

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <Root />
  </React.StrictMode>,
);
