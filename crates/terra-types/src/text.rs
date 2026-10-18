//! UTF-8 safe text helpers.
//!
//! Byte-index slicing (`&s[..57]`) panics when the index falls inside a
//! multi-byte character (CJK, emoji, accented letters). These helpers count
//! characters instead, so user-facing previews never crash on non-ASCII input.

/// The first `max_chars` characters of `s`, borrowed.
pub fn prefix_chars(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

/// Shorten `s` to at most `max_chars` characters, ending with `...` when cut.
pub fn truncate_ellipsis(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let keep = max_chars.saturating_sub(3);
    format!("{}...", prefix_chars(s, keep))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_chars_ascii_and_short() {
        assert_eq!(prefix_chars("hello", 3), "hel");
        assert_eq!(prefix_chars("hi", 10), "hi");
        assert_eq!(prefix_chars("", 5), "");
    }

    #[test]
    fn prefix_chars_never_splits_multibyte() {
        assert_eq!(prefix_chars("这个项目使用Rust", 4), "这个项目");
        assert_eq!(prefix_chars("café au lait", 4), "café");
        assert_eq!(prefix_chars("🦀🦀🦀", 2), "🦀🦀");
    }

    #[test]
    fn truncate_ellipsis_keeps_short_strings() {
        assert_eq!(truncate_ellipsis("short", 60), "short");
        let exactly = "a".repeat(60);
        assert_eq!(truncate_ellipsis(&exactly, 60), exactly);
    }

    #[test]
    fn truncate_ellipsis_cuts_long_cjk() {
        let long = "这个项目使用Rust编写，内存系统有七层，每一层都有不同的写权限和垃圾回收策略".repeat(2);
        let out = truncate_ellipsis(&long, 20);
        assert_eq!(out.chars().count(), 20);
        assert!(out.ends_with("..."));
    }
}
