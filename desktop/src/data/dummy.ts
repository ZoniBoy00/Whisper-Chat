import type { Conversation } from "../types";

const now = Date.now();
const MIN = 60_000;
const HOUR = 60 * MIN;

// Placeholder conversation used to demonstrate the message view while real
// chat plumbing is not wired up yet. Remove once conversations are real.
export const dummyConversations: Conversation[] = [
  {
    id: "c1",
    name: "Alex Chen",
    peerId: "a1b2c3d4e5f60718",
    messages: [
      { id: "m1", text: "Hey! Did you get my last message?", outgoing: false, timestamp: now - 26 * HOUR },
      { id: "m2", text: "Yes, sorry — work ran late. It's all good.", outgoing: true, timestamp: now - 25 * HOUR },
      { id: "m3", text: "No worries. Wanna catch up this weekend?", outgoing: false, timestamp: now - 24 * HOUR },
      { id: "m4", text: "Definitely. Saturday noon at the usual spot?", outgoing: true, timestamp: now - 23 * HOUR },
      { id: "m5", text: "Perfect. I'll bring the notes you asked for.", outgoing: false, timestamp: now - 40 * MIN },
      { id: "m6", text: "Perfect — that makes two of us then. See you!", outgoing: true, timestamp: now - 35 * MIN },
      { id: "m7", text: "By the way, the Whisper beta feels really smooth. No leaks so far 😉", outgoing: false, timestamp: now - 12 * MIN },
    ],
  },
];
