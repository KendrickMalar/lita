//! Voices bundled with Lita, for someone who has nothing of their own to
//! start from (D-70, 2026-09-24). Each is a hand-written `VoiceProfile`:
//! no Codex call, no evidence, no confidence. Picking one copies it into
//! the person's own voices, where it can be edited and, once their own
//! writing is added, relearned like any other.

use serde::Serialize;

use crate::voice::{VoiceProfile, WordUsage};

/// A bundled voice as listed: enough to choose by.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PresetSummary {
    pub id: &'static str,
    pub name: &'static str,
    pub one_line: &'static str,
}

pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    pub profile: fn() -> VoiceProfile,
}

const PRESETS: &[Preset] = &[Preset { id: "pr-polite-ja", name: "広報のです・ます", profile: pr_polite_ja }];

pub fn list() -> Vec<PresetSummary> {
    PRESETS.iter().map(|p| PresetSummary { id: p.id, name: p.name, one_line: (p.profile)().one_line.leak() }).collect()
}

pub fn find(id: &str) -> Option<(&'static str, VoiceProfile)> {
    PRESETS.iter().find(|p| p.id == id).map(|p| (p.name, (p.profile)()))
}

/// 広報のです・ます: a company account's plain, courteous voice. Reads
/// like a person on the team, not a press release.
fn pr_polite_ja() -> VoiceProfile {
    VoiceProfile {
        language: "ja".into(),
        first_person: "私たち".into(),
        formality: "です・ます調。敬語は丁寧語まで。謙譲語・尊敬語を重ねない".into(),
        tone: vec!["丁寧".into(), "平易".into(), "前向き".into(), "落ち着いている".into()],
        sentence_endings: vec!["です".into(), "ます".into(), "ました".into(), "ません".into(), "でしょうか".into()],
        avg_sentence_length_chars: 38,
        preferred_words: vec!["お伝えします".into(), "ご紹介します".into(), "いただく".into(), "ぜひ".into(), "少しずつ".into(), "背景".into()],
        avoided_words: vec!["弊社".into(), "貴社".into(), "させていただく".into(), "ソリューション".into(), "シナジー".into(), "！".into(), "絶対".into(), "業界初".into()],
        opens_with: "何の話かを 1 文で言い切ってから、背景を 1〜2 文で添える。挨拶や時候の言葉は置かない".into(),
        closes_with: "読者にしてほしいことを 1 つだけ、穏やかに言う。「ぜひ〜ください」の形か、次に伝える予定で締める".into(),
        uses_emoji: false,
        representative_excerpts: vec![],
        one_line: "あなたの文章は、です・ます調で平易に、要点を先に言い、読者への一つのお願いで静かに締めます。".into(),
        measured: None,
        kana_choices: vec!["できる".into(), "こと".into(), "ため".into(), "ほど".into(), "とき".into(), "いただく".into()],
        rhetoric: "結論 → 背景 → 具体 → お願い、の順に進む。段落は 2〜4 文。見出しは要点の言い換えで、問いかけにしない。比喩や誇張を使わず、事実と予定を分けて書く".into(),
        examples_and_numbers: "例は自分たちの取り組みから取り、数字は出所が言えるものだけを丸めずに書く。数字が無ければ無理に足さない".into(),
        never_does: vec![
            "感嘆符で盛り上げない".into(),
            "読者を煽らない（「今すぐ」「見逃せない」を使わない）".into(),
            "他社や競合の名前を出さない".into(),
            "社内用語や略語をそのまま使わない".into(),
        ],
        word_usage: vec![
            WordUsage { word: "お伝えします".into(), usage: "冒頭の 1 文で、何の話かを示すときに使う".into() },
            WordUsage { word: "ぜひ".into(), usage: "締めのお願いに 1 回だけ添える。本文中では使わない".into() },
            WordUsage { word: "背景".into(), usage: "決めた理由を説明する段落の入りで使う（「背景として」）".into() },
        ],
        topic_words: vec![],
        backing: Default::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_list_and_resolve() {
        let l = list();
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].id, "pr-polite-ja");
        let (name, p) = find("pr-polite-ja").unwrap();
        assert_eq!(name, "広報のです・ます");
        assert_eq!(p.language, "ja");
        assert!(p.backing.is_empty());
        assert!(find("nope").is_none());
    }

    #[test]
    fn preset_profile_generates_the_full_contract() {
        let (_, p) = find("pr-polite-ja").unwrap();
        let view = p.generation_view();
        assert_eq!(view.as_object().unwrap().len(), 11);
        assert!(view["preferred_words"].as_array().unwrap().len() >= 4);
    }
}
