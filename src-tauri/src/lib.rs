//! The Tauri shell. Everything the UI can ask the native side for is a
//! `#[tauri::command]` here; the actual work lives in the `lita-*` crates.

mod config;
mod errors;

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use errors::UiError;

use lita_auth::{Session, SessionStore, SupabaseAuth};
use lita_engine::post::{PlatformRules, PostDraft, generation_prompt, post_schema};
use lita_engine::presets;
use lita_engine::topics::{
    self, ArticleText, Existing, GatheredCloud, Lang, Policy, SuggestedBrief, TopicCloud, Topics,
};
use lita_engine::voice::{VoiceProfile, pipeline};
use lita_engine::{CodexCli, Preflight, Request};
use lita_sources::Piece;
use lita_store::{
    Article, ArticlePatch, ArticleStatus, ArticleSummary, ArticleVersion, NewArticle, NewSource,
    NewVersion, Platform, ProfileStore, PublishingProfile, QueueState, SourceKind, Store,
    StoredEffort, VersionKind, Voice, VoiceSummary,
};
use serde::{Deserialize, Serialize};
use tauri::menu::{MenuBuilder, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_updater::UpdaterExt;

/// What the UI needs to know about Codex before anything else can happen.
/// Each variant maps to a different next step for the user, which is why
/// they stay distinct instead of collapsing into ok/error.
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CodexStatus {
    Ready { version: String },
    NotLoggedIn { version: String },
    NotInstalled,
    ConfigBroken { version: String, message: String },
    Error { message: String },
}

impl From<Preflight> for CodexStatus {
    fn from(p: Preflight) -> Self {
        match p {
            Preflight::Ready { version } => CodexStatus::Ready { version },
            Preflight::NotLoggedIn { version } => CodexStatus::NotLoggedIn { version },
            Preflight::NotInstalled => CodexStatus::NotInstalled,
            Preflight::ConfigBroken { version, detail } => CodexStatus::ConfigBroken {
                version,
                message: detail,
            },
        }
    }
}

/// Whether someone is signed in to Lita (not to Codex; that is separate).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SessionStatus {
    /// The stored session is still being refreshed; ask again shortly.
    Restoring,
    SignedOut,
    SignedIn {
        email: Option<String>,
    },
}

impl From<Option<&Session>> for SessionStatus {
    fn from(s: Option<&Session>) -> Self {
        match s {
            Some(s) => SessionStatus::SignedIn {
                email: s.user.email.clone(),
            },
            None => SessionStatus::SignedOut,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct QueueJobId {
    profile_id: String,
    article_id: String,
}

impl QueueJobId {
    fn new(profile_id: &str, article_id: &str) -> Self {
        Self {
            profile_id: profile_id.to_owned(),
            article_id: article_id.to_owned(),
        }
    }
}

struct ActiveQueueJob {
    id: QueueJobId,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
struct QueueTransitions {
    active: Option<ActiveQueueJob>,
    recovered_stale: HashSet<QueueJobId>,
}

#[derive(Debug, PartialEq, Eq)]
enum QueueStopAction {
    CancelActive,
    PreserveDraft,
    Delete,
}

#[derive(Debug, PartialEq, Eq)]
enum QueueStartTransitionAction {
    Proceed,
    Stop,
    MarkFailed,
    Skip,
    RereadFailed,
}

fn queue_start_transition_action(
    status: Result<Option<QueueState>, ()>,
    error_code: &str,
) -> QueueStartTransitionAction {
    match status {
        Err(()) => QueueStartTransitionAction::RereadFailed,
        Ok(Some(QueueState::Writing)) => QueueStartTransitionAction::Proceed,
        Ok(Some(QueueState::Waiting)) if stops_the_queue(error_code) => {
            QueueStartTransitionAction::Stop
        }
        Ok(Some(QueueState::Waiting)) => QueueStartTransitionAction::MarkFailed,
        Ok(_) => QueueStartTransitionAction::Skip,
    }
}

enum QueueStartOutcome {
    Started(Arc<AtomicBool>),
    Skip,
    MarkedFailed,
}

#[derive(Debug, PartialEq, Eq)]
enum StaleWritingRecoveryAction {
    Skip,
    Requeue,
    ClearQueue,
    FailUncertain,
}

fn stale_writing_recovery_action(
    status: Option<QueueState>,
    is_active: bool,
    article: Option<&Article>,
    versions: &[ArticleVersion],
) -> StaleWritingRecoveryAction {
    if status != Some(QueueState::Writing) || is_active {
        return StaleWritingRecoveryAction::Skip;
    }
    let Some(article) = article else {
        return StaleWritingRecoveryAction::Skip;
    };

    let candidates: Vec<&ArticleVersion> = versions
        .iter()
        .filter(|version| {
            version.article_id == article.id
                && matches!(
                    version.kind,
                    VersionKind::Generated | VersionKind::Shortened
                )
                && article
                    .queue_started_at
                    .as_deref()
                    .is_none_or(|started_at| {
                        cmp_timestamp_full_precision(&version.created_at, started_at)
                            == std::cmp::Ordering::Greater
                    })
        })
        .collect();
    if candidates
        .iter()
        .any(|version| version.title == article.title && version.body == article.body)
    {
        StaleWritingRecoveryAction::ClearQueue
    } else if candidates.is_empty() {
        StaleWritingRecoveryAction::Requeue
    } else {
        StaleWritingRecoveryAction::FailUncertain
    }
}

fn for_each_stale_writing_article(
    articles: impl IntoIterator<Item = Article>,
    mut recover: impl FnMut(Article) -> Result<(), UiError>,
) -> Result<(), UiError> {
    for article in articles {
        if article.queue == Some(QueueState::Writing) {
            recover(article)?;
        }
    }
    Ok(())
}

impl QueueTransitions {
    fn is_active(&self, id: &QueueJobId) -> bool {
        self.active.as_ref().is_some_and(|active| active.id == *id)
    }

    fn recover_stale_writing(&mut self, id: &QueueJobId, status: Option<QueueState>) -> bool {
        if status != Some(QueueState::Writing)
            || self.active.as_ref().is_some_and(|active| active.id == *id)
        {
            return false;
        }
        self.recovered_stale.insert(id.clone());
        true
    }

    fn forget_recovered(&mut self, id: &QueueJobId) {
        self.recovered_stale.remove(id);
    }

    fn begin(&mut self, id: QueueJobId) -> Option<Arc<AtomicBool>> {
        if self.active.is_some() {
            return None;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.active = Some(ActiveQueueJob {
            id,
            cancel: Arc::clone(&cancel),
        });
        Some(cancel)
    }

    fn stop(&mut self, id: &QueueJobId, status: QueueState) -> QueueStopAction {
        if status == QueueState::Writing
            && self.active.as_ref().is_some_and(|active| active.id == *id)
        {
            return QueueStopAction::CancelActive;
        }
        if status == QueueState::Writing || self.recovered_stale.contains(id) {
            QueueStopAction::PreserveDraft
        } else {
            QueueStopAction::Delete
        }
    }

    fn cancel_active(&self, id: &QueueJobId) -> bool {
        let Some(active) = self.active.as_ref().filter(|active| active.id == *id) else {
            return false;
        };
        active.cancel.store(true, Ordering::Relaxed);
        true
    }

    fn finish(&mut self, id: &QueueJobId) {
        if self.active.as_ref().is_some_and(|active| active.id == *id) {
            self.active = None;
        }
    }

    #[cfg(test)]
    fn active_cancelled(&self, id: &QueueJobId) -> bool {
        self.active
            .as_ref()
            .filter(|active| active.id == *id)
            .is_some_and(|active| active.cancel.load(Ordering::Relaxed))
    }
}

#[derive(Default)]
struct QueueWorkerState {
    running: bool,
    pending_start: bool,
}

impl QueueWorkerState {
    fn request_start(&mut self) -> bool {
        if self.running {
            self.pending_start = true;
            false
        } else {
            self.running = true;
            true
        }
    }

    /// Returns true when a successful run must drain a pending start before idling.
    fn finish_run(&mut self, succeeded: bool) -> bool {
        if succeeded && self.pending_start {
            self.pending_start = false;
            true
        } else {
            self.pending_start = false;
            self.running = false;
            false
        }
    }
}

#[derive(Default)]
struct SessionGeneration(u64);

impl SessionGeneration {
    fn begin(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(1);
        self.0
    }
    fn current(&self) -> u64 {
        self.0
    }

    fn is_current(&self, generation: u64) -> bool {
        self.0 == generation
    }

    fn commit_if_current(&mut self, generation: u64) -> bool {
        if !self.is_current(generation) {
            return false;
        }
        self.begin();
        true
    }
}

#[derive(Default)]
struct SessionLifecycle {
    generation: SessionGeneration,
    session: Option<Session>,
    active_profile_id: Option<String>,
    selection_revision: u64,
    session_since: Option<Instant>,
}

impl SessionLifecycle {
    fn begin_profile_selection(&mut self, generation: u64) -> Option<u64> {
        if !self.generation.is_current(generation) {
            return None;
        }
        self.selection_revision = self.selection_revision.wrapping_add(1);
        Some(self.selection_revision)
    }

    fn profile_selection_is_current(&self, generation: u64, revision: u64) -> bool {
        self.generation.is_current(generation) && self.selection_revision == revision
    }

    fn commit_profile_selection(
        &mut self,
        generation: u64,
        revision: u64,
        profile_id: String,
    ) -> bool {
        if !self.profile_selection_is_current(generation, revision) {
            return false;
        }
        self.active_profile_id = Some(profile_id);
        true
    }

    fn clear_session(&mut self) {
        self.session = None;
        self.active_profile_id = None;
        self.session_since = None;
    }

    fn install(&mut self, session: Session, profile_id: String) {
        self.session_since = Some(Instant::now());
        self.session = Some(session);
        self.active_profile_id = Some(profile_id);
    }
}

/// Everything the commands share. `session` is `None` when signed out.
pub struct AppState {
    auth: SupabaseAuth,
    store: Store,
    keychain: SessionStore,
    lifecycle: Mutex<SessionLifecycle>,
    /// Serializes profile-selection commits with session lifecycle changes.
    selection_operation: Mutex<()>,
    /// True until the launch-time restore has finished.
    restoring: AtomicBool,
    /// Set by `cancel_sign_in`; cleared when a sign-in starts.
    sign_in_cancel: Arc<AtomicBool>,
    /// Set by `cancel_voice_build`; cleared when a build starts.
    build_cancel: Arc<AtomicBool>,
    /// Set by `cancel_generate`; cleared when a generation starts.
    generate_cancel: Arc<AtomicBool>,
    /// Serializes queue row transitions and tracks the account-wide active job.
    queue_transitions: Mutex<QueueTransitions>,
    /// One synchronized account-wide worker with a pending-start handoff.
    queue_worker: Mutex<QueueWorkerState>,
}

impl AppState {
    fn new() -> anyhow::Result<Self> {
        Ok(Self {
            auth: SupabaseAuth::new(config::SUPABASE_URL, config::SUPABASE_ANON_KEY)?,
            store: Store::new(config::SUPABASE_URL, config::SUPABASE_ANON_KEY)?,
            keychain: SessionStore::new(config::KEYCHAIN_SERVICE),
            lifecycle: Mutex::new(SessionLifecycle::default()),
            selection_operation: Mutex::new(()),
            restoring: AtomicBool::new(true),
            sign_in_cancel: Arc::new(AtomicBool::new(false)),
            build_cancel: Arc::new(AtomicBool::new(false)),
            generate_cancel: Arc::new(AtomicBool::new(false)),
            queue_transitions: Mutex::new(QueueTransitions::default()),
            queue_worker: Mutex::new(QueueWorkerState::default()),
        })
    }

    fn can_commit_profile_selection(&self, generation: u64, revision: u64) -> bool {
        self.lifecycle
            .lock()
            .unwrap()
            .profile_selection_is_current(generation, revision)
    }

    /// Loads the stored session and refreshes it, so a stale token never
    /// reaches the UI as "signed in". A refresh failure means signed out.
    fn begin_restore(&self) -> u64 {
        let _selection_operation = self.selection_operation.lock().unwrap();
        let mut lifecycle = self.lifecycle.lock().unwrap();
        let generation = lifecycle.generation.begin();
        lifecycle.clear_session();
        generation
    }

    fn begin_sign_in(&self) -> u64 {
        let _selection_operation = self.selection_operation.lock().unwrap();
        let mut lifecycle = self.lifecycle.lock().unwrap();
        let generation = lifecycle.generation.begin();
        lifecycle.clear_session();
        self.restoring.store(false, Ordering::Release);
        generation
    }

    fn restore(&self, generation: u64) {
        let stored = match self.keychain.load() {
            Ok(Some(stored)) => stored,
            Ok(None) => {
                self.finish_restore_without_session(generation);
                return;
            }
            Err(error) => {
                eprintln!("keychain unavailable: {error:#}");
                self.finish_restore_without_session(generation);
                return;
            }
        };

        let fresh = match self.auth.refresh(&stored.refresh_token) {
            Ok(fresh) => fresh,
            Err(error) => {
                eprintln!("stored session could not be refreshed; signing out: {error:#}");
                let _selection_operation = self.selection_operation.lock().unwrap();
                let mut lifecycle = self.lifecycle.lock().unwrap();
                if lifecycle.generation.is_current(generation) {
                    let _ = self.keychain.clear();
                    lifecycle.generation.commit_if_current(generation);
                    lifecycle.clear_session();
                    self.restoring.store(false, Ordering::Release);
                }
                return;
            }
        };

        let selected = self
            .store
            .as_user(fresh.access_token.clone())
            .selected_profile();

        let _selection_operation = self.selection_operation.lock().unwrap();
        let mut lifecycle = self.lifecycle.lock().unwrap();
        if !lifecycle.generation.is_current(generation) {
            return;
        }
        let _ = self.keychain.save(&fresh);
        lifecycle.generation.commit_if_current(generation);
        match selected {
            Ok(Some(profile)) => lifecycle.install(fresh, profile.id),
            Ok(None) => {
                eprintln!(
                    "signed-in account has no persisted active Profile; keeping refreshed session for retry"
                );
                lifecycle.clear_session();
            }
            Err(error) => {
                eprintln!(
                    "active Profile could not be restored; keeping refreshed session for retry: {error:#}"
                );
                lifecycle.clear_session();
            }
        }
        self.restoring.store(false, Ordering::Release);
    }

    fn finish_restore_without_session(&self, generation: u64) {
        let _selection_operation = self.selection_operation.lock().unwrap();
        let mut lifecycle = self.lifecycle.lock().unwrap();
        if lifecycle.generation.commit_if_current(generation) {
            lifecycle.clear_session();
            self.restoring.store(false, Ordering::Release);
        }
    }

    fn persist_sign_in_session(&self, generation: u64, session: &Session) -> Result<bool, UiError> {
        let _selection_operation = self.selection_operation.lock().unwrap();
        let lifecycle = self.lifecycle.lock().unwrap();
        if !lifecycle.generation.is_current(generation) {
            return Ok(false);
        }
        self.keychain.save(session).map_err(fail)?;
        Ok(true)
    }

    fn commit_sign_in(&self, generation: u64, session: Session, profile_id: String) -> bool {
        let _selection_operation = self.selection_operation.lock().unwrap();
        let mut lifecycle = self.lifecycle.lock().unwrap();
        if !lifecycle.generation.commit_if_current(generation) {
            return false;
        }
        lifecycle.install(session, profile_id);
        self.restoring.store(false, Ordering::Release);
        true
    }

    fn sign_out(&self) -> Result<(), UiError> {
        let _selection_operation = self.selection_operation.lock().unwrap();
        let mut lifecycle = self.lifecycle.lock().unwrap();
        lifecycle.generation.begin();
        lifecycle.clear_session();
        self.restoring.store(false, Ordering::Release);
        self.keychain.clear().map_err(fail)
    }

    /// Renews the access token when it is within five minutes of running
    /// out. Called from a background thread once a minute and from
    /// `access_token`, so a token never expires under a running app or
    /// after the Mac wakes from sleep. A failed renewal keeps the old
    /// token: the next request then fails as `session_expired`, and the
    /// screen offers to sign in again.
    fn refresh_if_stale(&self) {
        const MARGIN: Duration = Duration::from_secs(300);
        let (generation, refresh_token, user_id) = {
            let lifecycle = self.lifecycle.lock().unwrap();
            match (lifecycle.session.as_ref(), lifecycle.session_since) {
                (Some(session), Some(since)) => {
                    let lifetime = Duration::from_secs(session.expires_in.max(600));
                    if since.elapsed() + MARGIN < lifetime {
                        return;
                    }
                    (
                        lifecycle.generation.current(),
                        session.refresh_token.clone(),
                        session.user.id.clone(),
                    )
                }
                _ => return,
            }
        };

        match self.auth.refresh(&refresh_token) {
            Ok(fresh) => {
                let _selection_operation = self.selection_operation.lock().unwrap();
                let mut lifecycle = self.lifecycle.lock().unwrap();
                let identity_matches = lifecycle.session.as_ref().is_some_and(|session| {
                    session.refresh_token == refresh_token && session.user.id == user_id
                });
                if !lifecycle.generation.is_current(generation)
                    || !identity_matches
                    || fresh.user.id != user_id
                {
                    return;
                }
                let Some(profile_id) = lifecycle.active_profile_id.clone() else {
                    return;
                };
                let _ = self.keychain.save(&fresh);
                if lifecycle.generation.commit_if_current(generation) {
                    lifecycle.install(fresh, profile_id);
                }
            }
            Err(error) => eprintln!("session could not be renewed: {error:#}"),
        }
    }

    fn status(&self) -> SessionStatus {
        if self.restoring.load(Ordering::Acquire) {
            return SessionStatus::Restoring;
        }
        let lifecycle = self.lifecycle.lock().unwrap();
        SessionStatus::from(lifecycle.session.as_ref())
    }

    /// The current access token, or the error the UI shows when signed out.
    fn access_token(&self) -> Result<String, UiError> {
        self.refresh_if_stale();
        self.lifecycle
            .lock()
            .unwrap()
            .session
            .as_ref()
            .map(|session| session.access_token.clone())
            .ok_or_else(UiError::not_signed_in)
    }

    fn command_snapshot_with_generation(&self) -> Result<(String, String, u64), UiError> {
        self.refresh_if_stale();
        let lifecycle = self.lifecycle.lock().unwrap();
        let session = lifecycle
            .session
            .as_ref()
            .ok_or_else(UiError::not_signed_in)?;
        let profile_id = lifecycle
            .active_profile_id
            .clone()
            .ok_or_else(UiError::not_signed_in)?;
        Ok((
            session.access_token.clone(),
            profile_id,
            lifecycle.generation.current(),
        ))
    }

    fn command_snapshot(&self) -> Result<(String, String), UiError> {
        let (token, profile_id, _) = self.command_snapshot_with_generation()?;
        Ok((token, profile_id))
    }
}

fn fail(e: anyhow::Error) -> UiError {
    UiError::from(e)
}

/// Runs `codex doctor --json` off the main thread, because it takes about a
/// second and the window must not freeze while it does.
#[tauri::command]
async fn codex_status() -> CodexStatus {
    let result = tauri::async_runtime::spawn_blocking(|| CodexCli::on_path().preflight()).await;
    match result {
        Ok(Ok(preflight)) => preflight.into(),
        Ok(Err(e)) => CodexStatus::Error {
            message: format!("{e:#}"),
        },
        Err(e) => CodexStatus::Error {
            message: format!("preflight task failed: {e}"),
        },
    }
}

#[tauri::command]
fn session_status(state: State<'_, AppState>) -> SessionStatus {
    state.status()
}

#[tauri::command]
async fn list_profiles(app: AppHandle) -> Result<Vec<PublishingProfile>, UiError> {
    let token = app.state::<AppState>().access_token()?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AppState>()
            .store
            .as_user(token)
            .profiles()
            .map_err(fail)
    })
    .await
    .map_err(|error| UiError::unknown(error.to_string()))?
}

#[tauri::command]
async fn selected_profile(app: AppHandle) -> Result<PublishingProfile, UiError> {
    let (token, profile_id) = app.state::<AppState>().command_snapshot()?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AppState>()
            .store
            .as_user(token)
            .profiles()
            .map_err(fail)?
            .into_iter()
            .find(|profile| profile.id == profile_id)
            .ok_or_else(|| UiError::unknown("active Profile is no longer available"))
    })
    .await
    .map_err(|error| UiError::unknown(error.to_string()))?
}

#[tauri::command]
async fn create_profile(app: AppHandle, name: String) -> Result<PublishingProfile, UiError> {
    let (token, _, generation) = app.state::<AppState>().command_snapshot_with_generation()?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _selection_operation = state.selection_operation.lock().unwrap();
        let selection_revision = state
            .lifecycle
            .lock()
            .unwrap()
            .begin_profile_selection(generation)
            .ok_or_else(UiError::not_signed_in)?;
        if !state.can_commit_profile_selection(generation, selection_revision) {
            return Err(UiError::not_signed_in());
        }
        let profile = state
            .store
            .as_user(token.clone())
            .create_profile(&name)
            .map_err(fail)?;
        if !state.can_commit_profile_selection(generation, selection_revision) {
            return Err(UiError::not_signed_in());
        }
        state
            .store
            .as_user(token)
            .select_profile(&profile.id)
            .map_err(fail)?;
        let committed = state.lifecycle.lock().unwrap().commit_profile_selection(
            generation,
            selection_revision,
            profile.id.clone(),
        );
        if !committed {
            return Err(UiError::not_signed_in());
        }
        Ok(profile)
    })
    .await
    .map_err(|error| UiError::unknown(error.to_string()))?
}

#[tauri::command]
async fn rename_profile(app: AppHandle, id: String, name: String) -> Result<(), UiError> {
    let token = app.state::<AppState>().access_token()?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AppState>()
            .store
            .as_user(token)
            .rename_profile(&id, &name)
            .map_err(fail)
    })
    .await
    .map_err(|error| UiError::unknown(error.to_string()))?
}

#[tauri::command]
async fn select_profile(app: AppHandle, id: String) -> Result<PublishingProfile, UiError> {
    let (token, _, generation) = app.state::<AppState>().command_snapshot_with_generation()?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _selection_operation = state.selection_operation.lock().unwrap();
        let selection_revision = state
            .lifecycle
            .lock()
            .unwrap()
            .begin_profile_selection(generation)
            .ok_or_else(UiError::not_signed_in)?;
        let profile = state
            .store
            .as_user(token.clone())
            .profiles()
            .map_err(fail)?
            .into_iter()
            .find(|profile| profile.id == id)
            .ok_or_else(|| UiError::invalid("no such Profile"))?;
        if !state.can_commit_profile_selection(generation, selection_revision) {
            return Err(UiError::not_signed_in());
        }
        state
            .store
            .as_user(token)
            .select_profile(&profile.id)
            .map_err(fail)?;
        let committed = state.lifecycle.lock().unwrap().commit_profile_selection(
            generation,
            selection_revision,
            profile.id.clone(),
        );
        if !committed {
            return Err(UiError::not_signed_in());
        }
        Ok(profile)
    })
    .await
    .map_err(|error| UiError::unknown(error.to_string()))?
}

/// Opens the browser for Google sign-in and waits for it to come back.
/// Blocks for as long as the person takes, so it runs off the main thread.
#[tauri::command]
async fn sign_in(app: AppHandle) -> Result<SessionStatus, UiError> {
    let state = app.state::<AppState>();
    let generation = state.begin_sign_in();
    let cancel = {
        state.sign_in_cancel.store(false, Ordering::Relaxed);
        Arc::clone(&state.sign_in_cancel)
    };
    let opener = app.clone();
    let session = tauri::async_runtime::spawn_blocking(move || {
        let state = opener.state::<AppState>();
        state.auth.sign_in_with_provider_cancellable(
            "google",
            |url| {
                opener
                    .opener()
                    .open_url(url.as_str(), None::<&str>)
                    .map_err(Into::into)
            },
            &cancel,
        )
    })
    .await
    .map_err(|error| UiError::unknown(format!("sign-in task failed: {error}")))?
    .map_err(fail)?;

    if !state.persist_sign_in_session(generation, &session)? {
        return Err(UiError::unknown(
            "sign-in was superseded by a newer session action",
        ));
    }
    let lookup_app = app.clone();
    let access_token = session.access_token.clone();
    let selected = tauri::async_runtime::spawn_blocking(move || {
        lookup_app
            .state::<AppState>()
            .store
            .as_user(access_token)
            .selected_profile()
    })
    .await
    .map_err(|error| UiError::unknown(format!("Profile lookup task failed: {error}")))?
    .map_err(fail)?
    .ok_or_else(|| UiError::unknown("signed-in account has no persisted active Profile"))?;

    if !state.commit_sign_in(generation, session, selected.id) {
        return Err(UiError::unknown(
            "sign-in was superseded by a newer session action",
        ));
    }
    Ok(state.status())
}

#[tauri::command]
fn cancel_sign_in(state: State<'_, AppState>) {
    state.sign_in_cancel.store(true, Ordering::Relaxed);
}

#[tauri::command]
async fn sign_out(app: AppHandle) -> Result<SessionStatus, UiError> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        state.sign_out()?;
        Ok(state.status())
    })
    .await
    .map_err(|error| UiError::unknown(error.to_string()))?
}

// ----- voices ----------------------------------------------------------------

#[tauri::command]
async fn list_voices(app: AppHandle) -> Result<Vec<VoiceSummary>, UiError> {
    let (token, profile_id) = app.state::<AppState>().command_snapshot()?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AppState>()
            .store
            .as_user(token)
            .for_profile(&profile_id)
            .map_err(fail)?
            .voices()
            .map_err(fail)
    })
    .await
    .map_err(|e| UiError::unknown(e.to_string()))?
}

#[tauri::command]
async fn get_voice(app: AppHandle, id: String) -> Result<Option<Voice>, UiError> {
    let (token, profile_id) = app.state::<AppState>().command_snapshot()?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AppState>()
            .store
            .as_user(token)
            .for_profile(&profile_id)
            .map_err(fail)?
            .voice(&id)
            .map_err(fail)
    })
    .await
    .map_err(|e| UiError::unknown(e.to_string()))?
}

#[tauri::command]
async fn delete_voice(app: AppHandle, id: String) -> Result<bool, UiError> {
    let (token, profile_id) = app.state::<AppState>().command_snapshot()?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AppState>()
            .store
            .as_user(token)
            .for_profile(&profile_id)
            .map_err(fail)?
            .delete_voice(&id)
            .map_err(fail)
    })
    .await
    .map_err(|e| UiError::unknown(e.to_string()))?
}

#[tauri::command]
async fn rename_voice(app: AppHandle, id: String, name: String) -> Result<(), UiError> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(UiError::invalid("give the voice a name"));
    }
    let (token, profile_id) = app.state::<AppState>().command_snapshot()?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AppState>()
            .store
            .as_user(token)
            .for_profile(&profile_id)
            .map_err(fail)?
            .rename_voice(&id, &name)
            .map_err(fail)
    })
    .await
    .map_err(|e| UiError::unknown(e.to_string()))?
}

/// The person corrected part of the profile. Stored as given; the UI owns
/// the editing rules.
#[tauri::command]
async fn update_voice_profile(
    app: AppHandle,
    id: String,
    profile: VoiceProfile,
) -> Result<Option<Voice>, UiError> {
    let (token, profile_id) = app.state::<AppState>().command_snapshot()?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let user = state.store.as_user(token);
        let store = user.for_profile(&profile_id).map_err(fail)?;
        store.update_profile(&id, &profile).map_err(fail)?;
        store.voice(&id).map_err(fail)
    })
    .await
    .map_err(|e| UiError::unknown(e.to_string()))?
}

/// One piece of writing handed in by the UI.
#[derive(Debug, Deserialize)]
pub struct SourceInput {
    pub kind: SourceKind,
    pub origin: Option<String>,
    #[serde(default)]
    pub account: Option<String>,
    pub body: String,
}

fn to_sources(sources: Vec<SourceInput>) -> Vec<NewSource> {
    sources
        .into_iter()
        .map(|s| NewSource {
            kind: s.kind,
            origin: s.origin,
            account: s.account,
            body: s.body.trim().to_string(),
        })
        .filter(|s| !s.body.is_empty())
        .collect()
}

/// Reads a voice profile out of `sources` with the careful four-stage
/// pipeline (measure → read each piece → synthesise → check), reporting
/// the real stages on `voice-progress`. Shared by the first build and by
/// relearning (#68). Falls back to the single call inside the pipeline.
fn extract_profile(
    app: &AppHandle,
    sources: &[NewSource],
    cancel: Arc<AtomicBool>,
    working_dir: std::path::PathBuf,
) -> Result<VoiceProfile, UiError> {
    std::fs::create_dir_all(&working_dir).map_err(|e| UiError::unknown(e.to_string()))?;
    let bodies: Vec<&str> = sources.iter().map(|s| s.body.as_str()).collect();
    let emitter = app.clone();
    let outcome = pipeline::extract(
        &CodexCli::on_path(),
        &bodies,
        &pipeline::Config::default(),
        working_dir,
        cancel,
        |stage| {
            let p = match stage {
                pipeline::Stage::Measuring => VoiceProgress::Started,
                pipeline::Stage::Reading { done, total } => VoiceProgress::Reading { done, total },
                pipeline::Stage::Synthesising => VoiceProgress::Thinking,
                pipeline::Stage::Checking => VoiceProgress::Checking,
                pipeline::Stage::Done => VoiceProgress::Extracted,
            };
            let _ = emitter.emit("voice-progress", p);
        },
    )
    .map_err(fail)?;
    if outcome.report.fell_back {
        eprintln!(
            "voice extraction fell back to the single call: {:?}",
            outcome.report.notes
        );
    }
    Ok(outcome.profile)
}

/// Progress of a Voice build, for the stage display. Since 2026-09-23 the
/// stages are real (voice quality requirement, must 17): measuring, reading
/// piece n of m, synthesising, checking, done, saved.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "stage", rename_all = "snake_case")]
pub enum VoiceProgress {
    Started,
    Reading { done: usize, total: usize },
    Thinking,
    Checking,
    Extracted,
    Saved,
}

/// The limits `create_voice` applies, so the UI can show them.
#[derive(Debug, Serialize)]
pub struct MaterialBudget {
    pub per_piece_chars: usize,
    pub total_chars: usize,
}

#[tauri::command]
fn material_budget() -> MaterialBudget {
    let c = pipeline::Config::default();
    MaterialBudget {
        per_piece_chars: c.head_chars + c.tail_chars,
        total_chars: c.total_chars,
    }
}

/// Builds a Voice from the given writing: the four-stage pipeline reads the
/// profile (a few minutes), then it is stored with its sources. Emits
/// `voice-progress` events along the way. Material is capped by
/// `pipeline::Config::default()`; the sources are stored in full regardless.
#[tauri::command]
async fn create_voice(
    app: AppHandle,
    name: String,
    sources: Vec<SourceInput>,
) -> Result<Voice, UiError> {
    let sources = to_sources(sources);
    if sources.is_empty() {
        return Err(UiError::invalid("paste at least one piece of writing"));
    }
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(UiError::invalid("give the voice a name"));
    }
    let (token, profile_id, cancel) = {
        let state = app.state::<AppState>();
        let (token, profile_id) = state.command_snapshot()?;
        state.build_cancel.store(false, Ordering::Relaxed);
        (token, profile_id, Arc::clone(&state.build_cancel))
    };
    let working_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| UiError::unknown(e.to_string()))?;
    tauri::async_runtime::spawn_blocking(move || -> Result<Voice, UiError> {
        let profile = extract_profile(&app, &sources, cancel, working_dir)?;
        let voice = app
            .state::<AppState>()
            .store
            .as_user(token)
            .for_profile(&profile_id)
            .map_err(fail)?
            .create_voice(&name, &profile, &sources)
            .map_err(fail)?;
        let _ = app.emit("voice-progress", VoiceProgress::Saved);
        Ok(voice)
    })
    .await
    .map_err(|e| UiError::unknown(e.to_string()))?
}

/// Adds writing to a voice (#68): a new connection or new articles from one.
/// The profile stays; relearning is `rebuild_voice`.
#[tauri::command]
async fn add_voice_sources(
    app: AppHandle,
    voice_id: String,
    sources: Vec<SourceInput>,
) -> Result<Option<Voice>, UiError> {
    let sources = to_sources(sources);
    let (token, profile_id) = app.state::<AppState>().command_snapshot()?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let user = state.store.as_user(token);
        let store = user.for_profile(&profile_id).map_err(fail)?;
        store.add_sources(&voice_id, &sources).map_err(fail)?;
        store.voice(&voice_id).map_err(fail)
    })
    .await
    .map_err(|e| UiError::unknown(e.to_string()))?
}

/// Drops one connection (every piece of `kind` from `account`).
#[tauri::command]
async fn remove_voice_sources(
    app: AppHandle,
    voice_id: String,
    kind: SourceKind,
    account: Option<String>,
) -> Result<Option<Voice>, UiError> {
    let (token, profile_id) = app.state::<AppState>().command_snapshot()?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let user = state.store.as_user(token);
        let store = user.for_profile(&profile_id).map_err(fail)?;
        store
            .delete_sources(&voice_id, kind, account.as_deref())
            .map_err(fail)?;
        store.voice(&voice_id).map_err(fail)
    })
    .await
    .map_err(|e| UiError::unknown(e.to_string()))?
}

/// Drops a single piece (manual intake: one pasted post or one file).
#[tauri::command]
async fn remove_voice_source(
    app: AppHandle,
    voice_id: String,
    source_id: String,
) -> Result<Option<Voice>, UiError> {
    let (token, profile_id) = app.state::<AppState>().command_snapshot()?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let user = state.store.as_user(token);
        let store = user.for_profile(&profile_id).map_err(fail)?;
        store.delete_source(&voice_id, &source_id).map_err(fail)?;
        store.voice(&voice_id).map_err(fail)
    })
    .await
    .map_err(|e| UiError::unknown(e.to_string()))?
}

/// Reads the profile again from everything the voice now has. Hand edits
/// to the details are overwritten; the UI says so before calling this.
#[tauri::command]
async fn rebuild_voice(app: AppHandle, voice_id: String) -> Result<Voice, UiError> {
    let (token, profile_id, cancel) = {
        let state = app.state::<AppState>();
        let (token, profile_id) = state.command_snapshot()?;
        state.build_cancel.store(false, Ordering::Relaxed);
        (token, profile_id, Arc::clone(&state.build_cancel))
    };
    let working_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| UiError::unknown(e.to_string()))?;
    tauri::async_runtime::spawn_blocking(move || -> Result<Voice, UiError> {
        let state = app.state::<AppState>();
        let user = state.store.as_user(token);
        let store = user.for_profile(&profile_id).map_err(fail)?;
        let voice = store
            .voice(&voice_id)
            .map_err(fail)?
            .ok_or_else(|| UiError::invalid("no such voice"))?;
        let sources: Vec<NewSource> = voice
            .sources
            .iter()
            .map(|s| NewSource {
                kind: s.kind,
                origin: s.origin.clone(),
                account: s.account.clone(),
                body: s.body.clone(),
            })
            .collect();
        if sources.is_empty() {
            return Err(UiError::invalid("this voice has no writing to learn from"));
        }
        let profile = extract_profile(&app, &sources, cancel, working_dir)?;
        store.update_profile(&voice_id, &profile).map_err(fail)?;
        let _ = app.emit("voice-progress", VoiceProgress::Saved);
        store
            .voice(&voice_id)
            .map_err(fail)?
            .ok_or_else(|| UiError::invalid("voice vanished"))
    })
    .await
    .map_err(|e| UiError::unknown(e.to_string()))?
}

/// Voices bundled with Lita (D-70). Static; no network.
#[tauri::command]
fn list_voice_presets() -> Vec<presets::PresetSummary> {
    presets::list()
}

/// Copies a bundled voice into the person's own, with no material. The
/// name gets a number when it is taken already.
#[tauri::command]
async fn create_voice_from_preset(app: AppHandle, id: String) -> Result<Voice, UiError> {
    let (name, profile) = presets::find(&id).ok_or_else(|| UiError::invalid("no such preset"))?;
    let (token, profile_id) = app.state::<AppState>().command_snapshot()?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let user = state.store.as_user(token);
        let store = user.for_profile(&profile_id).map_err(fail)?;
        let taken: Vec<String> = store
            .voices()
            .map_err(fail)?
            .into_iter()
            .map(|v| v.name)
            .collect();
        let mut unique = name.to_string();
        let mut n = 2;
        while taken.contains(&unique) {
            unique = format!("{name} ({n})");
            n += 1;
        }
        store.create_voice(&unique, &profile, &[]).map_err(fail)
    })
    .await
    .map_err(|e| UiError::unknown(e.to_string()))?
}

#[tauri::command]
fn cancel_voice_build(state: State<'_, AppState>) {
    state.build_cancel.store(true, Ordering::Relaxed);
}

// ----- writing (B-07, B-08, B-13) ------------------------------------------

#[tauri::command]
async fn platforms(app: AppHandle) -> Result<Vec<Platform>, UiError> {
    let token = app.state::<AppState>().access_token()?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AppState>()
            .store
            .as_user(token)
            .platforms()
            .map_err(fail)
    })
    .await
    .map_err(|e| UiError::unknown(e.to_string()))?
}

fn rules_of(p: &Platform) -> PlatformRules {
    PlatformRules {
        name: p.name.clone(),
        max_chars: p.max_chars,
        rules: p.rules.clone(),
    }
}

/// The exact text that `generate_draft` would send (B-08).
#[tauri::command]
async fn preview_prompt(
    app: AppHandle,
    voice_id: String,
    brief: String,
    platform_id: String,
    previous: Option<String>,
) -> Result<String, UiError> {
    let (token, profile_id) = app.state::<AppState>().command_snapshot()?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let user = state.store.as_user(token);
        let profile_store = user.for_profile(&profile_id).map_err(fail)?;
        let voice = profile_store
            .voice(&voice_id)
            .map_err(fail)?
            .ok_or_else(|| UiError::invalid("no such voice"))?;
        let platform = user
            .platforms()
            .map_err(fail)?
            .into_iter()
            .find(|p| p.id == platform_id)
            .ok_or_else(|| UiError::invalid("no such platform"))?;
        Ok(prompt_for(
            &voice.profile,
            &brief,
            &rules_of(&platform),
            previous.as_deref(),
        ))
    })
    .await
    .map_err(|e| UiError::unknown(e.to_string()))?
}

/// What the write screen gets back from a generation. `draft` mirrors the
/// v0.1 shape (id = the article's id) so the current UI keeps working until
/// the editor replaces it (v0.2 phase 2).
#[derive(Debug, Serialize)]
pub struct Generated {
    pub draft: DraftView,
    pub voice_notes: String,
}

#[derive(Debug, Serialize)]
pub struct DraftView {
    pub id: String,
    pub brief_id: String,
    pub body: String,
    pub prompt_sent: String,
    pub model: Option<String>,
    pub elapsed_ms: i64,
    pub status: &'static str,
    pub created_at: String,
    pub decided_at: Option<String>,
}

/// One place decides between a fresh draft and a shorter rewrite (#40), so
/// the preview and the run can never disagree.
fn prompt_for(
    profile: &lita_engine::voice::VoiceProfile,
    brief: &str,
    rules: &lita_engine::post::PlatformRules,
    previous: Option<&str>,
) -> String {
    match previous.map(str::trim).filter(|p| !p.is_empty()) {
        Some(prev) => lita_engine::post::shorten_prompt(profile, brief, rules, prev),
        None => generation_prompt(profile, brief, rules),
    }
}

fn delete_cancelled_version(
    store: &lita_store::ProfileStore<'_>,
    version_id: &str,
) -> Result<(), UiError> {
    if store.delete_version(version_id).map_err(fail)? {
        Ok(())
    } else {
        Err(UiError::unknown(
            "cancelled generation left a Version that could not be removed",
        ))
    }
}

fn ensure_not_cancelled(cancel: &AtomicBool) -> Result<(), UiError> {
    if cancel.load(Ordering::Relaxed) {
        Err(UiError {
            code: "cancelled",
            detail: String::new(),
        })
    } else {
        Ok(())
    }
}

fn cmp_timestamp_full_precision(left: &str, right: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    let (Some(left_seconds), Some(right_seconds)) =
        (topics::epoch_seconds(left), topics::epoch_seconds(right))
    else {
        return left.cmp(right);
    };
    match left_seconds.cmp(&right_seconds) {
        Ordering::Equal => {}
        ordering => return ordering,
    }

    fn fraction(timestamp: &str) -> &str {
        timestamp
            .get(19..)
            .unwrap_or("")
            .strip_prefix('.')
            .map_or("", |fraction| {
                fraction
                    .split(|character: char| !character.is_ascii_digit())
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
            ordering => return ordering,
        }
    }
    Ordering::Equal
}

fn version_matches_attempt_after_article_snapshot(
    version: &ArticleVersion,
    request: &NewVersion,
    original_article_updated_at: &str,
) -> bool {
    version.article_id == request.article_id
        && version.kind == request.kind
        && version.title == request.title
        && version.body == request.body
        && version.prompt_sent == request.prompt_sent
        && version.elapsed_ms == request.elapsed_ms
        && cmp_timestamp_full_precision(&version.created_at, original_article_updated_at)
            == std::cmp::Ordering::Greater
}

fn uncertain_version_post_error(
    original_error: UiError,
    request: &NewVersion,
    reason: String,
) -> UiError {
    let original_detail = if original_error.detail.is_empty() {
        original_error.code.to_owned()
    } else {
        format!("{}: {}", original_error.code, original_error.detail)
    };
    UiError {
        code: "queue_completion_uncertain",
        detail: format!(
            "Version POST outcome is uncertain: {reason}; original error: {original_detail}; Article {}; attempted Version kind {:?}, title {:?}, elapsed_ms {:?}",
            request.article_id, request.kind, request.title, request.elapsed_ms
        ),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QueueArticleSnapshot<'a> {
    body: &'a str,
    title: &'a str,
    queue: Option<QueueState>,
}

#[derive(Debug, PartialEq, Eq)]
enum QueueCompletionReconciliation {
    Committed,
    NotCommitted { stopped: bool },
    Uncertain,
}

fn reconcile_queue_completion(
    current: Option<QueueArticleSnapshot<'_>>,
    original: QueueArticleSnapshot<'_>,
    generated_body: &str,
    generated_title: &str,
    stop_requested: bool,
) -> QueueCompletionReconciliation {
    let Some(current) = current else {
        return QueueCompletionReconciliation::Uncertain;
    };
    if stop_requested && current.queue.is_none() {
        return QueueCompletionReconciliation::NotCommitted { stopped: true };
    }
    if current.queue.is_none() && current.body == generated_body && current.title == generated_title
    {
        return QueueCompletionReconciliation::Committed;
    }
    if current.body == original.body && current.title == original.title {
        return QueueCompletionReconciliation::NotCommitted {
            stopped: current.queue != Some(QueueState::Writing),
        };
    }
    QueueCompletionReconciliation::Uncertain
}

fn append_error_detail(mut error: UiError, note: String) -> UiError {
    if error.detail.is_empty() {
        error.detail = note;
    } else {
        error.detail.push_str("; ");
        error.detail.push_str(&note);
    }
    error
}

fn queue_completion_uncertain_error(
    original_error: UiError,
    article_id: &str,
    version_id: &str,
    reason: String,
) -> UiError {
    let original_detail = if original_error.detail.is_empty() {
        original_error.code.to_owned()
    } else {
        format!("{}: {}", original_error.code, original_error.detail)
    };
    UiError {
        code: "queue_completion_uncertain",
        detail: format!(
            "Queue completion outcome is uncertain: {reason}; original error: {original_detail}; Article {article_id}; preserved Version {version_id}"
        ),
    }
}

fn queue_completion_cleanup_result(
    cleanup_result: Result<(), UiError>,
    original_error: UiError,
    article_id: &str,
    version_id: &str,
) -> UiError {
    match cleanup_result {
        Ok(()) => original_error,
        Err(cleanup_error) => queue_completion_uncertain_error(
            original_error,
            article_id,
            version_id,
            format!(
                "Version cleanup failed: {}: {}",
                cleanup_error.code, cleanup_error.detail
            ),
        ),
    }
}

fn cleanup_uncommitted_queued_version(
    store: &lita_store::ProfileStore<'_>,
    article_id: &str,
    version_id: &str,
    original_error: UiError,
    stopped: bool,
) -> UiError {
    let error = if stopped {
        UiError {
            code: "cancelled",
            detail: String::new(),
        }
    } else {
        original_error
    };
    queue_completion_cleanup_result(
        delete_cancelled_version(store, version_id),
        error,
        article_id,
        version_id,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArticleWriteMode {
    Interactive,
    Queue,
}

struct ArticleWriteContext<'a> {
    app: &'a AppHandle,
    token: String,
    profile_id: &'a str,
    cancel: Arc<AtomicBool>,
    working_dir: std::path::PathBuf,
}

/// Runs Codex for one article and records the result as a version and as
/// the article's current text. Shared by the v0.1 write screen and the
/// editor. Measured 6.6 s (Fast) / 10.8 s (Quality) for a short X post (D-24).
fn write_article(
    context: ArticleWriteContext<'_>,
    article_id: &str,
    mode: ArticleWriteMode,
    effort: StoredEffort,
    previous: Option<String>,
) -> Result<(Article, ArticleVersion, String), UiError> {
    let ArticleWriteContext {
        app,
        token,
        profile_id,
        cancel,
        working_dir,
    } = context;
    std::fs::create_dir_all(&working_dir).map_err(|e| UiError::unknown(e.to_string()))?;
    let state = app.state::<AppState>();
    let user = state.store.as_user(token);
    let store = user.for_profile(profile_id).map_err(fail)?;
    let article = store
        .article(article_id)
        .map_err(fail)?
        .ok_or_else(|| UiError::invalid("no such article"))?;
    if article.brief.trim().is_empty() {
        return Err(UiError::invalid("write a brief first"));
    }
    let voice_id = article
        .voice_id
        .clone()
        .ok_or_else(|| UiError::invalid("pick a voice first"))?;
    let voice = store
        .voice(&voice_id)
        .map_err(fail)?
        .ok_or_else(|| UiError::invalid("no such voice"))?;
    let platform = user
        .platforms()
        .map_err(fail)?
        .into_iter()
        .find(|platform| platform.id == article.platform_id)
        .ok_or_else(|| UiError::invalid("no such platform"))?;

    let prompt = prompt_for(
        &voice.profile,
        &article.brief,
        &rules_of(&platform),
        previous.as_deref(),
    );
    let req = Request {
        prompt: prompt.clone(),
        schema: post_schema(),
        model: None,
        effort: effort.into(),
        working_dir,
    };
    let run = CodexCli::on_path()
        .run_typed_cancellable::<PostDraft>(&req, |_| {}, Arc::clone(&cancel))
        .map_err(fail)?;

    let body = run.value.text.trim().to_string();
    let title = if run.value.title.trim().is_empty() {
        article.title.clone()
    } else {
        run.value.title.trim().to_string()
    };
    ensure_not_cancelled(&cancel)?;
    let version_request = NewVersion {
        article_id: article.id.clone(),
        kind: if previous.is_some() {
            VersionKind::Shortened
        } else {
            VersionKind::Generated
        },
        title: title.clone(),
        body: body.clone(),
        prompt_sent: Some(prompt),
        elapsed_ms: Some(run.elapsed.as_millis() as i64),
    };
    let version = match store.create_version(&version_request) {
        Ok(version) => version,
        Err(error) => {
            let original_error = fail(error);
            let versions = match store.versions(&article.id) {
                Ok(versions) => versions,
                Err(read_error) => {
                    return Err(uncertain_version_post_error(
                        original_error,
                        &version_request,
                        format!("Version reread failed: {read_error:#}"),
                    ));
                }
            };
            match versions.into_iter().find(|version| {
                version_matches_attempt_after_article_snapshot(
                    version,
                    &version_request,
                    &article.updated_at,
                )
            }) {
                Some(version) => version,
                None => {
                    return Err(uncertain_version_post_error(
                        original_error,
                        &version_request,
                        "no exact matching Version newer than the original Article snapshot was found".into(),
                    ));
                }
            }
        }
    };
    if let Err(cancelled) = ensure_not_cancelled(&cancel) {
        delete_cancelled_version(&store, &version.id)?;
        return Err(cancelled);
    }
    let mut reconciled_article = None;
    let completed = match mode {
        ArticleWriteMode::Queue => {
            match store.complete_queued_article(&article.id, &body, &title) {
                Ok(completed) => completed,
                Err(error) => {
                    let original_error = fail(error);
                    let reconciliation = {
                        let _transitions = state.queue_transitions.lock().unwrap();
                        store.article(&article.id).map(|current| {
                            current.map(|current| {
                                let stop_requested = cancel.load(Ordering::Relaxed);
                                let outcome = reconcile_queue_completion(
                                    Some(QueueArticleSnapshot {
                                        body: &current.body,
                                        title: &current.title,
                                        queue: current.queue,
                                    }),
                                    QueueArticleSnapshot {
                                        body: &article.body,
                                        title: &article.title,
                                        queue: article.queue,
                                    },
                                    &body,
                                    &title,
                                    stop_requested,
                                );
                                (current, outcome)
                            })
                        })
                    };
                    match reconciliation {
                        Ok(Some((current, QueueCompletionReconciliation::Committed))) => {
                            reconciled_article = Some(current);
                            true
                        }
                        Ok(Some((_, QueueCompletionReconciliation::NotCommitted { stopped }))) => {
                            return Err(cleanup_uncommitted_queued_version(
                                &store,
                                &article.id,
                                &version.id,
                                original_error,
                                stopped,
                            ));
                        }
                        Ok(Some((_, QueueCompletionReconciliation::Uncertain))) => {
                            return Err(queue_completion_uncertain_error(
                                original_error,
                                &article.id,
                                &version.id,
                                "Article state did not establish whether queue completion committed".into(),
                            ));
                        }
                        Ok(None) => {
                            return Err(queue_completion_uncertain_error(
                                original_error,
                                &article.id,
                                &version.id,
                                "Article was not found during completion reconciliation".into(),
                            ));
                        }
                        Err(read_error) => {
                            return Err(queue_completion_uncertain_error(
                                original_error,
                                &article.id,
                                &version.id,
                                format!("Article reread failed: {read_error:#}"),
                            ));
                        }
                    }
                }
            }
        }
        ArticleWriteMode::Interactive => {
            store
                .update_article(
                    &article.id,
                    &ArticlePatch {
                        body: Some(body),
                        title: Some(title),
                        ..Default::default()
                    },
                )
                .map_err(fail)?;
            true
        }
    };
    if !completed {
        delete_cancelled_version(&store, &version.id)?;
        return Err(UiError {
            code: "cancelled",
            detail: String::new(),
        });
    }
    let article = match reconciled_article {
        Some(article) => article,
        None => store
            .article(&article.id)
            .map_err(fail)?
            .ok_or_else(|| UiError::invalid("article vanished"))?,
    };
    Ok((article, version, run.value.voice_notes))
}

/// v0.1 write screen: brief → a new article → Codex → its first version.
/// `previous` (the over-long text) asks for a shorter rewrite of the same
/// article instead of a new one; the article is found by `article_id`.
#[tauri::command]
async fn generate_draft(
    app: AppHandle,
    voice_id: String,
    brief: String,
    platform_id: String,
    effort: StoredEffort,
    previous: Option<String>,
    article_id: Option<String>,
) -> Result<Generated, UiError> {
    validate_article_effort(effort)?;
    let brief = brief.trim().to_string();
    if brief.is_empty() {
        return Err(UiError::invalid("write a brief first"));
    }
    let (token, profile_id, cancel) = {
        let state = app.state::<AppState>();
        let (token, profile_id) = state.command_snapshot()?;
        state.generate_cancel.store(false, Ordering::Relaxed);
        (token, profile_id, Arc::clone(&state.generate_cancel))
    };
    let working_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| UiError::unknown(e.to_string()))?;

    tauri::async_runtime::spawn_blocking(move || -> Result<Generated, UiError> {
        let id = {
            let state = app.state::<AppState>();
            let user = state.store.as_user(token.clone());
            let store = user.for_profile(&profile_id).map_err(fail)?;
            match article_id {
                Some(id) => {
                    store
                        .update_article(
                            &id,
                            &ArticlePatch {
                                brief: Some(brief.clone()),
                                ..Default::default()
                            },
                        )
                        .map_err(fail)?;
                    id
                }
                None => {
                    store
                        .create_article(&NewArticle {
                            voice_id: Some(voice_id.clone()),
                            platform_id: platform_id.clone(),
                            brief: brief.clone(),
                            ..Default::default()
                        })
                        .map_err(fail)?
                        .id
                }
            }
        };
        let (article, version, voice_notes) = write_article(
            ArticleWriteContext {
                app: &app,
                token,
                profile_id: &profile_id,
                cancel,
                working_dir,
            },
            &id,
            ArticleWriteMode::Interactive,
            effort,
            previous,
        )?;
        Ok(Generated {
            draft: DraftView {
                id: article.id,
                brief_id: id,
                body: article.body,
                prompt_sent: version.prompt_sent.unwrap_or_default(),
                model: None,
                elapsed_ms: version.elapsed_ms.unwrap_or(0),
                status: "pending",
                created_at: version.created_at,
                decided_at: None,
            },
            voice_notes,
        })
    })
    .await
    .map_err(|e| UiError::unknown(e.to_string()))?
}

/// Editor: (re)write an article's text from its brief, or shorten `previous`.
#[tauri::command]
async fn generate_into_article(
    app: AppHandle,
    article_id: String,
    effort: StoredEffort,
    previous: Option<String>,
) -> Result<ArticleWritten, UiError> {
    validate_article_effort(effort)?;
    let (token, profile_id, cancel) = {
        let state = app.state::<AppState>();
        let (token, profile_id) = state.command_snapshot()?;
        state.generate_cancel.store(false, Ordering::Relaxed);
        (token, profile_id, Arc::clone(&state.generate_cancel))
    };
    let working_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| UiError::unknown(e.to_string()))?;
    tauri::async_runtime::spawn_blocking(move || {
        let (article, version, voice_notes) = write_article(
            ArticleWriteContext {
                app: &app,
                token,
                profile_id: &profile_id,
                cancel,
                working_dir,
            },
            &article_id,
            ArticleWriteMode::Interactive,
            effort,
            previous,
        )?;
        Ok(ArticleWritten {
            article,
            version,
            voice_notes,
        })
    })
    .await
    .map_err(|e| UiError::unknown(e.to_string()))?
}

#[derive(Debug, Serialize)]
pub struct ArticleWritten {
    pub article: Article,
    pub version: ArticleVersion,
    pub voice_notes: String,
}

#[tauri::command]
fn cancel_generate(state: State<'_, AppState>) {
    state.generate_cancel.store(true, Ordering::Relaxed);
}

/// v0.1 compat: the write screen's decision, applied to the article.
#[tauri::command]
async fn set_draft_status(app: AppHandle, id: String, status: String) -> Result<(), UiError> {
    let status = match status.as_str() {
        "approved" => ArticleStatus::Approved,
        "discarded" => ArticleStatus::Archived,
        _ => ArticleStatus::Draft,
    };
    update_article(
        app,
        id,
        ArticlePatch {
            status: Some(status),
            ..Default::default()
        },
    )
    .await
}

// ----- article queue (requirements 2026-09-23-article-queue) ---------------

/// What the UI hears about the queue. `Changed` says "list again";
/// `Stopped` carries the one failure that would hit every article.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum QueueEvent {
    Changed,
    Stopped { error: UiError },
}

fn emit_queue(app: &AppHandle, e: QueueEvent) {
    let _ = app.emit("queue-event", e);
}

fn stop_queue_article(
    store: &ProfileStore<'_>,
    transitions: &mut QueueTransitions,
    id: &QueueJobId,
    article_id: &str,
    status: QueueState,
) -> Result<(), UiError> {
    match transitions.stop(id, status) {
        QueueStopAction::CancelActive => {
            if store
                .stop_queued_article(article_id, status)
                .map_err(fail)?
            {
                transitions.cancel_active(id);
            }
        }
        QueueStopAction::PreserveDraft => {
            if store
                .stop_queued_article(article_id, status)
                .map_err(fail)?
            {
                transitions.forget_recovered(id);
            }
        }
        QueueStopAction::Delete => {
            store.delete_article(article_id).map_err(fail)?;
        }
    }
    Ok(())
}

/// Errors that stop the queue instead of advancing to another Article.
fn stops_the_queue(code: &str) -> bool {
    matches!(
        code,
        "codex_not_logged_in"
            | "codex_not_installed"
            | "codex_config_broken"
            | "queue_completion_uncertain"
            | "codex_quota"
            | "not_signed_in"
            | "session_expired"
            | "network"
    )
}

fn queue_error_mark(error: &UiError) -> Option<QueueState> {
    match error.code {
        "cancelled" => None,
        "queue_completion_uncertain" => Some(QueueState::Failed),
        code if stops_the_queue(code) => Some(QueueState::Waiting),
        _ => Some(QueueState::Failed),
    }
}
fn validate_article_effort(effort: StoredEffort) -> Result<(), UiError> {
    if effort == StoredEffort::Quality {
        Ok(())
    } else {
        Err(UiError::invalid(
            "Article generation always uses Quality effort",
        ))
    }
}

/// The material title suggestions and the policy draft are read from: the
/// head of every voice's sources, and what has already been written.
fn topic_material(
    store: &lita_store::ProfileStore<'_>,
) -> Result<(Vec<String>, Vec<Existing>, Lang), UiError> {
    let mut samples = Vec::new();
    let mut lang = Lang::Ja;
    for v in store.voices().map_err(fail)? {
        if let Some(voice) = store.voice(&v.id).map_err(fail)? {
            lang = Lang::of(&voice.profile.language);
            samples.extend(voice.sources.into_iter().map(|s| s.body));
        }
    }
    let existing = store
        .articles(None)
        .map_err(fail)?
        .into_iter()
        .take(50)
        .map(|a| Existing {
            title: a.title,
            first_line: a.excerpt,
        })
        .collect();
    Ok((samples, existing, lang))
}

#[tauri::command]
async fn get_policy(app: AppHandle) -> Result<Policy, UiError> {
    let (token, profile_id) = app.state::<AppState>().command_snapshot()?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let user = state.store.as_user(token);
        let store = user.for_profile(&profile_id).map_err(fail)?;
        store
            .settings()
            .map(|settings| settings.policy)
            .map_err(fail)
    })
    .await
    .map_err(|e| UiError::unknown(e.to_string()))?
}

#[tauri::command]
async fn set_policy(app: AppHandle, policy: Policy) -> Result<(), UiError> {
    let (token, profile_id) = app.state::<AppState>().command_snapshot()?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let user = state.store.as_user(token);
        let store = user.for_profile(&profile_id).map_err(fail)?;
        store.set_policy(&policy).map_err(fail)
    })
    .await
    .map_err(|e| UiError::unknown(e.to_string()))?
}

/// Codex drafts the four fields from the person's writing. Not saved: the
/// person edits it on screen and saves with `set_policy`.
#[tauri::command]
async fn draft_policy(app: AppHandle) -> Result<Policy, UiError> {
    let (token, profile_id) = app.state::<AppState>().command_snapshot()?;
    let working_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| UiError::unknown(e.to_string()))?;
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::create_dir_all(&working_dir).map_err(|e| UiError::unknown(e.to_string()))?;
        let state = app.state::<AppState>();
        let user = state.store.as_user(token);
        let store = user.for_profile(&profile_id).map_err(fail)?;
        let (samples, existing, lang) = topic_material(&store)?;
        if samples.is_empty() && existing.is_empty() {
            return Err(UiError::invalid("nothing to read yet"));
        }
        let refs: Vec<&str> = samples.iter().map(String::as_str).collect();
        let req = Request {
            prompt: topics::policy_prompt(&refs, &existing, lang),
            schema: topics::policy_schema(),
            model: None,
            effort: lita_engine::Effort::Quality,
            working_dir,
        };
        let run = CodexCli::on_path()
            .run_typed::<Policy>(&req, |_| {})
            .map_err(fail)?;
        let mut policy = run.value;
        policy.topics.clear();
        Ok(policy)
    })
    .await
    .map_err(|e| UiError::unknown(e.to_string()))?
}

/// A brief for one article, drafted from its title (#97). Not saved: the
/// person edits it in the editor and the autosave keeps it.
#[tauri::command]
async fn suggest_brief(app: AppHandle, article_id: String) -> Result<String, UiError> {
    let (token, profile_id) = app.state::<AppState>().command_snapshot()?;
    let working_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| UiError::unknown(e.to_string()))?;
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::create_dir_all(&working_dir).map_err(|e| UiError::unknown(e.to_string()))?;
        let state = app.state::<AppState>();
        let user = state.store.as_user(token);
        let store = user.for_profile(&profile_id).map_err(fail)?;
        let article = store
            .article(&article_id)
            .map_err(fail)?
            .ok_or_else(|| UiError::invalid("no such article"))?;
        if article.title.trim().is_empty() {
            return Err(UiError::invalid("write a title first"));
        }
        let policy = store.settings().map_err(fail)?.policy;
        let (samples, existing, lang) = topic_material(&store)?;
        let existing: Vec<Existing> = existing
            .into_iter()
            .filter(|e| e.title.trim() != article.title.trim())
            .collect();
        let refs: Vec<&str> = samples.iter().map(String::as_str).collect();
        let req = Request {
            prompt: topics::brief_prompt(&article.title, &policy, &refs, &existing, lang),
            schema: topics::brief_schema(),
            model: None,
            effort: lita_engine::Effort::Quality,
            working_dir,
        };
        // Stopped by the same `cancel_generate` the editor already has.
        state.generate_cancel.store(false, Ordering::Relaxed);
        let cancel = Arc::clone(&state.generate_cancel);
        let run = CodexCli::on_path()
            .run_typed_cancellable::<SuggestedBrief>(&req, |_| {}, cancel)
            .map_err(fail)?;
        Ok(run.value.brief.trim().to_string())
    })
    .await
    .map_err(|e| UiError::unknown(e.to_string()))?
}

/// The cloud as last gathered, plus whether there is more material since.
#[derive(Debug, Serialize)]
pub struct CloudView {
    pub cloud: Option<TopicCloud>,
    /// The person's own writing now; the screen compares with `cloud.material_count`.
    pub material_count: usize,
}

/// Only the person's own writing counts (#129): the sources of every voice
/// and the articles they edited. An article Lita queued and wrote is not
/// more of their words.
fn material_count(
    store: &lita_store::ProfileStore<'_>,
    articles: &[ArticleText],
) -> Result<usize, UiError> {
    let sources = store
        .voices()
        .map_err(fail)?
        .iter()
        .map(|v| v.source_count.max(0) as usize)
        .sum();
    Ok(topics::material_count(sources, articles))
}

#[tauri::command]
async fn get_topic_cloud(app: AppHandle) -> Result<CloudView, UiError> {
    let (token, profile_id) = app.state::<AppState>().command_snapshot()?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let user = state.store.as_user(token);
        let store = user.for_profile(&profile_id).map_err(fail)?;
        let mut cloud = store.settings().map_err(fail)?.topic_cloud;
        let articles = store.article_texts().map_err(fail)?;
        // Saved under an older way of counting: counted again as the writing
        // it was gathered from, without gathering again, and saved so it is
        // counted once and later edits make "more since".
        if let Some(c) = cloud
            .as_mut()
            .filter(|c| c.counting < topics::CLOUD_COUNTING)
        {
            let edited = articles.iter().filter(|a| a.edited_by_person()).count();
            if topics::recount(c, &store.source_times().map_err(fail)?, edited) {
                store.set_topic_cloud(c).map_err(fail)?;
            }
        }
        Ok(CloudView {
            material_count: material_count(&store, &articles)?,
            cloud,
        })
    })
    .await
    .map_err(|e| UiError::unknown(e.to_string()))?
}

/// Codex gathers the words the person keeps writing about, and the result
/// is saved as the one cloud (requirement 6). Stopped by `cancel_generate`.
#[tauri::command]
async fn gather_topic_cloud(app: AppHandle) -> Result<CloudView, UiError> {
    let (token, profile_id) = app.state::<AppState>().command_snapshot()?;
    let working_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| UiError::unknown(e.to_string()))?;
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::create_dir_all(&working_dir).map_err(|e| UiError::unknown(e.to_string()))?;
        let state = app.state::<AppState>();
        let user = state.store.as_user(token);
        let store = user.for_profile(&profile_id).map_err(fail)?;
        let policy = store.settings().map_err(fail)?.policy;
        let (samples, existing, lang) = topic_material(&store)?;
        if samples.is_empty() && existing.is_empty() {
            return Err(UiError::invalid("nothing to read yet"));
        }
        // The bodies the person edited reach the cloud only (#129), after
        // the writing behind the voices.
        let articles = store.article_texts().map_err(fail)?;
        let mut refs: Vec<&str> = samples.iter().map(String::as_str).collect();
        refs.extend(topics::own_bodies(&articles));
        let req = Request {
            prompt: topics::cloud_prompt(&policy, &refs, &existing, lang),
            schema: topics::cloud_schema(),
            model: None,
            effort: lita_engine::Effort::Quality,
            working_dir,
        };
        state.generate_cancel.store(false, Ordering::Relaxed);
        let cancel = Arc::clone(&state.generate_cancel);
        let run = CodexCli::on_path()
            .run_typed_cancellable::<GatheredCloud>(&req, |_| {}, cancel)
            .map_err(fail)?;
        let material_count = material_count(&store, &articles)?;
        let cloud = TopicCloud {
            words: topics::tidy_cloud(run.value.words),
            gathered_at: chrono_now(),
            material_count,
            counting: topics::CLOUD_COUNTING,
        };
        store.set_topic_cloud(&cloud).map_err(fail)?;
        Ok(CloudView {
            cloud: Some(cloud),
            material_count,
        })
    })
    .await
    .map_err(|e| UiError::unknown(e.to_string()))?
}

/// RFC 3339 without pulling in a date crate: seconds since the epoch is
/// enough for "when was this gathered".
fn chrono_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    secs.to_string()
}

/// Ten titles the person could write next, none already written.
#[tauri::command]
async fn suggest_topics(
    app: AppHandle,
    subjects: Vec<String>,
    direction: String,
) -> Result<Vec<String>, UiError> {
    const N: usize = 10;
    let (token, profile_id) = app.state::<AppState>().command_snapshot()?;
    let working_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| UiError::unknown(e.to_string()))?;
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::create_dir_all(&working_dir).map_err(|e| UiError::unknown(e.to_string()))?;
        let state = app.state::<AppState>();
        let user = state.store.as_user(token);
        let store = user.for_profile(&profile_id).map_err(fail)?;
        let policy = store.settings().map_err(fail)?.policy;
        let (samples, existing, lang) = topic_material(&store)?;
        let refs: Vec<&str> = samples.iter().map(String::as_str).collect();
        // Ask for a few more than shown, so dropping repeats still leaves ten.
        let req = Request {
            prompt: topics::topics_prompt(
                &policy,
                &refs,
                &existing,
                &subjects,
                &direction,
                N + 4,
                lang,
            ),
            schema: topics::topics_schema(),
            model: None,
            effort: lita_engine::Effort::Quality,
            working_dir,
        };
        let run = CodexCli::on_path()
            .run_typed::<Topics>(&req, |_| {})
            .map_err(fail)?;
        Ok(topics::dedupe(run.value.topics, &existing, N))
    })
    .await
    .map_err(|e| UiError::unknown(e.to_string()))?
}

/// One draft per title, brief filled from the policy, queued in the order
/// given. Starts the worker. Returns the new rows as the list shows them.
#[tauri::command]
async fn enqueue_articles(
    app: AppHandle,
    titles: Vec<String>,
    voice_id: String,
    platform_id: String,
    effort: StoredEffort,
    subjects: Vec<String>,
    direction: String,
) -> Result<Vec<ArticleSummary>, UiError> {
    let titles: Vec<String> = titles
        .into_iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();
    if titles.is_empty() || titles.len() > 5 {
        return Err(UiError::invalid("pick one to five titles"));
    }
    validate_article_effort(effort)?;
    let (token, profile_id) = app.state::<AppState>().command_snapshot()?;
    let created = {
        let app = app.clone();
        tauri::async_runtime::spawn_blocking(move || -> Result<Vec<ArticleSummary>, UiError> {
            let state = app.state::<AppState>();
            let user = state.store.as_user(token);
            let store = user.for_profile(&profile_id).map_err(fail)?;
            let policy = store.settings().map_err(fail)?.policy;
            let lang = store
                .voice(&voice_id)
                .map_err(fail)?
                .map(|v| Lang::of(&v.profile.language))
                .unwrap_or(Lang::Ja);
            let mut out = Vec::new();
            for title in titles {
                let a = store
                    .create_article(&NewArticle {
                        voice_id: Some(voice_id.clone()),
                        platform_id: platform_id.clone(),
                        title: title.clone(),
                        brief: topics::brief_for(&title, &policy, &subjects, &direction, lang),
                        queue: Some(QueueState::Waiting),
                        ..Default::default()
                    })
                    .map_err(fail)?;
                out.push(ArticleSummary {
                    id: a.id,
                    voice_id: a.voice_id,
                    platform_id: a.platform_id,
                    title: a.title,
                    excerpt: String::new(),
                    status: a.status,
                    queue: a.queue,
                    created_at: a.created_at,
                    updated_at: a.updated_at,
                });
            }
            Ok(out)
        })
        .await
        .map_err(|e| UiError::unknown(e.to_string()))??
    };
    emit_queue(&app, QueueEvent::Changed);
    start_queue(app);
    Ok(created)
}

/// Runs the account-wide queue until nothing is waiting.
/// Starts requested while this worker is running are drained before it idles.
#[tauri::command]
fn start_queue(app: AppHandle) {
    let state = app.state::<AppState>();
    if !state.queue_worker.lock().unwrap().request_start() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        loop {
            let outcome = run_queue(&app);
            let succeeded = outcome.is_ok();
            let keep_running = app
                .state::<AppState>()
                .queue_worker
                .lock()
                .unwrap()
                .finish_run(succeeded);
            if let Err(error) = outcome {
                emit_queue(&app, QueueEvent::Stopped { error });
            }
            if !keep_running {
                break;
            }
        }
    });
}

fn run_queue(app: &AppHandle) -> Result<(), UiError> {
    let state = app.state::<AppState>();
    let working_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| UiError::unknown(e.to_string()))?;
    let mut first = true;
    loop {
        let token = state.access_token()?;
        let store = state.store.as_user(token.clone());
        let queued = store.queued().map_err(fail)?;
        if first {
            first = false;
            for_each_stale_writing_article(queued.iter().cloned(), |article| {
                let id = QueueJobId::new(&article.profile_id, &article.id);
                let profile_store = store.for_profile(&article.profile_id).map_err(fail)?;
                let mut transitions = state.queue_transitions.lock().unwrap();
                let current = profile_store.article(&article.id).map_err(fail)?;
                let status = current.as_ref().and_then(|current| current.queue);
                let is_active = transitions.is_active(&id);
                let versions = if status == Some(QueueState::Writing) && !is_active {
                    match current.as_ref() {
                        Some(_) => profile_store.versions(&article.id).map_err(fail)?,
                        None => Vec::new(),
                    }
                } else {
                    Vec::new()
                };
                match stale_writing_recovery_action(status, is_active, current.as_ref(), &versions)
                {
                    StaleWritingRecoveryAction::Skip => {}
                    StaleWritingRecoveryAction::ClearQueue => {
                        if profile_store
                            .stop_queued_article(&article.id, QueueState::Writing)
                            .map_err(fail)?
                        {
                            transitions.forget_recovered(&id);
                            emit_queue(app, QueueEvent::Changed);
                        }
                    }
                    StaleWritingRecoveryAction::FailUncertain => {
                        if profile_store
                            .fail_queued_article_if_writing(&article.id)
                            .map_err(fail)?
                        {
                            transitions.forget_recovered(&id);
                            eprintln!(
                                "stale Writing Article {} has an unmatched Generated or Shortened Version; preserved the Version and marked the Article Failed",
                                article.id
                            );
                            emit_queue(app, QueueEvent::Changed);
                        }
                    }
                    StaleWritingRecoveryAction::Requeue => {
                        if transitions.recover_stale_writing(&id, status)
                            && let Err(error) = profile_store.update_article(
                                &article.id,
                                &ArticlePatch {
                                    queue: Some(Some(QueueState::Waiting)),
                                    ..Default::default()
                                },
                            )
                        {
                            transitions.forget_recovered(&id);
                            return Err(fail(error));
                        }
                    }
                }
                Ok(())
            })?;
        }
        let Some(next) = queued
            .into_iter()
            .find(|article| article.queue != Some(QueueState::Failed))
        else {
            return Ok(());
        };
        // Codex problems hit every article alike: say so once and stop.
        match CodexCli::on_path().preflight().map_err(fail)? {
            Preflight::Ready { .. } => {}
            Preflight::NotInstalled => {
                return Err(UiError {
                    code: "codex_not_installed",
                    detail: String::new(),
                });
            }
            Preflight::NotLoggedIn { .. } => {
                return Err(UiError {
                    code: "codex_not_logged_in",
                    detail: String::new(),
                });
            }
            Preflight::ConfigBroken { detail, .. } => {
                return Err(UiError {
                    code: "codex_config_broken",
                    detail,
                });
            }
        }
        let profile_store = store.for_profile(&next.profile_id).map_err(fail)?;
        let id = QueueJobId::new(&next.profile_id, &next.id);
        let start_outcome = {
            let mut transitions = state.queue_transitions.lock().unwrap();
            let current = profile_store.article(&next.id).map_err(fail)?;
            if current.as_ref().and_then(|article| article.queue) != Some(QueueState::Waiting) {
                continue;
            }
            let Some(cancel) = transitions.begin(id.clone()) else {
                continue;
            };
            match profile_store.update_article(
                &next.id,
                &ArticlePatch {
                    queue: Some(Some(QueueState::Writing)),
                    ..Default::default()
                },
            ) {
                Ok(()) => {
                    transitions.forget_recovered(&id);
                    QueueStartOutcome::Started(cancel)
                }
                Err(transition_error) => {
                    let transition_error = fail(transition_error);
                    let reread = profile_store.article(&next.id);
                    let current_status = reread
                        .as_ref()
                        .map(|current| current.as_ref().and_then(|article| article.queue))
                        .map_err(|_| ());
                    let action =
                        queue_start_transition_action(current_status, transition_error.code);
                    match (action, reread) {
                        (QueueStartTransitionAction::Proceed, Ok(_)) => {
                            transitions.forget_recovered(&id);
                            QueueStartOutcome::Started(cancel)
                        }
                        (QueueStartTransitionAction::Stop, Ok(_)) => {
                            transitions.finish(&id);
                            return Err(append_error_detail(
                                transition_error,
                                format!(
                                    "Article {} remains Waiting after queue-start transition failed; the queue worker stopped",
                                    next.id
                                ),
                            ));
                        }
                        (QueueStartTransitionAction::MarkFailed, Ok(_)) => {
                            match profile_store.fail_queued_article_if_waiting(&next.id) {
                                Ok(true) => {
                                    transitions.finish(&id);
                                    transitions.forget_recovered(&id);
                                    QueueStartOutcome::MarkedFailed
                                }
                                Ok(false) => {
                                    transitions.finish(&id);
                                    return Err(append_error_detail(
                                        transition_error,
                                        format!(
                                            "Article {} no longer matched Waiting while marking a queue-start failure",
                                            next.id
                                        ),
                                    ));
                                }
                                Err(mark_error) => {
                                    transitions.finish(&id);
                                    let mark_error = fail(mark_error);
                                    return Err(append_error_detail(
                                        mark_error,
                                        format!(
                                            "Queue-start PATCH for Article {} also failed: {}: {}",
                                            next.id, transition_error.code, transition_error.detail
                                        ),
                                    ));
                                }
                            }
                        }
                        (QueueStartTransitionAction::Skip, Ok(_)) => {
                            transitions.finish(&id);
                            QueueStartOutcome::Skip
                        }
                        (QueueStartTransitionAction::RereadFailed, Err(reread_error)) => {
                            transitions.finish(&id);
                            return Err(append_error_detail(
                                transition_error,
                                format!(
                                    "Article {} reread failed after queue-start transition error: {reread_error:#}",
                                    next.id
                                ),
                            ));
                        }
                        (_, _) => {
                            transitions.finish(&id);
                            return Err(append_error_detail(
                                transition_error,
                                format!(
                                    "Article {} queue-start reread could not be classified",
                                    next.id
                                ),
                            ));
                        }
                    }
                }
            }
        };
        let cancel = match start_outcome {
            QueueStartOutcome::Started(cancel) => cancel,
            QueueStartOutcome::Skip => continue,
            QueueStartOutcome::MarkedFailed => {
                emit_queue(app, QueueEvent::Changed);
                continue;
            }
        };
        emit_queue(app, QueueEvent::Changed);

        let effort = StoredEffort::Quality;
        let result = write_article(
            ArticleWriteContext {
                app,
                token,
                profile_id: &next.profile_id,
                cancel,
                working_dir: working_dir.clone(),
            },
            &next.id,
            ArticleWriteMode::Queue,
            effort,
            None,
        );
        let mark = match &result {
            Ok(_) => None,
            Err(error) => queue_error_mark(error),
        };
        // Serialize the final status write with stop/clear and never resurrect a cleared row.
        let transition_result = {
            let mut transitions = state.queue_transitions.lock().unwrap();
            let update = match profile_store.article(&next.id) {
                Ok(Some(article)) if article.queue == Some(QueueState::Writing) => profile_store
                    .update_article(
                        &next.id,
                        &ArticlePatch {
                            queue: Some(mark),
                            ..Default::default()
                        },
                    ),
                Ok(_) => Ok(()),
                Err(error) => Err(error),
            };
            transitions.finish(&id);
            update
        };
        if let Err(finalization_error) = transition_result {
            let finalization_error = fail(finalization_error);
            if let Err(original_error) = &result
                && original_error.code == "queue_completion_uncertain"
            {
                return Err(append_error_detail(
                    original_error.clone(),
                    format!(
                        "Final queue status reconciliation failed: {}: {}",
                        finalization_error.code, finalization_error.detail
                    ),
                ));
            }
            return Err(finalization_error);
        }
        emit_queue(app, QueueEvent::Changed);
        if let Err(error) = result
            && stops_the_queue(error.code)
        {
            return Err(error);
        }
    }
}

/// Waiting: the article is deleted (its body is empty; nothing is lost).
/// Writing: stopped; the empty draft stays. Failed: deleted.
#[tauri::command]
async fn dequeue_article(app: AppHandle, id: String) -> Result<(), UiError> {
    let (token, profile_id) = app.state::<AppState>().command_snapshot()?;
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<(), UiError> {
        let state = app2.state::<AppState>();
        let user = state.store.as_user(token);
        let store = user.for_profile(&profile_id).map_err(fail)?;
        let mut transitions = state.queue_transitions.lock().unwrap();
        let article = store
            .article(&id)
            .map_err(fail)?
            .ok_or_else(|| UiError::invalid("no such article"))?;
        if let Some(status) = article.queue {
            let key = QueueJobId::new(&profile_id, &article.id);
            stop_queue_article(&store, &mut transitions, &key, &article.id, status)?;
        }
        Ok(())
    })
    .await
    .map_err(|e| UiError::unknown(e.to_string()))??;
    emit_queue(&app, QueueEvent::Changed);
    Ok(())
}

/// Everything waiting is deleted and the one being written is stopped.
#[tauri::command]
async fn clear_queue(app: AppHandle) -> Result<(), UiError> {
    let (token, profile_id) = app.state::<AppState>().command_snapshot()?;
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<(), UiError> {
        let state = app2.state::<AppState>();
        let user = state.store.as_user(token);
        let store = user.for_profile(&profile_id).map_err(fail)?;
        let mut transitions = state.queue_transitions.lock().unwrap();
        for article in store.queued().map_err(fail)? {
            let Some(status @ (QueueState::Writing | QueueState::Waiting)) = article.queue else {
                continue;
            };
            let key = QueueJobId::new(&profile_id, &article.id);
            stop_queue_article(&store, &mut transitions, &key, &article.id, status)?;
        }
        Ok(())
    })
    .await
    .map_err(|e| UiError::unknown(e.to_string()))??;
    emit_queue(&app, QueueEvent::Changed);
    Ok(())
}

// ----- articles -------------------------------------------------------------

#[tauri::command]
async fn list_articles(
    app: AppHandle,
    status: Option<ArticleStatus>,
) -> Result<Vec<ArticleSummary>, UiError> {
    let (token, profile_id) = app.state::<AppState>().command_snapshot()?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let user = state.store.as_user(token);
        user.for_profile(&profile_id)
            .map_err(fail)?
            .articles(status)
            .map_err(fail)
    })
    .await
    .map_err(|e| UiError::unknown(e.to_string()))?
}

#[tauri::command]
async fn get_article(app: AppHandle, id: String) -> Result<Option<Article>, UiError> {
    let (token, profile_id) = app.state::<AppState>().command_snapshot()?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let user = state.store.as_user(token);
        user.for_profile(&profile_id)
            .map_err(fail)?
            .article(&id)
            .map_err(fail)
    })
    .await
    .map_err(|e| UiError::unknown(e.to_string()))?
}

#[tauri::command]
async fn update_article(app: AppHandle, id: String, patch: ArticlePatch) -> Result<(), UiError> {
    let (token, profile_id) = app.state::<AppState>().command_snapshot()?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let user = state.store.as_user(token);
        user.for_profile(&profile_id)
            .map_err(fail)?
            .update_article(&id, &patch)
            .map_err(fail)
    })
    .await
    .map_err(|e| UiError::unknown(e.to_string()))?
}

#[tauri::command]
async fn delete_article(app: AppHandle, id: String) -> Result<bool, UiError> {
    let (token, profile_id) = app.state::<AppState>().command_snapshot()?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let user = state.store.as_user(token);
        user.for_profile(&profile_id)
            .map_err(fail)?
            .delete_article(&id)
            .map_err(fail)
    })
    .await
    .map_err(|e| UiError::unknown(e.to_string()))?
}

#[tauri::command]
async fn list_versions(app: AppHandle, article_id: String) -> Result<Vec<ArticleVersion>, UiError> {
    let (token, profile_id) = app.state::<AppState>().command_snapshot()?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let user = state.store.as_user(token);
        user.for_profile(&profile_id)
            .map_err(fail)?
            .versions(&article_id)
            .map_err(fail)
    })
    .await
    .map_err(|e| UiError::unknown(e.to_string()))?
}

/// Snapshots the article's current text (`edited`, or `manual` when the
/// person asked for it). Skipped when the newest version already has this
/// text, so periodic snapshots never pile up identical rows.
#[tauri::command]
async fn snapshot_article(
    app: AppHandle,
    article_id: String,
    manual: bool,
) -> Result<Option<ArticleVersion>, UiError> {
    let (token, profile_id) = app.state::<AppState>().command_snapshot()?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let user = state.store.as_user(token);
        let store = user.for_profile(&profile_id).map_err(fail)?;
        let article = store
            .article(&article_id)
            .map_err(fail)?
            .ok_or_else(|| UiError::invalid("no such article"))?;
        let newest = store
            .versions(&article_id)
            .map_err(fail)?
            .into_iter()
            .next();
        if !manual
            && newest
                .as_ref()
                .is_some_and(|v| v.body == article.body && v.title == article.title)
        {
            return Ok(None);
        }
        store
            .create_version(&NewVersion {
                article_id,
                kind: if manual {
                    VersionKind::Manual
                } else {
                    VersionKind::Edited
                },
                title: article.title,
                body: article.body,
                prompt_sent: None,
                elapsed_ms: None,
            })
            .map(Some)
            .map_err(fail)
    })
    .await
    .map_err(|e| UiError::unknown(e.to_string()))?
}

/// Makes an earlier version the current text. The state being left is kept
/// as an `edited` snapshot first, and the restore itself is recorded as a
/// `restored` version, so nothing is lost and history stays linear.
#[tauri::command]
async fn restore_version(app: AppHandle, version_id: String) -> Result<Article, UiError> {
    let (token, profile_id) = app.state::<AppState>().command_snapshot()?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let user = state.store.as_user(token);
        let store = user.for_profile(&profile_id).map_err(fail)?;
        let version = store
            .version(&version_id)
            .map_err(fail)?
            .ok_or_else(|| UiError::invalid("no such version"))?;
        let article = store
            .article(&version.article_id)
            .map_err(fail)?
            .ok_or_else(|| UiError::invalid("no such article"))?;
        if article.body != version.body || article.title != version.title {
            let newest = store
                .versions(&article.id)
                .map_err(fail)?
                .into_iter()
                .next();
            if !newest
                .as_ref()
                .is_some_and(|v| v.body == article.body && v.title == article.title)
            {
                store
                    .create_version(&NewVersion {
                        article_id: article.id.clone(),
                        kind: VersionKind::Edited,
                        title: article.title.clone(),
                        body: article.body.clone(),
                        prompt_sent: None,
                        elapsed_ms: None,
                    })
                    .map_err(fail)?;
            }
        }
        store
            .create_version(&NewVersion {
                article_id: article.id.clone(),
                kind: VersionKind::Restored,
                title: version.title.clone(),
                body: version.body.clone(),
                prompt_sent: None,
                elapsed_ms: None,
            })
            .map_err(fail)?;
        store
            .update_article(
                &article.id,
                &ArticlePatch {
                    title: Some(version.title),
                    body: Some(version.body),
                    ..Default::default()
                },
            )
            .map_err(fail)?;
        store
            .article(&article.id)
            .map_err(fail)?
            .ok_or_else(|| UiError::invalid("article vanished"))
    })
    .await
    .map_err(|e| UiError::unknown(e.to_string()))?
}

// ----- sources (D-43) ---------------------------------------------------------

/// Progress of a service import, for the row's loading state (#83).
/// `total` is unknown until the listing has arrived (Medium never knows).
#[derive(Debug, Clone, Serialize)]
pub struct ImportProgress {
    pub kind: &'static str,
    pub done: usize,
    pub total: Option<usize>,
}

/// What an import found, beyond the pieces themselves.
#[derive(Debug, Serialize)]
pub struct Imported {
    pub pieces: Vec<Piece>,
    /// Everything the account has published, when the source reports it.
    pub total: Option<u64>,
    /// Paid articles that were left out.
    pub skipped_paid: usize,
    /// Whether the source only exposes its most recent items.
    pub recent_only: bool,
}

/// Up to twenty: D-20 measured that a Voice is stable from four pieces and
/// twenty mostly adds vocabulary.
const IMPORT_MAX: usize = 20;

#[tauri::command]
async fn import_note(app: AppHandle, account: String) -> Result<Imported, UiError> {
    tauri::async_runtime::spawn_blocking(move || {
        let emitter = app.clone();
        let (listing, pieces) =
            lita_sources::note::import_with(&account, IMPORT_MAX, |done, total| {
                let _ = emitter.emit(
                    "import-progress",
                    ImportProgress {
                        kind: "note",
                        done,
                        total: Some(total),
                    },
                );
            })
            .map_err(fail)?;
        Ok(Imported {
            pieces,
            total: Some(listing.total_count),
            skipped_paid: listing.skipped_paid,
            recent_only: false,
        })
    })
    .await
    .map_err(|e| UiError::unknown(e.to_string()))?
}

#[tauri::command]
async fn import_medium(app: AppHandle, handle: String) -> Result<Imported, UiError> {
    tauri::async_runtime::spawn_blocking(move || {
        let _ = app.emit(
            "import-progress",
            ImportProgress {
                kind: "medium",
                done: 0,
                total: None,
            },
        );
        let pieces = lita_sources::medium::import(&handle).map_err(fail)?;
        let _ = app.emit(
            "import-progress",
            ImportProgress {
                kind: "medium",
                done: pieces.len(),
                total: Some(pieces.len()),
            },
        );
        Ok(Imported {
            pieces,
            total: None,
            skipped_paid: 0,
            recent_only: true,
        })
    })
    .await
    .map_err(|e| UiError::unknown(e.to_string()))?
}

/// `contents` is the text of `data/tweets.js` from the archive, read by
/// the UI. Newest posts first, capped like the other imports.
#[tauri::command]
async fn import_x_archive(
    contents: String,
    handle: Option<String>,
    include_replies: bool,
) -> Result<Imported, UiError> {
    tauri::async_runtime::spawn_blocking(move || {
        let opts = lita_sources::x::Options {
            include_replies,
            ..Default::default()
        };
        let mut pieces =
            lita_sources::x::parse_tweets_js(&contents, handle.as_deref(), opts).map_err(fail)?;
        let total = pieces.len() as u64;
        pieces.truncate(IMPORT_MAX);
        Ok(Imported {
            pieces,
            total: Some(total),
            skipped_paid: 0,
            recent_only: false,
        })
    })
    .await
    .map_err(|e| UiError::unknown(e.to_string()))?
}

/// A pasted conversation, read on this machine only (D-77): who said what,
/// so the screen keeps the person's own messages. `known` are bodies added
/// from conversations before. Nothing is stored and nothing goes to Codex.
#[tauri::command]
async fn read_talk(
    text: String,
    known: Vec<String>,
) -> Result<lita_sources::talk::Reading, UiError> {
    tauri::async_runtime::spawn_blocking(move || lita_sources::talk::read(&text, &known))
        .await
        .map_err(|e| UiError::unknown(e.to_string()))
}

// ----- updates (#25) -------------------------------------------------------

/// An update the updater found, kept until the person decides to install it.
#[derive(Default)]
pub struct PendingUpdate(Mutex<Option<tauri_plugin_updater::Update>>);

#[derive(Debug, Serialize)]
pub struct UpdateInfo {
    pub version: String,
    pub current_version: String,
    pub notes: Option<String>,
    pub date: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(tag = "event", content = "data", rename_all = "snake_case")]
pub enum DownloadEvent {
    Started {
        content_length: Option<u64>,
    },
    Progress {
        downloaded: usize,
        content_length: Option<u64>,
    },
    Finished,
}

/// Asks the release feed whether a newer version exists. `None` means this
/// is the latest. Errors are ordinary UiErrors (network etc.).
#[tauri::command]
async fn fetch_update(
    app: AppHandle,
    pending: State<'_, PendingUpdate>,
) -> Result<Option<UpdateInfo>, UiError> {
    let update = app
        .updater()
        .map_err(|e| UiError::unknown(e.to_string()))?
        .check()
        .await
        .map_err(|e| UiError::unknown(format!("checking for updates: {e}")))?;
    let info = update.as_ref().map(|u| UpdateInfo {
        version: u.version.clone(),
        current_version: u.current_version.clone(),
        notes: u.body.clone(),
        date: u.date.map(|d| d.to_string()),
    });
    *pending
        .0
        .lock()
        .map_err(|_| UiError::unknown("update state poisoned"))? = update;
    Ok(info)
}

/// Downloads and installs the pending update, reporting progress on
/// `on_event`. The app must be restarted afterwards (`restart_app`).
#[tauri::command]
async fn install_update(
    pending: State<'_, PendingUpdate>,
    on_event: tauri::ipc::Channel<DownloadEvent>,
) -> Result<(), UiError> {
    let update = pending
        .0
        .lock()
        .map_err(|_| UiError::unknown("update state poisoned"))?
        .take()
        .ok_or_else(|| UiError::invalid("no update is pending; check first"))?;
    let mut downloaded = 0usize;
    let mut started = false;
    update
        .download_and_install(
            |chunk, total| {
                if !started {
                    started = true;
                    let _ = on_event.send(DownloadEvent::Started {
                        content_length: total,
                    });
                }
                downloaded += chunk;
                let _ = on_event.send(DownloadEvent::Progress {
                    downloaded,
                    content_length: total,
                });
            },
            || {
                let _ = on_event.send(DownloadEvent::Finished);
            },
        )
        .await
        .map_err(|e| UiError::unknown(format!("installing the update: {e}")))?;
    Ok(())
}

#[tauri::command]
fn restart_app(app: AppHandle) {
    app.restart();
}

#[tauri::command]
fn app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // GUI apps do not inherit the shell's PATH; without this `codex` (npm,
    // Homebrew) is invisible on a distributed build.
    let _ = fix_path_env::fix();
    let state = AppState::new().expect("Supabase configuration is valid");
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_process::init())
        .manage(state)
        .manage(PendingUpdate::default())
        .setup(|app| {
            #[cfg(desktop)]
            app.handle()
                .plugin(tauri_plugin_updater::Builder::new().build())?;

            // The menu bar. On macOS the first submenu is the app menu; the
            // Edit items are the predefined ones so text fields keep their
            // shortcuts. "Check for updates" is the one custom item (#25).
            let check =
                MenuItemBuilder::with_id("check_update", "アップデートを確認…").build(app)?;
            let app_menu = SubmenuBuilder::new(app, "Lita")
                .item(&PredefinedMenuItem::about(app, None, None)?)
                .separator()
                .item(&check)
                .separator()
                .services()
                .separator()
                .hide()
                .hide_others()
                .show_all()
                .separator()
                .quit()
                .build()?;
            let edit = SubmenuBuilder::new(app, "編集")
                .undo()
                .redo()
                .separator()
                .cut()
                .copy()
                .paste()
                .select_all()
                .build()?;
            let window = SubmenuBuilder::new(app, "ウインドウ")
                .minimize()
                .maximize()
                .separator()
                .close_window()
                .build()?;
            let menu = MenuBuilder::new(app)
                .items(&[&app_menu, &edit, &window])
                .build()?;
            app.set_menu(menu)?;
            app.on_menu_event(|handle, event| {
                if event.id().0 == "check_update" {
                    let _ = handle.emit("check-update", ());
                }
            });

            // Off the main thread: the refresh is a network round trip.
            let handle = app.handle().clone();
            let restore_generation = handle.state::<AppState>().begin_restore();
            tauri::async_runtime::spawn_blocking(move || {
                handle.state::<AppState>().restore(restore_generation)
            });
            // Keep the token fresh while the app runs (#105).
            let keeper = app.handle().clone();
            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(Duration::from_secs(60));
                    keeper.state::<AppState>().refresh_if_stale();
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            codex_status,
            session_status,
            list_profiles,
            selected_profile,
            create_profile,
            rename_profile,
            select_profile,
            sign_in,
            cancel_sign_in,
            sign_out,
            list_voices,
            get_voice,
            delete_voice,
            rename_voice,
            update_voice_profile,
            create_voice,
            add_voice_sources,
            remove_voice_sources,
            remove_voice_source,
            rebuild_voice,
            cancel_voice_build,
            list_voice_presets,
            create_voice_from_preset,
            material_budget,
            platforms,
            preview_prompt,
            generate_draft,
            cancel_generate,
            get_policy,
            set_policy,
            draft_policy,
            suggest_topics,
            suggest_brief,
            get_topic_cloud,
            gather_topic_cloud,
            enqueue_articles,
            start_queue,
            dequeue_article,
            clear_queue,
            set_draft_status,
            generate_into_article,
            list_articles,
            get_article,
            update_article,
            delete_article,
            list_versions,
            snapshot_article,
            restore_version,
            import_note,
            import_medium,
            import_x_archive,
            read_talk,
            fetch_update,
            install_update,
            restart_app,
            app_version
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod queue_transition_tests {
    use super::*;

    fn version_request_for_test() -> NewVersion {
        NewVersion {
            article_id: "article-1".into(),
            kind: VersionKind::Generated,
            title: "Generated title".into(),
            body: "Generated body".into(),
            prompt_sent: Some("exact prompt".into()),
            elapsed_ms: Some(321),
        }
    }

    fn version_for_test(request: &NewVersion, created_at: &str) -> ArticleVersion {
        ArticleVersion {
            id: "version-1".into(),
            article_id: request.article_id.clone(),
            kind: request.kind,
            title: request.title.clone(),
            body: request.body.clone(),
            prompt_sent: request.prompt_sent.clone(),
            elapsed_ms: request.elapsed_ms,
            created_at: created_at.into(),
        }
    }

    fn stale_article(
        id: &str,
        profile_id: &str,
        queue_started_at: Option<&str>,
        updated_at: &str,
    ) -> Article {
        Article {
            id: id.into(),
            profile_id: profile_id.into(),
            voice_id: None,
            platform_id: "x".into(),
            title: "current title".into(),
            body: "current body".into(),
            brief: "edited brief".into(),
            status: ArticleStatus::Draft,
            queue: Some(QueueState::Writing),
            queue_started_at: queue_started_at.map(str::to_owned),
            created_at: "2026-09-26T00:00:00+00:00".into(),
            updated_at: updated_at.into(),
        }
    }

    fn stale_version(
        article: &Article,
        id: &str,
        kind: VersionKind,
        created_at: &str,
        title: &str,
        body: &str,
    ) -> ArticleVersion {
        ArticleVersion {
            id: id.into(),
            article_id: article.id.clone(),
            kind,
            title: title.into(),
            body: body.into(),
            prompt_sent: None,
            elapsed_ms: None,
            created_at: created_at.into(),
        }
    }

    #[test]
    fn version_post_reconciliation_requires_an_exact_new_version_match() {
        let request = version_request_for_test();
        let original_updated_at = "2026-09-27T00:00:00.250000+00:00";
        let candidate = version_for_test(&request, "2026-09-27T00:00:00.250001+00:00");
        assert!(version_matches_attempt_after_article_snapshot(
            &candidate,
            &request,
            original_updated_at,
        ));

        let stale = version_for_test(&request, original_updated_at);
        assert!(!version_matches_attempt_after_article_snapshot(
            &stale,
            &request,
            original_updated_at,
        ));

        let mut mismatched = candidate.clone();
        mismatched.article_id = "article-2".into();
        assert!(!version_matches_attempt_after_article_snapshot(
            &mismatched,
            &request,
            original_updated_at,
        ));
        mismatched = candidate.clone();
        mismatched.kind = VersionKind::Shortened;
        assert!(!version_matches_attempt_after_article_snapshot(
            &mismatched,
            &request,
            original_updated_at,
        ));
        mismatched = candidate.clone();
        mismatched.title.push('!');
        assert!(!version_matches_attempt_after_article_snapshot(
            &mismatched,
            &request,
            original_updated_at,
        ));
        mismatched = candidate.clone();
        mismatched.body.push('!');
        assert!(!version_matches_attempt_after_article_snapshot(
            &mismatched,
            &request,
            original_updated_at,
        ));
        mismatched = candidate.clone();
        mismatched.prompt_sent = Some("different prompt".into());
        assert!(!version_matches_attempt_after_article_snapshot(
            &mismatched,
            &request,
            original_updated_at,
        ));
        mismatched = candidate;
        mismatched.elapsed_ms = Some(322);
        assert!(!version_matches_attempt_after_article_snapshot(
            &mismatched,
            &request,
            original_updated_at,
        ));
    }

    #[test]
    fn version_timestamp_comparison_preserves_fractional_precision_and_offsets() {
        use std::cmp::Ordering;

        assert_eq!(
            cmp_timestamp_full_precision(
                "2026-09-27T00:00:00.123456789+00:00",
                "2026-09-27T00:00:00.123456788+00:00",
            ),
            Ordering::Greater
        );
        assert_eq!(
            cmp_timestamp_full_precision(
                "2026-09-27T09:00:00.500000+09:00",
                "2026-09-27T00:00:00.5+00:00",
            ),
            Ordering::Equal
        );
        assert_eq!(
            cmp_timestamp_full_precision(
                "2026-09-27T09:00:00.499999+09:00",
                "2026-09-27T00:00:00.5+00:00",
            ),
            Ordering::Less
        );
    }

    #[test]
    fn stopping_during_stale_writing_recovery_preserves_the_empty_draft() {
        let stale = QueueJobId::new("profile-a", "article-stale");
        let mut recovery_wins = QueueTransitions::default();
        assert!(recovery_wins.recover_stale_writing(&stale, Some(QueueState::Writing)));

        // Recovery has persisted Writing -> Waiting while holding the transition lock.
        assert_eq!(
            recovery_wins.stop(&stale, QueueState::Waiting),
            QueueStopAction::PreserveDraft
        );

        // If stop wins first, recovery re-reads None and cannot restore Waiting.
        let mut stop_wins = QueueTransitions::default();
        assert_eq!(
            stop_wins.stop(&stale, QueueState::Writing),
            QueueStopAction::PreserveDraft
        );
        assert!(!stop_wins.recover_stale_writing(&stale, None));
    }

    #[test]
    fn stopping_one_profile_does_not_cancel_another_profiles_active_job() {
        let active = QueueJobId::new("profile-a", "article-active");
        let other = QueueJobId::new("profile-b", "article-active");
        let mut transitions = QueueTransitions::default();
        let cancel = transitions
            .begin(active.clone())
            .expect("queue job should start");

        assert_eq!(
            transitions.stop(&other, QueueState::Writing),
            QueueStopAction::PreserveDraft
        );
        assert!(!cancel.load(Ordering::Relaxed));
        assert!(!transitions.active_cancelled(&active));

        assert_eq!(
            transitions.stop(&active, QueueState::Writing),
            QueueStopAction::CancelActive
        );
        assert!(transitions.cancel_active(&active));
        assert!(cancel.load(Ordering::Relaxed));
    }

    #[test]
    fn uncertain_queue_completion_is_reported_failed_and_stops_retries() {
        let error = queue_completion_uncertain_error(
            UiError {
                code: "network",
                detail: "connection timed out".into(),
            },
            "article-1",
            "version-2",
            "Article reread failed".into(),
        );
        assert_eq!(error.code, "queue_completion_uncertain");
        assert!(error.detail.contains("network: connection timed out"));
        assert!(error.detail.contains("Article article-1"));
        assert!(error.detail.contains("Version version-2"));
        assert!(stops_the_queue(error.code));
        assert_eq!(queue_error_mark(&error), Some(QueueState::Failed));
    }

    #[test]
    fn failed_version_cleanup_becomes_uncertain_and_prevents_retry() {
        let error = queue_completion_cleanup_result(
            Err(UiError {
                code: "network",
                detail: "Version delete timed out".into(),
            }),
            UiError {
                code: "network",
                detail: "completion PATCH timed out".into(),
            },
            "article-7",
            "version-9",
        );
        assert_eq!(error.code, "queue_completion_uncertain");
        assert!(error.detail.contains("network: completion PATCH timed out"));
        assert!(error.detail.contains("network: Version delete timed out"));
        assert!(error.detail.contains("Article article-7"));
        assert!(error.detail.contains("Version version-9"));
        assert!(stops_the_queue(error.code));
        assert_eq!(queue_error_mark(&error), Some(QueueState::Failed));

        let cleaned_up = queue_completion_cleanup_result(
            Ok(()),
            UiError {
                code: "network",
                detail: "completion PATCH timed out".into(),
            },
            "article-7",
            "version-9",
        );
        assert_eq!(cleaned_up.code, "network");
        assert_eq!(queue_error_mark(&cleaned_up), Some(QueueState::Waiting));
    }

    #[test]
    fn cleaned_up_network_completion_failure_remains_waiting_for_retry() {
        let error = UiError {
            code: "network",
            detail: "completion response failed after Version cleanup".into(),
        };
        assert_eq!(queue_error_mark(&error), Some(QueueState::Waiting));
    }

    #[test]
    fn queue_start_transition_error_uses_reread_status_and_error_kind() {
        assert_eq!(
            queue_start_transition_action(Ok(Some(QueueState::Writing)), "network"),
            QueueStartTransitionAction::Proceed
        );
        assert_eq!(
            queue_start_transition_action(Ok(Some(QueueState::Waiting)), "network"),
            QueueStartTransitionAction::Stop
        );
        assert_eq!(
            queue_start_transition_action(Ok(Some(QueueState::Waiting)), "invalid_input"),
            QueueStartTransitionAction::MarkFailed
        );
        assert_eq!(
            queue_start_transition_action(Ok(Some(QueueState::Failed)), "invalid_input"),
            QueueStartTransitionAction::Skip
        );
        assert_eq!(
            queue_start_transition_action(Ok(None), "network"),
            QueueStartTransitionAction::Skip
        );
        assert_eq!(
            queue_start_transition_action(Err(()), "network"),
            QueueStartTransitionAction::RereadFailed
        );
    }

    #[test]
    fn legacy_t1_version_matching_after_t2_brief_edit_completes_without_regeneration() {
        let article = stale_article(
            "article-legacy",
            "profile-a",
            None,
            "2026-09-27T02:00:00+00:00",
        );
        let version = stale_version(
            &article,
            "version-t1",
            VersionKind::Generated,
            "2026-09-27T01:00:00+00:00",
            &article.title,
            &article.body,
        );

        assert_eq!(
            cmp_timestamp_full_precision(&version.created_at, &article.updated_at),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            stale_writing_recovery_action(
                Some(QueueState::Writing),
                false,
                Some(&article),
                &[version],
            ),
            StaleWritingRecoveryAction::ClearQueue
        );
    }

    #[test]
    fn legacy_writing_without_generated_versions_requeues() {
        let article = stale_article(
            "article-legacy",
            "profile-a",
            None,
            "2026-09-27T02:00:00+00:00",
        );
        assert_eq!(
            stale_writing_recovery_action(Some(QueueState::Writing), false, Some(&article), &[],),
            StaleWritingRecoveryAction::Requeue
        );
    }

    #[test]
    fn newer_matching_version_completes_using_fractional_offset_ordering() {
        let article = stale_article(
            "article-current",
            "profile-a",
            Some("2026-09-27T00:00:00.123456788+00:00"),
            "2026-09-27T02:00:00+00:00",
        );
        let version = stale_version(
            &article,
            "version-newer",
            VersionKind::Shortened,
            "2026-09-27T09:00:00.123456789+09:00",
            &article.title,
            &article.body,
        );

        assert_eq!(
            stale_writing_recovery_action(
                Some(QueueState::Writing),
                false,
                Some(&article),
                &[version],
            ),
            StaleWritingRecoveryAction::ClearQueue
        );
    }

    #[test]
    fn uncertain_stale_article_failure_does_not_prevent_the_next_queue_item() {
        let first = stale_article("article-a", "profile-a", None, "2026-09-27T02:00:00+00:00");
        let second = stale_article("article-b", "profile-b", None, "2026-09-27T02:00:00+00:00");
        let mut visited = Vec::new();
        let mut failed = Vec::new();
        let mut requeued = Vec::new();

        for_each_stale_writing_article([first, second], |article| {
            visited.push(article.id.clone());
            let versions = if article.id == "article-a" {
                vec![stale_version(
                    &article,
                    "version-unmatched",
                    VersionKind::Generated,
                    "2026-09-27T01:00:00+00:00",
                    "different title",
                    "different body",
                )]
            } else {
                Vec::new()
            };
            match stale_writing_recovery_action(article.queue, false, Some(&article), &versions) {
                StaleWritingRecoveryAction::FailUncertain => {
                    failed.push((article.profile_id, article.id));
                }
                StaleWritingRecoveryAction::Requeue => {
                    requeued.push((article.profile_id, article.id));
                }
                action => panic!("unexpected recovery action: {action:?}"),
            }
            Ok(())
        })
        .unwrap();

        assert_eq!(visited, ["article-a", "article-b"]);
        assert_eq!(failed, [("profile-a".into(), "article-a".into())]);
        assert_eq!(requeued, [("profile-b".into(), "article-b".into())]);
    }
    #[test]
    fn start_requested_during_empty_exit_handoff_is_drained_by_the_worker() {
        let mut worker = QueueWorkerState::default();
        assert!(worker.request_start());
        assert!(!worker.request_start());
        assert!(worker.finish_run(true));
        assert!(!worker.finish_run(true));
        assert!(worker.request_start());
    }

    #[test]
    fn failed_worker_run_discards_pending_start() {
        let mut worker = QueueWorkerState::default();
        assert!(worker.request_start());
        assert!(!worker.request_start());
        assert!(!worker.finish_run(false));
        assert!(!worker.running);
        assert!(!worker.pending_start);
        assert!(worker.request_start());
    }

    #[test]
    fn start_requested_after_worker_is_idle_launches_a_new_worker() {
        let mut worker = QueueWorkerState::default();
        assert!(worker.request_start());
        assert!(!worker.finish_run(true));
        assert!(worker.request_start());
    }

    #[test]
    fn pre_persistence_cancel_gate_returns_cancelled() {
        let cancel = AtomicBool::new(false);
        assert!(ensure_not_cancelled(&cancel).is_ok());
        cancel.store(true, Ordering::Relaxed);
        let error = ensure_not_cancelled(&cancel).unwrap_err();
        assert_eq!(error.code, "cancelled");
    }
    #[test]
    fn article_generation_accepts_only_quality_effort() {
        assert!(validate_article_effort(StoredEffort::Quality).is_ok());
        assert_eq!(
            validate_article_effort(StoredEffort::Fast)
                .unwrap_err()
                .code,
            "invalid_input"
        );
        assert_eq!(
            validate_article_effort(StoredEffort::Best)
                .unwrap_err()
                .code,
            "invalid_input"
        );
    }

    #[test]
    fn stale_profile_selection_commit_cannot_change_active_profile() {
        let mut lifecycle = SessionLifecycle::default();
        let generation = lifecycle.generation.begin();
        lifecycle.active_profile_id = Some("profile-a".into());
        let revision = lifecycle.begin_profile_selection(generation).unwrap();
        lifecycle.generation.begin();
        assert!(!lifecycle.commit_profile_selection(generation, revision, "profile-b".into(),));
        assert_eq!(lifecycle.active_profile_id.as_deref(), Some("profile-a"));
    }

    #[test]
    fn out_of_order_selection_ticket_cannot_issue_or_commit() {
        let mut lifecycle = SessionLifecycle::default();
        let generation = lifecycle.generation.begin();
        let older = lifecycle.begin_profile_selection(generation).unwrap();
        let newer = lifecycle.begin_profile_selection(generation).unwrap();
        assert!(!lifecycle.profile_selection_is_current(generation, older));
        assert!(lifecycle.profile_selection_is_current(generation, newer));
        assert!(lifecycle.commit_profile_selection(generation, newer, "profile-b".into(),));
        assert!(!lifecycle.commit_profile_selection(generation, older, "profile-a".into(),));
        assert_eq!(lifecycle.active_profile_id.as_deref(), Some("profile-b"));
    }

    #[test]
    fn queued_completion_reconciliation_distinguishes_committed_stopped_and_uncertain() {
        let original = QueueArticleSnapshot {
            body: "old body",
            title: "old title",
            queue: Some(QueueState::Writing),
        };
        let committed = QueueArticleSnapshot {
            body: "generated body",
            title: "generated title",
            queue: None,
        };
        assert_eq!(
            reconcile_queue_completion(
                Some(committed),
                original,
                "generated body",
                "generated title",
                false,
            ),
            QueueCompletionReconciliation::Committed
        );

        let stopped = QueueArticleSnapshot {
            queue: None,
            ..original
        };
        assert_eq!(
            reconcile_queue_completion(
                Some(stopped),
                original,
                "generated body",
                "generated title",
                false,
            ),
            QueueCompletionReconciliation::NotCommitted { stopped: true }
        );

        assert_eq!(
            reconcile_queue_completion(
                Some(original),
                original,
                "generated body",
                "generated title",
                false,
            ),
            QueueCompletionReconciliation::NotCommitted { stopped: false }
        );

        let changed = QueueArticleSnapshot {
            body: "other body",
            title: "other title",
            queue: None,
        };
        assert_eq!(
            reconcile_queue_completion(
                Some(changed),
                original,
                "generated body",
                "generated title",
                false,
            ),
            QueueCompletionReconciliation::Uncertain
        );
        assert_eq!(
            reconcile_queue_completion(None, original, "generated body", "generated title", false,),
            QueueCompletionReconciliation::Uncertain
        );
    }

    #[test]
    fn empty_queue_output_reconciliation_respects_confirmed_stop() {
        let original = QueueArticleSnapshot {
            body: "",
            title: "",
            queue: Some(QueueState::Writing),
        };
        let queue_cleared = QueueArticleSnapshot {
            queue: None,
            ..original
        };

        assert_eq!(
            reconcile_queue_completion(Some(queue_cleared), original, "", "", true),
            QueueCompletionReconciliation::NotCommitted { stopped: true }
        );
        assert_eq!(
            reconcile_queue_completion(Some(queue_cleared), original, "", "", false),
            QueueCompletionReconciliation::Committed
        );
    }
    #[test]
    fn refresh_completion_after_sign_out_is_rejected_by_generation() {
        let mut generation = SessionGeneration::default();
        let refresh = generation.begin();
        let sign_out = generation.begin();

        assert!(!generation.commit_if_current(refresh));
        assert!(generation.is_current(sign_out));
    }

    #[test]
    fn restore_completion_after_newer_sign_in_is_rejected_by_generation() {
        let mut generation = SessionGeneration::default();
        let restore = generation.begin();
        let sign_in = generation.begin();

        assert!(!generation.commit_if_current(restore));
        assert!(generation.commit_if_current(sign_in));
        assert!(!generation.is_current(sign_in));
    }
}
