//! e2ee-core — Operation Ghost shared crypto core.
//!
//! The single, audited crypto implementation shared by every client:
//! Tauri desktop, Flutter mobile and the test harness. Provides identity
//! key management, X3DH handshakes, Double Ratchet sessions (via
//! vodozemac) and the versioned wire protocol.
//!
//! # Security model
//!
//! - All cryptography is delegated to [`vodozemac`] (X3DH key exchange and the
//!   Double Ratchet). No hand-rolled crypto primitives are used.
//! - The relay is zero-knowledge: it only ever forwards serialized
//!   [`Envelope`]s and never sees plaintext or key material.
//!
//! # Modules
//!
//! - [`identity`]: long-term X25519 + Ed25519 identity keys, peer IDs and
//!   key persistence.
//! - [`prekey`]: authenticated [`PreKeyBundle`]s used for the X3DH handshake.
//! - [`profile`]: signed username bindings and profile helpers.
//! - [`session`]: Double Ratchet sessions on top of the X3DH handshake.
//! - [`group`]: Megolm group sessions for end-to-end encrypted group chat.
//! - [`wire`]: versioned, serde-compatible protocol types used by the relay.

pub mod group;
pub mod identity;
pub mod prekey;
pub mod profile;
pub mod session;
pub mod wire;

pub use group::{GroupError, InboundGroup, OutboundGroup};
pub use identity::{HelloError, Identity, IdentityError, SignedHello};
pub use prekey::{PreKeyBundle, PreKeyBundleError};
pub use profile::{
    canonical_bytes, sign_username, validate_username, verify_username_signature,
    RESERVED_USERNAMES, USERNAME_MAX_LENGTH, USERNAME_MIN_LENGTH,
};
pub use session::{ChatSession, InboundSession, SessionError};
pub use wire::{Envelope, EnvelopeContent, Handshake, Message, ReceiptKind, WIRE_VERSION};
