//! Reusable email building blocks.
//!
//! Transport-agnostic: `Message` describes what to send, `Mailer`
//! describes how. Concrete backends (Gmail REST, future SMTP) live
//! in sibling modules and never leak into `Message` construction —
//! the same `Message` can be handed to any backend.

pub mod gmail;
pub mod mailer;
pub mod message;

pub use gmail::GmailMailer;
pub use mailer::{Mailer, SendOutcome};
pub use message::{Mailbox, Message};
