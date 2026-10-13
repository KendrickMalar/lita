//! Titles to write next, and the editorial policy behind them (article
//! queue, 2026-09-23). Neither touches the generation contract in `post`:
//! what the queue learns here reaches an article only through its brief.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The editorial policy: one per person, four short free-text fields. What
/// to write about, as opposed to the voice (how to write). All may be empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Policy {
    /// Who the writing is for.
    #[serde(default)]
    pub audience: String,
    /// What a reader should take away.
    #[serde(default)]
    pub takeaway: String,
    /// Subjects this person keeps returning to (at most three).
    #[serde(default)]
    pub topics: Vec<String>,
    /// What never gets written about.
    #[serde(default)]
    pub avoid: String,
}

impl Policy {
    pub fn is_empty(&self) -> bool {
        self.audience.trim().is_empty()
            && self.takeaway.trim().is_empty()
            && self.topics.iter().all(|t| t.trim().is_empty())
            && self.avoid.trim().is_empty()
    }

    /// The policy as lines for a prompt or a brief; nothing when empty.
    fn lines(&self, lang: Lang) -> Vec<String> {
        let mut out = Vec::new();
        let (a, t, s, v) = match lang {
            Lang::Ja => ("誰に向けて", "持ち帰ってほしいこと", "よく書く題材", "触れないこと"),
            Lang::En => ("Written for", "What the reader should take away", "Usual subjects", "Never about"),
        };
        if !self.audience.trim().is_empty() {
            out.push(format!("{a}: {}", self.audience.trim()));
        }
        if !self.takeaway.trim().is_empty() {
            out.push(format!("{t}: {}", self.takeaway.trim()));
        }
        // `topics` is no longer shown or written (D-66: the cloud carries the
        // subjects); old rows may still hold it, and it stays out of prompts.
        let _ = s;
        if !self.avoid.trim().is_empty() {
            out.push(format!("{v}: {}", self.avoid.trim()));
        }
        out
    }
}

/// Which language the brief is written in. Follows the voice's profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Ja,
    En,
}

impl Lang {
    pub fn of(profile_language: &str) -> Self {
        if profile_language.trim().to_lowercase().starts_with("en") { Lang::En } else { Lang::Ja }
    }
}

/// An article the person already has: its title and first line, so new
/// titles do not repeat it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Existing {
    pub title: String,
    pub first_line: String,
}

/// One word of the cloud: what the person keeps writing about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudWord {
    pub word: String,
    /// 1 to 5: how often this person comes back to it (not a count).
    pub weight: u8,
    /// True when an article with this subject already exists.
    #[serde(default)]
    pub written: bool,
}

/// The cloud as stored: words plus when and from how much it was gathered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TopicCloud {
    #[serde(default)]
    pub words: Vec<CloudWord>,
    #[serde(default)]
    pub gathered_at: String,
    /// The person's own writing at gathering time, so the screen can say
    /// "more since": the sources of every voice and the articles the person
    /// edited (`material_count`). Articles Lita only wrote do not count (#129).
    #[serde(default)]
    pub material_count: usize,
    /// How `material_count` was counted: 0 for clouds saved before #129
    /// (writing and articles together), 1 for the writing only, `CLOUD_COUNTING` since.
    #[serde(default)]
    pub counting: u8,
}

/// The way `material_count` is counted now: the person's own writing and
/// the articles they edited.
pub const CLOUD_COUNTING: u8 = 2;

/// A cloud saved under an older way of counting cannot be compared with the
/// count now. Counts it again as the writing that already existed when it
/// was gathered, from the `created_at` of every piece there is now, plus
/// every article the person has edited by now (`edited`; no time says when
/// the edit was made). No Codex call. Whatever cannot be placed counts as
/// already there, so nothing is called new by mistake.
///
/// True when it counted again: the caller saves the cloud then, so this
/// happens once. Were it only counted in memory, every opening would take
/// the articles edited by that moment as already there, and an edit would
/// never make "more since" until the next gathering.
pub fn recount(cloud: &mut TopicCloud, source_times: &[String], edited: usize) -> bool {
    if cloud.counting >= CLOUD_COUNTING {
        return false;
    }
    let gathered = cloud.gathered_at.trim().parse::<i64>().ok();
    cloud.material_count = source_times
        .iter()
        .filter(|t| match (gathered, epoch_seconds(t)) {
            (Some(g), Some(s)) => s <= g,
            _ => true,
        })
        .count()
        + edited;
    cloud.counting = CLOUD_COUNTING;
    true
}

/// One article as the cloud sees it (#129, stage 3): its text now, and every
/// text Lita wrote for it (`generated` and `shortened` versions).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ArticleText {
    pub body: String,
    /// `None` when its versions could not be read: then it is not taken as
    /// the person's, since Lita's text would pass for theirs.
    pub lita_wrote: Option<Vec<String>>,
}

impl ArticleText {
    /// True when the text now is the person's words: not empty, and not a
    /// text Lita wrote (left as it was, or put back from the history).
    /// Only the ends are trimmed; any other change counts as an edit.
    pub fn edited_by_person(&self) -> bool {
        let Some(lita_wrote) = &self.lita_wrote else { return false };
        let body = self.body.trim();
        !body.is_empty() && lita_wrote.iter().all(|l| l.trim() != body)
    }
}

/// The bodies of the articles the person edited or wrote themselves. They
/// reach the cloud only; titles, briefs and the voice never read them.
pub fn own_bodies(articles: &[ArticleText]) -> Vec<&str> {
    articles.iter().filter(|a| a.edited_by_person()).map(|a| a.body.as_str()).collect()
}

/// The person's own writing: the pieces behind every voice (`sources`) and
/// the articles they edited. What "more since" compares.
pub fn material_count(sources: usize, articles: &[ArticleText]) -> usize {
    sources + articles.iter().filter(|a| a.edited_by_person()).count()
}

/// Seconds since the epoch from an RFC 3339 time as Postgres returns it
/// (`2026-09-20T09:00:00.123456+09:00`, `Z`, or a space for `T`).
/// Fractions of a second are dropped.
pub fn epoch_seconds(t: &str) -> Option<i64> {
    let t = t.trim();
    let num = |r: std::ops::Range<usize>| t.get(r)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, se) = (num(0..4)?, num(5..7)?, num(8..10)?, num(11..13)?, num(14..16)?, num(17..19)?);
    let seps = [(4, b'-'), (7, b'-'), (13, b':'), (16, b':')];
    if seps.iter().any(|&(i, c)| t.as_bytes()[i] != c) || !matches!(t.as_bytes()[10], b'T' | b't' | b' ') {
        return None;
    }
    let rest = t[19..].trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    let offset = match rest {
        "Z" | "z" => 0,
        _ if rest.len() == 6 && rest.as_bytes()[3] == b':' => {
            let sign = match rest.as_bytes()[0] {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            sign * (rest[1..3].parse::<i64>().ok()? * 3600 + rest[4..6].parse::<i64>().ok()? * 60)
        }
        _ => return None,
    };
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if mo > 2 { mo - 3 } else { mo + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3600 + mi * 60 + se - offset)
}

/// What Codex returns when gathering the cloud.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatheredCloud {
    pub words: Vec<CloudWord>,
}

pub fn cloud_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["words"],
        "properties": {
            "words": {
                "type": "array",
                "description": "The subjects this person keeps writing about, as many as the writing supports and at most forty, most central first.",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["word", "weight", "written"],
                    "properties": {
                        "word": { "type": "string", "description": "A noun phrase: 2 to 10 characters in Japanese, 1 to 3 words in English. Not a generic word (こと, 仕事, 今日), not a person's or company's name, not a number." },
                        "weight": { "type": "integer", "description": "1 to 5: how often the person comes back to this subject across their writing. 5 means it runs through most pieces; 1 means it appears once or twice but clearly matters to them." },
                        "written": { "type": "boolean", "description": "True if one of the ARTICLES ALREADY WRITTEN is about this subject." }
                    }
                }
            }
        }
    })
}

/// The prompt that gathers the cloud from the person's writing.
pub fn cloud_prompt(policy: &Policy, samples: &[&str], existing: &[Existing], lang: Lang) -> String {
    let language = match lang {
        Lang::Ja => "Japanese",
        Lang::En => "English",
    };
    let policy_lines = policy.lines(lang);
    let policy_block = if policy_lines.is_empty() { "(none written yet)".to_string() } else { policy_lines.join("\n") };
    let existing_block = existing_block(existing);
    let existing_block = if existing_block.is_empty() { "(none yet)".to_string() } else { existing_block };
    format!(
        "Read this person's writing and list the subjects they keep coming back to: the things they think \
         about, argue about, and return to across pieces. List as many as the writing supports and no more \
         than forty (thirty when there is plenty), in {language}, most central first. When the writing is thin, \
         list fewer; never pad the list with generic words. Each is a short noun phrase a reader would \
         recognise as a subject (資金繰り, 撤退の基準, 採用面接), not a generic word, not a name, not a number. \
         Weight each 1 to 5 by how often the person returns to it, not by how often the string appears. Mark \
         `written` true when an article already written is about that subject.\n\n\
         EDITORIAL POLICY:\n{policy_block}\n\n\
         WRITING SAMPLES:\n{samples}\n\n\
         ARTICLES ALREADY WRITTEN:\n{existing_block}",
        samples = cloud_samples_block(samples),
    )
}

/// Keeps the cloud tidy: trims, drops empties and repeats, clamps weights,
/// orders heaviest first (Codex does not always), caps at forty.
pub fn tidy_cloud(words: Vec<CloudWord>) -> Vec<CloudWord> {
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<CloudWord> = words
        .into_iter()
        .map(|w| CloudWord { word: w.word.trim().to_string(), weight: w.weight.clamp(1, 5), written: w.written })
        .filter(|w| !w.word.is_empty())
        .filter(|w| seen.insert(w.word.to_lowercase()))
        .collect();
    out.sort_by(|a, b| b.weight.cmp(&a.weight));
    out.truncate(40);
    out
}

/// What the suggestion call returns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Topics {
    pub topics: Vec<String>,
}

pub fn topics_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["topics"],
        "properties": {
            "topics": {
                "type": "array",
                "description": "Titles the person could write next, one string each. Titles only: no subtitles, no explanation, no numbering.",
                "items": { "type": "string" }
            }
        }
    })
}

/// How much of each sample reaches a prompt. Titles need the person's
/// subjects, not their every sentence.
const SAMPLE_HEAD_CHARS: usize = 400;
const MAX_SAMPLES: usize = 24;

fn samples_block(samples: &[&str]) -> String {
    samples
        .iter()
        .filter(|s| !s.trim().is_empty())
        .take(MAX_SAMPLES)
        .enumerate()
        .map(|(i, s)| {
            let head: String = s.trim().chars().take(SAMPLE_HEAD_CHARS).collect();
            format!("--- {} ---\n{head}", i + 1)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// How much of the writing the cloud reads (#129, D-72). Unlike titles, it
/// drops no sample: the budget is shared, so the more samples, the shorter
/// each one gets.
const CLOUD_TOTAL_CHARS: usize = 40_000;
const CLOUD_SAMPLE_MAX_CHARS: usize = 2_000;

/// Every non-empty sample, each cut to its share of the budget. A cut sample
/// keeps its head and its tail (three to one): subjects show up where a piece
/// opens and where it closes.
fn cloud_samples_block(samples: &[&str]) -> String {
    let samples: Vec<&str> = samples.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    let Some(share) = CLOUD_TOTAL_CHARS.checked_div(samples.len()) else { return String::new() };
    let share = share.min(CLOUD_SAMPLE_MAX_CHARS);
    samples
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let chars: Vec<char> = s.chars().collect();
            let text: String = if chars.len() <= share {
                s.to_string()
            } else {
                let head = share * 3 / 4;
                let tail = share - head;
                let head: String = chars[..head].iter().collect();
                let tail: String = chars[chars.len() - tail..].iter().collect();
                format!("{head}\n…\n{tail}")
            };
            format!("--- {} ---\n{text}", i + 1)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn existing_block(existing: &[Existing]) -> String {
    existing
        .iter()
        .filter(|e| !e.title.trim().is_empty() || !e.first_line.trim().is_empty())
        .map(|e| {
            let title = if e.title.trim().is_empty() { "(no title)" } else { e.title.trim() };
            let line: String = e.first_line.trim().chars().take(80).collect();
            if line.is_empty() { format!("- {title}") } else { format!("- {title} — {line}") }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The prompt that asks for `n` titles. `direction` is the person's one
/// line for this round ("今回は採用寄りで") and may be empty.
pub fn topics_prompt(policy: &Policy, samples: &[&str], existing: &[Existing], subjects: &[String], direction: &str, n: usize, lang: Lang) -> String {
    let policy_lines = policy.lines(lang);
    let policy_block = if policy_lines.is_empty() {
        "(none written yet: infer the subjects from the writing samples)".to_string()
    } else {
        policy_lines.join("\n")
    };
    let direction = direction.trim();
    let direction_block = if direction.is_empty() { String::new() } else { format!("\nDIRECTION FOR THIS ROUND (weigh it heavily):\n{direction}\n") };
    let subjects: Vec<&str> = subjects.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    let subjects_block = if subjects.is_empty() { String::new() } else { format!("\nSUBJECTS THE PERSON PICKED FOR THIS ROUND (every title is about one or more of these):\n{}\n", subjects.join("、")) };
    let existing_block = existing_block(existing);
    let existing_block = if existing_block.is_empty() { "(none yet)".to_string() } else { existing_block };
    let language = match lang {
        Lang::Ja => "Japanese",
        Lang::En => "English",
    };
    format!(
        "Suggest {n} titles for articles this person could write next, in their own field and from their own \
         experience. The person will pick a few and have them written.\n\n\
         EDITORIAL POLICY (what to write about; the voice is handled elsewhere):\n{policy_block}\n\
         {subjects_block}{direction_block}\n\
         WRITING SAMPLES BY THIS PERSON (their subjects and stance; do not copy their titles):\n{samples}\n\n\
         ALREADY WRITTEN (do not repeat these or close variants):\n{existing}\n\n\
         Rules: each title is one line in {language}, concrete and specific, at most 30 characters in Japanese \
         or 12 words in English. It states a point or a question the person could argue from experience, not \
         a generic how-to, not a listicle, not news. No two titles cover the same ground. No titles that need \
         research the person has not done, no statistics, no other people's names or companies. Return exactly \
         {n} titles, most fitting first.",
        samples = samples_block(samples),
        existing = existing_block,
    )
}

pub fn policy_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["audience", "takeaway", "avoid"],
        "properties": {
            "audience": { "type": "string", "description": "Who the writing is for, in one short phrase." },
            "takeaway": { "type": "string", "description": "What a reader should take away, in one short sentence." },
            "avoid": { "type": "string", "description": "What this writing never touches, in one short phrase. Empty if nothing stands out." }
        }
    })
}

/// The prompt that drafts a policy from the person's writing and articles.
/// The person edits the result; nothing is saved from here.
pub fn policy_prompt(samples: &[&str], existing: &[Existing], lang: Lang) -> String {
    let language = match lang {
        Lang::Ja => "Japanese",
        Lang::En => "English",
    };
    let existing_block = existing_block(existing);
    let existing_block = if existing_block.is_empty() { "(none yet)".to_string() } else { existing_block };
    format!(
        "Read this person's writing and draft their editorial policy: three short fields describing what they \
         write about (not how; the voice is handled elsewhere). Write it as the person would state it \
         themselves, in {language}, in plain words. Stay within what the writing shows; do not invent a \
         business or an audience it does not point to.\n\n\
         WRITING SAMPLES:\n{samples}\n\n\
         ARTICLES ALREADY WRITTEN:\n{existing}\n\n\
         `audience`: who this is for (one phrase). `takeaway`: what a reader should leave with (one sentence). \
         `avoid`: what the writing steers clear of, if anything stands out; otherwise an empty string.",
        samples = samples_block(samples),
        existing = existing_block,
    )
}

/// What the brief suggestion returns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuggestedBrief {
    pub brief: String,
}

pub fn brief_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["brief"],
        "properties": {
            "brief": { "type": "string", "description": "The brief: two to four short sentences in the person's language saying what the article is about, who it is for, and the two or three points it makes from the person's own experience. Plain prose, no headings, no bullets, no title line." }
        }
    })
}

/// The prompt that drafts a brief for one article from its title (#97).
/// The person edits the result before writing; nothing is saved from here.
pub fn brief_prompt(title: &str, policy: &Policy, samples: &[&str], existing: &[Existing], lang: Lang) -> String {
    let language = match lang {
        Lang::Ja => "Japanese",
        Lang::En => "English",
    };
    let policy_lines = policy.lines(lang);
    let policy_block = if policy_lines.is_empty() { "(none written yet)".to_string() } else { policy_lines.join("\n") };
    let existing_block = existing_block(existing);
    let existing_block = if existing_block.is_empty() { "(none yet)".to_string() } else { existing_block };
    format!(
        "Write the brief for an article this person is about to write, titled:\n{title}\n\n\
         A brief says what the article is about, who it is for, and the two or three points it makes. \
         Two to four short sentences in {language}, in the person's own words as if they wrote the note to \
         themselves. Draw the points from what their writing shows they know and think; do not invent \
         facts, figures or events, and do not add research. Do not repeat an article they already wrote.\n\n\
         EDITORIAL POLICY:\n{policy_block}\n\n\
         WRITING SAMPLES BY THIS PERSON:\n{samples}\n\n\
         ARTICLES ALREADY WRITTEN:\n{existing_block}",
        title = title.trim(),
        samples = samples_block(samples),
    )
}

/// The brief a queued article starts with. This is how the policy, the
/// direction, the length and the no-research rule reach generation without
/// changing `post::generation_prompt` (D-28/D-29/D-59). The person sees and
/// can edit it in the editor like any brief.
pub fn brief_for(title: &str, policy: &Policy, subjects: &[String], direction: &str, lang: Lang) -> String {
    let mut lines = Vec::new();
    let title = title.trim();
    let direction = direction.trim();
    let subjects: Vec<&str> = subjects.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    match lang {
        Lang::Ja => {
            lines.push(format!("題: {title}"));
            lines.extend(policy.lines(lang));
            if !subjects.is_empty() {
                lines.push(format!("題材: {}", subjects.join("、")));
            }
            if !direction.is_empty() {
                lines.push(format!("今回の方向: {direction}"));
            }
            lines.push("長さ: 1,500〜3,000 字。".to_string());
            lines.push("外部の事実は調べず、自分の経験と意見の範囲で書く。数字や出来事を断定しない。".to_string());
        }
        Lang::En => {
            lines.push(format!("Title: {title}"));
            lines.extend(policy.lines(lang));
            if !subjects.is_empty() {
                lines.push(format!("Subjects: {}", subjects.join(", ")));
            }
            if !direction.is_empty() {
                lines.push(format!("Direction this round: {direction}"));
            }
            lines.push("Length: 800 to 1,500 words.".to_string());
            lines.push("Do not research outside facts; write from your own experience and opinion, and do not assert figures or events.".to_string());
        }
    }
    lines.join("\n")
}

/// Drops titles already written (same text after trimming and case), drops
/// duplicates among the suggestions, and keeps at most `n`.
pub fn dedupe(titles: Vec<String>, existing: &[Existing], n: usize) -> Vec<String> {
    let norm = |s: &str| s.trim().to_lowercase().replace(char::is_whitespace, "");
    let taken: std::collections::HashSet<String> = existing.iter().map(|e| norm(&e.title)).filter(|t| !t.is_empty()).collect();
    let mut seen = std::collections::HashSet::new();
    titles
        .into_iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .filter(|t| !taken.contains(&norm(t)))
        .filter(|t| seen.insert(norm(t)))
        .take(n)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn existing(title: &str) -> Existing {
        Existing { title: title.into(), first_line: String::new() }
    }

    #[test]
    fn dedupe_drops_written_titles_and_repeats() {
        let out = dedupe(
            vec!["資金繰りは週で見る".into(), " 資金繰りは週で見る ".into(), "撤退の基準".into(), "".into(), "採用の話".into()],
            &[existing("撤退の基準")],
            10,
        );
        assert_eq!(out, vec!["資金繰りは週で見る", "採用の話"]);
    }

    #[test]
    fn dedupe_keeps_at_most_n() {
        let out = dedupe((0..20).map(|i| format!("題 {i}")).collect(), &[], 10);
        assert_eq!(out.len(), 10);
    }

    #[test]
    fn brief_carries_policy_direction_length_and_no_research() {
        let policy = Policy { audience: "これから会社を買う経営者".into(), takeaway: String::new(), topics: vec!["資金繰り".into(), "".into()], avoid: "個別の会社名".into() };
        let b = brief_for("撤退の基準は、始める前に決める", &policy, &["撤退".into(), "".into()], "今回は採用寄りで", Lang::Ja);
        assert!(b.contains("題材: 撤退\n今回の方向: 今回は採用寄りで"));
        assert!(b.starts_with("題: 撤退の基準は、始める前に決める\n"));
        assert!(b.contains("誰に向けて: これから会社を買う経営者"));
        assert!(!b.contains("持ち帰ってほしいこと"));
        assert!(!b.contains("よく書く題材"));
        assert!(b.contains("今回の方向: 今回は採用寄りで"));
        assert!(b.contains("1,500〜3,000 字"));
        assert!(b.contains("調べず"));
    }

    #[test]
    fn empty_policy_still_makes_a_prompt_and_a_brief() {
        let p = Policy::default();
        assert!(p.is_empty());
        let prompt = topics_prompt(&p, &["資本政策の相談で最初に聞くのは"], &[], &[], "", 10, Lang::Ja);
        assert!(!prompt.contains("SUBJECTS THE PERSON PICKED"));
        assert!(prompt.contains("(none written yet"));
        assert!(prompt.contains("exactly 10 titles"));
        let b = brief_for("題", &p, &[], "", Lang::Ja);
        assert_eq!(b.lines().count(), 3);
    }

    #[test]
    fn brief_prompt_carries_title_and_policy() {
        let policy = Policy { audience: "経営者".into(), ..Default::default() };
        let p = brief_prompt("撤退の基準は、始める前に決める", &policy, &["本文"], &[existing("前の記事")], Lang::Ja);
        assert!(p.contains("titled:\n撤退の基準は、始める前に決める"));
        assert!(p.contains("誰に向けて: 経営者"));
        assert!(p.contains("- 前の記事"));
        assert!(p.contains("Japanese"));
    }

    #[test]
    fn subjects_reach_the_title_prompt_as_their_own_block() {
        let p = topics_prompt(&Policy::default(), &["x"], &[], &["資金繰り".into(), "採用".into()], "今回は短めに", 10, Lang::Ja);
        assert!(p.contains("SUBJECTS THE PERSON PICKED FOR THIS ROUND (every title is about one or more of these):\n資金繰り、採用\n"));
        assert!(p.contains("DIRECTION FOR THIS ROUND"));
    }

    #[test]
    fn tidy_cloud_drops_repeats_and_clamps() {
        let out = tidy_cloud(vec![
            CloudWord { word: " 資金繰り ".into(), weight: 9, written: false },
            CloudWord { word: "資金繰り".into(), weight: 3, written: true },
            CloudWord { word: "".into(), weight: 3, written: false },
            CloudWord { word: "採用".into(), weight: 0, written: true },
        ]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], CloudWord { word: "資金繰り".into(), weight: 5, written: false });
        assert_eq!(out[1].weight, 1);
        let sorted = tidy_cloud(vec![CloudWord { word: "a".into(), weight: 1, written: false }, CloudWord { word: "b".into(), weight: 4, written: false }]);
        assert_eq!(sorted[0].word, "b");
    }

    #[test]
    fn a_cloud_counted_before_129_is_recounted_as_the_writing_it_was_gathered_from() {
        // 10 pieces of writing and 20 articles: saved as 30 before #129.
        // 2026-09-20T00:00:00Z is 1_789_862_400.
        let mut cloud = TopicCloud { words: vec![], gathered_at: "1789862400".into(), material_count: 30, counting: 0 };
        let mut times: Vec<String> = (0..10).map(|i| format!("2026-09-19T12:00:{i:02}.123456+00:00")).collect();
        // Written the same second as the gathering, in another offset: already there.
        times.push("2026-09-20T09:00:00+09:00".into());
        recount(&mut cloud, &times, 0);
        assert_eq!((cloud.material_count, cloud.counting), (11, CLOUD_COUNTING));
        // One more piece added after gathering: now 12 against 11, so "more since".
        let mut cloud = TopicCloud { words: vec![], gathered_at: "1789862400".into(), material_count: 30, counting: 0 };
        times.push("2026-09-25T08:00:00.5Z".into());
        recount(&mut cloud, &times, 0);
        assert_eq!(cloud.material_count, 11);
        assert!(times.len() > cloud.material_count);
    }

    #[test]
    fn a_current_cloud_is_left_as_counted() {
        let mut cloud = TopicCloud { words: vec![], gathered_at: "1789862400".into(), material_count: 4, counting: CLOUD_COUNTING };
        recount(&mut cloud, &["2026-09-25T08:00:00Z".to_string()], 0);
        assert_eq!(cloud.material_count, 4);
    }

    #[test]
    fn a_cloud_whose_time_cannot_be_read_counts_everything_as_already_there() {
        let mut cloud = TopicCloud { words: vec![], gathered_at: String::new(), material_count: 30, counting: 0 };
        recount(&mut cloud, &["2026-09-25T08:00:00Z".to_string(), "?".to_string()], 0);
        assert_eq!(cloud.material_count, 2);
        let old: TopicCloud = serde_json::from_str(r#"{"words":[],"gathered_at":"1","material_count":3}"#).unwrap();
        assert_eq!(old.counting, 0);
    }

    fn article(body: &str, lita_wrote: &[&str]) -> ArticleText {
        ArticleText { body: body.into(), lita_wrote: Some(lita_wrote.iter().map(|s| s.to_string()).collect()) }
    }

    #[test]
    fn an_article_whose_versions_were_not_read_is_not_the_persons() {
        let unread = ArticleText { body: "Lita が書いたかもしれない本文".into(), lita_wrote: None };
        assert!(!unread.edited_by_person());
        assert!(own_bodies(std::slice::from_ref(&unread)).is_empty());
        assert_eq!(material_count(2, &[unread]), 2);
    }

    #[test]
    fn a_cloud_is_recounted_once_so_a_later_edit_makes_more_since() {
        // Saved by stage 1 (counting 1): two pieces then, one edited article now.
        let mut cloud = TopicCloud { words: vec![], gathered_at: "1789862400".into(), material_count: 2, counting: 1 };
        let times = vec!["2026-09-19T00:00:00Z".to_string(), "2026-09-19T00:00:01Z".to_string()];
        let mut articles = vec![article("直した", &["Lita"]), article("Lita の二本目", &["Lita の二本目"])];
        let edited = articles.iter().filter(|a| a.edited_by_person()).count();
        assert!(recount(&mut cloud, &times, edited), "counted again, so the caller saves it");
        assert!(material_count(2, &articles) <= cloud.material_count);
        // Opened again after the person edits the second article: the saved
        // cloud is not counted again, and the edit is more since.
        articles[1].body = "Lita の二本目を自分で直した".into();
        let edited = articles.iter().filter(|a| a.edited_by_person()).count();
        assert!(!recount(&mut cloud, &times, edited), "already saved under the current counting");
        assert_eq!(cloud.material_count, 3);
        assert!(material_count(2, &articles) > cloud.material_count);
    }

    #[test]
    fn an_article_is_the_persons_only_when_they_changed_what_lita_wrote() {
        // Written by Lita and left as it is (a trailing newline is not an edit).
        assert!(!article("Lita の本文", &["Lita の本文"]).edited_by_person());
        assert!(!article("Lita の本文\n", &["Lita の本文"]).edited_by_person());
        // Changed after Lita's last text, or never written by Lita.
        assert!(article("Lita の本文に、自分で足した一文", &["Lita の本文"]).edited_by_person());
        assert!(article("自分で書いた本文", &[]).edited_by_person());
        // Put back to an earlier text Lita wrote: still Lita's words.
        assert!(!article("最初の本文", &["短くした本文", "最初の本文"]).edited_by_person());
        // Nothing written yet (an empty draft, a queued article).
        assert!(!article("  ", &[]).edited_by_person());
    }

    #[test]
    fn articles_lita_only_wrote_do_not_make_more_since_but_an_edit_does() {
        // Acceptance 5: gathered from two pieces of writing.
        let cloud = TopicCloud { words: vec![], gathered_at: "1789862400".into(), material_count: material_count(2, &[]), counting: CLOUD_COUNTING };
        // Three articles queued and written by Lita: nothing new.
        let mut written = vec![article("一本目", &["一本目"]), article("二本目", &["二本目"]), article("三本目", &["三本目"])];
        assert!(material_count(2, &written) <= cloud.material_count);
        // The person edits one: "more since".
        written[1].body = "二本目を自分で直した".into();
        assert!(material_count(2, &written) > cloud.material_count);
    }

    #[test]
    fn the_cloud_reads_the_body_the_person_edited_and_not_what_lita_left() {
        let articles = vec![
            article("Lita の本文。題材ゼロについて。", &["Lita の本文。題材ゼロについて。"]),
            article("Lita の本文を直して、題材イチについて書き足した。", &["Lita の本文。"]),
            article("自分で書いた。題材ニについて。", &[]),
        ];
        let mut samples = vec!["元にした文章"];
        samples.extend(own_bodies(&articles));
        let p = cloud_prompt(&Policy::default(), &samples, &[], Lang::Ja);
        assert!(p.contains("題材イチについて") && p.contains("題材ニについて") && p.contains("元にした文章"));
        assert!(!p.contains("題材ゼロについて"));
    }

    #[test]
    fn a_cloud_counted_before_edited_articles_counted_is_recounted_with_them_as_already_there() {
        // Saved by #129 stage 1 (writing only, counting 1): two pieces then,
        // one edited article now. Counted again as 3, so nothing is new.
        let mut cloud = TopicCloud { words: vec![], gathered_at: "1789862400".into(), material_count: 2, counting: 1 };
        let times = vec!["2026-09-19T00:00:00Z".to_string(), "2026-09-19T00:00:01Z".to_string()];
        recount(&mut cloud, &times, 1);
        assert_eq!((cloud.material_count, cloud.counting), (3, CLOUD_COUNTING));
        assert!(material_count(2, &[article("直した", &["Lita"])]) <= cloud.material_count);
    }

    #[test]
    fn epoch_seconds_reads_what_postgres_returns() {
        assert_eq!(epoch_seconds("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(epoch_seconds("2026-09-20T00:00:00+00:00"), Some(1_789_862_400));
        assert_eq!(epoch_seconds("2026-09-20 09:00:00.999+09:00"), Some(1_789_862_400));
        assert_eq!(epoch_seconds("2024-02-29T23:59:59-01:30"), Some(1_709_251_199 + 5_400));
        assert_eq!(epoch_seconds("yesterday"), None);
    }

    #[test]
    fn cloud_reads_every_sample_even_past_the_twenty_fifth() {
        let samples: Vec<String> = (0..30).map(|i| format!("{i} 番目の文章。題材{i}について書いた。")).collect();
        let refs: Vec<&str> = samples.iter().map(String::as_str).collect();
        let p = cloud_prompt(&Policy::default(), &refs, &[], Lang::Ja);
        assert!(p.contains("題材29について"));
        assert!(p.contains("--- 30 ---"));
        // Titles still read the first 24, heads only (the wait there matters).
        let t = topics_prompt(&Policy::default(), &refs, &[], &[], "", 10, Lang::Ja);
        assert!(!t.contains("題材29について"));
    }

    #[test]
    fn cloud_takes_head_and_tail_within_the_budget() {
        // One long sample: at most 2,000 characters, head and tail both kept.
        let long = format!("冒頭の題材{}締めの題材", "あ".repeat(10_000));
        let block = cloud_samples_block(&[long.as_str()]);
        assert!(block.contains("冒頭の題材"));
        assert!(block.contains("締めの題材"));
        assert!(block.chars().count() < CLOUD_SAMPLE_MAX_CHARS + 50);
        // Many long samples: none dropped, the whole stays near 40,000.
        let many: Vec<String> = (0..60).map(|i| format!("頭{i}頭{}尾{i}尾", "い".repeat(5_000))).collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        let block = cloud_samples_block(&refs);
        assert!(block.contains("頭59頭") && block.contains("尾59尾") && block.contains("頭0頭"));
        assert!(block.chars().count() < CLOUD_TOTAL_CHARS + 60 * 20);
        // A short sample goes in whole, without a gap mark.
        assert_eq!(cloud_samples_block(&["短い文章", "  "]), "--- 1 ---\n短い文章");
    }

    #[test]
    fn cloud_asks_only_for_what_the_writing_supports() {
        let p = cloud_prompt(&Policy::default(), &["x"], &[], Lang::Ja);
        assert!(p.contains("as many as the writing supports"));
        assert!(p.contains("no more than forty"));
        assert!(!p.contains("no fewer than twenty"));
    }

    #[test]
    fn prompts_never_carry_credentials() {
        let p = topics_prompt(&Policy::default(), &["x"], &[existing("y")], &["s".into()], "z", 10, Lang::En);
        crate::assert_no_credentials(&p);
        crate::assert_no_credentials(&cloud_prompt(&Policy::default(), &["x"], &[], Lang::Ja));
        crate::assert_no_credentials(&policy_prompt(&["x"], &[], Lang::Ja));
    }
}
