//! What can be counted is counted here, not guessed by the model (voice
//! quality requirement, 2026-09-23, must 4). Everything is a pure function
//! of the material, so two builds on the same pieces measure the same.
//!
//! Sentence and paragraph splitting are deliberately simple: 。！？!? plus a
//! closing bracket end a sentence; a blank line ends a paragraph. Good
//! enough to describe rhythm, not a parser.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Measured facts about a body of writing. Integers only: percentages are
/// whole percent, lengths are characters.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Measured {
    pub pieces: usize,
    pub chars: usize,
    pub sentences: usize,
    pub sentence_length: SentenceLength,
    pub paragraphs: Paragraphs,
    pub punctuation: Punctuation,
    pub script: Script,
    /// Sentence-ending forms (the last few characters before the final
    /// punctuation), most frequent first, with counts.
    pub endings: Vec<Counted>,
    /// First-person pronouns seen, with counts.
    pub first_person: Vec<Counted>,
    pub emoji: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counted {
    pub form: String,
    pub count: usize,
    /// In how many distinct pieces it appears.
    pub pieces: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SentenceLength {
    pub median: usize,
    pub p10: usize,
    pub p90: usize,
    /// Percent of sentences at or under 15 characters.
    pub short_pct: usize,
    /// Percent of sentences over 60 characters.
    pub long_pct: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Paragraphs {
    pub count: usize,
    pub median_chars: usize,
    /// Sentences per paragraph, times ten (2.5 sentences → 25).
    pub sentences_per_paragraph_x10: usize,
    /// Percent of paragraphs that are a single sentence.
    pub one_sentence_pct: usize,
}

/// Counts per 10,000 characters.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Punctuation {
    pub comma: usize,
    pub period: usize,
    pub quote_brackets: usize,
    pub ellipsis: usize,
    pub exclamation: usize,
    pub question: usize,
    pub parentheses: usize,
    pub middle_dot: usize,
    pub dash: usize,
}

/// Percent of characters by script (whitespace and punctuation excluded).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Script {
    pub kanji_pct: usize,
    pub hiragana_pct: usize,
    pub katakana_pct: usize,
    pub latin_pct: usize,
}

const PRONOUNS: &[&str] = &["私たち", "私達", "僕ら", "僕たち", "我々", "われわれ", "わたし", "私", "僕", "ぼく", "俺", "自分", "弊社", "当社", "筆者", "I "];

pub fn split_sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            if !cur.trim().is_empty() { out.push(cur.trim().to_string()); }
            cur.clear();
            i += 1;
            continue;
        }
        cur.push(c);
        if matches!(c, '。' | '！' | '？' | '!' | '?') {
            // Swallow a closing bracket or repeated punctuation that follows.
            while i + 1 < chars.len() && matches!(chars[i + 1], '」' | '』' | '）' | ')' | '。' | '！' | '？' | '!' | '?') {
                i += 1;
                cur.push(chars[i]);
            }
            if !cur.trim().is_empty() { out.push(cur.trim().to_string()); }
            cur.clear();
        }
        i += 1;
    }
    if !cur.trim().is_empty() { out.push(cur.trim().to_string()); }
    out
}

pub fn split_paragraphs(text: &str) -> Vec<String> {
    text.split("\n\n").map(|p| p.trim()).filter(|p| !p.is_empty()).map(|p| p.to_string()).collect()
}

fn percentile(sorted: &[usize], p: f64) -> usize {
    if sorted.is_empty() { return 0; }
    let idx = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

/// The last form of a sentence: up to four characters before its final
/// punctuation, cut at a particle boundary where it helps grouping
/// (「のだろうか。」「と思う。」「です。」).
fn ending_form(sentence: &str) -> Option<String> {
    let trimmed: String = sentence.trim_end_matches(|c: char| matches!(c, '」' | '』' | '）' | ')')).to_string();
    let last = trimmed.chars().last()?;
    if !matches!(last, '。' | '！' | '？' | '!' | '?') { return None; }
    let body: Vec<char> = trimmed.chars().collect();
    let end = body.len() - 1;
    let start = end.saturating_sub(4);
    Some(body[start..=end].iter().collect())
}

fn is_kanji(c: char) -> bool { ('\u{4E00}'..='\u{9FFF}').contains(&c) || ('\u{3400}'..='\u{4DBF}').contains(&c) || c == '々' }
fn is_hiragana(c: char) -> bool { ('\u{3040}'..='\u{309F}').contains(&c) }
fn is_katakana(c: char) -> bool { ('\u{30A0}'..='\u{30FF}').contains(&c) || ('\u{31F0}'..='\u{31FF}').contains(&c) }
fn is_emoji(c: char) -> bool {
    let u = c as u32;
    (0x1F300..=0x1FAFF).contains(&u) || (0x2600..=0x27BF).contains(&u) || (0x1F000..=0x1F2FF).contains(&u)
}

pub fn measure<S: AsRef<str>>(pieces: &[S]) -> Measured {
    let mut m = Measured::default();
    let mut lengths: Vec<usize> = Vec::new();
    let mut para_chars: Vec<usize> = Vec::new();
    let mut para_sentences: Vec<usize> = Vec::new();
    let mut endings: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut pronouns: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    let (mut kanji, mut hira, mut kata, mut latin, mut letters) = (0usize, 0usize, 0usize, 0usize, 0usize);
    let mut p = Punctuation::default();

    for piece in pieces {
        let text = piece.as_ref();
        if text.trim().is_empty() { continue; }
        m.pieces += 1;
        m.chars += text.chars().count();
        let mut seen_endings: BTreeMap<String, usize> = BTreeMap::new();
        for para in split_paragraphs(text) {
            let sents = split_sentences(&para);
            para_chars.push(para.chars().count());
            para_sentences.push(sents.len().max(1));
            for s in &sents {
                lengths.push(s.chars().count());
                if let Some(e) = ending_form(s) { *seen_endings.entry(e).or_default() += 1; }
            }
        }
        for (e, n) in seen_endings {
            let entry = endings.entry(e).or_default();
            entry.0 += n;
            entry.1 += 1;
        }
        for pr in PRONOUNS {
            let n = text.matches(pr).count();
            if n > 0 { let e = pronouns.entry(pr).or_default(); e.0 += n; e.1 += 1; }
        }
        for c in text.chars() {
            match c {
                '、' | ',' => p.comma += 1,
                '。' => p.period += 1,
                '「' | '『' => p.quote_brackets += 1,
                '…' => p.ellipsis += 1,
                '！' | '!' => p.exclamation += 1,
                '？' | '?' => p.question += 1,
                '（' | '(' => p.parentheses += 1,
                '・' => p.middle_dot += 1,
                '—' | '―' | 'ー' if false => p.dash += 1,
                '—' | '―' | '–' => p.dash += 1,
                _ => {}
            }
            if is_emoji(c) { m.emoji += 1; }
            if c.is_whitespace() || c.is_ascii_punctuation() || matches!(c, '、' | '。' | '「' | '」' | '『' | '』' | '（' | '）' | '・' | '…' | '！' | '？' | '—' | '―') { continue; }
            letters += 1;
            if is_kanji(c) { kanji += 1 } else if is_hiragana(c) { hira += 1 } else if is_katakana(c) { kata += 1 } else if c.is_ascii_alphanumeric() { latin += 1 }
        }
    }

    m.sentences = lengths.len();
    let mut sorted = lengths.clone();
    sorted.sort_unstable();
    let pct = |n: usize, d: usize| if d == 0 { 0 } else { (n * 100 + d / 2) / d };
    m.sentence_length = SentenceLength {
        median: percentile(&sorted, 0.5),
        p10: percentile(&sorted, 0.1),
        p90: percentile(&sorted, 0.9),
        short_pct: pct(lengths.iter().filter(|&&l| l <= 15).count(), lengths.len()),
        long_pct: pct(lengths.iter().filter(|&&l| l > 60).count(), lengths.len()),
    };
    let mut pc = para_chars.clone();
    pc.sort_unstable();
    let total_sents: usize = para_sentences.iter().sum();
    m.paragraphs = Paragraphs {
        count: para_chars.len(),
        median_chars: percentile(&pc, 0.5),
        sentences_per_paragraph_x10: if para_sentences.is_empty() { 0 } else { total_sents * 10 / para_sentences.len() },
        one_sentence_pct: pct(para_sentences.iter().filter(|&&n| n == 1).count(), para_sentences.len()),
    };
    let per10k = |n: usize| if m.chars == 0 { 0 } else { n * 10_000 / m.chars };
    m.punctuation = Punctuation {
        comma: per10k(p.comma), period: per10k(p.period), quote_brackets: per10k(p.quote_brackets), ellipsis: per10k(p.ellipsis),
        exclamation: per10k(p.exclamation), question: per10k(p.question), parentheses: per10k(p.parentheses), middle_dot: per10k(p.middle_dot), dash: per10k(p.dash),
    };
    m.script = Script { kanji_pct: pct(kanji, letters), hiragana_pct: pct(hira, letters), katakana_pct: pct(kata, letters), latin_pct: pct(latin, letters) };
    let mut ends: Vec<Counted> = endings.into_iter().map(|(form, (count, pieces))| Counted { form, count, pieces }).collect();
    ends.sort_by(|a, b| b.count.cmp(&a.count).then(a.form.cmp(&b.form)));
    ends.truncate(10);
    m.endings = ends;
    let mut prs: Vec<Counted> = pronouns.into_iter().map(|(form, (count, pieces))| Counted { form: form.trim().to_string(), count, pieces }).collect();
    prs.sort_by(|a, b| b.count.cmp(&a.count));
    m.first_person = prs;
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_sentences_on_japanese_punctuation() {
        let s = split_sentences("これは一つ目。「二つ目だ！」三つ目か？\n四つ目");
        assert_eq!(s, vec!["これは一つ目。", "「二つ目だ！」", "三つ目か？", "四つ目"]);
    }

    #[test]
    fn measures_the_same_twice() {
        let pieces = ["私は思う。短い。とても長い文をここに書いてみるとどうなるのだろうか、と思うのである。\n\n二段落目だ。", "僕はこう考える。私も。"];
        let a = measure(&pieces);
        let b = measure(&pieces);
        assert_eq!(a, b);
        assert_eq!(a.pieces, 2);
        assert_eq!(a.paragraphs.count, 3);
        assert_eq!(a.first_person[0].form, "私");
        assert_eq!(a.first_person[0].pieces, 2);
        assert!(a.endings.iter().any(|e| e.form.ends_with("思う。")));
        assert!(a.script.kanji_pct + a.script.hiragana_pct + a.script.katakana_pct + a.script.latin_pct <= 100);
    }

    #[test]
    fn empty_material_is_all_zero() {
        let m = measure::<&str>(&[]);
        assert_eq!(m, Measured::default());
    }
}
