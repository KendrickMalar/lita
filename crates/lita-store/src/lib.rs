//! Lita's data, read and written through PostgREST as the signed-in user.
//!
//! Every call carries the person's Supabase access token, and the database
//! enforces ownership with row level security (see
//! `supabase/migrations/20260922000000_init.sql`). This crate therefore
//! never filters by user itself: what the server returns is, by
//! construction, only that person's rows.

use anyhow::{Context, Result, bail};
use lita_engine::Effort;
pub use lita_engine::topics::{ArticleText, Policy, TopicCloud};
use lita_engine::voice::VoiceProfile;
use reqwest::blocking::{Client, RequestBuilder};
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderValue};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::json;

/// A row id: a UUID as PostgREST prints it.
pub type Id = String;

/// A publishing Profile owned by the signed-in account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishingProfile {
    pub id: Id,
    pub name: String,
    pub is_initial: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Platform {
    pub id: String,
    pub name: String,
    pub max_chars: Option<i64>,
    pub rules: String,
}

/// A voice as listed. The profile is left out because lists do not need it
/// and it is the bulk of the row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceSummary {
    pub id: Id,
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
    pub source_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Voice {
    pub id: Id,
    pub name: String,
    pub profile: VoiceProfile,
    pub created_at: String,
    pub updated_at: String,
    #[serde(rename = "voice_sources", default)]
    pub sources: Vec<VoiceSource>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Paste,
    File,
    /// A note.com article; `origin` is its URL.
    Note,
    /// A Medium post; `origin` is its URL.
    Medium,
    /// A post from an X archive; `origin` is its URL when the handle was known.
    X,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceSource {
    pub id: Id,
    pub kind: SourceKind,
    /// File name for `File`; `None` for `Paste`.
    pub origin: Option<String>,
    /// Where the piece was imported from: the note account or Medium handle,
    /// `archive` for an X archive, `None` for pasted text and files (#68).
    #[serde(default)]
    pub account: Option<String>,
    pub body: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NewSource {
    pub kind: SourceKind,
    pub origin: Option<String>,
    #[serde(default)]
    pub account: Option<String>,
    pub body: String,
}

/// `Effort` as the database spells it. Kept separate from
/// `lita_engine::Effort` so the CLI mapping ("low"/"medium") and the storage
/// mapping ("fast"/"quality") can drift independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoredEffort {
    Fast,
    Quality,
    Best,
}

impl From<Effort> for StoredEffort {
    fn from(e: Effort) -> Self {
        match e {
            Effort::Fast => StoredEffort::Fast,
            Effort::Quality => StoredEffort::Quality,
            Effort::Best => StoredEffort::Best,
        }
    }
}

impl From<StoredEffort> for Effort {
    fn from(e: StoredEffort) -> Self {
        match e {
            StoredEffort::Fast => Effort::Fast,
            StoredEffort::Quality => Effort::Quality,
            StoredEffort::Best => Effort::Best,
        }
    }
}

/// Where an article stands. `Approved` means the person took it (copied it
/// out); `Archived` means they put it away without using it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArticleStatus {
    Draft,
    Approved,
    Archived,
}

/// Where a queued article stands (article queue, 2026-09-23). Cleared once
/// the text is written, so an ordinary article has none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueState {
    Waiting,
    Writing,
    Failed,
}

/// One piece of writing: its own brief, voice, destination, current text,
/// and (in `article_versions`) a history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Article {
    pub id: Id,
    /// Immutable owner Profile; queue workers must use this instead of the active selection.
    pub profile_id: Id,
    pub voice_id: Option<Id>,
    pub platform_id: String,
    pub title: String,
    pub body: String,
    pub brief: String,
    pub status: ArticleStatus,
    #[serde(default)]
    pub queue: Option<QueueState>,
    #[serde(default)]
    pub queue_started_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// An article as listed: no body, but a short excerpt for the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArticleSummary {
    pub id: Id,
    pub voice_id: Option<Id>,
    pub platform_id: String,
    pub title: String,
    pub excerpt: String,
    pub status: ArticleStatus,
    #[serde(default)]
    pub queue: Option<QueueState>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct NewArticle {
    pub voice_id: Option<Id>,
    pub platform_id: String,
    pub title: String,
    pub body: String,
    pub brief: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queue: Option<QueueState>,
}

/// Fields of an article that can change. `None` leaves a field as it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ArticlePatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice_id: Option<Option<Id>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brief: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<ArticleStatus>,
    /// `Some(None)` clears the queue mark.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queue: Option<Option<QueueState>>,
}

/// What produced a version (see the migration for the meaning of each).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionKind {
    Generated,
    Shortened,
    Edited,
    Restored,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArticleVersion {
    pub id: Id,
    pub article_id: Id,
    pub kind: VersionKind,
    pub title: String,
    pub body: String,
    /// For generated / shortened: exactly what went to Codex (B-08).
    pub prompt_sent: Option<String>,
    pub elapsed_ms: Option<i64>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NewVersion {
    pub article_id: Id,
    pub kind: VersionKind,
    pub title: String,
    pub body: String,
    pub prompt_sent: Option<String>,
    pub elapsed_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct UserSettings {
    pub default_voice_id: Option<Id>,
    #[serde(default)]
    pub policy: Policy,
    /// The words the person keeps writing about; `None` until gathered.
    #[serde(default)]
    pub topic_cloud: Option<TopicCloud>,
}

/// Connection details for one Supabase project. Cheap to clone.
#[derive(Clone)]
pub struct Store {
    rest_url: String,
    anon_key: String,
    http: Client,
}

/// `Store` bound to one person's access token.
pub struct UserStore<'a> {
    store: &'a Store,
    access_token: String,
}

/// An immutable Profile-scoped view of one signed-in account's store.
pub struct ProfileStore<'a> {
    user: UserStore<'a>,
    profile_id: Id,
}

fn profile_insert_body(name: &str) -> Result<serde_json::Value> {
    let name = name.trim();
    if name.is_empty() {
        bail!("give the Profile a name");
    }
    Ok(json!({ "name": name }))
}

fn profiles_query() -> Vec<(&'static str, String)> {
    vec![("select", "id,name,is_initial,created_at".into())]
}

fn profile_query(id: &str) -> Vec<(&'static str, String)> {
    vec![
        ("select", "id,name,is_initial,created_at".into()),
        ("id", format!("eq.{id}")),
        ("limit", "1".into()),
    ]
}

fn selected_profile_query() -> Vec<(&'static str, String)> {
    vec![
        ("select", "active_profile_id".into()),
        ("limit", "1".into()),
    ]
}

fn settings_owner_query() -> Vec<(&'static str, String)> {
    vec![("select", "user_id".into()), ("limit", "1".into())]
}

fn active_profile_update_query(user_id: &str) -> Vec<(&'static str, String)> {
    vec![
        ("user_id", format!("eq.{user_id}")),
        ("select", "user_id".into()),
    ]
}

fn active_profile_update_body(profile_id: &str) -> serde_json::Value {
    json!({ "active_profile_id": profile_id })
}

fn require_visible_profile(
    rows: Vec<PublishingProfile>,
    requested_id: &str,
) -> Result<PublishingProfile> {
    rows.into_iter()
        .find(|profile| profile.id == requested_id)
        .context("publishing Profile is not owned by this account")
}

fn voices_query_for_profile(profile_id: &str) -> Vec<(&'static str, String)> {
    vec![
        (
            "select",
            "id,name,created_at,updated_at,voice_sources!voice_sources_voice_profile_fk(count)"
                .into(),
        ),
        ("profile_id", format!("eq.{profile_id}")),
    ]
}

fn voice_query_for_profile(voice_id: &str, profile_id: &str) -> Vec<(&'static str, String)> {
    vec![
        (
            "select",
            "*,voice_sources!voice_sources_voice_profile_fk(*)".into(),
        ),
        ("id", format!("eq.{voice_id}")),
        ("profile_id", format!("eq.{profile_id}")),
        ("limit", "1".into()),
    ]
}

fn borrowed_query<'a>(query: &'a [(&'static str, String)]) -> Vec<(&'static str, &'a str)> {
    query
        .iter()
        .map(|(key, value)| (*key, value.as_str()))
        .collect()
}

impl Store {
    /// `base_url` is the project URL, e.g. `https://<ref>.supabase.co`.
    pub fn new(base_url: impl Into<String>, anon_key: impl Into<String>) -> Result<Self> {
        Ok(Self {
            rest_url: format!("{}/rest/v1", base_url.into().trim_end_matches('/')),
            anon_key: anon_key.into(),
            http: Client::builder().build()?,
        })
    }

    pub fn as_user(&self, access_token: impl Into<String>) -> UserStore<'_> {
        UserStore {
            store: self,
            access_token: access_token.into(),
        }
    }
}

impl UserStore<'_> {
    // ----- publishing Profiles ----------------------------------------------

    pub fn profiles(&self) -> Result<Vec<PublishingProfile>> {
        let query = profiles_query();
        read_pages(
            |after| self.get_page("publishing_profiles", &borrowed_query(&query), after),
            |profile: &PublishingProfile| profile.id.as_str(),
        )
    }

    /// Creates a named Profile. The database insert trigger creates profile_settings atomically.
    pub fn create_profile(&self, name: &str) -> Result<PublishingProfile> {
        let body = profile_insert_body(name)?;
        self.insert_one("publishing_profiles", &body)
    }

    pub fn rename_profile(&self, id: &str, name: &str) -> Result<()> {
        let body = profile_insert_body(name)?;
        self.patch_one("publishing_profiles", id, &body)
    }

    pub fn selected_profile(&self) -> Result<Option<PublishingProfile>> {
        #[derive(Deserialize)]
        struct ActiveProfileRow {
            active_profile_id: Option<Id>,
        }
        let query = selected_profile_query();
        let rows: Vec<ActiveProfileRow> =
            self.get_many("user_settings", &borrowed_query(&query))?;
        let Some(active_id) = rows
            .into_iter()
            .next()
            .and_then(|row| row.active_profile_id)
        else {
            return Ok(None);
        };
        self.profile(&active_id)?
            .map(Some)
            .context("the selected Profile is not readable by this account")
    }

    /// Persists a selection only after RLS confirms the Profile belongs to this account.
    pub fn select_profile(&self, id: &str) -> Result<()> {
        #[derive(Deserialize)]
        struct UserSettingsOwner {
            user_id: Id,
        }
        if self.profile(id)?.is_none() {
            bail!("publishing Profile is not owned by this account");
        }
        let owner_query = settings_owner_query();
        let owners: Vec<UserSettingsOwner> =
            self.get_many("user_settings", &borrowed_query(&owner_query))?;
        let owner = owners
            .first()
            .context("user settings row is missing for this account")?;
        let query = active_profile_update_query(&owner.user_id);
        let updated: Vec<UserSettingsOwner> = self.parse(
            self.request(
                self.store
                    .http
                    .patch(self.url("user_settings"))
                    .query(&borrowed_query(&query))
                    .header("Prefer", "return=representation")
                    .json(&active_profile_update_body(id)),
            )?,
        )?;
        if updated.is_empty() {
            bail!("could not persist the selected Profile");
        }
        Ok(())
    }

    /// Binds an immutable Profile ID after RLS confirms that the caller owns it.
    pub fn for_profile(&self, id: &str) -> Result<ProfileStore<'_>> {
        let query = profile_query(id);
        let profiles: Vec<PublishingProfile> =
            self.get_many("publishing_profiles", &borrowed_query(&query))?;
        let profile = require_visible_profile(profiles, id)?;
        Ok(ProfileStore {
            user: UserStore {
                store: self.store,
                access_token: self.access_token.clone(),
            },
            profile_id: profile.id,
        })
    }

    fn profile(&self, id: &str) -> Result<Option<PublishingProfile>> {
        let query = profile_query(id);
        let profiles: Vec<PublishingProfile> =
            self.get_many("publishing_profiles", &borrowed_query(&query))?;
        Ok(profiles.into_iter().find(|profile| profile.id == id))
    }

    // ----- platforms -------------------------------------------------------

    pub fn platforms(&self) -> Result<Vec<Platform>> {
        self.get_many("platforms", &[("select", "*"), ("order", "name")])
    }

    /// Account-wide queue discovery. Per-article work must bind to `Article::profile_id`.
    pub fn queued(&self) -> Result<Vec<Article>> {
        let query = vec![
            (
                "select",
                "id,profile_id,voice_id,platform_id,title,body,brief,status,queue,queue_started_at,created_at,updated_at".to_owned(),
            ),
            ("queue", "not.is.null".to_owned()),
        ];
        queued_articles(|after| self.get_page("articles", &borrowed_query(&query), after))
    }

    // ----- plumbing --------------------------------------------------------

    fn url(&self, table: &str) -> String {
        format!("{}/{table}", self.store.rest_url)
    }

    fn get_many<T: DeserializeOwned>(&self, table: &str, query: &[(&str, &str)]) -> Result<Vec<T>> {
        self.parse(self.request(self.store.http.get(self.url(table)).query(query))?)
    }

    /// One page of `get_many` (see `keyset`).
    fn get_page<T: DeserializeOwned>(
        &self,
        table: &str,
        query: &[(&str, &str)],
        after: Option<&str>,
    ) -> Result<Vec<T>> {
        self.parse(
            self.request(
                self.store
                    .http
                    .get(self.url(table))
                    .query(query)
                    .query(&keyset(after)),
            )?,
        )
    }

    fn insert_one<T: DeserializeOwned, B: Serialize>(&self, table: &str, body: &B) -> Result<T> {
        self.parse(
            self.request(
                self.store
                    .http
                    .post(self.url(table))
                    .header("Prefer", "return=representation")
                    .header(
                        ACCEPT,
                        HeaderValue::from_static("application/vnd.pgrst.object+json"),
                    )
                    .json(body),
            )?,
        )
    }

    fn patch_one<B: Serialize>(&self, table: &str, id: &str, body: &B) -> Result<()> {
        let updated: Vec<serde_json::Value> = self.parse(
            self.request(
                self.store
                    .http
                    .patch(self.url(table))
                    .query(&[("id", format!("eq.{id}")), ("select", "id".into())])
                    .header("Prefer", "return=representation")
                    .json(body),
            )?,
        )?;
        if updated.is_empty() {
            bail!("no {table} row with id {id} (or it is not yours)");
        }
        Ok(())
    }

    fn request(&self, builder: RequestBuilder) -> Result<String> {
        let resp = builder
            .header("apikey", &self.store.anon_key)
            .header(AUTHORIZATION, format!("Bearer {}", self.access_token))
            .send()
            .context("reaching Supabase")?;
        let status = resp.status();
        let text = resp.text()?;
        if !status.is_success() {
            #[derive(Deserialize)]
            struct PgError {
                message: Option<String>,
                code: Option<String>,
            }
            let parsed: PgError = serde_json::from_str(&text).unwrap_or(PgError {
                message: None,
                code: None,
            });
            bail!(
                "Supabase returned {status}{}: {}",
                parsed.code.map(|c| format!(" ({c})")).unwrap_or_default(),
                parsed.message.unwrap_or(text)
            );
        }
        Ok(text)
    }

    fn parse<T: DeserializeOwned>(&self, text: String) -> Result<T> {
        let text = if text.is_empty() {
            "[]".to_string()
        } else {
            text
        };
        serde_json::from_str(&text).context("reading what Supabase returned")
    }
}

/// True when Supabase refused the request for lack of privileges
/// (Postgres `42501`, e.g. a role with no GRANT on the table).
/// `request` reports failures as plain `bail!` messages, so this matches the
/// ` (42501): ` part of the message it formats instead of a typed error.
pub fn is_permission_denied(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        let message = cause.to_string();
        message.starts_with("Supabase returned ") && message.contains(" (42501): ")
    })
}

fn profile_voice_mutation_query(voice_id: &str, profile_id: &str) -> Vec<(&'static str, String)> {
    vec![
        ("id", format!("eq.{voice_id}")),
        ("profile_id", format!("eq.{profile_id}")),
        ("select", "id".into()),
    ]
}

fn replacement_rpc_body(
    voice_id: &str,
    profile_id: &str,
    profile: &VoiceProfile,
    sources: &[NewSource],
) -> serde_json::Value {
    json!({
        "p_voice_id": voice_id,
        "p_profile_id": profile_id,
        "p_profile": profile,
        "p_sources": sources.iter().map(|source| json!({
            "kind": source.kind, "origin": source.origin, "account": source.account, "body": source.body,
        })).collect::<Vec<_>>(),
    })
}

fn voice_creation_cleanup_error(
    source_error: anyhow::Error,
    cleanup_error: anyhow::Error,
) -> anyhow::Error {
    anyhow::anyhow!(
        "inserting Voice sources failed: {source_error:#}; scoped Voice cleanup also failed: {cleanup_error:#}"
    )
}

fn voice_read_cleanup_error(
    voice_id: &str,
    read_error: anyhow::Error,
    cleanup_error: anyhow::Error,
) -> anyhow::Error {
    anyhow::anyhow!(
        "reading newly created Voice failed: {read_error:#}; scoped cleanup of Voice {voice_id} also failed: {cleanup_error:#}"
    )
}

fn profile_source_mutation_query(
    voice_id: &str,
    profile_id: &str,
    source_id: Option<&str>,
    kind: Option<SourceKind>,
    account: Option<Option<&str>>,
) -> Vec<(&'static str, String)> {
    let mut query = vec![
        ("voice_id", format!("eq.{voice_id}")),
        ("profile_id", format!("eq.{profile_id}")),
    ];
    if let Some(id) = source_id {
        query.push(("id", format!("eq.{id}")));
    }
    if let Some(kind) = kind {
        let kind = serde_json::to_value(kind).expect("SourceKind serializes");
        query.push((
            "kind",
            format!("eq.{}", kind.as_str().expect("enum serializes as string")),
        ));
    }
    if let Some(account) = account {
        query.push(match account {
            Some(account) => ("account", format!("eq.{account}")),
            None => ("account", "is.null".into()),
        });
    }
    query
}

fn profile_source_rows(
    voice_id: &str,
    profile_id: &str,
    sources: &[NewSource],
) -> Vec<serde_json::Value> {
    sources
        .iter()
        .map(|source| {
            json!({
                "voice_id": voice_id, "profile_id": profile_id, "kind": source.kind,
                "origin": source.origin, "account": source.account, "body": source.body,
            })
        })
        .collect()
}

impl ProfileStore<'_> {
    pub fn voices(&self) -> Result<Vec<VoiceSummary>> {
        #[derive(Deserialize)]
        struct Row {
            id: Id,
            name: String,
            created_at: String,
            updated_at: String,
            voice_sources: Vec<Count>,
        }
        #[derive(Deserialize)]
        struct Count {
            count: i64,
        }
        let query = voices_query_for_profile(&self.profile_id);
        let mut rows: Vec<Row> = read_pages(
            |after| self.user.get_page("voices", &borrowed_query(&query), after),
            |row: &Row| row.id.as_str(),
        )?;
        sort_timestamp_desc(&mut rows, |row| &row.created_at, |row| &row.id);
        Ok(rows
            .into_iter()
            .map(|row| VoiceSummary {
                id: row.id,
                name: row.name,
                created_at: row.created_at,
                updated_at: row.updated_at,
                source_count: row.voice_sources.first().map_or(0, |count| count.count),
            })
            .collect())
    }

    pub fn create_voice(
        &self,
        name: &str,
        profile: &VoiceProfile,
        sources: &[NewSource],
    ) -> Result<Voice> {
        let created: Voice = self.user.insert_one(
            "voices",
            &json!({ "profile_id": self.profile_id, "name": name, "profile": profile }),
        )?;
        if let Err(source_error) = self.insert_sources(&created.id, sources) {
            let query = profile_voice_mutation_query(&created.id, &self.profile_id);
            let cleanup = self.user.request(
                self.user
                    .store
                    .http
                    .delete(self.user.url("voices"))
                    .query(&borrowed_query(&query)),
            );
            if let Err(cleanup_error) = cleanup {
                return Err(voice_creation_cleanup_error(source_error, cleanup_error));
            }
            return Err(source_error);
        }
        let created_id = created.id;
        match self.voice(&created_id) {
            Ok(Some(voice)) => Ok(voice),
            Ok(None) => {
                let read_error = anyhow::anyhow!("the voice just created is not readable");
                Err(self.cleanup_voice_read_error(&created_id, read_error))
            }
            Err(read_error) => Err(self.cleanup_voice_read_error(&created_id, read_error)),
        }
    }

    fn cleanup_voice_read_error(&self, id: &str, read_error: anyhow::Error) -> anyhow::Error {
        match self.delete_voice(id) {
            Ok(_) => read_error,
            Err(cleanup_error) => voice_read_cleanup_error(id, read_error, cleanup_error),
        }
    }

    pub fn rename_voice(&self, id: &str, name: &str) -> Result<()> {
        self.patch_voice(id, &json!({ "name": name }))
    }

    pub fn update_profile(&self, id: &str, profile: &VoiceProfile) -> Result<()> {
        self.patch_voice(id, &json!({ "profile": profile }))
    }

    pub fn replace_voice_profile(
        &self,
        id: &str,
        profile: &VoiceProfile,
        sources: &[NewSource],
    ) -> Result<()> {
        let body = replacement_rpc_body(id, &self.profile_id, profile, sources);
        self.user.request(
            self.user
                .store
                .http
                .post(self.user.url("rpc/replace_profile_voice_sources"))
                .json(&body),
        )?;
        Ok(())
    }

    pub fn add_sources(&self, voice_id: &str, sources: &[NewSource]) -> Result<()> {
        self.insert_sources(voice_id, sources)
    }

    fn patch_voice(&self, id: &str, body: &serde_json::Value) -> Result<()> {
        let query = profile_voice_mutation_query(id, &self.profile_id);
        let updated: Vec<serde_json::Value> = self.user.parse(
            self.user.request(
                self.user
                    .store
                    .http
                    .patch(self.user.url("voices"))
                    .query(&borrowed_query(&query))
                    .header("Prefer", "return=representation")
                    .json(body),
            )?,
        )?;
        if updated.is_empty() {
            bail!("voice is not in this Profile");
        }
        Ok(())
    }

    fn insert_sources(&self, voice_id: &str, sources: &[NewSource]) -> Result<()> {
        if sources.is_empty() {
            return Ok(());
        }
        if self.voice(voice_id)?.is_none() {
            bail!("voice is not in this Profile");
        }
        let rows = profile_source_rows(voice_id, &self.profile_id, sources);
        self.user.request(
            self.user
                .store
                .http
                .post(self.user.url("voice_sources"))
                .header("Prefer", "return=minimal")
                .json(&rows),
        )?;
        Ok(())
    }

    pub fn source_times(&self) -> Result<Vec<String>> {
        #[derive(Deserialize)]
        struct Row {
            id: Id,
            created_at: String,
        }
        let query = vec![
            ("select", "id,created_at".to_owned()),
            ("profile_id", format!("eq.{}", self.profile_id)),
        ];
        let rows: Vec<Row> = read_pages(
            |after| {
                self.user
                    .get_page("voice_sources", &borrowed_query(&query), after)
            },
            |row: &Row| row.id.as_str(),
        )?;
        Ok(rows.into_iter().map(|row| row.created_at).collect())
    }

    pub fn delete_sources(
        &self,
        voice_id: &str,
        kind: SourceKind,
        account: Option<&str>,
    ) -> Result<()> {
        let query = profile_source_mutation_query(
            voice_id,
            &self.profile_id,
            None,
            Some(kind),
            Some(account),
        );
        self.user.request(
            self.user
                .store
                .http
                .delete(self.user.url("voice_sources"))
                .query(&borrowed_query(&query)),
        )?;
        Ok(())
    }

    pub fn delete_source(&self, voice_id: &str, source_id: &str) -> Result<()> {
        let query =
            profile_source_mutation_query(voice_id, &self.profile_id, Some(source_id), None, None);
        self.user.request(
            self.user
                .store
                .http
                .delete(self.user.url("voice_sources"))
                .query(&borrowed_query(&query)),
        )?;
        Ok(())
    }

    pub fn delete_voice(&self, id: &str) -> Result<bool> {
        let query = profile_voice_mutation_query(id, &self.profile_id);
        let deleted: Vec<serde_json::Value> = self.user.parse(
            self.user.request(
                self.user
                    .store
                    .http
                    .delete(self.user.url("voices"))
                    .query(&borrowed_query(&query))
                    .header("Prefer", "return=representation"),
            )?,
        )?;
        Ok(!deleted.is_empty())
    }

    pub fn voice(&self, id: &str) -> Result<Option<Voice>> {
        let query = voice_query_for_profile(id, &self.profile_id);
        let rows: Vec<Voice> = self.user.get_many("voices", &borrowed_query(&query))?;
        Ok(rows.into_iter().next())
    }
    // ----- articles --------------------------------------------------------
    pub fn articles(&self, status: Option<ArticleStatus>) -> Result<Vec<ArticleSummary>> {
        let mut query = vec![
            (
                "select",
                "id,voice_id,platform_id,title,body,status,queue,created_at,updated_at".to_owned(),
            ),
            ("profile_id", format!("eq.{}", self.profile_id)),
        ];
        if let Some(status) = status {
            let value = serde_json::to_value(status)?;
            query.push((
                "status",
                format!("eq.{}", value.as_str().context("status")?),
            ));
        }
        #[derive(Deserialize)]
        struct Row {
            id: Id,
            voice_id: Option<Id>,
            platform_id: String,
            title: String,
            body: String,
            status: ArticleStatus,
            queue: Option<QueueState>,
            created_at: String,
            updated_at: String,
        }
        let mut rows: Vec<Row> = read_pages(
            |after| {
                self.user
                    .get_page("articles", &borrowed_query(&query), after)
            },
            |row: &Row| row.id.as_str(),
        )?;
        rows.sort_by(|a, b| {
            cmp_timestamp(&b.updated_at, &a.updated_at).then_with(|| a.id.cmp(&b.id))
        });
        Ok(rows
            .into_iter()
            .map(|row| ArticleSummary {
                id: row.id,
                voice_id: row.voice_id,
                platform_id: row.platform_id,
                title: row.title,
                excerpt: excerpt_of(&row.body),
                status: row.status,
                queue: row.queue,
                created_at: row.created_at,
                updated_at: row.updated_at,
            })
            .collect())
    }

    pub fn queued(&self) -> Result<Vec<Article>> {
        let mut rows = self.get_articles(&[("queue", "not.is.null".into())])?;
        sort_queue_articles(&mut rows);
        Ok(rows)
    }

    pub fn empty_drafts(&self) -> Result<Vec<Article>> {
        let mut rows = self.get_articles(&[
            ("status", "eq.draft".into()),
            ("title", "eq.".into()),
            ("body", "eq.".into()),
            ("brief", "eq.".into()),
            ("queue", "is.null".into()),
        ])?;
        rows.sort_by(|a, b| {
            cmp_timestamp(&b.updated_at, &a.updated_at).then_with(|| a.id.cmp(&b.id))
        });
        Ok(rows)
    }

    fn get_articles(&self, filters: &[(&'static str, String)]) -> Result<Vec<Article>> {
        let mut query = vec![("profile_id", format!("eq.{}", self.profile_id))];
        query.extend(filters.iter().map(|(key, value)| (*key, value.clone())));
        let rows: Vec<Article> = read_pages(
            |after| {
                self.user
                    .get_page("articles", &borrowed_query(&query), after)
            },
            |article: &Article| article.id.as_str(),
        )?;
        Ok(rows)
    }

    pub fn article(&self, id: &str) -> Result<Option<Article>> {
        let query = vec![
            ("id", format!("eq.{id}")),
            ("profile_id", format!("eq.{}", self.profile_id)),
            ("limit", "1".into()),
        ];
        Ok(self
            .user
            .get_many("articles", &borrowed_query(&query))?
            .into_iter()
            .next())
    }

    pub fn create_article(&self, new: &NewArticle) -> Result<Article> {
        let mut body = serde_json::to_value(new)?;
        body["profile_id"] = json!(self.profile_id);
        self.user.insert_one("articles", &body)
    }

    pub fn update_article(&self, id: &str, patch: &ArticlePatch) -> Result<()> {
        let query = vec![
            ("id", format!("eq.{id}")),
            ("profile_id", format!("eq.{}", self.profile_id)),
            ("select", "id".into()),
        ];
        let updated: Vec<serde_json::Value> = self.user.parse(
            self.user.request(
                self.user
                    .store
                    .http
                    .patch(self.user.url("articles"))
                    .query(&borrowed_query(&query))
                    .header("Prefer", "return=representation")
                    .json(patch),
            )?,
        )?;
        if updated.is_empty() {
            bail!("article is not in this Profile");
        }
        Ok(())
    }

    /// Atomically completes a queued Article only while its queue state is still Writing.
    pub fn complete_queued_article(&self, id: &str, body: &str, title: &str) -> Result<bool> {
        self.patch_queued_article_if_status(
            id,
            QueueState::Writing,
            &queue_completion_body(body, title),
        )
    }

    /// Clears a queue marker only if the Article still has the expected queue state.
    pub fn stop_queued_article(&self, id: &str, expected_status: QueueState) -> Result<bool> {
        self.patch_queued_article_if_status(id, expected_status, &json!({ "queue": null }))
    }

    /// Marks an Article failed only while its queue state is still Writing.
    pub fn fail_queued_article_if_writing(&self, id: &str) -> Result<bool> {
        self.patch_queued_article_if_status(id, QueueState::Writing, &json!({ "queue": "failed" }))
    }

    /// Marks an Article failed only while its queue state is still Waiting.
    pub fn fail_queued_article_if_waiting(&self, id: &str) -> Result<bool> {
        self.patch_queued_article_if_status(id, QueueState::Waiting, &json!({ "queue": "failed" }))
    }

    fn patch_queued_article_if_status(
        &self,
        id: &str,
        expected_status: QueueState,
        body: &serde_json::Value,
    ) -> Result<bool> {
        let query = profile_queued_article_query(id, &self.profile_id, expected_status);
        let updated: Vec<serde_json::Value> = self.user.parse(
            self.user.request(
                self.user
                    .store
                    .http
                    .patch(self.user.url("articles"))
                    .query(&borrowed_query(&query))
                    .header("Prefer", "return=representation")
                    .json(body),
            )?,
        )?;
        Ok(!updated.is_empty())
    }

    pub fn delete_article(&self, id: &str) -> Result<bool> {
        let query = vec![
            ("id", format!("eq.{id}")),
            ("profile_id", format!("eq.{}", self.profile_id)),
            ("select", "id".into()),
        ];
        let deleted: Vec<serde_json::Value> = self.user.parse(
            self.user.request(
                self.user
                    .store
                    .http
                    .delete(self.user.url("articles"))
                    .query(&borrowed_query(&query))
                    .header("Prefer", "return=representation"),
            )?,
        )?;
        Ok(!deleted.is_empty())
    }

    pub fn versions(&self, article_id: &str) -> Result<Vec<ArticleVersion>> {
        let query = profile_versions_query(&self.profile_id, article_id);
        let mut versions: Vec<ArticleVersion> = read_pages(
            |after| {
                self.user
                    .get_page("article_versions", &borrowed_query(&query), after)
            },
            |version: &ArticleVersion| version.id.as_str(),
        )?;
        versions.sort_by(|a, b| {
            cmp_timestamp(&b.created_at, &a.created_at).then_with(|| a.id.cmp(&b.id))
        });
        Ok(versions)
    }

    /// Returns a generated or shortened Version newer than the Article state.
    pub fn generated_version_after(
        &self,
        article_id: &str,
        updated_at: &str,
    ) -> Result<Option<ArticleVersion>> {
        Ok(first_generated_version_after(
            self.versions(article_id)?,
            updated_at,
        ))
    }

    pub fn version(&self, id: &str) -> Result<Option<ArticleVersion>> {
        Ok(self
            .scoped_versions(&[("id", format!("eq.{id}")), ("limit", "1".into())])?
            .into_iter()
            .next())
    }

    fn scoped_versions(&self, filters: &[(&'static str, String)]) -> Result<Vec<ArticleVersion>> {
        let mut query = vec![("profile_id", format!("eq.{}", self.profile_id))];
        query.extend(filters.iter().map(|(key, value)| (*key, value.clone())));
        self.user
            .get_many("article_versions", &borrowed_query(&query))
    }

    pub fn create_version(&self, new: &NewVersion) -> Result<ArticleVersion> {
        if self.article(&new.article_id)?.is_none() {
            bail!("article is not in this Profile");
        }
        let body = json!({
            "article_id": new.article_id, "profile_id": self.profile_id, "kind": new.kind,
            "title": new.title, "body": new.body, "prompt_sent": new.prompt_sent, "elapsed_ms": new.elapsed_ms
        });
        self.user.insert_one("article_versions", &body)
    }

    pub fn delete_version(&self, id: &str) -> Result<bool> {
        let query = profile_version_delete_query(id, &self.profile_id);
        let deleted: Vec<serde_json::Value> = self.user.parse(
            self.user.request(
                self.user
                    .store
                    .http
                    .delete(self.user.url("article_versions"))
                    .query(&borrowed_query(&query))
                    .header("Prefer", "return=representation"),
            )?,
        )?;
        Ok(!deleted.is_empty())
    }

    pub fn article_texts(&self) -> Result<Vec<ArticleText>> {
        let query = vec![
            (
                "select",
                "id,body,updated_at,article_versions!article_versions_article_profile_fk(body)"
                    .to_owned(),
            ),
            ("profile_id", format!("eq.{}", self.profile_id)),
            (
                "article_versions.kind",
                "in.(generated,shortened)".to_owned(),
            ),
        ];
        let rows: Vec<ArticleTextRow> = read_pages(
            |after| {
                self.user
                    .get_page("articles", &borrowed_query(&query), after)
            },
            |row: &ArticleTextRow| row.id.as_str(),
        )?;
        Ok(article_texts_from(rows))
    }

    pub fn settings(&self) -> Result<UserSettings> {
        let query = vec![
            ("select", "default_voice_id,policy,topic_cloud".to_owned()),
            ("profile_id", format!("eq.{}", self.profile_id)),
            ("limit", "1".to_owned()),
        ];
        Ok(self
            .user
            .get_many("profile_settings", &borrowed_query(&query))?
            .into_iter()
            .next()
            .unwrap_or_default())
    }

    pub fn set_topic_cloud(&self, cloud: &TopicCloud) -> Result<()> {
        self.upsert_settings(&json!({"topic_cloud": cloud}))
    }
    pub fn set_policy(&self, policy: &Policy) -> Result<()> {
        self.upsert_settings(&json!({"policy": policy}))
    }
    pub fn set_default_voice(&self, voice_id: Option<&str>) -> Result<()> {
        if let Some(id) = voice_id
            && self.voice(id)?.is_none()
        {
            bail!("voice is not in this Profile");
        }
        self.upsert_settings(&json!({"default_voice_id": voice_id}))
    }
    fn upsert_settings(&self, values: &serde_json::Value) -> Result<()> {
        let mut body = values.clone();
        body["profile_id"] = json!(self.profile_id);
        self.user.request(
            self.user
                .store
                .http
                .post(self.user.url("profile_settings"))
                .header("Prefer", "resolution=merge-duplicates,return=minimal")
                .json(&body),
        )?;
        Ok(())
    }
}

fn profile_versions_query(profile_id: &str, article_id: &str) -> Vec<(&'static str, String)> {
    vec![
        ("profile_id", format!("eq.{profile_id}")),
        ("article_id", format!("eq.{article_id}")),
    ]
}

fn profile_version_delete_query(version_id: &str, profile_id: &str) -> Vec<(&'static str, String)> {
    vec![
        ("id", format!("eq.{version_id}")),
        ("profile_id", format!("eq.{profile_id}")),
        ("select", "id".into()),
    ]
}
fn queue_state_name(state: QueueState) -> &'static str {
    match state {
        QueueState::Waiting => "waiting",
        QueueState::Writing => "writing",
        QueueState::Failed => "failed",
    }
}

fn profile_queued_article_query(
    article_id: &str,
    profile_id: &str,
    expected_status: QueueState,
) -> Vec<(&'static str, String)> {
    vec![
        ("id", format!("eq.{article_id}")),
        ("profile_id", format!("eq.{profile_id}")),
        ("queue", format!("eq.{}", queue_state_name(expected_status))),
        ("select", "id".into()),
    ]
}

fn queue_completion_body(body: &str, title: &str) -> serde_json::Value {
    json!({ "body": body, "title": title, "queue": null })
}

fn queued_articles(page: impl FnMut(Option<&str>) -> Result<Vec<Article>>) -> Result<Vec<Article>> {
    let mut articles = read_pages(page, |article: &Article| article.id.as_str())?;
    sort_queue_articles(&mut articles);
    Ok(articles)
}

fn sort_queue_articles(articles: &mut [Article]) {
    articles
        .sort_by(|a, b| cmp_timestamp(&a.created_at, &b.created_at).then_with(|| a.id.cmp(&b.id)));
}

fn cmp_timestamp(left: &str, right: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let Some(left_seconds) = lita_engine::topics::epoch_seconds(left) else {
        return left.cmp(right);
    };
    let Some(right_seconds) = lita_engine::topics::epoch_seconds(right) else {
        return left.cmp(right);
    };
    match left_seconds.cmp(&right_seconds) {
        Ordering::Equal => {}
        order => return order,
    }
    fn fraction(timestamp: &str) -> &str {
        timestamp
            .get(19..)
            .unwrap_or("")
            .strip_prefix('.')
            .map_or("", |fraction| {
                fraction
                    .split(|c: char| !c.is_ascii_digit())
                    .next()
                    .unwrap_or("")
            })
    }
    let left_fraction = fraction(left).as_bytes();
    let right_fraction = fraction(right).as_bytes();
    for index in 0..left_fraction.len().max(right_fraction.len()) {
        let left_digit = left_fraction.get(index).copied().unwrap_or(b'0');
        let right_digit = right_fraction.get(index).copied().unwrap_or(b'0');
        match left_digit.cmp(&right_digit) {
            Ordering::Equal => {}
            order => return order,
        }
    }
    Ordering::Equal
}
fn first_generated_version_after(
    versions: impl IntoIterator<Item = ArticleVersion>,
    timestamp: &str,
) -> Option<ArticleVersion> {
    versions.into_iter().find(|version| {
        matches!(
            version.kind,
            VersionKind::Generated | VersionKind::Shortened,
        ) && cmp_timestamp(&version.created_at, timestamp) == std::cmp::Ordering::Greater
    })
}
fn sort_timestamp_desc<T, F, G>(rows: &mut [T], timestamp: F, id: G)
where
    F: for<'a> Fn(&'a T) -> &'a str,
    G: for<'a> Fn(&'a T) -> &'a str,
{
    rows.sort_by(|a, b| cmp_timestamp(timestamp(b), timestamp(a)).then_with(|| id(a).cmp(id(b))));
}

/// Rows asked for per read when every row is needed. At or below
/// PostgREST's `max_rows` (1000 in `supabase/config.toml`); a lower cap on
/// the server only means more reads, never fewer rows.
const PAGE: usize = 200;

/// The query for one page of `PAGE` rows, ordered by id, after the id `after`.
fn keyset(after: Option<&str>) -> Vec<(&'static str, String)> {
    let mut q: Vec<(&str, String)> = vec![("order", "id".into())];
    q.push(("limit", PAGE.to_string()));
    if let Some(after) = after {
        q.push(("id", format!("gt.{after}")));
    }
    q
}

/// Every row, read a page at a time after the last id seen, until a page
/// comes back empty. Stopping at a short page would trust the server's cap
/// to be at least `PAGE`; an empty page does not.
fn read_pages<T>(
    mut page: impl FnMut(Option<&str>) -> Result<Vec<T>>,
    id: impl Fn(&T) -> &str,
) -> Result<Vec<T>> {
    let mut all: Vec<T> = Vec::new();
    loop {
        let rows = page(all.last().map(&id))?;
        if rows.is_empty() {
            return Ok(all);
        }
        all.extend(rows);
    }
}

/// An article as `article_texts` reads it.
#[derive(Deserialize)]
struct ArticleTextRow {
    id: Id,
    body: String,
    updated_at: String,
    /// Missing when the versions did not come with the row.
    #[serde(default)]
    article_versions: Option<Vec<LitaText>>,
}

#[derive(Deserialize)]
struct LitaText {
    body: String,
}

/// Newest article first; an article whose versions are missing is marked
/// unread (`None`), so it is not taken as the person's.
fn article_texts_from(mut rows: Vec<ArticleTextRow>) -> Vec<ArticleText> {
    rows.sort_by(|a, b| cmp_timestamp(&b.updated_at, &a.updated_at).then_with(|| a.id.cmp(&b.id)));
    rows.into_iter()
        .map(|r| ArticleText {
            body: r.body,
            lita_wrote: r
                .article_versions
                .map(|v| v.into_iter().map(|l| l.body).collect()),
        })
        .collect()
}

/// The first line or so of a body, for lists.
fn excerpt_of(body: &str) -> String {
    let first = body
        .trim()
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    let mut out: String = first.chars().take(80).collect();
    if first.chars().count() > 80 {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_denied_is_recognised_only_for_42501() {
        let denied = anyhow::anyhow!(
            "Supabase returned 401 Unauthorized (42501): permission denied for table publishing_profiles"
        );
        assert!(is_permission_denied(&denied));
        assert!(is_permission_denied(&denied.context("listing Profiles")));
        assert!(!is_permission_denied(&anyhow::anyhow!(
            "Supabase returned 401 Unauthorized (PGRST301): JWT expired"
        )));
        assert!(!is_permission_denied(&anyhow::anyhow!(
            "Supabase returned 500 Internal Server Error: boom"
        )));
        assert!(!is_permission_denied(&anyhow::anyhow!("reaching Supabase")));
    }

    #[test]
    fn excerpt_is_the_first_non_empty_line_cut_short() {
        assert_eq!(excerpt_of("\n\n  短い一行  \n二行目"), "短い一行");
        let long = "あ".repeat(100);
        let e = excerpt_of(&long);
        assert_eq!(e.chars().count(), 81);
        assert!(e.ends_with('…'));
    }

    #[test]
    fn article_patch_serialises_only_what_is_set() {
        let p = ArticlePatch {
            body: Some("x".into()),
            voice_id: Some(None),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_string(&p).unwrap(),
            r#"{"voice_id":null,"body":"x"}"#
        );
        assert_eq!(
            serde_json::to_string(&ArticlePatch::default()).unwrap(),
            "{}"
        );
    }

    #[test]
    fn effort_maps_both_ways() {
        assert_eq!(StoredEffort::from(Effort::Fast), StoredEffort::Fast);
        assert_eq!(Effort::from(StoredEffort::Quality), Effort::Quality);
        assert_eq!(
            serde_json::to_string(&StoredEffort::Fast).unwrap(),
            "\"fast\""
        );
        assert_eq!(
            serde_json::to_string(&ArticleStatus::Archived).unwrap(),
            "\"archived\""
        );
        assert_eq!(
            serde_json::to_string(&VersionKind::Shortened).unwrap(),
            "\"shortened\""
        );
        assert_eq!(
            serde_json::to_string(&SourceKind::Paste).unwrap(),
            "\"paste\""
        );
    }

    #[test]
    fn voice_row_with_embedded_sources_parses() {
        let text = r#"{"id":"u1","user_id":"x","name":"me","profile":{"language":"ja","first_person":"僕","formality":"casual","tone":[],"sentence_endings":[],"avg_sentence_length_chars":20,"preferred_words":[],"avoided_words":[],"opens_with":"","closes_with":"","uses_emoji":false,"representative_excerpts":[]},"created_at":"t","updated_at":"t","voice_sources":[{"id":"s1","user_id":"x","voice_id":"u1","kind":"file","origin":"a.txt","body":"b","created_at":"t"}]}"#;
        let v: Voice = serde_json::from_str(text).unwrap();
        assert_eq!(v.sources.len(), 1);
        assert_eq!(v.sources[0].kind, SourceKind::File);
        assert_eq!(v.profile.first_person, "僕");
    }

    #[test]
    fn read_pages_reads_every_row_even_under_a_cap_smaller_than_a_page() {
        // 1234 rows behind a server that returns at most 150 per read.
        let ids: Vec<String> = (0..1234).map(|i| format!("{i:05}")).collect();
        let mut reads = 0;
        let all = read_pages(
            |after| {
                reads += 1;
                Ok(ids
                    .iter()
                    .filter(|i| after.is_none_or(|a| i.as_str() > a))
                    .take(150)
                    .cloned()
                    .collect())
            },
            |r: &String| r.as_str(),
        )
        .unwrap();
        assert_eq!(all, ids);
        assert_eq!(reads, 1234usize.div_ceil(150) + 1);
    }

    #[test]
    fn keyset_orders_by_id_and_starts_after_the_last_one() {
        let q = keyset(Some("u9"));
        assert!(
            q.contains(&("order", "id".into()))
                && q.contains(&("limit", PAGE.to_string()))
                && q.contains(&("id", "gt.u9".into()))
        );
        assert!(!keyset(None).iter().any(|(k, _)| *k == "id"));
    }

    #[test]
    fn article_texts_come_newest_first_and_an_article_without_its_versions_is_not_the_persons() {
        let rows: Vec<ArticleTextRow> = serde_json::from_str(
            r#"[
              {"id":"a","body":"Lita の本文","updated_at":"2026-09-20T00:00:00+00:00","article_versions":[{"body":"Lita の本文"}]},
              {"id":"b","body":"直した本文","updated_at":"2026-09-21T00:00:00.5+00:00","article_versions":[{"body":"Lita の本文"}]},
              {"id":"c","body":"版の読めなかった本文","updated_at":"2026-09-19T00:00:00+00:00"},
              {"id":"d","body":"自分で書いた本文","updated_at":"2026-09-21 08:59:59+09:00","article_versions":[]},
              {"id":"f","body":"offset fractional earlier","updated_at":"2026-09-21T09:00:00.250000+09:00","article_versions":[]},
              {"id":"e","body":"offset fractional later","updated_at":"2026-09-21T00:00:00.750000+00:00","article_versions":[]}
            ]"#,
        )
        .unwrap();
        let texts = article_texts_from(rows);
        let bodies: Vec<&str> = texts.iter().map(|t| t.body.as_str()).collect();
        assert_eq!(
            bodies,
            [
                "offset fractional later",
                "直した本文",
                "offset fractional earlier",
                "自分で書いた本文",
                "Lita の本文",
                "版の読めなかった本文"
            ]
        );
        let edited: Vec<bool> = texts.iter().map(ArticleText::edited_by_person).collect();
        assert_eq!(edited, [true, true, true, true, false, false]);
    }

    #[test]
    fn publishing_profile_response_round_trips() {
        let profile: PublishingProfile = serde_json::from_str(
            r#"{"id":"p1","name":"Company","is_initial":true,"created_at":"now"}"#,
        )
        .unwrap();
        assert_eq!(profile.id, "p1");
        assert_eq!(profile.name, "Company");
        assert!(profile.is_initial);
        assert_eq!(serde_json::to_value(profile).unwrap()["id"], "p1");
    }

    #[test]
    fn profile_creation_trims_and_rejects_blank_names() {
        assert_eq!(
            profile_insert_body("  Company  ").unwrap(),
            json!({"name":"Company"})
        );
        assert!(profile_insert_body(" \t ").is_err());
    }

    #[test]
    fn profile_selection_serializes_only_the_active_profile_id() {
        assert_eq!(
            active_profile_update_body("p1"),
            json!({"active_profile_id":"p1"})
        );
    }

    #[test]
    fn profile_and_selection_queries_have_owner_scopes_and_expected_columns() {
        assert_eq!(
            profiles_query(),
            vec![("select", "id,name,is_initial,created_at".into())]
        );
        assert_eq!(
            profile_query("p1"),
            vec![
                ("select", "id,name,is_initial,created_at".into()),
                ("id", "eq.p1".into()),
                ("limit", "1".into())
            ]
        );
        assert_eq!(
            selected_profile_query(),
            vec![
                ("select", "active_profile_id".into()),
                ("limit", "1".into())
            ]
        );
        assert_eq!(
            settings_owner_query(),
            vec![("select", "user_id".into()), ("limit", "1".into())]
        );
        assert_eq!(
            active_profile_update_query("user-1"),
            vec![
                ("user_id", "eq.user-1".into()),
                ("select", "user_id".into())
            ]
        );
    }

    #[test]
    fn profile_voice_queries_use_the_composite_source_relationship_and_profile_filter() {
        let mut query = voices_query_for_profile("p1");
        assert_eq!(
            query,
            vec![
                (
                    "select",
                    "id,name,created_at,updated_at,voice_sources!voice_sources_voice_profile_fk(count)"
                        .into(),
                ),
                ("profile_id", "eq.p1".into())
            ]
        );
        query.extend(keyset(Some("v10")));
        assert_eq!(
            query,
            vec![
                (
                    "select",
                    "id,name,created_at,updated_at,voice_sources!voice_sources_voice_profile_fk(count)"
                        .into(),
                ),
                ("profile_id", "eq.p1".into()),
                ("order", "id".into()),
                ("limit", PAGE.to_string()),
                ("id", "gt.v10".into())
            ]
        );
        assert_eq!(
            voice_query_for_profile("v1", "p1"),
            vec![
                (
                    "select",
                    "*,voice_sources!voice_sources_voice_profile_fk(*)".into()
                ),
                ("id", "eq.v1".into()),
                ("profile_id", "eq.p1".into()),
                ("limit", "1".into())
            ]
        );
    }

    #[test]
    fn profile_rows_are_paged_past_one_thousand() {
        let expected: Vec<PublishingProfile> = (0..1205)
            .map(|index| PublishingProfile {
                id: format!("p{index:04}"),
                name: format!("Profile {index}"),
                is_initial: index == 0,
                created_at: format!("t{index:04}"),
            })
            .collect();
        let mut requests = 0;
        let actual = read_pages(
            |after| {
                requests += 1;
                Ok(expected
                    .iter()
                    .filter(|profile| after.is_none_or(|last| profile.id.as_str() > last))
                    .take(137)
                    .cloned()
                    .collect())
            },
            |profile: &PublishingProfile| profile.id.as_str(),
        )
        .unwrap();
        assert_eq!(actual, expected);
        assert_eq!(requests, 1205usize.div_ceil(137) + 1);
    }

    #[test]
    fn profile_voice_rows_are_paged_past_one_thousand() {
        let expected: Vec<VoiceSummary> = (0..1205)
            .map(|index| VoiceSummary {
                id: format!("v{index:04}"),
                name: format!("Voice {index}"),
                created_at: format!("t{index:04}"),
                updated_at: format!("t{index:04}"),
                source_count: 1,
            })
            .collect();
        let mut requests = 0;
        let actual = read_pages(
            |after| {
                requests += 1;
                Ok(expected
                    .iter()
                    .filter(|voice| after.is_none_or(|last| voice.id.as_str() > last))
                    .take(137)
                    .cloned()
                    .collect())
            },
            |voice: &VoiceSummary| voice.id.as_str(),
        )
        .unwrap();
        assert_eq!(actual, expected);
        assert_eq!(requests, 1205usize.div_ceil(137) + 1);
    }

    #[test]
    fn profile_store_binding_rejects_a_different_profile_response() {
        let unexpected = PublishingProfile {
            id: "another-user-profile".into(),
            name: "Other".into(),
            is_initial: false,
            created_at: "now".into(),
        };
        assert!(require_visible_profile(vec![unexpected], "requested-profile").is_err());
    }
    #[test]
    fn profile_voice_writes_and_deletes_always_include_profile_scope() {
        assert_eq!(
            profile_voice_mutation_query("v1", "p2"),
            vec![
                ("id", "eq.v1".into()),
                ("profile_id", "eq.p2".into()),
                ("select", "id".into())
            ]
        );
        assert_eq!(
            profile_source_mutation_query("v1", "p2", Some("s1"), None, None),
            vec![
                ("voice_id", "eq.v1".into()),
                ("profile_id", "eq.p2".into()),
                ("id", "eq.s1".into())
            ]
        );
        assert_eq!(
            profile_source_mutation_query("v1", "p2", None, Some(SourceKind::Paste), Some(None)),
            vec![
                ("voice_id", "eq.v1".into()),
                ("profile_id", "eq.p2".into()),
                ("kind", "eq.paste".into()),
                ("account", "is.null".into())
            ]
        );
    }

    #[test]
    fn profile_source_insert_body_includes_the_scoped_parent_profile() {
        let source = NewSource {
            kind: SourceKind::Paste,
            origin: None,
            account: None,
            body: "text".into(),
        };
        assert_eq!(
            profile_source_rows("v1", "p2", &[source]),
            vec![
                json!({"voice_id":"v1", "profile_id":"p2", "kind":"paste", "origin":null, "account":null, "body":"text"})
            ]
        );
    }
    #[test]
    fn voice_replacement_rpc_payload_and_cleanup_error_preserve_both_causes() {
        let profile = VoiceProfile::default();
        let source = NewSource {
            kind: SourceKind::Paste,
            origin: None,
            account: None,
            body: "replacement".into(),
        };
        let payload = replacement_rpc_body("voice-1", "profile-2", &profile, &[source]);
        assert_eq!(payload["p_voice_id"], "voice-1");
        assert_eq!(payload["p_profile_id"], "profile-2");
        assert_eq!(payload["p_profile"]["language"], "");
        assert_eq!(payload["p_sources"][0]["kind"], "paste");
        assert_eq!(payload["p_sources"][0]["body"], "replacement");
        let error = voice_creation_cleanup_error(
            anyhow::anyhow!("insert failed"),
            anyhow::anyhow!("cleanup failed"),
        );
        let message = format!("{error:#}");
        assert!(message.contains("insert failed"));
        assert!(message.contains("cleanup failed"));
    }

    #[test]
    fn voice_read_cleanup_error_preserves_read_failure_cleanup_failure_and_id() {
        let error = voice_read_cleanup_error(
            "voice-7",
            anyhow::anyhow!("final Voice read failed"),
            anyhow::anyhow!("delete timed out"),
        );
        let message = format!("{error:#}");
        assert!(message.contains("voice-7"));
        assert!(message.contains("final Voice read failed"));
        assert!(message.contains("delete timed out"));
    }

    #[test]
    fn timestamp_comparison_preserves_fractional_precision() {
        assert_eq!(
            cmp_timestamp(
                "2026-01-01T12:00:00.250000+00:00",
                "2026-01-01T12:00:00.500000+00:00"
            ),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            cmp_timestamp(
                "2026-01-01T12:00:00.25+00:00",
                "2026-01-01T12:00:00.250000+00:00"
            ),
            std::cmp::Ordering::Equal
        );
        assert_eq!(
            cmp_timestamp(
                "2026-01-01T12:00:00.250000+09:00",
                "2026-01-01T03:00:00.500000+00:00"
            ),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn generated_version_after_uses_full_timestamp_and_generation_kind() {
        let version = |id: &str, kind: VersionKind, created_at: &str| ArticleVersion {
            id: id.into(),
            article_id: "article-1".into(),
            kind,
            title: String::new(),
            body: String::new(),
            prompt_sent: None,
            elapsed_ms: None,
            created_at: created_at.into(),
        };
        let found = first_generated_version_after(
            vec![
                version(
                    "equal",
                    VersionKind::Generated,
                    "2026-01-01T12:00:00.25+00:00",
                ),
                version(
                    "older",
                    VersionKind::Generated,
                    "2026-01-01T12:00:00.249999+00:00",
                ),
                version(
                    "manual",
                    VersionKind::Manual,
                    "2026-01-01T12:00:00.900000+00:00",
                ),
                version(
                    "newer",
                    VersionKind::Shortened,
                    "2026-01-01T12:00:00.250001+00:00",
                ),
            ],
            "2026-01-01T12:00:00.250000+00:00",
        );
        assert_eq!(found.map(|version| version.id), Some("newer".into()));
    }

    #[test]
    fn queue_completion_patch_is_profile_scoped_and_sets_body_title_and_null_queue() {
        assert_eq!(
            profile_queued_article_query("article-1", "profile-2", QueueState::Writing),
            vec![
                ("id", "eq.article-1".into()),
                ("profile_id", "eq.profile-2".into()),
                ("queue", "eq.writing".into()),
                ("select", "id".into())
            ]
        );
        assert_eq!(
            queue_completion_body("generated body", "generated title"),
            json!({
                "body": "generated body",
                "title": "generated title",
                "queue": null
            })
        );
    }

    #[test]
    fn queue_failure_update_is_profile_scoped_and_waiting_conditional() {
        assert_eq!(
            profile_queued_article_query("article-1", "profile-2", QueueState::Waiting),
            vec![
                ("id", "eq.article-1".into()),
                ("profile_id", "eq.profile-2".into()),
                ("queue", "eq.waiting".into()),
                ("select", "id".into())
            ]
        );
    }

    #[test]
    fn profile_version_deletion_query_scopes_id_and_owner() {
        assert_eq!(
            profile_version_delete_query("version-1", "profile-2"),
            vec![
                ("id", "eq.version-1".into()),
                ("profile_id", "eq.profile-2".into()),
                ("select", "id".into())
            ]
        );
    }

    #[test]
    fn profile_version_query_is_scoped_and_versions_page_past_one_thousand() {
        assert_eq!(
            profile_versions_query("p1", "a1"),
            vec![
                ("profile_id", "eq.p1".into()),
                ("article_id", "eq.a1".into())
            ]
        );
        let expected: Vec<ArticleVersion> = (0..1205)
            .map(|index| ArticleVersion {
                id: format!("v{index:04}"),
                article_id: "a1".into(),
                kind: VersionKind::Generated,
                title: String::new(),
                body: String::new(),
                prompt_sent: None,
                elapsed_ms: None,
                created_at: format!(
                    "2026-01-01T12:00:{:02}.{:06}+00:00",
                    index / 1_000_000,
                    index % 1_000_000
                ),
            })
            .collect();
        let mut requests = 0;
        let actual = read_pages(
            |after| {
                requests += 1;
                Ok(expected
                    .iter()
                    .filter(|version| after.is_none_or(|last| version.id.as_str() > last))
                    .take(137)
                    .cloned()
                    .collect())
            },
            |version: &ArticleVersion| version.id.as_str(),
        )
        .unwrap();
        assert_eq!(actual, expected);
        assert_eq!(requests, 1205usize.div_ceil(137) + 1);
    }

    #[test]
    fn profile_voice_sort_keeps_fractional_creation_order_ahead_of_uuid_order() {
        let mut rows = vec![
            VoiceSummary {
                id: "z".into(),
                name: "later".into(),
                created_at: "2026-01-01T12:00:00.250+00:00".into(),
                updated_at: String::new(),
                source_count: 0,
            },
            VoiceSummary {
                id: "a".into(),
                name: "earlier".into(),
                created_at: "2026-01-01T12:00:00.750+00:00".into(),
                updated_at: String::new(),
                source_count: 0,
            },
        ];
        sort_timestamp_desc(&mut rows, |row| &row.created_at, |row| &row.id);
        assert_eq!(
            rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
            ["a", "z"]
        );
    }

    #[test]
    fn timestamp_sorting_is_independent_of_uuid_order_and_breaks_ties_by_id() {
        let mut rows = [
            ArticleSummary {
                id: "c".into(),
                voice_id: None,
                platform_id: "x".into(),
                title: "".into(),
                excerpt: "".into(),
                status: ArticleStatus::Draft,
                queue: None,
                created_at: "".into(),
                updated_at: "2026-01-01T12:00:00.250+00:00".into(),
            },
            ArticleSummary {
                id: "a".into(),
                voice_id: None,
                platform_id: "x".into(),
                title: "".into(),
                excerpt: "".into(),
                status: ArticleStatus::Draft,
                queue: None,
                created_at: "".into(),
                updated_at: "2026-01-01T12:00:00.500+00:00".into(),
            },
            ArticleSummary {
                id: "b".into(),
                voice_id: None,
                platform_id: "x".into(),
                title: "".into(),
                excerpt: "".into(),
                status: ArticleStatus::Draft,
                queue: None,
                created_at: "".into(),
                updated_at: "2026-01-01T12:00:00.500+00:00".into(),
            },
        ];
        rows.sort_by(|a, b| {
            cmp_timestamp(&b.updated_at, &a.updated_at).then_with(|| a.id.cmp(&b.id))
        });
        assert_eq!(
            rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
    }
    #[test]
    fn queued_article_deserializes_its_immutable_profile_owner_and_optional_start_time() {
        let legacy: Article = serde_json::from_str(
            r#"{"id":"article-1","profile_id":"profile-a","voice_id":null,"platform_id":"x","title":"Queued","body":"","brief":"","status":"draft","queue":"writing","created_at":"now","updated_at":"now"}"#,
        )
        .unwrap();
        assert_eq!(legacy.profile_id, "profile-a");
        assert_eq!(legacy.queue_started_at, None);
        assert_eq!(
            serde_json::to_value(&legacy).unwrap()["profile_id"],
            "profile-a"
        );

        let timestamped: Article = serde_json::from_str(
            r#"{"id":"article-2","profile_id":"profile-a","voice_id":null,"platform_id":"x","title":"Queued","body":"","brief":"","status":"draft","queue":"writing","queue_started_at":"2026-09-27T12:00:00.123456+00:00","created_at":"now","updated_at":"now"}"#,
        )
        .unwrap();
        assert_eq!(
            timestamped.queue_started_at.as_deref(),
            Some("2026-09-27T12:00:00.123456+00:00")
        );
    }

    #[test]
    fn account_queue_pagination_keeps_a_newer_waiting_article_in_another_profile() {
        let mut rows: Vec<Article> = (0..1205)
            .map(|index| Article {
                id: format!("article-{:04}", 1204 - index),
                profile_id: if index == 1204 {
                    "profile-b"
                } else {
                    "profile-a"
                }
                .into(),
                voice_id: None,
                platform_id: "x".into(),
                title: String::new(),
                body: String::new(),
                brief: String::new(),
                status: ArticleStatus::Draft,
                queue: Some(if index == 1204 {
                    QueueState::Waiting
                } else {
                    QueueState::Failed
                }),
                queue_started_at: None,
                created_at: format!("2026-01-01T00:{:02}:{:02}+00:00", index / 60, index % 60),
                updated_at: String::new(),
            })
            .collect();
        rows.sort_by(|a, b| a.id.cmp(&b.id));
        let mut pages = 0;
        let queued = queued_articles(|after| {
            pages += 1;
            Ok(rows
                .iter()
                .filter(|article| after.is_none_or(|last| article.id.as_str() > last))
                .take(PAGE)
                .cloned()
                .collect())
        })
        .unwrap();

        assert_eq!(queued.len(), 1205);
        assert_eq!(pages, 1205usize.div_ceil(PAGE) + 1);
        assert_eq!(
            queued.first().unwrap().created_at,
            "2026-01-01T00:00:00+00:00"
        );
        let waiting = queued.last().unwrap();
        assert_eq!(waiting.profile_id, "profile-b");
        assert_eq!(waiting.queue, Some(QueueState::Waiting));
        assert!(
            queued
                .windows(2)
                .all(|pair| { cmp_timestamp(&pair[0].created_at, &pair[1].created_at).is_le() })
        );
    }

    #[test]
    fn articles_queue_and_empty_drafts_sort_all_1205_rows_after_keyset_paging() {
        let mut expected: Vec<Article> = (0..1205)
            .map(|index| Article {
                id: format!("{:04}", 1204 - index),
                profile_id: "profile-owner".into(),
                voice_id: None,
                platform_id: "x".into(),
                title: String::new(),
                body: String::new(),
                brief: String::new(),
                status: ArticleStatus::Draft,
                queue: Some(QueueState::Waiting),
                queue_started_at: None,
                created_at: format!(
                    "2026-01-01T12:00:{:02}.{:06}+00:00",
                    index / 1_000_000,
                    index % 1_000_000
                ),
                updated_at: format!(
                    "2026-01-02T12:00:{:02}.{:06}+00:00",
                    index / 1_000_000,
                    index % 1_000_000
                ),
            })
            .collect();
        expected.sort_by(|a, b| a.id.cmp(&b.id));
        let mut paged = read_pages(
            |after| {
                Ok(expected
                    .iter()
                    .filter(|row| after.is_none_or(|last| row.id.as_str() > last))
                    .take(137)
                    .cloned()
                    .collect())
            },
            |row: &Article| row.id.as_str(),
        )
        .unwrap();
        assert_eq!(paged.len(), 1205);
        paged.sort_by(|a, b| {
            cmp_timestamp(&b.updated_at, &a.updated_at).then_with(|| a.id.cmp(&b.id))
        });
        assert_eq!(
            paged.first().unwrap().updated_at,
            "2026-01-02T12:00:00.001204+00:00"
        );
        assert_eq!(paged.first().unwrap().id, "0000");
        paged.sort_by(|a, b| {
            cmp_timestamp(&a.created_at, &b.created_at).then_with(|| a.id.cmp(&b.id))
        });
        assert_eq!(
            paged.first().unwrap().created_at,
            "2026-01-01T12:00:00.000000+00:00"
        );
        assert_eq!(paged.first().unwrap().id, "1204");
    }
}
