import { useEffect, useState } from "react";
import { useSettingsStore, useSearchStore } from "../lib/store";
import type {
    AppSettings,
    AudioFormat,
    ContainerFormat,
    SubtitleFormat,
} from "../types";

export function SettingsPage() {
    const { settings, loading, loadSettings, updateSettings, pickDirectory } =
        useSettingsStore();
    const providers = useSearchStore((s) => s.providers);
    const loadProviders = useSearchStore((s) => s.loadProviders);
    const [draft, setDraft] = useState<AppSettings | null>(null);

    useEffect(() => {
        loadSettings();
        void loadProviders();
    }, [loadSettings, loadProviders]);

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
                <label>Default Remux Format</label>
                <select
                    className="filter-select"
                    value={draft.default_remux ?? ""}
                    onChange={(e) =>
                        setDraft({
                            ...draft,
                            default_remux:
                                (e.target.value as ContainerFormat) || null,
                        })
                    }
                >
                    <option value="">None</option>
                    <option value="mp4">MP4</option>
                    <option value="mkv">MKV</option>
                    <option value="webm">WebM</option>
                </select>
            </div>

            <div className="setting-row">
                <label>Default Audio Extraction</label>
                <select
                    className="filter-select"
                    value={draft.default_extract_audio ?? ""}
                    onChange={(e) =>
                        setDraft({
                            ...draft,
                            default_extract_audio:
                                (e.target.value as AudioFormat) || null,
                        })
                    }
                >
                    <option value="">None</option>
                    <option value="mp3">MP3</option>
                    <option value="aac">AAC</option>
                    <option value="opus">Opus</option>
                    <option value="flac">FLAC</option>
                </select>
            </div>

            <div className="setting-row">
                <label>Default Subtitle Format</label>
                <select
                    className="filter-select"
                    value={draft.default_subtitle_format ?? ""}
                    onChange={(e) =>
                        setDraft({
                            ...draft,
                            default_subtitle_format:
                                (e.target.value as SubtitleFormat) || null,
                        })
                    }
                >
                    <option value="">None</option>
                    <option value="srt">SRT</option>
                    <option value="vtt">VTT</option>
                    <option value="ass">ASS</option>
                </select>
            </div>

            <div className="setting-row">
                <label>Default Subtitle Languages</label>
                <input
                    className="settings-text-input"
                    type="text"
                    placeholder="en,sv,ja"
                    value={draft.default_subtitle_langs.join(",")}
                    onChange={(e) =>
                        setDraft({
                            ...draft,
                            default_subtitle_langs: e.target.value
                                .split(",")
                                .map((s) => s.trim())
                                .filter(Boolean),
                        })
                    }
                />
            </div>

            <div className="setting-row">
                <label>Default Search Provider</label>
                <select
                    className="filter-select"
                    value={draft.default_search_provider ?? ""}
                    onChange={(e) =>
                        setDraft({
                            ...draft,
                            default_search_provider:
                                e.target.value || null,
                        })
                    }
                >
                    <option value="">Auto</option>
                    {providers.map((p) => (
                        <option key={p.name} value={p.name}>
                            {p.display_name}
                        </option>
                    ))}
                </select>
            </div>

            <div className="setting-row">
                <label>
                    <input
                        type="checkbox"
                        checked={draft.embed_thumbnail}
                        onChange={(e) =>
                            setDraft({
                                ...draft,
                                embed_thumbnail: e.target.checked,
                            })
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
                            setDraft({
                                ...draft,
                                embed_metadata: e.target.checked,
                            })
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
                            setDraft({
                                ...draft,
                                verbose: e.target.checked,
                            })
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
