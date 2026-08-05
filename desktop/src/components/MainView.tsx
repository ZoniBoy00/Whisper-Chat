import { useCallback, useEffect, useMemo, useState } from "react";
import type { Theme } from "../types";
import {
  createGroup,
  demoteMember,
  getGroupInfo,
  getSettings,
  leaveGroup,
  promoteMember,
  registerProfile,
  removeMember,
  resetRelay,
  setAvatar,
  setPrivacy,
  setRelayUrl as persistRelayUrl,
  setTheme as persistTheme,
  updateSettings,
} from "../lib/relay";
import { buildConversations } from "../lib/chatList";
import { useChatState } from "../hooks/useChatState";
import { useOwnProfile } from "../hooks/useOwnProfile";
import { usePresencePolling } from "../hooks/usePresencePolling";
import { Sidebar } from "./Sidebar";
import { ChatView } from "./ChatView";
import { AddContactDialog } from "./AddContactDialog";
import { GroupInfoDialog } from "./GroupInfoDialog";
import { NewGroupDialog } from "./NewGroupDialog";
import { ProfileDialog } from "./ProfileDialog";
import { SettingsDialog } from "./SettingsDialog";

interface MainViewProps {
  peerId: string;
  onReset: () => void;
}

export function MainView({ peerId, onReset }: MainViewProps) {
  const { myProfile, refreshOwnProfile } = useOwnProfile(peerId);

  const [theme, setTheme] = useState<Theme>("dark");
  const [relayUrl, setRelayUrl] = useState("");
  // Privacy / notification preferences, hydrated from the settings store on
  // mount and persisted on change.
  const [presenceVisible, setPresenceVisible] = useState(true);
  const [readReceipts, setReadReceipts] = useState(true);
  const [typingIndicator, setTypingIndicator] = useState(true);
  const [notificationsEnabled, setNotificationsEnabled] = useState(true);
  const [notificationPreview, setNotificationPreview] = useState(true);
  // Dialog open state for the various overlay panels.
  const [addDialogOpen, setAddDialogOpen] = useState(false);
  const [newGroupOpen, setNewGroupOpen] = useState(false);
  const [groupInfoGroupId, setGroupInfoGroupId] = useState<string | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  // Peer whose profile dialog is open; null when closed.
  const [profilePeerId, setProfilePeerId] = useState<string | null>(null);

  const chat = useChatState({ notificationsEnabled, notificationPreview });

  // Real-time presence pushes come through the `presence` event (registered in
  // useChatState); the poll re-seeds the active peer and covers reconnects.
  usePresencePolling({
    activePeerId: chat.activePeerId,
    connected: chat.connected,
    onPresence: chat.updatePresence,
  });

  // Keep the DOM attribute in sync with the active theme. The stylesheet
  // defines a light variant under `[data-theme="light"]`; anything else
  // falls back to the default dark palette.
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);

  // Load persisted settings (relay URL + theme) once on mount so the UI
  // reflects the user's saved choices immediately.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const settings = await getSettings();
        if (cancelled) return;
        if (settings.theme === "dark" || settings.theme === "light") {
          setTheme(settings.theme);
        }
        if (settings.relay_url) setRelayUrl(settings.relay_url);
        if (settings.presence_visible != null) setPresenceVisible(settings.presence_visible);
        setReadReceipts(settings.read_receipts ?? true);
        setTypingIndicator(settings.typing_indicator ?? true);
        setNotificationsEnabled(settings.notifications_enabled ?? true);
        setNotificationPreview(settings.notification_preview ?? true);
      } catch {
        // Settings are best-effort; the defaults (dark, default relay) apply.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const handleThemeChange = useCallback((next: Theme) => {
    setTheme(next);
    void persistTheme(next).catch(() => {
      // The theme is applied in memory immediately; persistence only affects
      // the next launch, so a failure here is non-fatal.
    });
  }, []);

  const handleRelayUrlSave = useCallback(
    async (url: string) => {
      const trimmed = url.trim();
      await persistRelayUrl(trimmed);
      setRelayUrl(trimmed);
      // Reconnect so the new endpoint takes effect, then resync state.
      await chat.connect();
      await chat.refresh();
    },
    [chat.connect, chat.refresh]
  );

  const handleSend = useCallback(
    (text: string) => {
      if (!chat.activePeerId) return;
      void chat.sendMessage(chat.activePeerId, text);
    },
    [chat.activePeerId, chat.sendMessage]
  );

  const handleTypingChange = useCallback(
    (isTyping: boolean) => {
      if (!chat.activePeerId) return;
      chat.sendTyping(chat.activePeerId, isTyping);
    },
    [chat.activePeerId, chat.sendTyping]
  );

  const handleRegisterUsername = useCallback(
    async (username: string) => {
      await registerProfile(username);
      // Re-fetch the profile so the UI picks up the new username.
      await refreshOwnProfile();
    },
    [refreshOwnProfile]
  );

  const handleSetAvatar = useCallback(
    async (avatarBase64: string) => {
      await setAvatar(avatarBase64);
      // Re-fetch the profile so the avatar_url (and preview) refresh.
      await refreshOwnProfile();
    },
    [refreshOwnProfile]
  );

  // Privacy / notification preference handlers: apply in memory immediately
  // and persist best-effort (a store failure must never block the toggle).
  const handlePresenceVisibleChange = useCallback((value: boolean) => {
    setPresenceVisible(value);
    void setPrivacy(value).catch(() => {});
  }, []);

  const handleReadReceiptsChange = useCallback((value: boolean) => {
    setReadReceipts(value);
    void updateSettings({ read_receipts: value }).catch(() => {});
  }, []);

  const handleTypingIndicatorChange = useCallback((value: boolean) => {
    setTypingIndicator(value);
    void updateSettings({ typing_indicator: value }).catch(() => {});
  }, []);

  const handleNotificationsEnabledChange = useCallback((value: boolean) => {
    setNotificationsEnabled(value);
    void updateSettings({ notifications_enabled: value }).catch(() => {});
  }, []);

  const handleNotificationPreviewChange = useCallback((value: boolean) => {
    setNotificationPreview(value);
    void updateSettings({ notification_preview: value }).catch(() => {});
  }, []);

  // Profile dialog wiring: opening focuses the active conversation's peer;
  // "Message" just closes the dialog (the chat is already open).
  const handleOpenProfile = useCallback(() => {
    if (chat.activePeerId) setProfilePeerId(chat.activePeerId);
  }, [chat.activePeerId]);

  const handleRemoveContact = useCallback(
    (targetPeerId: string) => {
      setProfilePeerId(null);
      void chat.removeContact(targetPeerId);
    },
    [chat.removeContact]
  );

  // ---- Group chat wiring --------------------------------------------------

  /** Create a group with the given name and members, then resync so the chat
   *  list shows it immediately. */
  const handleCreateGroup = useCallback(
    async (name: string, memberIds: string[]): Promise<string> => {
      const groupId = await createGroup(name, memberIds);
      await chat.refresh();
      chat.setActivePeerId(groupId);
      return groupId;
    },
    [chat.refresh, chat.setActivePeerId]
  );

  /** Fetch fresh group metadata (name, roster, roles) for the info panel. */
  const handleFetchGroupInfo = useCallback(
    (groupId: string) => getGroupInfo(groupId),
    []
  );

  /** Group admin actions: run the relay call, then resync the roster. */
  const handlePromote = useCallback(
    async (groupId: string, peerId: string) => {
      await promoteMember(groupId, peerId);
      await chat.refresh();
    },
    [chat.refresh]
  );

  const handleDemote = useCallback(
    async (groupId: string, peerId: string) => {
      await demoteMember(groupId, peerId);
      await chat.refresh();
    },
    [chat.refresh]
  );

  const handleRemoveMember = useCallback(
    async (groupId: string, peerId: string) => {
      await removeMember(groupId, peerId);
      await chat.refresh();
    },
    [chat.refresh]
  );

  const handleLeaveGroup = useCallback(
    async (groupId: string) => {
      await leaveGroup(groupId);
      setGroupInfoGroupId(null);
      await chat.refresh();
      // Close the conversation if the active chat was the group we left.
      chat.setActivePeerId((prev) => (prev === groupId ? null : prev));
    },
    [chat.refresh, chat.setActivePeerId]
  );

  const handleReset = useCallback(() => {
    void resetRelay();
    onReset();
  }, [onReset]);

  // Conversations ordered by recency of the last message so the chat list
  // behaves like Signal/WhatsApp: most recent activity first.
  const conversations = useMemo(
    () => buildConversations(chat.contacts, chat.groups, chat.messages),
    [chat.contacts, chat.groups, chat.messages]
  );

  const active =
    conversations.find((c) => c.peerId === chat.activePeerId) ?? null;

  // The contact shown in the profile dialog; falls back gracefully to the
  // peer ID when the conversation was just removed.
  const profileTarget = profilePeerId
    ? conversations.find((c) => c.peerId === profilePeerId) ?? null
    : null;

  return (
    <div className="flex h-screen overflow-hidden bg-wp-bg text-wp-text">
      <Sidebar
        peerId={peerId}
        myDisplayName={chat.myDisplayName}
        conversations={conversations}
        presence={chat.presence}
        activeId={chat.activePeerId}
        connected={chat.connected}
        connecting={chat.connecting}
        connectionError={chat.connectionError}
        relayUrl={relayUrl}
        onSelect={chat.setActivePeerId}
        onAddContact={() => setAddDialogOpen(true)}
        onNewGroup={() => setNewGroupOpen(true)}
        onStartChat={chat.addContact}
        onOpenSettings={() => setSettingsOpen(true)}
        onReconnect={() => void chat.connect()}
        onReset={handleReset}
      />
      <ChatView
        conversation={active}
        isTyping={active ? chat.typing[active.peerId] ?? false : false}
        presence={active ? chat.presence[active.peerId] ?? null : null}
        relayUrl={relayUrl}
        onSend={handleSend}
        onTypingChange={handleTypingChange}
        onOpenProfile={handleOpenProfile}
        onOpenGroupInfo={
          active?.isGroup ? () => setGroupInfoGroupId(active.peerId) : undefined
        }
      />
      <AddContactDialog
        open={addDialogOpen}
        onOpenChange={setAddDialogOpen}
        onAdd={chat.addContact}
      />
      <NewGroupDialog
        open={newGroupOpen}
        onOpenChange={setNewGroupOpen}
        onCreate={handleCreateGroup}
        myPeerId={peerId}
      />
      <GroupInfoDialog
        open={groupInfoGroupId !== null}
        groupId={groupInfoGroupId}
        onOpenChange={(open) => {
          if (!open) setGroupInfoGroupId(null);
        }}
        onFetchInfo={handleFetchGroupInfo}
        onPromote={handlePromote}
        onDemote={handleDemote}
        onRemove={handleRemoveMember}
        onLeave={handleLeaveGroup}
      />
      <ProfileDialog
        open={profilePeerId !== null}
        onOpenChange={(open) => {
          if (!open) setProfilePeerId(null);
        }}
        peerId={profilePeerId ?? ""}
        relayUrl={relayUrl}
        fallbackDisplayName={profileTarget?.displayName ?? null}
        fallbackUsername={profileTarget?.username ?? null}
        fallbackAvatarUrl={profileTarget?.avatarUrl ?? null}
        initialPresence={profilePeerId ? chat.presence[profilePeerId] ?? null : null}
        onMessage={() => setProfilePeerId(null)}
        onRemoveContact={handleRemoveContact}
      />
      <SettingsDialog
        open={settingsOpen}
        onOpenChange={setSettingsOpen}
        peerId={peerId}
        myDisplayName={chat.myDisplayName}
        myUsername={myProfile?.username ?? null}
        myAvatarUrl={myProfile?.avatar_url ?? null}
        theme={theme}
        onThemeChange={handleThemeChange}
        relayUrl={relayUrl}
        onSaveRelayUrl={handleRelayUrlSave}
        onSaveDisplayName={chat.saveDisplayName}
        onRegisterUsername={handleRegisterUsername}
        onSetAvatar={handleSetAvatar}
        onReset={handleReset}
        presenceVisible={presenceVisible}
        onPresenceVisibleChange={handlePresenceVisibleChange}
        readReceipts={readReceipts}
        onReadReceiptsChange={handleReadReceiptsChange}
        typingIndicator={typingIndicator}
        onTypingIndicatorChange={handleTypingIndicatorChange}
        notificationsEnabled={notificationsEnabled}
        onNotificationsEnabledChange={handleNotificationsEnabledChange}
        notificationPreview={notificationPreview}
        onNotificationPreviewChange={handleNotificationPreviewChange}
      />
    </div>
  );
}
