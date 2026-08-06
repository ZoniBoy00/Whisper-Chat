import { useState } from "react";
import type { ReactNode } from "react";
import { Bell, Info, ScrollText, Settings, ShieldCheck } from "lucide-react";
import { cx } from "../../lib/format";
import { useI18n } from "../../i18n/I18nContext";
import type { TranslationKey } from "../../i18n/types";

export type TabId = "general" | "privacy" | "notifications" | "logs" | "about";

interface TabDef {
  id: TabId;
  labelKey: TranslationKey;
  icon: typeof Settings;
}

const TABS: TabDef[] = [
  { id: "general", labelKey: "settings.tab_general", icon: Settings },
  { id: "privacy", labelKey: "settings.tab_privacy", icon: ShieldCheck },
  { id: "notifications", labelKey: "settings.tab_notifications", icon: Bell },
  { id: "logs", labelKey: "settings.tab_logs", icon: ScrollText },
  { id: "about", labelKey: "settings.tab_about", icon: Info },
];

interface SettingsTabsProps {
  /** Renders the tab panels; the currently selected tab id is passed through
   *  so each panel can gate itself with `hidden`. */
  children: (activeTab: TabId) => ReactNode;
}

/** Tab strip (WAI-ARIA tabs pattern) + the scrollable panel container. The
 *  selected-tab state lives here so arrow-key navigation stays local. */
export function SettingsTabs({ children }: SettingsTabsProps) {
  const { t } = useI18n();
  const [activeTab, setActiveTab] = useState<TabId>("general");

  /** Arrow-key navigation for the tab strip (a11y: WAI-ARIA tabs pattern). */
  const handleTabKey = (event: React.KeyboardEvent) => {
    const index = TABS.findIndex((tab) => tab.id === activeTab);
    let next = index;
    if (event.key === "ArrowRight") next = (index + 1) % TABS.length;
    else if (event.key === "ArrowLeft") next = (index - 1 + TABS.length) % TABS.length;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = TABS.length - 1;
    else return;
    event.preventDefault();
    const tab = TABS[next];
    setActiveTab(tab.id);
    document.getElementById(`settings-tab-${tab.id}`)?.focus();
  };

  return (
    <>
      <div
        role="tablist"
        aria-label={t("settings.sections_aria")}
        className="flex shrink-0 flex-wrap gap-1.5 border-b border-wp-line/10 px-6 pb-4 pt-4"
      >
        {TABS.map((tab) => {
          const selected = activeTab === tab.id;
          return (
            <button
              key={tab.id}
              type="button"
              role="tab"
              id={`settings-tab-${tab.id}`}
              aria-selected={selected}
              aria-controls={`settings-panel-${tab.id}`}
              tabIndex={selected ? 0 : -1}
              onClick={() => setActiveTab(tab.id)}
              onKeyDown={handleTabKey}
              className={cx(
                "inline-flex items-center gap-2 rounded-xl px-4 py-2.5 text-sm font-semibold transition",
                selected
                  ? "bg-wp-accent text-wp-accent-fg shadow-sm"
                  : "text-wp-dim hover:bg-wp-panel-3 hover:text-wp-text"
              )}
            >
              <tab.icon className="h-4 w-4" aria-hidden="true" />
              {t(tab.labelKey)}
            </button>
          );
        })}
      </div>

      <div className="min-h-0 flex-1 space-y-6 overflow-y-auto px-6 py-6">
        {children(activeTab)}
      </div>
    </>
  );
}
