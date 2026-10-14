//! Public API types (section 13). All IDs are 16 random bytes; across the
//! JSON boundary they are lowercase hex strings.

use serde::{Deserialize, Serialize};

pub type Id16 = [u8; 16];

pub mod hex16 {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &[u8; 16], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(v))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 16], D::Error> {
        let s = String::deserialize(d)?;
        let v = hex::decode(&s).map_err(serde::de::Error::custom)?;
        v.try_into()
            .map_err(|_| serde::de::Error::custom("expected 16 bytes"))
    }
}

pub mod hex16_opt {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &Option<[u8; 16]>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(v) => s.serialize_some(&hex::encode(v)),
            None => s.serialize_none(),
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<[u8; 16]>, D::Error> {
        let s: Option<String> = Option::deserialize(d)?;
        match s {
            None => Ok(None),
            Some(s) => {
                let v = hex::decode(&s).map_err(serde::de::Error::custom)?;
                Ok(Some(v.try_into().map_err(|_| {
                    serde::de::Error::custom("expected 16 bytes")
                })?))
            }
        }
    }
}

pub mod b64 {
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&base64::engine::general_purpose::STANDARD.encode(v))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        base64::engine::general_purpose::STANDARD
            .decode(s)
            .map_err(serde::de::Error::custom)
    }
}

pub type PeerHandle = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TransportKind {
    Ble,
    WifiP2p,
    Lora,
}

/// How a message actually left this phone or arrived on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Via {
    Ble,
    WifiP2p,
    Lora,
    /// QR handoff by camera.
    Qr,
    /// The internet route (Nym mixnet, amendment N1).
    Internet,
}

impl From<TransportKind> for Via {
    fn from(k: TransportKind) -> Self {
        match k {
            TransportKind::Ble => Via::Ble,
            TransportKind::WifiP2p => Via::WifiP2p,
            TransportKind::Lora => Via::Lora,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RadioMode {
    Public,
    Community,
    ListenOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TxPower {
    UltraLow,
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub wifi_p2p: bool,
    pub lora_paired: bool,
    pub tx_power_control: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum KeySlot {
    RelayDb,
    MessageDb,
    Duress,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum KeystoreError {
    #[error("hardware key unavailable")]
    Unavailable,
    #[error("user cancelled")]
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
#[serde(tag = "kind", content = "detail")]
pub enum CoreError {
    #[error("locked")]
    Locked,
    #[error("not found")]
    NotFound,
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("trust blocked")]
    TrustBlocked,
    #[error("storage full")]
    StorageFull,
    #[error("crypto")]
    Crypto,
    #[error("transport: {0}")]
    Transport(String),
    #[error("keystore")]
    Keystore,
    #[error("internal: {0}")]
    Internal(u32),
}

pub type CoreResult<T> = Result<T, CoreError>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result")]
pub enum UnlockResult {
    Ok,
    WrongPin { remaining: Option<u32> },
    Wiped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Trust {
    Unverified,
    Verified,
    KeyChanged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityInfo {
    pub nickname: String,
    /// Hex of the Ed25519 identity key.
    pub identity_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contact {
    #[serde(with = "hex16")]
    pub id: Id16,
    pub nickname: String,
    pub trust: Trust,
    pub identity_key: String,
    /// In range right now (beacon recognised).
    pub nearby: bool,
    /// Fastest live link to this contact right now, if any.
    pub link: Option<TransportKind>,
    /// Reachable over the internet route right now (they shared an address
    /// and our route is up).
    pub internet: bool,
    #[serde(with = "hex16_opt")]
    pub conversation: Option<Id16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConversationKind {
    Direct,
    Group,
    Channel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationSummary {
    #[serde(with = "hex16")]
    pub id: Id16,
    pub kind: ConversationKind,
    pub title: String,
    pub unread: u32,
    pub last_activity: u64,
    pub last_preview: String,
    pub trust: Option<Trust>,
    pub disappear_secs: u32,
    pub is_admin: bool,
    /// Public channels: the random name we post under there.
    pub my_handle: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageKind {
    Text,
    Image,
    File,
    System,
    /// Appended last: stored records keep their variant numbers.
    Voice,
    /// A call note ("Call · 3:42 · over Wi-Fi", "Missed call"): local only.
    Call,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeliveryState {
    Held,
    Queued,
    Sent,
    Delivered,
    Read,
    Failed,
    Received,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    #[serde(with = "hex16")]
    pub id: Id16,
    #[serde(with = "hex16")]
    pub conversation: Id16,
    pub outgoing: bool,
    pub sender: String,
    pub sender_verified: bool,
    pub kind: MessageKind,
    /// Text body (sanitised) or file name.
    pub text: String,
    pub file_size: Option<u64>,
    pub chunks_have: u32,
    pub chunks_total: u32,
    pub state: DeliveryState,
    pub sent_at: u64,
    pub received_at: Option<u64>,
    pub expires_at: Option<u64>,
    pub release_at: Option<u64>,
    /// Remote pairing offer: present on channel posts (hex reply tag).
    pub can_start_chat: bool,
    /// Carrier this message used (sent: link it left on; received: link it
    /// arrived on). None when unknown, e.g. still queued.
    pub via: Option<Via>,
    /// The message this one answers (swipe to reply).
    pub reply: Option<ReplyView>,
    /// Voice note details (kind Voice).
    pub voice: Option<VoiceView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceView {
    /// Known once the whole note is here (0 while it arrives).
    pub duration_ms: u32,
    /// Up to 64 bars, 0–255.
    pub waveform: Vec<u8>,
    /// Played on this phone (always true for our own notes).
    pub listened: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyView {
    #[serde(with = "hex16")]
    pub to: Id16,
    /// Snippet of the original (sanitised).
    pub quote: String,
    /// Who wrote the original: "You", a name, or None when unknown here.
    pub author: Option<String>,
    /// The original is on this phone (tap to jump to it).
    pub found: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SendWhen {
    #[default]
    Now,
    In10Min,
    In30Min,
    In2Hours,
    Hold,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendOptions {
    #[serde(default)]
    pub when: SendWhen,
    /// 0 = conversation default.
    #[serde(default)]
    pub expiry_secs: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", content = "detail")]
pub enum PairingOutcome {
    Paired {
        #[serde(with = "hex16")]
        contact: Id16,
        nickname: String,
        verified: bool,
    },
    Failed(String),
    /// Remote request awaiting the user's Accept / Decline.
    Request {
        channel: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkStatus {
    pub neighbours: u32,
    pub contacts_nearby: u32,
    pub transports: Vec<TransportKind>,
    pub last_delivery_ms: Option<u64>,
    pub interference: bool,
    pub radio_mode: RadioMode,
    pub high_risk: bool,
    /// Sealed envelopes held for other people (not ours, not for us).
    pub relay_envelopes: u32,
    pub relay_bytes: u64,
    pub crypto_failures: u32,
    pub lora_paired: bool,
    pub tx_power: TxPower,
    pub advertising: bool,
    /// This device can open Wi-Fi Direct links (host capability).
    pub wifi_p2p_capable: bool,
    /// The Settings switch "Wi-Fi Direct for large files" (default on).
    pub wifi_direct_on: bool,
    /// Settings switch "Internet route" (default off).
    pub internet_on: bool,
    /// The route is connected right now.
    pub internet_online: bool,
    /// Connecting without a disguise bridge is allowed.
    pub internet_direct: bool,
    /// Disguise bridges stored on this phone.
    pub bridges: u32,
}

/// A disguise bridge as the app shows it (the share link itself only for QR).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bridge {
    pub id: String,
    pub host: String,
    pub port: u16,
    /// The real site the bridge borrows its TLS handshake from.
    pub disguise: String,
    pub from_contact: bool,
    pub added_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Warning {
    KeyChanged {
        #[serde(with = "hex16")]
        contact: Id16,
    },
    ClockSkew,
    DeviceIntegrity(String),
    StorageFull,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntegrityStatus {
    Ok,
    Warning(String),
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Community {
    #[serde(with = "hex16")]
    pub id: Id16,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub radio_mode: RadioMode,
    pub high_risk: bool,
    pub read_receipts: bool,
    pub disappear_default_secs: u32,
    pub auto_lock_ms: u64,
    pub relay_enabled: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            radio_mode: RadioMode::Public,
            high_risk: false,
            read_receipts: true,
            disappear_default_secs: mesh_constants::DISAPPEARING_DEFAULT_SECS,
            auto_lock_ms: mesh_constants::AUTO_LOCK_MS,
            relay_enabled: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupMember {
    pub nickname: String,
    pub identity_key: String,
    pub trust: Trust,
    pub is_admin: bool,
    pub is_me: bool,
    #[serde(with = "hex16_opt")]
    pub contact: Option<Id16>,
    /// Member id for `remove_member` (also for members who are not contacts).
    #[serde(with = "hex16_opt")]
    pub member: Option<Id16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    #[serde(with = "b64")]
    pub rgba: Vec<u8>,
}

/// Events delivered to the UI (EventSink, section 13).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event")]
pub enum Event {
    MessageAdded {
        #[serde(with = "hex16")]
        conversation: Id16,
        message: Message,
    },
    MessageState {
        #[serde(with = "hex16")]
        message: Id16,
        state: DeliveryState,
    },
    MessageDeleted {
        #[serde(with = "hex16")]
        message: Id16,
    },
    ContactChanged {
        contact: Contact,
    },
    ConversationsChanged,
    PairingResult {
        #[serde(with = "hex16")]
        pairing: Id16,
        outcome: PairingOutcome,
    },
    NetworkChanged {
        status: NetworkStatus,
    },
    Interference {
        suspected: bool,
    },
    Warning {
        warning: Warning,
    },
    Locked,
    Wiped,
    /// A sealed message for us arrived (no content; also fires while locked).
    IncomingMessage,
    /// A nearby call rang, connected or ended.
    CallChanged {
        call: CallView,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallState {
    /// We are ringing them.
    Calling,
    /// They are ringing us.
    Ringing,
    Active,
    Ended,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallView {
    /// Hex call id.
    pub id: String,
    #[serde(with = "hex16")]
    pub contact: Id16,
    pub outgoing: bool,
    pub state: CallState,
    pub started_at: u64,
    pub answered_at: Option<u64>,
    /// Why it ended ("declined", "busy", "no answer", "connection lost", …).
    pub reason: Option<String>,
    /// The link carrying it now (Bluetooth or Wi-Fi Direct).
    pub link: Option<TransportKind>,
    /// Audio getting through from the other phone, 0 (none) to 4 (all of
    /// it), over the last couple of seconds. Only while active.
    pub quality: u8,
    /// Active, but no audio has arrived for a few seconds (moving apart, or
    /// the link changing): the call is trying to recover.
    pub reconnecting: bool,
}
