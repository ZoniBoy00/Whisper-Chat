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
//! - [`session`]: Double Ratchet sessions on top of the X3DH handshake.
//! - [`wire`]: versioned, serde-compatible protocol types used by the relay.

pub mod identity;
pub mod prekey;
pub mod session;
pub mod wire;

pub use identity::{HelloError, Identity, IdentityError, SignedHello};
pub use prekey::{PreKeyBundle, PreKeyBundleError};
pub use session::{ChatSession, InboundSession, SessionError};
pub use wire::{Envelope, EnvelopeContent, Handshake, Message, WIRE_VERSION};
