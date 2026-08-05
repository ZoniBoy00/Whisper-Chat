import { useEffect, useRef, useState } from "react";
import { Loader2, Plus, UserPlus, Users, X } from "lucide-react";
import { cx, shortPeerId } from "../lib/format";
import { useI18n } from "../i18n/I18nContext";

interface NewGroupDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Create the group; resolves with the relay-assigned group ID. */
  onCreate: (name: string, memberIds: string[]) => Promise<string>;
  /** Our own peer ID, rejected as a member. */
  myPeerId: string;
}

/** Whisper IDs are 24 lowercase hex characters. */
const PEER_ID_PATTERN = /^[0-9a-f]{24}$/i;

export function NewGroupDialog({
  open,
  onOpenChange,
  onCreate,
  myPeerId,
}: NewGroupDialogProps) {
  const { t } = useI18n();
  const dialogRef = useRef<HTMLDialogElement>(null);
  const nameInputRef = useRef<HTMLInputElement>(null);
  const [name, setName] = useState("");
  const [memberInput, setMemberInput] = useState("");
  const [members, setMembers] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [memberError, setMemberError] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    if (open && !dialog.open) {
      setName("");
      setMemberInput("");
      setMembers([]);
      setError(null);
      setMemberError(null);
      dialog.showModal();
      nameInputRef.current?.focus();
    } else if (!open && dialog.open) {
      dialog.close();
    }
  }, [open]);

  const close = () => {
    if (creating) return;
    onOpenChange(false);
  };

  const addMember = () => {
    const peerId = memberInput.trim().toLowerCase();
    if (!PEER_ID_PATTERN.test(peerId)) {
      setMemberError(t("newGroup.invalid_peer_id_24"));
      return;
    }
    if (peerId === myPeerId) {
      setMemberError(t("newGroup.already_owner"));
      return;
    }
    if (members.includes(peerId)) {
      setMemberError(t("newGroup.member_already_added"));
      return;
    }
    setMembers((prev) => [...prev, peerId]);
    setMemberInput("");
    setMemberError(null);
  };

  const removeMember = (peerId: string) => {
    setMembers((prev) => prev.filter((id) => id !== peerId));
  };

  const submit = async () => {
    const trimmedName = name.trim();
    if (!trimmedName) {
      setError(t("newGroup.group_name_required"));
      return;
    }
    if (trimmedName.length > 64) {
      setError(t("newGroup.group_name_too_long"));
      return;
    }
    if (members.length === 0) {
      setError(t("newGroup.add_member_required"));
      return;
    }
    setCreating(true);
    setError(null);
    try {
      await onCreate(trimmedName, members);
      onOpenChange(false);
    } catch (err) {
      setError(String(err).replace(/^Error:\s*/, ""));
    } finally {
      setCreating(false);
    }
  };

  return (
    <dialog
      ref={dialogRef}
      className="wp-dialog"
      aria-labelledby="new-group-title"
      onCancel={(e) => {
        e.preventDefault();
        close();
      }}
    >
      <div className="w-[min(92vw,26rem)] rounded-2xl bg-wp-panel-2 p-5">
        <div className="flex items-start justify-between gap-4">
          <div className="flex items-center gap-3">
            <div className="rounded-xl bg-wp-panel-3 p-2 text-wp-accent">
              <Users className="h-4 w-4" aria-hidden="true" />
            </div>
            <div>
              <h2
                id="new-group-title"
                className="font-display text-lg font-semibold tracking-tight text-wp-text"
              >
                {t("common.new_group")}
              </h2>
              <p className="mt-0.5 text-sm leading-relaxed text-wp-dim">
                {t("newGroup.hint")}
              </p>
            </div>
          </div>
          <button
            type="button"
            onClick={close}
            aria-label={t("common.close_dialog")}
            className="rounded-lg p-2 text-wp-dim transition hover:bg-wp-panel-3 hover:text-wp-text"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        <div className="mt-5 flex flex-col gap-4">
          <div>
            <label
              htmlFor="new-group-name"
              className="text-xs font-medium text-wp-dim"
            >
              {t("newGroup.group_name")}
            </label>
            <input
              id="new-group-name"
              ref={nameInputRef}
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="e.g. Ghost Squad"
              maxLength={64}
              autoComplete="off"
              spellCheck={false}
              aria-invalid={error ? true : undefined}
              className="mt-1.5 w-full rounded-xl bg-wp-panel-3 px-3.5 py-2.5 text-sm text-wp-text placeholder-wp-faint outline-none transition focus:ring-1 focus:ring-wp-accent/60"
            />
          </div>

          <div>
            <label
              htmlFor="new-group-member"
              className="text-xs font-medium text-wp-dim"
            >
              {t("newGroup.add_members_by_id")}
            </label>
            <div className="mt-1.5 flex gap-2">
              <input
                id="new-group-member"
                type="text"
                value={memberInput}
                onChange={(e) => {
                  setMemberInput(e.target.value);
                  setMemberError(null);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    addMember();
                  }
                }}
                placeholder="e.g. 3f2a91c07b44d8e5a1b2c3d4"
                autoComplete="off"
                spellCheck={false}
                aria-invalid={memberError ? true : undefined}
                aria-describedby={
                  memberError ? "new-group-member-error" : undefined
                }
                className="min-w-0 flex-1 rounded-xl bg-wp-panel-3 px-3.5 py-2.5 font-mono text-sm text-wp-text placeholder-wp-faint outline-none transition focus:ring-1 focus:ring-wp-accent/60"
              />
              <button
                type="button"
                onClick={addMember}
                disabled={!memberInput.trim()}
                className={cx(
                  "inline-flex shrink-0 items-center gap-1.5 rounded-xl bg-wp-panel-3 px-3.5 py-2.5 text-sm font-semibold text-wp-text transition hover:bg-wp-panel-3",
                  "disabled:cursor-not-allowed disabled:opacity-50"
                )}
              >
                <Plus className="h-4 w-4" aria-hidden="true" />
                {t("newGroup.add")}
              </button>
            </div>
            {memberError ? (
              <p
                id="new-group-member-error"
                role="alert"
                className="mt-2 text-xs leading-snug text-wp-danger"
              >
                {memberError}
              </p>
            ) : null}
          </div>

          {members.length > 0 ? (
            <ul className="flex flex-col gap-1.5" aria-label={t("newGroup.selected_members")}>
              {members.map((peerId) => (
                <li
                  key={peerId}
                  className="flex items-center gap-2 rounded-lg bg-wp-panel-3 px-3 py-2"
                >
                  <span className="min-w-0 flex-1 truncate font-mono text-xs text-wp-dim">
                    {peerId}
                  </span>
                  <span className="shrink-0 font-mono text-xs text-wp-faint">
                    {shortPeerId(peerId, 16)}
                  </span>
                  <button
                    type="button"
                    onClick={() => removeMember(peerId)}
                    aria-label={t("newGroup.remove_member_aria", { peerId })}
                    className="shrink-0 rounded-md p-1 text-wp-dim transition hover:bg-wp-panel-2 hover:text-wp-danger"
                  >
                    <X className="h-3.5 w-3.5" />
                  </button>
                </li>
              ))}
            </ul>
          ) : null}

          {error ? (
            <p role="alert" className="text-xs leading-snug text-wp-danger">
              {error}
            </p>
          ) : null}

          <button
            type="button"
            onClick={() => void submit()}
            disabled={creating || !name.trim() || members.length === 0}
            className={cx(
              "inline-flex items-center justify-center gap-2 rounded-xl px-4 py-2.5 text-sm font-semibold transition",
              "bg-wp-accent text-wp-accent-fg hover:bg-wp-accent-strong",
              "disabled:cursor-not-allowed disabled:opacity-50"
            )}
          >
            {creating ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <UserPlus className="h-4 w-4" />
            )}
            {creating ? t("newGroup.creating_group") : t("newGroup.create_group")}
          </button>
        </div>
      </div>
    </dialog>
  );
}
