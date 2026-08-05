export interface Message {
  id: string;
  text: string;
  outgoing: boolean;
  timestamp: number;
}

export interface Conversation {
  id: string;
  name: string;
  peerId: string;
  messages: Message[];
}
