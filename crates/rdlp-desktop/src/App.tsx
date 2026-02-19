import { useState } from "react";

type Tab = "search" | "queue" | "settings";

function App() {
  const [activeTab, setActiveTab] = useState<Tab>("search");

  return (
    <div className="app">
      <nav className="tab-bar">
        <button
          className={`tab-button ${activeTab === "search" ? "active" : ""}`}
          onClick={() => setActiveTab("search")}
        >
          Search
        </button>
        <button
          className={`tab-button ${activeTab === "queue" ? "active" : ""}`}
          onClick={() => setActiveTab("queue")}
        >
          Queue
        </button>
        <button
          className={`tab-button ${activeTab === "settings" ? "active" : ""}`}
          onClick={() => setActiveTab("settings")}
        >
          Settings
        </button>
      </nav>

      <main className="content">
        {activeTab === "search" && (
          <div className="tab-content">
            <h2>Search</h2>
            <p>Enter a URL to search for available formats.</p>
          </div>
        )}
        {activeTab === "queue" && (
          <div className="tab-content">
            <h2>Download Queue</h2>
            <p>Active and completed downloads will appear here.</p>
          </div>
        )}
        {activeTab === "settings" && (
          <div className="tab-content">
            <h2>Settings</h2>
            <p>Configure download preferences and output options.</p>
          </div>
        )}
      </main>
    </div>
  );
}

export default App;
