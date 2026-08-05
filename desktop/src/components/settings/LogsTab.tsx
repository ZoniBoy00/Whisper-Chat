import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Check, Copy, RefreshCw, ScrollText, TriangleAlert } from "lucide-react";
import type { LogEntry } from "../../types";
import { getClientLogs } from "../../lib/relay";
import { copyText } from "../../lib/clipboard";
import { cx } from "../../lib/format";
import { useI18n } from "../../i18n/I18nContext";
import { useToast } from "../../hooks/useToast";
import { SectionHeading } from "./controls";

/** Cap on the number of lines fetched from the Rust ring buffer. */
const LOG_FETCH_LIMIT = 1000;

/** Filter modes: every line, or only ERROR lines. */
type LogFilter = "all" | "errors";

/** Render a log timestamp as a compact local clock time with milliseconds. */
function formatLogTime(timestamp: number): string {
  return new Date(timestamp).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    fractionalSecondDigits: 3,
  });
}

/** Level colouring for the terminal-style log line prefix. */
function levelClass(level: string): string {
  switch (level) {
    case "ERROR":
      return "text-wp-danger";
    case "WARN":
      return "text-wp-accent";
    case "INFO":
      return "text-wp-online";
    default:
      return "text-wp-faint";
  }
}

/** One log line rendered in the terminal-style listing. */
function LogLine({ entry }: { entry: LogEntry }) {
  return (
    <p className="break-all">
      <span className="text-wp-faint">[{formatLogTime(entry.timestamp)}]</span>{" "}
      <span className={cx("font-semibold", levelClass(entry.level))}>
        {entry.level}
      </span>{" "}
      <span className="text-wp-dim">{entry.target}:</span>{" "}
      <span className="text-wp-text">{entry.message}</span>
    </p>
  );
}

/** The Logs settings tab: a terminal-style view over the client log ring
 *  buffer (Rust tracing events + webview errors). Refreshes on open, offers
 *  Refresh / Copy and an All/Errors filter. Deliberately NOT a live region —
 *  announcing every line would drown screen-reader users. */
export function LogsTab({ active }: { active: boolean }) {
  const { t } = useI18n();
  const { toast } = useToast();
  const [entries, setEntries] = useState<LogEntry[]>([]);
  const [filter, setFilter] = useState<LogFilter>("all");
  const [loading, setLoading] = useState(false);
  const [copied, setCopied] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const logs = await getClientLogs(LOG_FETCH_LIMIT);
      setEntries(logs);
    } catch {
      toast(t("logs.load_failed"), "error");
    } finally {
      setLoading(false);
    }
  }, [t, toast]);

  // Refresh whenever the tab becomes active so a reopened dialog always shows
  // the newest lines (the Rust buffer keeps growing in the background).
  useEffect(() => {
    if (active) void load();
  }, [active, load]);

  // Auto-scroll to the bottom when the visible lines change (newest last).
  useEffect(() => {
    const region = scrollRef.current;
    if (region) region.scrollTop = region.scrollHeight;
  }, [entries, filter]);

  const visible = useMemo(
    () =>
      filter === "errors"
        ? entries.filter((entry) => entry.level === "ERROR")
        : entries,
    [entries, filter]
  );

  const handleCopy = async () => {
    const text = visible
      .map(
        (entry) =>
          `[${formatLogTime(entry.timestamp)}] ${entry.level} ${entry.target}: ${entry.message}`
      )
      .join("\n");
    const ok = await copyText(text);
    if (ok) {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1600);
    }
  };

  const CopyIcon = copied ? Check : Copy;

  return (
    <div
      role="tabpanel"
      id="settings-panel-logs"
      aria-labelledby="settings-tab-logs"
      className="space-y-3"
      hidden={!active}
    >
      <SectionHeading
        id="settings-logs-title"
        icon={<ScrollText className="h-3.5 w-3.5" />}
        label={t("settings.tab_logs")}
      />
      <p className="text-xs leading-snug text-wp-faint">{t("logs.intro")}</p>

      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex shrink-0 gap-1 rounded-xl bg-wp-panel-2 p-1">
          {(["all", "errors"] as const).map((value) => (
            <button
              key={value}
              type="button"
              aria-pressed={filter === value}
              onClick={() => setFilter(value)}
              className={cx(
                "inline-flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-xs font-semibold transition",
                filter === value
                  ? "bg-wp-accent text-wp-accent-fg"
                  : "text-wp-dim hover:text-wp-text"
              )}
            >
              {value === "errors" && filter === "errors" ? (
                <TriangleAlert className="h-3.5 w-3.5" aria-hidden="true" />
              ) : null}
              {value === "all" ? t("logs.filter_all") : t("logs.filter_errors")}
            </button>
          ))}
        </div>
        <div className="flex shrink-0 gap-2">
          <button
            type="button"
            onClick={() => void load()}
            disabled={loading}
            className="inline-flex items-center gap-1.5 rounded-lg border border-wp-line/10 bg-wp-panel-2 px-3 py-1.5 text-xs font-semibold text-wp-text transition hover:bg-wp-panel-3 disabled:cursor-not-allowed disabled:opacity-50"
          >
            <RefreshCw
              className={cx("h-3.5 w-3.5", loading && "animate-spin")}
              aria-hidden="true"
            />
            {t("logs.refresh")}
          </button>
          <button
            type="button"
            onClick={() => void handleCopy()}
            disabled={visible.length === 0}
            className="inline-flex items-center gap-1.5 rounded-lg border border-wp-line/10 bg-wp-panel-2 px-3 py-1.5 text-xs font-semibold text-wp-text transition hover:bg-wp-panel-3 disabled:cursor-not-allowed disabled:opacity-50"
          >
            <CopyIcon className="h-3.5 w-3.5" aria-hidden="true" />
            {copied ? t("common.copied") : t("logs.copy")}
          </button>
        </div>
      </div>

      <div
        ref={scrollRef}
        role="region"
        aria-label={t("logs.list_aria")}
        tabIndex={0}
        className="max-h-64 overflow-y-auto rounded-xl border border-wp-line/10 bg-wp-panel-3 p-3 font-mono text-[11px] leading-relaxed"
      >
        {visible.length === 0 ? (
          <p className="py-8 text-center text-xs text-wp-faint">
            {t("logs.empty")}
          </p>
        ) : (
          visible.map((entry, index) => (
            <LogLine
              key={`${entry.timestamp}-${index}`}
              entry={entry}
            />
          ))
        )}
      </div>
    </div>
  );
}
