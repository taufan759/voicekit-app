//! Script handling: splitting long scripts into request-sized pieces and joining the audio.

/// Splits a script into pieces of at most `limit` characters, preferring paragraph, then
/// sentence, then word boundaries. Audio tags such as `[whispers]` are never cut.
pub fn chunk_text(text: &str, limit: usize) -> Vec<String> {
    let limit = limit.max(1);
    let mut chunks = Vec::new();
    let mut current = String::new();
    for sentence in sentences(text) {
        let candidate_len = current.chars().count() + sentence.chars().count();
        if candidate_len <= limit {
            current.push_str(&sentence);
            continue;
        }
        push_trimmed(&mut chunks, std::mem::take(&mut current));
        if sentence.chars().count() <= limit {
            current = sentence;
        } else {
            for piece in split_words(&sentence, limit) {
                push_trimmed(&mut chunks, piece);
            }
        }
    }
    push_trimmed(&mut chunks, current);
    chunks
}

fn push_trimmed(chunks: &mut Vec<String>, piece: String) {
    let piece = piece.trim();
    if !piece.is_empty() {
        chunks.push(piece.to_string());
    }
}

/// Sentences with their trailing whitespace; a break needs end punctuation or a newline
/// outside of square brackets.
fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        current.push(c);
        match c {
            '[' => depth += 1,
            ']' => depth = depth.saturating_sub(1),
            _ => {}
        }
        let ends = depth == 0
            && (c == '\n' || (matches!(c, '.' | '!' | '?' | '…') && chars.peek().is_none_or(|n| n.is_whitespace())));
        if ends {
            while let Some(&n) = chars.peek() {
                if !n.is_whitespace() {
                    break;
                }
                current.push(n);
                chars.next();
            }
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Breaks an overlong sentence between words, keeping each `[tag]` whole.
fn split_words(sentence: &str, limit: usize) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut current = String::new();
    for word in words(sentence) {
        if current.chars().count() + word.chars().count() > limit && !current.is_empty() {
            pieces.push(std::mem::take(&mut current));
        }
        if word.chars().count() > limit {
            let chars: Vec<char> = word.chars().collect();
            pieces.extend(chars.chunks(limit).map(|c| c.iter().collect::<String>()));
        } else {
            current.push_str(&word);
        }
    }
    if !current.is_empty() {
        pieces.push(current);
    }
    pieces
}

/// Words with their trailing spaces; spaces inside brackets don't split.
fn words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    for c in text.chars() {
        match c {
            '[' => depth += 1,
            ']' => depth = depth.saturating_sub(1),
            _ => {}
        }
        if c.is_whitespace() && depth == 0 {
            current.push(c);
            continue;
        }
        if current.ends_with(char::is_whitespace) {
            out.push(std::mem::take(&mut current));
        }
        current.push(c);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Joins MP3 files made with the same format. Later parts lose their ID3v2 header so
/// players don't stop at the second tag.
pub fn concat_mp3(parts: Vec<Vec<u8>>) -> Vec<u8> {
    let mut out = Vec::with_capacity(parts.iter().map(Vec::len).sum());
    for (i, part) in parts.into_iter().enumerate() {
        let skip = if i == 0 { 0 } else { id3v2_len(&part) };
        out.extend_from_slice(&part[skip.min(part.len())..]);
    }
    out
}

fn id3v2_len(data: &[u8]) -> usize {
    if data.len() < 10 || &data[..3] != b"ID3" {
        return 0;
    }
    let size = data[6..10].iter().fold(0usize, |acc, b| (acc << 7) | usize::from(b & 0x7f));
    let footer = if data[5] & 0x10 != 0 { 10 } else { 0 };
    10 + size + footer
}

/// A short, file-system-safe name from the first words of the script.
pub fn slug(text: &str, max_len: usize) -> String {
    let mut out = String::new();
    let mut last_dash = true;
    let cleaned = strip_tags(text);
    for c in cleaned.chars() {
        if out.chars().count() >= max_len {
            break;
        }
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() { "audio".into() } else { out }
}

fn strip_tags(text: &str) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    for c in text.chars() {
        match c {
            '[' => depth += 1,
            ']' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_short_scripts_whole() {
        assert_eq!(chunk_text("  Halo semua. Apa kabar?  ", 100), vec!["Halo semua. Apa kabar?"]);
        assert!(chunk_text("   ", 100).is_empty());
    }

    #[test]
    fn splits_at_sentences_within_the_limit() {
        let chunks = chunk_text("Satu dua tiga. Empat lima enam. Tujuh delapan.", 32);
        assert_eq!(chunks, vec!["Satu dua tiga. Empat lima enam.", "Tujuh delapan."]);
        assert!(chunks.iter().all(|c| c.chars().count() <= 32));
    }

    #[test]
    fn never_cuts_audio_tags() {
        let text = "[whispers. softly] Ini rahasia. [laughs] Ketahuan deh.";
        let chunks = chunk_text(text, 35);
        assert_eq!(chunks[0], "[whispers. softly] Ini rahasia.");
        let long = "[said slowly like a storyteller] kata kata kata kata kata kata";
        for chunk in chunk_text(long, 40) {
            assert_eq!(chunk.matches('[').count(), chunk.matches(']').count());
        }
    }

    #[test]
    fn splits_overlong_sentences_by_words() {
        let chunks = chunk_text("aaa bbb ccc ddd eee", 8);
        assert_eq!(chunks, vec!["aaa bbb", "ccc ddd", "eee"]);
        assert_eq!(chunk_text("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
    }

    #[test]
    fn joins_mp3_without_repeated_id3() {
        let tagged = |body: &[u8]| [b"ID3\x04\x00\x00\x00\x00\x00\x02xx".as_slice(), body].concat();
        let joined = concat_mp3(vec![tagged(b"AA"), tagged(b"BB"), b"CC".to_vec()]);
        assert_eq!(joined, [tagged(b"AA").as_slice(), b"BBCC"].concat());
    }

    #[test]
    fn makes_safe_slugs() {
        assert_eq!(slug("[excited] Halo, Dunia! Ini VoiceKit.", 20), "halo-dunia-ini-voice");
        assert_eq!(slug("[laughs]", 20), "audio");
    }
}
