//! Persistent records (D34): typed values stored in the two key-value
//! databases, encoded with postcard.

use crate::types::*;
use mesh_storage::{Kv, Op};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

pub mod t {
    // relay.db
    pub const SETTINGS: &str = "settings";
    pub const ENV: &str = "env";
    pub const ACK: &str = "ack";
    pub const TAGKEY: &str = "tagkey";
    pub const PRESENCE: &str = "presence";
    pub const RELAY_ME: &str = "me";
    pub const COMMUNITY: &str = "community";
    pub const DELIVERED: &str = "delivered";
    /// Internet route (amendment N1): contact -> their Nym address.
    pub const NETADDR: &str = "netaddr";
    /// Envelope id -> contacts still owed it over the internet.
    pub const NETOUT: &str = "netout";
    /// Contact -> the address of ours they last received.
    pub const NETSHARED: &str = "netshared";
    /// Bridge id -> BridgeRec (disguise bridges).
    pub const BRIDGE: &str = "bridge";
    // message.db
    pub const IDENTITY: &str = "identity";
    pub const PEER: &str = "peer";
    pub const OLM: &str = "olm";
    pub const CONV: &str = "conv";
    pub const MSG: &str = "msg";
    pub const MSGIDX: &str = "msgidx";
    /// Carrier per message (uuid -> Via). Separate table so existing
    /// postcard-encoded MsgRec rows stay readable.
    pub const MSGVIA: &str = "msgvia";
    /// Reply metadata per message (uuid -> ReplyRec). Separate table so
    /// existing postcard-encoded MsgRec rows stay readable.
    pub const MSGREPLY: &str = "msgreply";
    /// Voice note details per message (uuid -> VoiceRec).
    pub const MSGVOICE: &str = "msgvoice";
    pub const CHUNK: &str = "chunk";
    pub const GROUP: &str = "group";
    pub const MEGOLM_IN: &str = "megolm_in";
    pub const MEGOLM_SEEN: &str = "megolm_seen";
    pub const CHANNEL: &str = "channel";
    pub const POST: &str = "post";
    pub const OUTENV: &str = "outenv";
    pub const PENDING_OUT: &str = "pending_out";
    pub const MEDIA_KEY: &str = "media_key";
    pub const PENDING_CHUNK: &str = "pending_chunk";
}

pub fn enc<T: Serialize>(v: &T) -> Vec<u8> {
    postcard::to_allocvec(v).expect("postcard encoding of owned records cannot fail")
}

pub fn dec<T: DeserializeOwned>(b: &[u8]) -> Option<T> {
    postcard::from_bytes(b).ok()
}

pub fn load<T: DeserializeOwned>(kv: &dyn Kv, table: &str, key: &[u8]) -> Option<T> {
    kv.get(table, key).ok().flatten().and_then(|b| dec(&b))
}

pub fn load_all<T: DeserializeOwned>(kv: &dyn Kv, table: &str) -> Vec<(Vec<u8>, T)> {
    kv.scan(table)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(k, v)| dec(&v).map(|d| (k, d)))
        .collect()
}

pub fn put<T: Serialize>(table: &'static str, key: &[u8], v: &T) -> Op {
    Op::Put(table, key.to_vec(), enc(v))
}

// ===== relay.db =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvRec {
    pub raw: Vec<u8>,
    pub received_ms: u64,
    pub mine: bool,
    pub for_me: bool,
    pub offer: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TagKind {
    /// Contact or group member (peer id).
    Pair,
    /// Group (conversation id).
    Group,
    /// Public channel (conversation id).
    Channel,
    /// Reply tag of one of our posts (post message id); handshake target.
    Reply,
    /// Return tag of a remote pairing we started (pairing id).
    Return,
    /// In-person pairing token (pairing id).
    PairingIk,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagKeyRec {
    pub kind: TagKind,
    pub owner: Id16,
    pub key: [u8; 32],
    pub inbound_dir: u8,
    pub outbound_dir: u8,
    /// For handshake tags: the literal tag; zero otherwise.
    pub tag: [u8; 16],
    pub created_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayMe {
    pub presence_key: [u8; 32],
    pub has_identity: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityRec {
    pub name: String,
    pub key: [u8; 32],
    pub joined_ms: u64,
}

// ===== message.db =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerRec {
    pub ed25519: [u8; 32],
    pub curve25519: [u8; 32],
    pub noise_static: [u8; 32],
    pub nickname: String,
    pub trust: Trust,
    /// Listed in Contacts (false = group member only).
    pub listed: bool,
    pub presence_key: [u8; 32],
    pub tag_key: [u8; 32],
    pub remote_otks: Vec<[u8; 32]>,
    pub remote_fallback: [u8; 32],
    /// How many of our one-time keys this peer still holds.
    pub my_otks_held: u32,
    pub conversation: Option<Id16>,
    pub created_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OlmRec {
    pub pickle: String,
    pub last_decrypt_ms: u64,
    pub created_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvRec {
    pub kind: ConversationKind,
    pub peer: Option<Id16>,
    pub title: String,
    pub disappear_secs: u32,
    pub last_activity: u64,
    pub unread: u32,
    pub last_preview: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgRec {
    pub uuid: Id16,
    pub conversation: Id16,
    pub outgoing: bool,
    pub sender_peer: Option<Id16>,
    pub sender_pseudonym: Option<[u8; 32]>,
    pub kind: MessageKind,
    pub body: Vec<u8>,
    pub file_name: Option<String>,
    pub total_size: Option<u64>,
    pub chunks_have: u32,
    pub chunks_total: u32,
    pub state: DeliveryState,
    pub release_at: Option<u64>,
    pub sent_at: u64,
    pub received_at: Option<u64>,
    pub expires_at: Option<u64>,
    pub disappear_secs: u32,
    pub reply_tag: Option<[u8; 16]>,
    pub read: bool,
    /// Outbound envelope IDs not yet acknowledged.
    pub pending_envs: Vec<Id16>,
    pub lora_sent: bool,
}

/// A voice note's length (known once complete), waveform and played flag.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceRec {
    pub duration_ms: u32,
    pub waveform: Vec<u8>,
    pub listened: bool,
}

/// A message that answers an earlier one: what it quotes, and who wrote the
/// original as far as this phone knows (resolved when stored).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplyRec {
    pub to: Id16,
    pub quote: String,
    /// The original is on this phone.
    pub found: bool,
    /// We wrote the original.
    pub author_me: bool,
    pub author_peer: Option<Id16>,
    pub author_pseudonym: Option<[u8; 32]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupRec {
    pub group_id: Id16,
    pub name: String,
    pub tag_key: [u8; 32],
    pub outbound_pickle: String,
    pub outbound_session_id: [u8; 32],
    pub outbound_count: u32,
    pub outbound_created_ms: u64,
    pub admin: [u8; 32],
    pub created_by_me: bool,
    /// Members other than us (peer ids).
    pub members: Vec<Id16>,
    /// Members that hold our current outbound session key.
    pub shared_with: Vec<Id16>,
    pub left: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InMegolmRec {
    pub group_conv: Id16,
    pub sender: Option<Id16>,
    pub pickle: String,
    pub created_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelRec {
    pub name: String,
    pub pseudonym_seed: [u8; 32],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostRec {
    pub channel_conv: Id16,
    pub reply_tag: [u8; 16],
    pub pseudonym_pk: [u8; 32],
    pub mine: bool,
    pub created_ms: u64,
}

pub fn msg_key(conv: &Id16, sent_ms: u64, uuid: &Id16) -> Vec<u8> {
    [conv.as_slice(), &sent_ms.to_be_bytes(), uuid].concat()
}

/// A disguise bridge (VLESS+REALITY share link).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeRec {
    pub link: String,
    /// The verified contact it came from, if any.
    pub from: Option<Id16>,
    pub added_ms: u64,
}

/// Content key for a chunked message (D42).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaKeyRec {
    pub key: [u8; 32],
    pub hash: [u8; 32],
    pub sender: Id16,
    pub group: Option<Id16>,
    pub count: u16,
    pub created_ms: u64,
}

/// A sealed chunk that arrived before its media key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingChunkRec {
    pub from: Option<Id16>,
    pub group: Option<Id16>,
    pub sealed: Vec<u8>,
    pub received_ms: u64,
}
