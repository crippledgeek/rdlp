import { useEffect, useState } from "react";
import { useSettingsStore } from "../lib/store";
import type { AppSettings } from "../types";

export function SettingsPage() {
  const { settings, loading, loadSettings, updateSettings, pickDirectory } =
    useSettingsStore();
  const [draft, setDraft] = useState<AppSettings | null>(null);

  useEffect(() => {
    loadSettings();
  }, [loadSettings]);

  useEffect(() => {
    if (settings) {
      setDraft(settings);
    }
  }, [settings]);

  if (loading || !draft) {
    return <div className="status-message">Loading settings...</div>;
  }

  const handleSave = async () => {
    await updateSettings(draft);
  };

  const handlePickDir = async () => {
    const dir = await pickDirectory();
    if (dir) {
      setDraft({ ...draft, output_dir: dir });
    }
  };

  return (
    <div className="settings-page">
      <h2>Settings</h2>

      <div className="setting-row">
        <label>Output Directory</label>
        <div className="dir-picker">
          <input type="text" value={draft.output_dir} readOnly />
          <button onClick={handlePickDir}>Browse</button>
        </div>
      </div>

      <div className="setting-row">
        <label>
          <input
            type="checkbox"
            checked={draft.embed_thumbnail}
            onChange={(e) =>
              setDraft({ ...draft, embed_thumbnail: e.target.checked })
            }
          />
          Embed thumbnails
        </label>
      </div>

      <div className="setting-row">
        <label>
          <input
            type="checkbox"
            checked={draft.embed_metadata}
            onChange={(e) =>
              setDraft({ ...draft, embed_metadata: e.target.checked })
            }
          />
          Embed metadata
        </label>
      </div>

      <div className="setting-row">
        <label>
          <input
            type="checkbox"
            checked={draft.verbose}
            onChange={(e) =>
              setDraft({ ...draft, verbose: e.target.checked })
            }
          />
          Verbose logging
        </label>
      </div>

      <button className="save-btn" onClick={handleSave}>
        Save Settings
      </button>
    </div>
  );
}
