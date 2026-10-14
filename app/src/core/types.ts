// Mirrors core/api/src/types.rs (serde JSON shapes). IDs are hex strings.

export type Hex = string;

export type Trust = 'Unverified' | 'Verified' | 'KeyChanged';
export type RadioMode = 'Public' | 'Community' | 'ListenOnly';
export type TxPower = 'UltraLow' | 'Low' | 'Medium' | 'High';
export type ConversationKind = 'Direct' | 'Group' | 'Channel';
export type MessageKind = 'Text' | 'Image' | 'File' | 'System' | 'Voice' | 'Call';
export type DeliveryState = 'Held' | 'Queued' | 'Sent' | 'Delivered' | 'Read' | 'Failed' | 'Received';
export type SendWhen = 'Now' | 'In10Min' | 'In30Min' | 'In2Hours' | 'Hold';
export type TransportKind = 'Ble' | 'WifiP2p' | 'Lora';
/** Carrier a message actually used (core-recorded). */
export type Via = 'Ble' | 'WifiP2p' | 'Lora' | 'Qr' | 'Internet';

export interface IdentityInfo {
  nickname: string;
  identity_key: string;
}

export interface Contact {
  id: Hex;
  nickname: string;
  trust: Trust;
  identity_key: string;
  nearby: boolean;
  /** Fastest live link to this contact right now. */
  link: TransportKind | null;
  /** Reachable over the internet route right now (address shared, route up). */
  internet: boolean;
  conversation: Hex | null;
}

/** A disguise bridge (VLESS+REALITY). The share link is only fetched for a QR. */
export interface Bridge {
  id: string;
  host: string;
  port: number;
  /** The real site whose TLS handshake the bridge borrows. */
  disguise: string;
  from_contact: boolean;
  added_ms: number;
}

export interface ConversationSummary {
  id: Hex;
  kind: ConversationKind;
  title: string;
  unread: number;
  last_activity: number;
  last_preview: string;
  trust: Trust | null;
  disappear_secs: number;
  is_admin: boolean;
  /** Public channels: the random name we post under there. */
  my_handle: string | null;
}

export type CallState = 'Calling' | 'Ringing' | 'Active' | 'Ended';

export interface CallView {
  id: string;
  contact: Hex;
  outgoing: boolean;
  state: CallState;
  started_at: number;
  answered_at: number | null;
  reason: string | null;
  link: TransportKind | null;
  /** Audio getting through from the other phone, 0–4 (active calls). */
  quality: number;
  /** Active, but no audio for a few seconds: trying to recover. */
  reconnecting: boolean;
}

export interface MessageReply {
  to: Hex;
  /** Snippet of the original. */
  quote: string;
  /** "You", a name, or null when unknown on this phone. */
  author: string | null;
  /** The original is on this phone (tap to jump to it). */
  found: boolean;
}

export interface Message {
  id: Hex;
  conversation: Hex;
  outgoing: boolean;
  sender: string;
  sender_verified: boolean;
  kind: MessageKind;
  text: string;
  file_size: number | null;
  chunks_have: number;
  chunks_total: number;
  state: DeliveryState;
  sent_at: number;
  received_at: number | null;
  expires_at: number | null;
  release_at: number | null;
  can_start_chat: boolean;
  via: Via | null;
  /** The message this one answers (swipe to reply). */
  reply: MessageReply | null;
  /** Voice note details (kind 'Voice'). */
  voice: { duration_ms: number; waveform: number[]; listened: boolean } | null;
}

export interface SendOptions {
  when?: SendWhen;
  expiry_secs?: number;
}

export type PairingOutcome =
  | { outcome: 'Paired'; detail: { contact: Hex; nickname: string; verified: boolean } }
  | { outcome: 'Failed'; detail: string }
  | { outcome: 'Request'; detail: { channel: string } };

export interface NetworkStatus {
  neighbours: number;
  contacts_nearby: number;
  transports: TransportKind[];
  last_delivery_ms: number | null;
  interference: boolean;
  radio_mode: RadioMode;
  high_risk: boolean;
  relay_envelopes: number;
  relay_bytes: number;
  crypto_failures: number;
  lora_paired: boolean;
  tx_power: TxPower;
  advertising: boolean;
  wifi_p2p_capable: boolean;
  /** Settings switch "Wi-Fi Direct for large files" (default on). */
  wifi_direct_on: boolean;
  /** Settings switch "Internet route" (default off; never in High-Risk). */
  internet_on: boolean;
  /** The route is connected now. */
  internet_online: boolean;
  /** Connecting without a disguise bridge is allowed. */
  internet_direct: boolean;
  bridges: number;
}

export type Warning =
  | { KeyChanged: { contact: Hex } }
  | 'ClockSkew'
  | { DeviceIntegrity: string }
  | 'StorageFull';

export type IntegrityStatus = 'Ok' | 'Unknown' | { Warning: string };

export interface Settings {
  radio_mode: RadioMode;
  high_risk: boolean;
  read_receipts: boolean;
  disappear_default_secs: number;
  auto_lock_ms: number;
  relay_enabled: boolean;
}

export interface Community {
  id: Hex;
  name: string;
}

export interface GroupMember {
  nickname: string;
  identity_key: string;
  trust: Trust;
  is_admin: boolean;
  is_me: boolean;
  contact: Hex | null;
  /** Member id for remove_member (also for non-contacts). */
  member: Hex | null;
}

export interface QrMatrix {
  size: number;
  /** base64, one byte per module, 1 = dark */
  modules: string;
}

export interface RgbaImage {
  width: number;
  height: number;
  rgba: string; // base64
}

export type UnlockResult =
  | { result: 'Ok' }
  | { result: 'WrongPin'; remaining: number | null }
  | { result: 'Wiped' };

export interface HandoffProgress {
  received: number;
  needed: number;
  done: boolean;
  envelopes: number;
  error: string | null;
}

export type CoreEvent =
  | { event: 'MessageAdded'; conversation: Hex; message: Message }
  | { event: 'MessageState'; message: Hex; state: DeliveryState }
  | { event: 'MessageDeleted'; message: Hex }
  | { event: 'ContactChanged'; contact: Contact }
  | { event: 'ConversationsChanged' }
  | { event: 'PairingResult'; pairing: Hex; outcome: PairingOutcome }
  | { event: 'NetworkChanged'; status: NetworkStatus }
  | { event: 'Interference'; suspected: boolean }
  | { event: 'Warning'; warning: Warning }
  | { event: 'Locked' }
  | { event: 'Wiped' }
  | { event: 'IncomingMessage' }
  | { event: 'CallChanged'; call: CallView };

export interface CoreErrorShape {
  kind: 'Locked' | 'NotFound' | 'InvalidInput' | 'TrustBlocked' | 'StorageFull' | 'Crypto' | 'Transport' | 'Keystore' | 'Internal';
  detail?: string | number;
}

export class CoreError extends Error {
  constructor(public readonly shape: CoreErrorShape) {
    super(describeError(shape));
  }
  get kind() {
    return this.shape.kind;
  }
}

export function describeError(e: CoreErrorShape): string {
  switch (e.kind) {
    case 'Locked':
      return 'The app is locked.';
    case 'NotFound':
      return 'That item no longer exists.';
    case 'InvalidInput':
      return String(e.detail ?? 'Invalid input.');
    case 'TrustBlocked':
      return "This contact's key changed. Re-pair in person before sending.";
    case 'StorageFull':
      return 'Storage is full.';
    case 'Crypto':
      return 'A message could not be decrypted.';
    case 'Transport':
      return 'Radio unavailable. Turn on Bluetooth and allow permissions.';
    case 'Keystore':
      return 'The secure hardware key is unavailable.';
    default:
      return `Internal error (${e.detail ?? '?'}).`;
  }
}
