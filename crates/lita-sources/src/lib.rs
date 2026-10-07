//! Where a Voice's material comes from when the person does not want to
//! paste (D-43): their own published writing. Each source turns into plain
//! `Piece`s that the intake screen offers as candidates; nothing here talks
//! to Codex or to Lita's store.
//!
//! All three sources need no login and no API key. note has a public JSON
//! API, Medium publishes RSS with full bodies (latest ten only), and X
//! reads are paid so the official data archive is the route.

pub mod html;
pub mod medium;
pub mod note;
pub mod talk;
pub mod x;

use serde::{Deserialize, Serialize};

/// One candidate piece of writing, already reduced to plain text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Piece {
    pub title: Option<String>,
    pub url: Option<String>,
    /// As the source reported it; format varies by source.
    pub published_at: Option<String>,
    pub text: String,
}

pub(crate) const USER_AGENT: &str = "Lita/0.1 (+https://github.com/KendrickMalar/lita)";

pub(crate) fn http() -> anyhow::Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(std::time::Duration::from_secs(30))
        .build()?)
}
