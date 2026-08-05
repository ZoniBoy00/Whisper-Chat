import { useEffect, useRef, useState } from "react";
import { Send } from "lucide-react";
import { useI18n } from "../i18n/I18nContext";

interface ComposerProps {
  onSend: (text: string) => void;
  /** Called when the user starts/stops typing so the backend can emit the
   *  end-to-end typing indicator for the active conversation. */
  onTypingChange: (isTyping: boolean) => void;
}

/** How often the "typing" indicator is (re)sent while the user types. */
const TYPING_SEND_INTERVAL_MS = 3000;
/** Send "stopped" after this much inactivity so the peer never gets stuck. */
const TYPING_STOP_AFTER_MS = 4000;

export function Composer({ onSend, onTypingChange }: ComposerProps) {
  const { t } = useI18n();
  const [text, setText] = useState("");
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const stopTimerRef = useRef<number | null>(null);
  const lastTypingSentRef = useRef(0);

  useEffect(() => {
    return () => {
      if (stopTimerRef.current !== null) window.clearTimeout(stopTimerRef.current);
    };
  }, []);

  const sendTypingState = (isTyping: boolean) => {
    onTypingChange(isTyping);
    if (stopTimerRef.current !== null) window.clearTimeout(stopTimerRef.current);
    if (isTyping) {
      stopTimerRef.current = window.setTimeout(
        () => sendTypingState(false),
        TYPING_STOP_AFTER_MS
      );
    }
  };

  const submit = () => {
    const trimmed = text.trim();
    if (!trimmed) return;
    onSend(trimmed);
    setText("");
    lastTypingSentRef.current = Date.now();
    sendTypingState(false);
    const el = inputRef.current;
    if (el) el.style.height = "auto";
  };

  const handleChange = (value: string) => {
    setText(value);
    const el = inputRef.current;
    if (el) {
      el.style.height = "auto";
      el.style.height = `${Math.min(el.scrollHeight, 144)}px`;
    }
    const now = Date.now();
    if (value.trim()) {
      // Throttle "typing" to every 3 s; the first keystroke fires at once.
      if (now - lastTypingSentRef.current >= TYPING_SEND_INTERVAL_MS) {
        lastTypingSentRef.current = now;
        sendTypingState(true);
      }
    } else {
      // Empty input: tell the peer we stopped immediately.
      lastTypingSentRef.current = now;
      sendTypingState(false);
    }
  };

  return (
    <div className="border-t border-wp-line/10 bg-wp-panel px-4 py-3">
      <div className="mx-auto flex max-w-3xl items-end gap-3">
        <textarea
          ref={inputRef}
          rows={1}
          value={text}
          onChange={(e) => handleChange(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              submit();
            }
          }}
          placeholder={t("composer.type_a_message")}
          aria-label={t("composer.message_aria")}
          className="max-h-36 min-h-[44px] flex-1 resize-none rounded-2xl bg-wp-panel-2 px-4 py-3 text-sm text-wp-text placeholder-wp-faint outline-none transition focus:ring-1 focus:ring-wp-accent/50"
        />
        <button
          type="button"
          onClick={submit}
          disabled={!text.trim()}
          title={t("common.send_message")}
          aria-label={t("common.send_message")}
          className="inline-flex h-11 w-11 shrink-0 items-center justify-center rounded-full bg-wp-accent text-wp-accent-fg shadow-lg shadow-wp-accent/25 transition hover:bg-wp-accent-strong active:scale-95 disabled:cursor-not-allowed disabled:opacity-40"
        >
          <Send className="h-4 w-4" strokeWidth={2.2} />
        </button>
      </div>
    </div>
  );
}
