import { useState } from "react";
import type { ReactNode } from "react";
import { Bell, Info, Settings, ShieldCheck } from "lucide-react";
import { cx } from "../../lib/format";

export type TabId = "general" | "privacy" | "notifications" | "about";

const TABS: { id: TabId; label: string; icon: typeof Settings }[] = [
  { id: "general", label: "General", icon: Settings },
  { id: "privacy", label: "Privacy", icon: ShieldCheck },
  { id: "notifications", label: "Notifications", icon: Bell },
  { id: "about", label: "About", icon: Info },
];

interface SettingsTabsProps {
  /** Renders the tab panels; the currently selected tab id is passed through
   *  so each panel can gate itself with `hidden`. */
  children: (activeTab: TabId) => ReactNode;
}

/** Tab strip (WAI-ARIA tabs pattern) + the scrollable panel container. The
 *  selected-tab state lives here so arrow-key navigation stays local. */
export function SettingsTabs({ children }: SettingsTabsProps) {
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
        aria-label="Settings sections"
        className="flex gap-1 border-b border-wp-line/10 px-4 pt-3"
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
                "inline-flex items-center gap-1.5 rounded-t-lg px-3 py-2 text-xs font-semibold transition",
                selected
                  ? "border-b-2 border-wp-accent text-wp-text"
                  : "border-b-2 border-transparent text-wp-dim hover:text-wp-text"
              )}
            >
              <tab.icon className="h-3.5 w-3.5" aria-hidden="true" />
              {tab.label}
            </button>
          );
        })}
      </div>

      <div className="max-h-[70vh] space-y-6 overflow-y-auto px-5 py-5">
        {children(activeTab)}
      </div>
    </>
  );
}
