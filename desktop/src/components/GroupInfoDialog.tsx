import { useCallback, useEffect, useRef, useState } from "react";
import {
  Crown,
  Loader2,
  LogOut,
  ShieldCheck,
  Trash2,
  UserMinus,
  UserPlus,
  Users,
  X,
} from "lucide-react";
import type { GroupInfo, GroupMember } from "../types";
import { cx, shortPeerId } from "../lib/format";
import type { TFunction } from "../i18n/types";
import { useI18n } from "../i18n/I18nContext";
import { Avatar } from "./Avatar";

interface GroupInfoDialogProps {
  open: boolean;
  /** The group to show; null clears the panel. */
  groupId: string | null;
  onOpenChange: (open: boolean) => void;
  onFetchInfo: (groupId: string) => Promise<GroupInfo>;
  onPromote: (groupId: string, peerId: string) => Promise<void>;
  onDemote: (groupId: string, peerId: string) => Promise<void>;
  onRemove: (groupId: string, peerId: string) => Promise<void>;
  onLeave: (groupId: string) => Promise<void>;
}

/** A role badge. The owner badge is yellow/highlighted per WhatsApp/Signal
 *  convention; admins get a subtle shield badge. */
function RoleBadge({ role, t }: { role: GroupMember["role"]; t: TFunction }) {
  if (role === "owner") {
    return (
      <span className="inline-flex items-center gap-1 rounded-full bg-[rgb(var(--wp-owner))] px-2 py-0.5 text-[10px] font-bold uppercase tracking-wide text-[rgb(var(--wp-owner-fg))]">
        <Crown className="h-3 w-3" aria-hidden="true" />
        {t("common.owner")}
      </span>
    );
  }
  if (role === "admin") {
    return (
      <span className="inline-flex items-center gap-1 rounded-full bg-wp-panel-3 px-2 py-0.5 text-[10px] font-bold uppercase tracking-wide text-wp-dim">
        <ShieldCheck className="h-3 w-3" aria-hidden="true" />
        {t("common.admin")}
      </span>
    );
  }
  return (
    <span className="rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-wp-faint">
      {t("common.member")}
    </span>
  );
}

export function GroupInfoDialog({
  open,
  groupId,
  onOpenChange,
  onFetchInfo,
  onPromote,
  onDemote,
  onRemove,
  onLeave,
}: GroupInfoDialogProps) {
  const { t } = useI18n();
  const dialogRef = useRef<HTMLDialogElement>(null);
  const [info, setInfo] = useState<GroupInfo | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busyPeer, setBusyPeer] = useState<string | null>(null);
  const [confirmingLeave, setConfirmingLeave] = useState(false);

  const reload = useCallback(
    async (id: string) => {
      setLoading(true);
      setError(null);
      try {
        setInfo(await onFetchInfo(id));
      } catch (err) {
        setError(String(err).replace(/^Error:\s*/, ""));
      } finally {
        setLoading(false);
      }
    },
    [onFetchInfo]
  );

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    if (open && !dialog.open) {
      setConfirmingLeave(false);
      setError(null);
      dialog.showModal();
      if (groupId) void reload(groupId);
    } else if (!open && dialog.open) {
      dialog.close();
    }
  }, [open, groupId, reload]);

  const close = () => {
    if (busyPeer) return;
    onOpenChange(false);
  };

  const runAction = async (action: () => Promise<void>, peerId: string) => {
    setBusyPeer(peerId);
    setError(null);
    try {
      await action();
      if (groupId) await reload(groupId);
    } catch (err) {
      setError(String(err).replace(/^Error:\s*/, ""));
    } finally {
      setBusyPeer(null);
    }
  };

  const handleLeave = async () => {
    if (!groupId) return;
    if (!confirmingLeave) {
      setConfirmingLeave(true);
      return;
    }
    setBusyPeer("__leave__");
    setError(null);
    try {
      await onLeave(groupId);
      onOpenChange(false);
    } catch (err) {
      setError(String(err).replace(/^Error:\s*/, ""));
      setConfirmingLeave(false);
    } finally {
      setBusyPeer(null);
    }
  };

  if (!groupId) return null;

  const myRole = info?.my_role ?? null;
  const canPromote = myRole === "owner" || myRole === "admin";
  const canManage = myRole === "owner";

  return (
    <dialog
      ref={dialogRef}
      className="wp-dialog"
      aria-labelledby="group-info-title"
      onCancel={(e) => {
        e.preventDefault();
        close();
      }}
    >
      <div className="w-[min(92vw,26rem)] rounded-2xl bg-wp-panel-2">
        <div className="flex items-center justify-between gap-4 border-b border-wp-line/10 px-5 py-4">
          <div className="flex items-center gap-3">
            <div className="rounded-xl bg-wp-panel-3 p-2 text-wp-accent">
              <Users className="h-4 w-4" aria-hidden="true" />
            </div>
            <div>
              <h2
                id="group-info-title"
                className="font-display text-lg font-semibold tracking-tight text-wp-text"
              >
                {info?.name ?? t("common.group_info")}
              </h2>
              {info ? (
                <p className="text-xs text-wp-faint">
                  {t("common.members_count", { n: info.members.length })}
                </p>
              ) : null}
            </div>
          </div>
          <button
            type="button"
            onClick={close}
            aria-label={t("groupInfo.close_group_info")}
            className="rounded-lg p-2 text-wp-dim transition hover:bg-wp-panel-3 hover:text-wp-text"
          >
            <X className="h-4 w-4" aria-hidden="true" />
          </button>
        </div>

        <div className="max-h-[70vh] space-y-4 overflow-y-auto px-5 py-5">
          {loading && !info ? (
            <div className="flex items-center justify-center gap-2 py-10 text-wp-faint">
              <Loader2 className="h-4 w-4 animate-spin" aria-hidden="true" />
              {t("groupInfo.loading_members")}
            </div>
          ) : info ? (
            <ul className="flex flex-col gap-1" aria-label={t("groupInfo.group_members")}>
              {info.members.map((member) => {
                const busy = busyPeer === member.peer_id;
                return (
                  <li
                    key={member.peer_id}
                    className="flex items-center gap-3 rounded-xl px-2 py-2.5 transition hover:bg-wp-panel-3"
                  >
                    <Avatar name={member.peer_id} size={36} />
                    <div className="min-w-0 flex-1">
                      <p className="truncate font-mono text-xs text-wp-text">
                        {member.peer_id}
                      </p>
                      <p className="truncate font-mono text-[10px] text-wp-faint">
                        {shortPeerId(member.peer_id, 24)}
                      </p>
                    </div>
                    <RoleBadge role={member.role} t={t} />
                    {canPromote && member.role === "member" ? (
                      <button
                        type="button"
                        onClick={() =>
                          void runAction(
                            () => onPromote(groupId, member.peer_id),
                            member.peer_id
                          )
                        }
                        disabled={busy}
                        title={t("groupInfo.make_admin")}
                        aria-label={t("groupInfo.make_admin_aria", { peerId: member.peer_id })}
                        className="rounded-lg p-1.5 text-wp-dim transition hover:bg-wp-panel-2 hover:text-wp-accent disabled:opacity-40"
                      >
                        {busy ? (
                          <Loader2 className="h-4 w-4 animate-spin" />
                        ) : (
                          <UserPlus className="h-4 w-4" />
                        )}
                      </button>
                    ) : null}
                    {canManage && member.role === "admin" ? (
                      <button
                        type="button"
                        onClick={() =>
                          void runAction(
                            () => onDemote(groupId, member.peer_id),
                            member.peer_id
                          )
                        }
                        disabled={busy}
                        title={t("groupInfo.demote_from_admin")}
                        aria-label={t("groupInfo.demote_aria", { peerId: member.peer_id })}
                        className="rounded-lg p-1.5 text-wp-dim transition hover:bg-wp-panel-2 hover:text-wp-danger disabled:opacity-40"
                      >
                        {busy ? (
                          <Loader2 className="h-4 w-4 animate-spin" />
                        ) : (
                          <UserMinus className="h-4 w-4" />
                        )}
                      </button>
                    ) : null}
                    {canManage && member.role !== "owner" ? (
                      <button
                        type="button"
                        onClick={() =>
                          void runAction(
                            () => onRemove(groupId, member.peer_id),
                            member.peer_id
                          )
                        }
                        disabled={busy}
                        title={t("groupInfo.remove_from_group")}
                        aria-label={t("groupInfo.remove_from_group_aria", { peerId: member.peer_id })}
                        className="rounded-lg p-1.5 text-wp-dim transition hover:bg-wp-panel-2 hover:text-wp-danger disabled:opacity-40"
                      >
                        {busy ? (
                          <Loader2 className="h-4 w-4 animate-spin" />
                        ) : (
                          <Trash2 className="h-4 w-4" />
                        )}
                      </button>
                    ) : null}
                  </li>
                );
              })}
            </ul>
          ) : null}

          {error ? (
            <p role="alert" className="text-xs leading-snug text-wp-danger">
              {error}
            </p>
          ) : null}

          <div className="border-t border-wp-line/10 pt-4">
            {myRole === "owner" ? (
              <p className="mb-2 text-xs leading-snug text-wp-faint">
                {t("groupInfo.leave_group_owner_hint")}
              </p>
            ) : null}
            <button
              type="button"
              onClick={() => void handleLeave()}
              disabled={busyPeer !== null}
              className={cx(
                "inline-flex w-full items-center justify-center gap-2 rounded-xl px-4 py-2.5 text-sm font-semibold transition",
                confirmingLeave
                  ? "bg-wp-danger/15 text-wp-danger"
                  : "border border-wp-line/10 text-wp-danger hover:bg-wp-danger/10"
              )}
            >
              {busyPeer === "__leave__" ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <LogOut className="h-4 w-4" aria-hidden="true" />
              )}
              {confirmingLeave ? t("common.confirm_again") : t("groupInfo.leave_group")}
            </button>
          </div>
        </div>
      </div>
    </dialog>
  );
}
