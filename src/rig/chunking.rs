/// Recursively splits text into chunks of at most `max_chars`, preferring
/// semantic boundaries (paragraphs → lines → sentences → words).
/// Adjacent chunks share `overlap` characters of trailing context.
pub fn chunk_input(text: &str, max_chars: usize, overlap: usize) -> Vec<String> {
    const SEPARATORS: &[&str] = &["\n\n\n", "\n\n", "\n", ". ", ", ", " "];

            fn split_recursive(text: &str, max_chars: usize, sep_idx: usize) -> Vec<String> {
                if text.len() <= max_chars || sep_idx >= SEPARATORS.len() {
                    if text.len() <= max_chars {
                        return if text.trim().is_empty() {
                            vec![]
                        } else {
                            vec![text.to_string()]
                        };
                    }
                    let chars: Vec<char> = text.chars().collect();
                    let mut chunks = Vec::new();
                    let mut start = 0;
                    while start < chars.len() {
                        let end = (start + max_chars).min(chars.len());
                        if end == chars.len() {
                            let s: String = chars[start..end].iter().collect();
                            if !s.trim().is_empty() { chunks.push(s); }
                            break;
                        }
                        let break_at = chars[start..end].iter().rposition(|c| c.is_whitespace())
                            .map(|pos| start + pos + 1)
                            .unwrap_or(end);
                        let s: String = chars[start..break_at].iter().collect();
                        if !s.trim().is_empty() { chunks.push(s); }
                        start = break_at;
                    }
                    return chunks;
                }

                let sep = SEPARATORS[sep_idx];
                let parts: Vec<&str> = text.split(sep).collect();

                if parts.len() == 1 {
                    return split_recursive(text, max_chars, sep_idx + 1);
                }

                let mut chunks = Vec::new();
                let mut current = String::new();

                for part in parts {
                    let candidate = if current.is_empty() {
                        part.to_string()
                    } else {
                        format!("{}{}{}", current, sep, part)
                    };
                    if candidate.len() <= max_chars {
                        current = candidate;
                    } else {
                        if !current.trim().is_empty() {
                            chunks.push(current);
                        }
                        if part.len() > max_chars {
                            chunks.extend(split_recursive(part, max_chars, sep_idx + 1));
                            current = String::new();
                        } else {
                            current = part.to_string();
                        }
                    }
                }
                if !current.trim().is_empty() {
                    chunks.push(current);
                }
                chunks
            }

    let raw = split_recursive(text, max_chars, 0);

    if raw.len() <= 1 || overlap == 0 {
        return raw;
    }

    let mut result = Vec::with_capacity(raw.len());
    result.push(raw[0].clone());

    for i in 1..raw.len() {
        let prev_chars: Vec<char> = raw[i - 1].chars().collect();
        let take = overlap.min(prev_chars.len());
        let mut start = prev_chars.len() - take;

        while start < prev_chars.len() && !prev_chars[start].is_whitespace() {
            start += 1;
        }

        let overlap_text: String = prev_chars[start..].iter().collect();
        if overlap_text.trim().is_empty() {
            result.push(raw[i].clone());
        } else {
            result.push(format!("{}{}", overlap_text, raw[i]));
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::chunk_input;

    const EXAMPLE: &str = include_str!("../../input/example.txt");

    #[test]
    fn no_empty_chunks() {
        let chunks = chunk_input(EXAMPLE, 400, 80);
        for (i, chunk) in chunks.iter().enumerate() {
            assert!(!chunk.trim().is_empty(), "chunk {i} is empty");
        }
    }

    #[test]
    fn raw_chunks_respect_max_chars() {
        // Without overlap, every chunk must fit within max_chars
        let chunks = chunk_input(EXAMPLE, 400, 0);
        for (i, chunk) in chunks.iter().enumerate() {
            assert!(
                chunk.len() <= 400,
                "chunk {i} is {} chars (max 400): {:?}",
                chunk.len(),
                &chunk[..60.min(chunk.len())]
            );
        }
    }

    #[test]
    fn no_mid_word_starts() {
        let chunks = chunk_input(EXAMPLE, 400, 80);
        for (i, chunk) in chunks.iter().enumerate().skip(1) {
            let first_char = chunk.chars().next().unwrap();
            // Each chunk should start with whitespace or at a word boundary.
            // If it starts with a non-whitespace char, the *previous* chunk
            // must have ended with whitespace or a separator.
            let prev_last = chunks[i - 1].chars().last().unwrap();
            if !first_char.is_whitespace() {
                assert!(
                    prev_last.is_whitespace() || prev_last == '.' || prev_last == ',',
                    "chunk {i} starts mid-word: prev ends with {:?}, chunk starts with {:?}",
                    prev_last,
                    &chunk[..20.min(chunk.len())]
                );
            }
        }
    }

    #[test]
    fn overlap_shares_context() {
        let raw = chunk_input(EXAMPLE, 400, 0);
        let overlapped = chunk_input(EXAMPLE, 400, 80);

        if raw.len() <= 1 {
            return;
        }

        // Each overlapped chunk (after the first) should contain some text
        // from the tail of the previous raw chunk
        for i in 1..raw.len().min(overlapped.len()) {
            let prev_tail: String = raw[i - 1].chars().rev().take(80).collect::<Vec<_>>().into_iter().rev().collect();
            // Find a word from the tail that should appear at the start of the overlapped chunk
            if let Some(word) = prev_tail.split_whitespace().last() {
                assert!(
                    overlapped[i].contains(word),
                    "chunk {i} missing overlap word {:?} from previous chunk tail",
                    word
                );
            }
        }
    }

    #[test]
    fn all_content_preserved() {
        // Every word in the original should appear in at least one chunk
        let chunks = chunk_input(EXAMPLE, 400, 80);
        let combined: String = chunks.join(" ");
        for word in EXAMPLE.split_whitespace() {
            assert!(
                combined.contains(word),
                "word {:?} missing from chunks",
                word
            );
        }
    }

    #[test]
    fn small_input_returns_single_chunk() {
        let chunks = chunk_input("Hello world", 400, 80);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "Hello world");
    }

    #[test]
    fn empty_input_returns_nothing() {
        let chunks = chunk_input("", 400, 80);
        assert!(chunks.is_empty());
    }

    #[test]
    fn whitespace_only_returns_nothing() {
        let chunks = chunk_input("   \n\n  \n  ", 400, 80);
        assert!(chunks.is_empty());
    }
}