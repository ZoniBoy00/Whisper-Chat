import { Hash, MessageSquare, ShieldCheck, UserPlus, Users } from "lucide-react";
import { Logo } from "./Logo";
import { CopyButton } from "./CopyButton";

interface MainViewProps {
  peerId: string;
}

export function MainView({ peerId }: MainViewProps) {
  return (
    <div className="flex h-screen overflow-hidden bg-[#0a0e14] text-slate-200">
      <aside className="flex w-72 shrink-0 flex-col border-r border-white/5 bg-[#10141c]">
        <header className="flex items-center gap-3 border-b border-white/5 px-4 py-4">
          <Logo size={36} />
          <div>
            <h1 className="text-sm font-semibold tracking-tight text-slate-100">Whisper</h1>
            <p className="flex items-center gap-1 text-xs text-slate-500">
              <ShieldCheck className="h-3 w-3" />
              End-to-end encrypted
            </p>
          </div>
        </header>

        <div className="flex items-center justify-between border-b border-white/5 px-4 py-3">
          <h2 className="flex items-center gap-2 text-xs font-semibold uppercase tracking-wider text-slate-500">
            <Users className="h-3.5 w-3.5" />
            Contacts
          </h2>
          <button
            type="button"
            title="Add contact"
            className="rounded-lg p-1.5 text-slate-400 transition hover:bg-white/5 hover:text-white"
          >
            <UserPlus className="h-4 w-4" />
          </button>
        </div>

        <div className="flex flex-1 flex-col items-center justify-center gap-3 px-6 text-center">
          <div className="rounded-full bg-white/5 p-3 text-slate-500">
            <UserPlus className="h-5 w-5" />
          </div>
          <div>
            <p className="text-sm font-medium text-slate-400">No contacts yet</p>
            <p className="mt-1 text-xs leading-relaxed text-slate-600">
              Add a friend by their Whisper ID to start an encrypted conversation.
            </p>
          </div>
          <button
            type="button"
            className="mt-1 rounded-lg border border-white/10 bg-white/5 px-4 py-2 text-xs font-medium text-slate-300 transition hover:bg-white/10 hover:text-white"
          >
            Add contact
          </button>
        </div>
      </aside>

      <main className="flex flex-1 flex-col">
        <header className="flex items-center justify-between gap-4 border-b border-white/5 px-6 py-3.5">
          <div className="flex items-center gap-2 text-slate-400">
            <MessageSquare className="h-4 w-4" />
            <span className="text-sm font-medium text-slate-300">Whisper</span>
          </div>
          <div className="flex items-center gap-2 rounded-full border border-white/10 bg-white/5 py-1.5 pl-3 pr-1.5">
            <Hash className="h-3.5 w-3.5 text-slate-500" />
            <span className="font-mono text-xs text-slate-300">{peerId}</span>
            <CopyButton value={peerId} />
          </div>
        </header>

        <div className="flex flex-1 items-center justify-center px-8">
          <div className="text-center">
            <div className="mx-auto flex h-16 w-16 items-center justify-center rounded-2xl bg-white/5 text-slate-600">
              <MessageSquare className="h-8 w-8" />
            </div>
            <h2 className="mt-5 text-lg font-semibold text-slate-200">Select a conversation</h2>
            <p className="mx-auto mt-2 max-w-sm text-sm leading-relaxed text-slate-500">
              Choose a contact from the sidebar to start a private, encrypted
              conversation. No one else — not even Whisper — can read your
              messages.
            </p>
          </div>
        </div>
      </main>
    </div>
  );
}
