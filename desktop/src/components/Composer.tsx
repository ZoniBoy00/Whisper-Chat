import { useRef, useState } from "react";
import { Send } from "lucide-react";

interface ComposerProps {
  onSend: (text: string) => void;
}

export function Composer({ onSend }: ComposerProps) {
  const [text, setText] = useState("");
  const inputRef = useRef<HTMLTextAreaElement>(null);

  const submit = () => {
    const trimmed = text.trim();
    if (!trimmed) return;
    onSend(trimmed);
    setText("");
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
          placeholder="Type a message"
          aria-label="Message"
          className="max-h-36 min-h-[42px] flex-1 resize-none rounded-2xl bg-wp-panel-2 px-4 py-2.5 text-sm text-wp-text placeholder-wp-faint outline-none transition focus:ring-1 focus:ring-wp-accent/50"
        />
        <button
          type="button"
          onClick={submit}
          disabled={!text.trim()}
          title="Send message"
          aria-label="Send message"
          className="inline-flex h-10 w-10 shrink-0 items-center justify-center rounded-full bg-wp-accent text-wp-accent-fg shadow-lg shadow-wp-accent/25 transition hover:bg-wp-accent-strong disabled:cursor-not-allowed disabled:opacity-40"
        >
          <Send className="h-4 w-4" strokeWidth={2.2} />
        </button>
      </div>
    </div>
  );
}
