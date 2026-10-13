//! Live check for the store: sign in, then create → read → update → delete
//! a voice with sources, an article with versions, and the default-voice
//! setting, through RLS. Leaves nothing
//! behind. Needs LITA_SUPABASE_URL and LITA_SUPABASE_ANON_KEY; opens the
//! browser once for sign-in.

use anyhow::{Context, Result, ensure};
use lita_auth::SupabaseAuth;
use lita_engine::voice::{Excerpt, VoiceProfile};
use lita_store::{
    ArticlePatch, ArticleStatus, NewArticle, NewSource, NewVersion, SourceKind, Store, VersionKind,
    is_permission_denied,
};

fn main() -> Result<()> {
    let url = std::env::var("LITA_SUPABASE_URL").context("LITA_SUPABASE_URL is not set")?;
    let key =
        std::env::var("LITA_SUPABASE_ANON_KEY").context("LITA_SUPABASE_ANON_KEY is not set")?;
    let session = SupabaseAuth::new(&url, &key)?.sign_in_with_provider("google", |u| {
        std::process::Command::new("open")
            .arg(u.as_str())
            .status()?;
        Ok(())
    })?;
    println!("signed in as {:?}", session.user.email);

    let store = Store::new(&url, &key)?;
    let account = store.as_user(&session.access_token);
    let publishing_profile = account
        .selected_profile()?
        .context("no persisted selected Profile")?;
    let me = account.for_profile(&publishing_profile.id)?;
    let profiles = account.profiles()?;
    ensure!(
        profiles.len() >= 2,
        "roundtrip requires an existing second Profile; no Profile is created or deleted"
    );
    let other_profile = profiles
        .into_iter()
        .find(|profile| profile.id != publishing_profile.id)
        .context("no second Profile is available")?;
    let other = account.for_profile(&other_profile.id)?;
    let selected_settings = me.settings()?;
    let other_settings = other.settings()?;
    ensure!(
        selected_settings.policy != other_settings.policy
            && selected_settings.topic_cloud != other_settings.topic_cloud,
        "roundtrip requires distinct policy and topic-cloud values in both Profiles"
    );

    let platforms = account.platforms()?;
    ensure!(platforms.iter().any(|p| p.id == "x"), "platform x missing");
    println!("platforms ok: {}", platforms.len());

    let profile = VoiceProfile {
        language: "ja".into(),
        first_person: "僕".into(),
        formality: "casual".into(),
        tone: vec!["direct".into()],
        sentence_endings: vec!["です".into()],
        avg_sentence_length_chars: 30,
        preferred_words: vec!["ざっくり".into()],
        avoided_words: vec![],
        opens_with: String::new(),
        closes_with: String::new(),
        uses_emoji: false,
        representative_excerpts: vec![Excerpt {
            excerpt: "例".into(),
            why: "短い".into(),
        }],
        one_line: String::new(),
        ..Default::default()
    };
    let sources = vec![
        NewSource {
            kind: SourceKind::Paste,
            origin: None,
            account: None,
            body: "一つ目".into(),
        },
        NewSource {
            kind: SourceKind::File,
            origin: Some("notes.txt".into()),
            account: None,
            body: "二つ目".into(),
        },
    ];
    let mut temporary_voice_id: Option<String> = None;
    let mut temporary_article_id: Option<String> = None;
    let isolation_result = (|| -> Result<()> {
        account.select_profile(&other_profile.id)?;
        let selected_after_switch = account
            .selected_profile()?
            .context("selected Profile disappeared after switch")?;
        ensure!(
            selected_after_switch.id == other_profile.id,
            "Profile selection was not persisted"
        );
        ensure!(
            me.settings()?.policy == selected_settings.policy
                && me.settings()?.topic_cloud == selected_settings.topic_cloud,
            "selected Profile policy/cloud changed during isolation check"
        );
        ensure!(
            other.settings()?.policy == other_settings.policy
                && other.settings()?.topic_cloud == other_settings.topic_cloud,
            "alternate Profile policy/cloud changed during isolation check"
        );

        let temporary_voice = other.create_voice(
            &format!("roundtrip isolation {}", std::process::id()),
            &profile,
            &sources[..1],
        )?;
        temporary_voice_id = Some(temporary_voice.id.clone());
        ensure!(
            me.voice(&temporary_voice.id)?.is_none(),
            "selected Profile read a Voice owned by the other Profile"
        );
        ensure!(
            me.rename_voice(&temporary_voice.id, "must not rename")
                .is_err(),
            "selected Profile renamed another Profile's Voice"
        );
        ensure!(
            me.add_sources(&temporary_voice.id, &sources[..1]).is_err(),
            "selected Profile added sources to another Profile's Voice"
        );
        ensure!(
            !me.delete_voice(&temporary_voice.id)?,
            "selected Profile deleted another Profile's Voice"
        );

        let temporary_article = other.create_article(&NewArticle {
            voice_id: Some(temporary_voice.id.clone()),
            platform_id: "x".into(),
            title: "roundtrip isolation article".into(),
            brief: "temporary cross-Profile check".into(),
            ..Default::default()
        })?;
        temporary_article_id = Some(temporary_article.id.clone());
        ensure!(
            temporary_article.profile_id == other_profile.id,
            "temporary Article has the wrong owner Profile"
        );
        let temporary_version = other.create_version(&NewVersion {
            article_id: temporary_article.id.clone(),
            kind: VersionKind::Manual,
            title: "other Profile version".into(),
            body: "temporary version".into(),
            prompt_sent: None,
            elapsed_ms: None,
        })?;
        ensure!(
            me.version(&temporary_version.id)?.is_none(),
            "selected Profile read a version owned by the other Profile"
        );
        ensure!(
            me.article(&temporary_article.id)?.is_none(),
            "selected Profile read an Article owned by the other Profile"
        );
        ensure!(
            me.versions(&temporary_article.id)?.is_empty(),
            "selected Profile read versions owned by the other Profile"
        );
        ensure!(
            me.update_article(
                &temporary_article.id,
                &ArticlePatch {
                    title: Some("must not update".into()),
                    ..Default::default()
                },
            )
            .is_err(),
            "selected Profile updated another Profile's Article"
        );
        ensure!(
            me.create_version(&NewVersion {
                article_id: temporary_article.id.clone(),
                kind: VersionKind::Manual,
                title: String::new(),
                body: String::new(),
                prompt_sent: None,
                elapsed_ms: None,
            })
            .is_err(),
            "selected Profile created a version for another Profile's Article"
        );
        ensure!(
            !me.delete_article(&temporary_article.id)?,
            "selected Profile deleted another Profile's Article"
        );
        Ok(())
    })();

    let mut cleanup_errors = Vec::new();
    if let Some(id) = temporary_article_id.as_deref() {
        match other.delete_article(id) {
            Ok(true) => {}
            Ok(false) => cleanup_errors.push(format!("temporary Article {id} was not deleted")),
            Err(error) => {
                cleanup_errors.push(format!("temporary Article cleanup failed: {error:#}"))
            }
        }
    }
    if let Some(id) = temporary_voice_id.as_deref() {
        match other.delete_voice(id) {
            Ok(true) => {}
            Ok(false) => cleanup_errors.push(format!("temporary Voice {id} was not deleted")),
            Err(error) => cleanup_errors.push(format!("temporary Voice cleanup failed: {error:#}")),
        }
    }
    match account.select_profile(&publishing_profile.id) {
        Ok(()) => match account.selected_profile() {
            Ok(Some(profile)) if profile.id == publishing_profile.id => {}
            Ok(_) => cleanup_errors.push("original Profile selection was not restored".into()),
            Err(error) => {
                cleanup_errors.push(format!("selection restore verification failed: {error:#}"))
            }
        },
        Err(error) => cleanup_errors.push(format!(
            "original Profile selection restore failed: {error:#}"
        )),
    }
    if let Err(error) = isolation_result {
        if !cleanup_errors.is_empty() {
            return Err(error.context(format!(
                "cleanup also reported: {}",
                cleanup_errors.join("; ")
            )));
        }
        return Err(error);
    }
    ensure!(
        cleanup_errors.is_empty(),
        "roundtrip isolation cleanup failed: {}",
        cleanup_errors.join("; ")
    );
    println!("cross-Profile isolation and persisted selection ok");

    let original_default_voice_id = selected_settings.default_voice_id.clone();
    let mut created_voice_id: Option<String> = None;
    let mut created_article_id: Option<String> = None;
    let mut unexpected_mastodon_article_id: Option<String> = None;
    let live_result = (|| -> Result<()> {
        let before = me.voices()?.len();
        let voice = me.create_voice("roundtrip", &profile, &sources)?;
        created_voice_id = Some(voice.id.clone());
        ensure!(
            voice.sources.len() == 2,
            "expected 2 sources, got {}",
            voice.sources.len()
        );
        ensure!(voice.profile == profile, "profile did not round-trip");
        println!("voice ok: {}", voice.id);

        let list = me.voices()?;
        ensure!(list.len() == before + 1, "list did not grow");
        ensure!(
            list.iter().any(|v| v.id == voice.id && v.source_count == 2),
            "summary count wrong"
        );

        me.rename_voice(&voice.id, "roundtrip renamed")?;
        let mut p2 = profile.clone();
        p2.uses_emoji = true;
        me.replace_voice_profile(&voice.id, &p2, &sources[..1])?;
        let v2 = me.voice(&voice.id)?.context("voice vanished")?;
        ensure!(
            v2.name == "roundtrip renamed" && v2.profile.uses_emoji && v2.sources.len() == 1,
            "update failed"
        );
        println!("update ok");

        ensure!(
            platforms.iter().any(|p| p.id == "note") && platforms.iter().any(|p| p.id == "medium"),
            "long-form platforms missing"
        );

        let article = me.create_article(&NewArticle {
            voice_id: Some(voice.id.clone()),
            platform_id: "x".into(),
            title: String::new(),
            body: String::new(),
            brief: "新機能の告知".into(),
            ..Default::default()
        })?;
        created_article_id = Some(article.id.clone());
        ensure!(article.status == ArticleStatus::Draft && article.body.is_empty());
        let v1 = me.create_version(&NewVersion {
            article_id: article.id.clone(),
            kind: VersionKind::Generated,
            title: String::new(),
            body: "出来た文".into(),
            prompt_sent: Some("the prompt".into()),
            elapsed_ms: Some(6600),
        })?;
        me.update_article(
            &article.id,
            &ArticlePatch {
                body: Some("出来た文を直した".into()),
                title: Some("題".into()),
                ..Default::default()
            },
        )?;
        me.create_version(&NewVersion {
            article_id: article.id.clone(),
            kind: VersionKind::Edited,
            title: "題".into(),
            body: "出来た文を直した".into(),
            prompt_sent: None,
            elapsed_ms: None,
        })?;
        let a2 = me.article(&article.id)?.context("article vanished")?;
        ensure!(
            a2.body == "出来た文を直した"
                && a2.title == "題"
                && a2.updated_at >= article.updated_at,
            "article update failed"
        );
        let versions = me.versions(&article.id)?;
        ensure!(
            versions.len() == 2
                && versions[0].kind == VersionKind::Edited
                && versions[1].id == v1.id,
            "versions wrong: {versions:?}"
        );
        me.update_article(
            &article.id,
            &ArticlePatch {
                status: Some(ArticleStatus::Approved),
                ..Default::default()
            },
        )?;
        let listed = me.articles(Some(ArticleStatus::Approved))?;
        ensure!(
            listed
                .iter()
                .any(|a| a.id == article.id && a.excerpt == "出来た文を直した"),
            "approved list missing the article"
        );
        ensure!(
            me.articles(Some(ArticleStatus::Archived))?
                .iter()
                .all(|a| a.id != article.id),
            "archived list has the article"
        );
        println!("article/versions ok");

        // Edited snapshots are pruned to the latest 50 per article.
        for i in 0..55 {
            me.create_version(&NewVersion {
                article_id: article.id.clone(),
                kind: VersionKind::Edited,
                title: String::new(),
                body: format!("e{i}"),
                prompt_sent: None,
                elapsed_ms: None,
            })?;
        }
        let vs = me.versions(&article.id)?;
        let edited = vs.iter().filter(|v| v.kind == VersionKind::Edited).count();
        ensure!(
            edited == 50 && vs.iter().any(|v| v.id == v1.id),
            "prune wrong: {edited} edited, generated kept={}",
            vs.iter().any(|v| v.id == v1.id)
        );
        println!("prune ok");

        me.set_default_voice(Some(&voice.id))?;
        ensure!(
            me.settings()?.default_voice_id.as_deref() == Some(voice.id.as_str()),
            "default voice not stored"
        );
        println!("settings updated; original value will be restored in cleanup");

        let unknown = me.create_article(&NewArticle {
            voice_id: None,
            platform_id: "mastodon".into(),
            ..Default::default()
        });
        if let Ok(article) = &unknown {
            unexpected_mastodon_article_id = Some(article.id.clone());
        }
        ensure!(unknown.is_err(), "unknown platform was accepted");

        ensure!(me.delete_voice(&voice.id)?, "delete reported nothing");
        let orphan = me
            .article(&article.id)?
            .context("article should survive voice deletion")?;
        ensure!(
            orphan.voice_id.is_none(),
            "voice_id should be null after the voice is deleted"
        );
        ensure!(
            me.delete_article(&article.id)?,
            "article delete reported nothing"
        );
        ensure!(me.version(&v1.id)?.is_none(), "versions did not cascade");
        ensure!(
            !me.delete_voice(&voice.id)?,
            "second delete should be a no-op"
        );
        println!("delete + cascade ok");
        Ok(())
    })();

    let mut selected_cleanup_errors = Vec::new();
    if let Some(id) = created_article_id.as_deref()
        && let Err(error) = me.delete_article(id)
    {
        selected_cleanup_errors.push(format!(
            "selected Profile Article cleanup failed: {error:#}"
        ));
    }
    if let Some(id) = unexpected_mastodon_article_id.as_deref()
        && let Err(error) = me.delete_article(id)
    {
        selected_cleanup_errors.push(format!(
            "unexpected Mastodon Article cleanup failed: {error:#}"
        ));
    }
    if let Some(id) = created_voice_id.as_deref()
        && let Err(error) = me.delete_voice(id)
    {
        selected_cleanup_errors.push(format!("selected Profile Voice cleanup failed: {error:#}"));
    }
    match me.set_default_voice(original_default_voice_id.as_deref()) {
        Ok(()) => match me.settings() {
            Ok(settings)
                if settings.default_voice_id.as_deref() == original_default_voice_id.as_deref() => {
            }
            Ok(_) => selected_cleanup_errors
                .push("selected Profile default Voice was not restored".into()),
            Err(error) => selected_cleanup_errors.push(format!(
                "default Voice restore verification failed: {error:#}"
            )),
        },
        Err(error) => {
            selected_cleanup_errors.push(format!("default Voice restore failed: {error:#}"))
        }
    }
    if let Err(error) = live_result {
        if !selected_cleanup_errors.is_empty() {
            return Err(error.context(format!(
                "selected Profile cleanup also reported: {}",
                selected_cleanup_errors.join("; ")
            )));
        }
        return Err(error);
    }
    ensure!(
        selected_cleanup_errors.is_empty(),
        "selected Profile cleanup failed: {}",
        selected_cleanup_errors.join("; ")
    );

    // Someone else's session must not see or touch these rows. Using the
    // anon key alone as a stand-in: `anon` has no GRANT on the table, so a
    // permission error (401, 42501) is expected; an empty list also passes.
    let anon = store.as_user(&key);
    match anon.profiles() {
        Ok(profiles) => ensure!(profiles.is_empty(), "anon saw {} Profiles", profiles.len()),
        Err(error) if is_permission_denied(&error) => {}
        Err(error) => return Err(error.context("anon Profile list failed unexpectedly")),
    }
    println!("all good");
    Ok(())
}
