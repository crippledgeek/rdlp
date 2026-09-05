import React from "react";
import ReactDOM from "react-dom/client";
import { attachConsole } from "@tauri-apps/plugin-log";
import App from "./App";
import "./index.css";

// Forward the Rust side's `log` records into the devtools console. Attached at
// module scope rather than in an effect so StrictMode's double-invoke cannot
// register two listeners. The returned detach handle is unused: the listener
// should live as long as the window.
void attachConsole();

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
