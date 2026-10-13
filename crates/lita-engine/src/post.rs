//! Generating one post in a Voice. The profile is passed as a contract
//! (`generation_view`, D-28/D-29), the brief as the person wrote it, and
//! the platform's rules as plain language.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::voice::VoiceProfile;

/// What generation returns. `voice_notes` is for the person: one sentence
/// on which parts of the profile the draft leaned on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostDraft {
    /// For long-form destinations: the title, on its own. Empty for posts.
    #[serde(default)]
    pub title: String,
    pub text: String,
    pub char_count: i64,
    pub voice_notes: String,
}

/// Output rules for one destination, as stored in `platforms`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformRules {
    pub name: String,
    pub max_chars: Option<i64>,
    pub rules: String,
}

pub fn post_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["title", "text", "char_count", "voice_notes"],
        "properties": {
            "title": { "type": "string", "description": "For a long-form article: its title, on its own. For a short post: an empty string." },
            "text": { "type": "string", "description": "The piece, ready to publish as-is: no preamble, no quotes around it, no title line (the title is returned separately)." },
            "char_count": { "type": "integer", "description": "The number of characters in `text`." },
            "voice_notes": { "type": "string", "description": "One sentence, in the same language as the post, on which parts of the profile the draft leaned on." }
        }
    })
}

/// The exact prompt sent to Codex. Shown to the person before sending (B-08),
/// so it must contain nothing the person would not want to see.
pub fn generation_prompt(profile: &VoiceProfile, brief: &str, platform: &PlatformRules) -> String {
    // A destination with a hard limit gets a post; one without gets an
    // article with its own title, paragraphs and a length the brief implies.
    let (what, shape) = match platform.max_chars {
        Some(n) => ("a single post".to_string(), format!("Stay under {n} characters. `title` is an empty string.\n")),
        None => (
            "a full article".to_string(),
            "Put the title in `title` (one line, no quotes) and only the body in `text`. Paragraphs are \
             separated by one blank line. Use a heading line starting with \"## \" only where the argument \
             turns; most articles need two or three, short ones none. Length follows the brief; if it says \
             nothing, aim for what a reader finishes in four to six minutes (about 1,500 to 2,500 characters \
             in Japanese, 700 to 1,200 words in English). Open with the point, not a preamble; end the way the \
             profile says the person ends.\n"
                .to_string(),
        ),
    };
    format!(
        "Write {what} for {name} in the voice described by this profile.\n\n\
         VOICE PROFILE:\n{profile}\n\n\
         BRIEF:\n{brief}\n\n\
         RULES FOR {name}:\n{rules}\n{shape}\
         Write in the profile's language ({language}). Match the profile's first person, \
         sentence endings, preferred words and emoji habit exactly; the profile is a contract, \
         not a suggestion. Do not explain the text, do not add hashtags or links unless the brief \
         asks for them. `char_count` is the character count of `text`.",
        name = platform.name,
        profile = serde_json::to_string_pretty(&profile.generation_view()).unwrap_or_default(),
        brief = brief.trim(),
        rules = platform.rules.trim(),
        language = profile.language,
    )
}

/// The prompt for "短く書き直す": the same contract, plus the draft that ran
/// over the limit. Cutting is asked for explicitly, because "shorter" alone
/// tends to come back as a summary in a flatter voice.
pub fn shorten_prompt(profile: &VoiceProfile, brief: &str, platform: &PlatformRules, previous: &str) -> String {
    let over = platform
        .max_chars
        .map(|n| {
            let len = previous.chars().count() as i64;
            if len > n { format!(" It is {} characters over the limit of {n}.", len - n) } else { String::new() }
        })
        .unwrap_or_default();
    format!(
        "{base}\n\n\
         PREVIOUS DRAFT (too long{over_note}):\n{previous}\n\n\
         Rewrite the previous draft so it fits.{over} Keep its point, its order and its voice; \
         cut words and clauses rather than summarising, and do not add anything new.",
        base = generation_prompt(profile, brief, platform),
        over_note = if over.is_empty() { String::new() } else { format!(", {} characters", previous.chars().count()) },
        previous = previous.trim(),
        over = over,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::Excerpt;

    #[test]
    fn prompt_carries_brief_rules_and_limit_but_no_excerpts() {
        let profile = VoiceProfile {
            language: "ja".into(),
            first_person: "僕".into(),
            formality: "casual".into(),
            tone: vec![],
            sentence_endings: vec![],
            avg_sentence_length_chars: 30,
            preferred_words: vec![],
            avoided_words: vec![],
            opens_with: String::new(),
            closes_with: String::new(),
            uses_emoji: false,
            representative_excerpts: vec![Excerpt { excerpt: "SECRET-EXCERPT".into(), why: "".into() }],
            one_line: String::new(),
            ..Default::default()
        };
        let p = generation_prompt(
            &profile,
            "v2 を告知",
            &PlatformRules { name: "X".into(), max_chars: Some(280), rules: "No hashtags.".into() },
        );
        assert!(p.contains("v2 を告知"));
        assert!(p.contains("No hashtags."));
        assert!(p.contains("under 280"));
        assert!(p.contains("(ja)"));
        assert!(!p.contains("SECRET-EXCERPT"));
    }

    #[test]
    fn shorten_prompt_carries_the_previous_draft_and_the_overrun() {
        let profile = VoiceProfile {
            language: "ja".into(), first_person: "僕".into(), formality: "casual".into(), tone: vec![],
            sentence_endings: vec![], avg_sentence_length_chars: 30, preferred_words: vec![], avoided_words: vec![],
            opens_with: String::new(), closes_with: String::new(), uses_emoji: false, representative_excerpts: vec![],
            one_line: String::new(),
            ..Default::default()
        };
        let platform = PlatformRules { name: "X".into(), max_chars: Some(10), rules: "".into() };
        let p = shorten_prompt(&profile, "brief", &platform, "twelve chars");
        assert!(p.contains("PREVIOUS DRAFT"));
        assert!(p.contains("twelve chars"));
        assert!(p.contains("2 characters over the limit of 10"));
        assert!(p.contains("BRIEF:\nbrief"));
    }

    #[test]
    fn long_form_prompt_asks_for_a_title_and_paragraphs() {
        let profile = VoiceProfile {
            language: "ja".into(), first_person: "私".into(), formality: "常体".into(), tone: vec![],
            sentence_endings: vec![], avg_sentence_length_chars: 30, preferred_words: vec![], avoided_words: vec![],
            opens_with: String::new(), closes_with: String::new(), uses_emoji: false, representative_excerpts: vec![],
            one_line: String::new(),
            ..Default::default()
        };
        let note = PlatformRules { name: "note".into(), max_chars: None, rules: "Plain text.".into() };
        let p = generation_prompt(&profile, "b", &note);
        assert!(p.contains("a full article") && p.contains("`title`") && p.contains("## "));
        let x = PlatformRules { name: "X".into(), max_chars: Some(280), rules: "".into() };
        let q = generation_prompt(&profile, "b", &x);
        assert!(q.contains("a single post") && q.contains("Stay under 280"));
    }
}
