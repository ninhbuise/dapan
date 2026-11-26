//! Splits long text into sentence-sized chunks so speech can start after the first
//! chunk is ready and no single request gets too long.

/// Target maximum characters per chunk.
pub const CHUNK_CHARS: usize = 200;

pub fn split(text: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    for sentence in sentences(text) {
        if sentence.chars().count() <= CHUNK_CHARS {
            push_merged(&mut chunks, sentence);
        } else {
            // Too long: break at commas/semicolons, then at spaces.
            for part in split_long(&sentence) {
                push_merged(&mut chunks, part);
            }
        }
    }
    chunks
}

/// Joins very short sentences ("Ừ. Được.") with the previous chunk.
fn push_merged(chunks: &mut Vec<String>, s: String) {
    if let Some(last) = chunks.last_mut()
        && last.chars().count() + 1 + s.chars().count() <= CHUNK_CHARS
        && (last.chars().count() < 40 || s.chars().count() < 40)
    {
        last.push(' ');
        last.push_str(&s);
        return;
    }
    chunks.push(s);
}

fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\n' {
            flush(&mut out, &mut cur);
            continue;
        }
        cur.push(c);
        if matches!(c, '.' | '!' | '?' | '…') {
            // keep "..." / "?!" together; split only before whitespace
            if chars.peek().is_none_or(|n| n.is_whitespace()) {
                flush(&mut out, &mut cur);
            }
        }
    }
    flush(&mut out, &mut cur);
    out
}

fn flush(out: &mut Vec<String>, cur: &mut String) {
    let t = cur.trim();
    if !t.is_empty() {
        out.push(t.to_string());
    }
    cur.clear();
}

fn split_long(sentence: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for word in sentence.split_whitespace() {
        let would_be = cur.chars().count() + 1 + word.chars().count();
        if !cur.is_empty() && would_be > CHUNK_CHARS {
            out.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
        // Prefer to break right after a comma once the chunk is reasonably long.
        if cur.chars().count() > CHUNK_CHARS / 2 && (word.ends_with(',') || word.ends_with(';')) {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_is_one_chunk() {
        assert_eq!(split("Xin chào!"), vec!["Xin chào!"]);
    }

    #[test]
    fn splits_sentences_and_merges_tiny_ones() {
        let text = "Ừ. Được rồi. Hôm nay trời Hà Nội khá đẹp, nắng nhẹ và gió mát, rất thích hợp để đi dạo quanh hồ Gươm cùng bạn bè vào buổi chiều tối nha. Còn ngày mai thì sao? Mai chắc là mưa to lắm đó, nhớ mang ô theo nha bạn ơi, kẻo ướt hết quần áo rồi lại cảm lạnh thì khổ lắm.";
        let c = split(text);
        // Tiny sentences are merged into the next one instead of being spoken alone.
        assert!(c[0].starts_with("Ừ. Được rồi. Hôm nay"), "{c:?}");
        assert!(c.len() >= 2, "{c:?}");
        assert!(c.iter().all(|s| s.chars().count() <= CHUNK_CHARS));
        assert_eq!(c.join(" "), text);
    }

    #[test]
    fn very_long_sentence_is_broken_up_without_losing_words() {
        let long = "một hai ba bốn năm, ".repeat(40);
        let c = split(&long);
        assert!(c.len() > 2);
        assert!(c.iter().all(|s| s.chars().count() <= CHUNK_CHARS));
        assert_eq!(
            c.join(" ").split_whitespace().count(),
            long.split_whitespace().count()
        );
    }

    #[test]
    fn keeps_ellipsis_and_numbers_together() {
        assert_eq!(
            split("Giá là 3.5 triệu... thật á?!"),
            vec!["Giá là 3.5 triệu... thật á?!"]
        );
    }
}
