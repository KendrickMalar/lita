//! Function words and stock phrases that must never be offered as
//! 「あなたらしい言葉」 (voice quality requirement, must 8). Built in so the
//! filter needs no dictionary or tokenizer; extend when a generic word slips
//! through in practice. Stance adverbs and interjections (ぜひ, 一体, 結局,
//! さっさと) are NOT here: they carry voice, and the first PoC lost the
//! writer when they were dropped (2026-09-23).

const JA: &[&str] = &[
    // particles, copulas, auxiliaries
    "の", "に", "は", "を", "が", "と", "で", "も", "へ", "や", "か", "な", "ね", "よ", "ぞ", "さ", "わ", "から", "まで", "より", "だけ", "ほど", "くらい", "ぐらい", "など", "なんて", "しか", "こそ", "でも", "ても", "って", "とか", "たり", "ながら", "つつ", "ので", "のに", "けど", "けれど", "けれども", "しかし", "だが", "でも", "それで", "そして", "また", "さらに", "つまり", "ただ", "ただし", "なお", "ところで", "さて", "では", "じゃあ", "いわゆる", "いわば", "たとえば", "例えば", "なぜなら", "ちなみに", "というのも", "それでも", "それに", "そのため", "したがって", "よって", "ゆえに",
    "です", "ます", "でした", "ました", "ません", "でしょう", "ましょう", "だ", "である", "だった", "ではない", "じゃない", "ない", "なかった", "たい", "たく", "らしい", "ようだ", "みたい", "そうだ", "べき", "はず", "つもり", "わけ", "こと", "もの", "ところ", "とき", "時", "場合", "ため", "ほう", "方", "よう", "気", "感じ", "ふう", "風",
    // pronouns and demonstratives
    "これ", "それ", "あれ", "どれ", "この", "その", "あの", "どの", "ここ", "そこ", "あそこ", "どこ", "こう", "そう", "ああ", "どう", "こんな", "そんな", "あんな", "どんな", "こういう", "そういう", "ああいう", "どういう", "誰", "だれ", "何", "なに", "なん", "何を", "何か", "いつ", "私", "僕", "俺", "自分", "あなた", "彼", "彼女", "我々", "私たち", "僕ら", "みんな", "皆", "人", "ひと", "人たち", "自身",
    // common verbs / adjectives / adverbs
    "する", "した", "して", "します", "しない", "できる", "出来る", "できない", "なる", "なった", "なって", "なります", "ある", "あった", "あって", "あります", "いる", "いた", "いて", "います", "いう", "言う", "言った", "思う", "思います", "思った", "考える", "考えて", "見る", "見て", "行く", "来る", "くる", "いく", "やる", "やって", "わかる", "分かる", "知る", "知って", "使う", "作る", "書く", "読む", "聞く", "話す", "持つ", "出る", "出す", "入る", "取る", "置く", "つける", "続ける", "始める", "終わる", "変わる", "変える", "感じる", "みる", "しまう", "おく", "くれる", "もらう", "あげる",
    "いい", "良い", "よい", "悪い", "多い", "少ない", "大きい", "小さい", "高い", "低い", "早い", "遅い", "新しい", "古い", "長い", "短い", "強い", "弱い", "同じ", "違う", "大事", "大切", "必要", "重要", "簡単", "難しい", "面白い", "すごい", "とても", "非常に", "少し", "ちょっと", "もっと", "ずっと", "まだ", "もう", "すぐ", "まず", "先に", "次に", "最後に", "最初に", "今", "いま", "今日", "昨日", "明日", "最近", "以前", "今回", "前回", "後で", "あとで", "後", "前", "上", "下", "中", "外", "間", "ほか", "他", "全部", "全て", "すべて", "一つ", "ひとつ", "一番", "いちばん", "実際", "特に", "とくに", "基本的に", "個人的に", "たぶん", "多分", "おそらく", "必ず", "あまり", "ほとんど", "だいたい", "いろいろ", "色々", "さまざま", "様々", "ような", "ように", "みたいな", "的な", "的に", "という", "といった", "として", "について", "に関して", "に対して", "によって", "における", "ための", "ことが", "ことは", "ことを", "ものが", "ものは", "のは", "のが", "のを",
    "はい", "いいえ", "ええ", "うん", "ありがとう", "ありがとうございます", "よろしく", "お願いします", "すみません", "ごめん", "なるほど", "そうですね", "ですね", "ますね", "でしょうか", "ますか", "ですか", "かな", "かも", "かもしれない", "かもしれません", "ではないか", "だろうか", "のだろうか", "と思う", "と思います", "と考える", "気がする", "ことがある", "ことができる", "必要がある", "してみる", "してみた", "してほしい", "したい",
];

const EN: &[&str] = &[
    "the", "a", "an", "and", "or", "but", "so", "if", "then", "than", "because", "as", "of", "to", "in", "on", "at", "by", "for", "with", "from", "into", "onto", "over", "under", "about", "after", "before", "between", "through", "during", "without", "within", "up", "down", "out", "off", "not", "no", "yes",
    "i", "me", "my", "mine", "we", "us", "our", "you", "your", "he", "him", "his", "she", "her", "it", "its", "they", "them", "their", "this", "that", "these", "those", "there", "here", "what", "which", "who", "whom", "whose", "when", "where", "why", "how", "all", "any", "some", "each", "every", "both", "few", "many", "much", "more", "most", "other", "another", "such", "only", "own", "same", "very", "too", "also", "just", "even", "still", "yet", "already", "again", "always", "never", "often", "sometimes", "usually", "really", "actually", "basically", "literally", "probably", "maybe", "perhaps", "quite", "rather", "almost", "enough", "well", "now", "today", "then",
    "be", "am", "is", "are", "was", "were", "been", "being", "have", "has", "had", "having", "do", "does", "did", "doing", "done", "will", "would", "shall", "should", "can", "could", "may", "might", "must", "get", "got", "gets", "make", "made", "makes", "go", "goes", "went", "come", "came", "take", "took", "give", "gave", "see", "saw", "know", "knew", "think", "thought", "say", "said", "tell", "told", "want", "need", "use", "used", "try", "find", "found", "look", "seem", "feel", "felt", "like", "want",
    "good", "bad", "big", "small", "new", "old", "long", "short", "high", "low", "great", "little", "large", "important", "different", "same", "first", "last", "next", "early", "late", "right", "wrong", "true", "false", "thing", "things", "way", "ways", "time", "times", "people", "person", "lot", "lots", "kind", "sort", "case", "point", "fact", "example", "for example", "in fact", "of course", "in other words", "on the other hand", "at the end of the day", "i think", "i believe", "it seems", "you know",
];

/// True when `word` is a function word or stock phrase in either language,
/// or too short to carry a style on its own.
pub fn is_stop(word: &str) -> bool {
    let w = word.trim();
    if w.is_empty() { return true; }
    let lower = w.to_lowercase();
    if JA.contains(&w) || EN.contains(&lower.as_str()) { return true; }
    // Two characters or fewer of kana/latin alone are never a signature.
    let n = w.chars().count();
    let ascii = w.is_ascii();
    (ascii && n <= 3) || (!ascii && n <= 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn known_offenders_are_stopped() {
        for w in ["先に", "何を", "ので", "こと", "思います", "たとえば", "the", "I think"] { assert!(is_stop(w), "{w}"); }
    }
    #[test]
    fn signature_words_pass() {
        for w in ["エクイティ", "ファイナンス", "構造", "帰着する", "資本政策", "handwavy", "ぜひ", "一体", "狂う説"] { assert!(!is_stop(w), "{w}"); }
    }
}
