use enumset::EnumSet;
use rkyv::{Archive, Deserialize, Serialize};
use ux_api::minigfx::*;

use super::attach::Attach;
use crate::PostFlag;

#[derive(Archive, Serialize, Deserialize, Debug)]
pub struct Post {
    author_id: u16,
    timestamp: u64,
    text: String,
    attach: Option<Attach>,
    pub flags: u16,
    pub bounding_box: Option<Rectangle>,
    /// Chrome drawn above the bubble in `GlyphStyle::Bold`, outside
    /// the bubble border. Never derived from `text`: the app sets it
    /// from its own authenticated identity data, so body text cannot
    /// forge an author line (ignore/notes-r5-author-identity.md
    /// section 4).
    header: Option<String>,
    /// Cached measured extent of the header TextView, cleared
    /// alongside `bounding_box` when the post style changes.
    pub header_box: Option<Rectangle>,
    /// Chrome drawn below the bubble in `GlyphStyle::Regular`, outside
    /// the bubble border, aligned to the same side as the body. Never
    /// derived from `text` (ISSUES r6 row 20).
    footer: Option<String>,
    /// Cached measured extent of the footer TextView, cleared
    /// alongside `bounding_box` and `header_box` when the post style
    /// changes.
    pub footer_box: Option<Rectangle>,
    /// True for a centered, borderless, box-free system row (a date
    /// separator or the message-request instruction bar; ISSUES r6
    /// rows 21/22) rather than an ordinary left/right bubble. A
    /// centered post carries no header or footer.
    center: bool,
}

#[allow(dead_code)]
impl Post {
    pub fn new(
        author_id: u16,
        timestamp: u64,
        text: &str,
        header: Option<&str>,
        footer: Option<&str>,
        center: bool,
        attach: Option<Attach>,
    ) -> Self {
        Self {
            author_id,
            timestamp,
            text: text.to_string(),
            attach,
            flags: 0,
            bounding_box: None,
            header: header.map(|h| h.to_string()),
            header_box: None,
            footer: footer.map(|f| f.to_string()),
            footer_box: None,
            center,
        }
    }

    pub fn author_id(&self) -> u16 { self.author_id }

    pub fn flag_is(&self, flag: PostFlag) -> bool { self.flags_get().contains(flag) }

    pub fn flags_get(&self) -> EnumSet<PostFlag> { EnumSet::<PostFlag>::from_u16(self.flags) }

    pub fn flags_set(&mut self, flags: EnumSet<PostFlag>) { self.flags = flags.as_u16(); }

    pub fn text(&self) -> &str { self.text.as_str() }

    pub fn header(&self) -> Option<&str> { self.header.as_deref() }

    pub fn footer(&self) -> Option<&str> { self.footer.as_deref() }

    pub fn center(&self) -> bool { self.center }

    pub fn timestamp(&self) -> u64 { self.timestamp }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn post_new_keeps_header_and_text_separate() {
        // A Post built with a header does not carry it in text(), and
        // a body that looks like a header stays in text() only: the
        // two fields never mix, which is what the anti-spoof rendering
        // relies on.
        let p = Post::new(1, 42, "body text", Some("Alice (14:32)"), None, false, None);
        assert_eq!(p.text(), "body text");
        assert_eq!(p.header(), Some("Alice (14:32)"));
        assert!(!p.text().contains("Alice"));

        let no_header = Post::new(1, 43, "Alice (14:32)\nyou owe me money", None, None, false, None);
        assert_eq!(no_header.header(), None);
        assert!(no_header.text().starts_with("Alice (14:32)"));
    }

    #[test]
    fn post_new_keeps_footer_and_text_separate() {
        // Mirrors the header test (ISSUES r6 row 20): a footer never
        // leaks into text(), and a body that imitates one stays put.
        let p = Post::new(1, 42, "body text", None, Some("14:32 \u{2714}"), false, None);
        assert_eq!(p.text(), "body text");
        assert_eq!(p.footer(), Some("14:32 \u{2714}"));
        assert!(!p.text().contains('\u{2714}'));
    }

    #[test]
    fn post_new_carries_the_center_flag() {
        let centered = Post::new(1, 42, "2026-08-15", None, None, true, None);
        assert!(centered.center());
        let ordinary = Post::new(1, 42, "hello", None, None, false, None);
        assert!(!ordinary.center());
    }
}
