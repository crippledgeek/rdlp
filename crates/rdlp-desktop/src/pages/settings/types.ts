import type { AppSettings } from "../../types";

/** Props shared by all settings section components. */
export interface SettingsSectionProps {
    draft: AppSettings;
    onChange: (next: AppSettings) => void;
}
