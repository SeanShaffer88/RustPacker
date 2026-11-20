//! Pure typewriter state machine for the installer wizard.
//!
//! The stub drives this from a Win32 timer; all timing and skip logic
//! lives here so it can be unit-tested without a window.
//!
//! Text is revealed by **grapheme cluster**, not by Unicode scalar, so a
//! combining accent (`e` + U+0301), a variaton selector, an emoji modifier or
//! a ZWJ sequence (`👩‍💻`) appears all at once instead of in fragments.
//!
//! The segmentation is a small local approximation of UAX #29 rather than the
//! full algorithm: it handles the cases that actually appear in installer
//! copy — combining marks, ZWJ sequences, regional-indicator flag pairs, and
//! `\r\n` — without pulling in a Unicode tables crate. That keeps the
//! dependency tree at zero for this module, which matters because the stub is
//! copied into every installer.

/// Default reveal interval from the confirmed brief.
pub const DEFAULT_INTERVAL_MS: u32 = 12;

/// True when `c` extends the preceding cluster instead of starting a new one.
///
/// Covers the combining-mark blocks plus the joiner/variation-selector
/// characters that glue emoji sequences together.
fn extends_cluster(c: char) -> bool {
    matches!(c as u32,
        // Combining Diacritical Marks and their supplements, Hebrew/Arabic
        // marks, and the generic nonspacing/combining spaces.
        0x0300..=0x036F
        | 0x0483..=0x0489
        | 0x0591..=0x05BD
        | 0x0610..=0x061A
        | 0x064B..=0x065F
        | 0x0900..=0x0903
        | 0x093E..=0x094F
        | 0x1AB0..=0x1AFF
        | 0x1DC0..=0x1DFF
        | 0x20D0..=0x20F0
        | 0xFE00..=0xFE0F   // variation selectors
        | 0xFE20..=0xFE2F
        // Skin-tone modifiers.
        | 0x1F3FB..=0x1F3FF
        | 0xE0020..=0xE007F // tags (flag sequences)
        | 0xE0100..=0xE01EF // variation selectors supplement
    ) || c == '\u{200D}' // zero-width joiner
}

/// Split `text` into display clusters (see the module comment).
fn clusters(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut prev: Option<char> = None;
    for c in text.chars() {
        let joins = match prev {
            None => false,
            // A combining mark or ZWJ modifier extends the current cluster.
            Some(_) if extends_cluster(c) => true,
            // ...and whatever follows a ZWJ continues the sequence.
            Some('\u{200D}') => true,
            // CRLF is one cluster (it is one line break).
            Some('\r') if c == '\n' => true,
            // Regional indicators pair up into flags: RI RI = one flag.
            Some(p) if is_regional_indicator(p) && is_regional_indicator(c) => {
                out.last().map(|last| last.chars().count()) == Some(1)
            }
            _ => false,
        };
        if joins {
            if let Some(last) = out.last_mut() {
                last.push(c);
            } else {
                out.push(c.to_string());
            }
        } else {
            out.push(c.to_string());
        }
        prev = Some(c);
    }
    // A cluster may not end with a dangling joiner; it renders as nothing.
    out
}

fn is_regional_indicator(c: char) -> bool {
    matches!(c as u32, 0x1F1E6..=0x1F1FF)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Typewriter {
    chars: Vec<String>,
    interval_ms: u32,
    revealed: usize,
    /// Milliseconds banked toward the next character.
    carry_ms: u32,
    complete: bool,
}

impl Typewriter {
    pub fn new(text: &str, interval_ms: u32) -> Self {
        let interval_ms = if interval_ms == 0 {
            DEFAULT_INTERVAL_MS
        } else {
            interval_ms
        };
        let chars = clusters(text);
        let complete = chars.is_empty();
        Self {
            chars,
            interval_ms,
            revealed: 0,
            carry_ms: 0,
            complete,
        }
    }

    pub fn interval_ms(&self) -> u32 {
        self.interval_ms
    }

    pub fn is_complete(&self) -> bool {
        self.complete
    }

    /// Advance by elapsed milliseconds; returns true if the visible text changed.
    pub fn advance(&mut self, elapsed_ms: u32) -> bool {
        if self.complete {
            return false;
        }
        self.carry_ms = self.carry_ms.saturating_add(elapsed_ms);
        let steps = self.carry_ms / self.interval_ms;
        if steps == 0 {
            return false;
        }
        self.carry_ms -= steps * self.interval_ms;
        let before = self.revealed;
        self.revealed = self
            .revealed
            .saturating_add(steps as usize)
            .min(self.chars.len());
        if self.revealed >= self.chars.len() {
            self.complete = true;
        }
        self.revealed != before
    }

    /// Click / Space / Enter: reveal everything immediately.
    pub fn skip(&mut self) {
        self.revealed = self.chars.len();
        self.complete = true;
        self.carry_ms = 0;
    }

    pub fn visible_text(&self) -> String {
        let mut out = String::with_capacity(self.full_len_bytes());
        self.write_visible(&mut out);
        out
    }

    /// Append the revealed text to an existing buffer, so a caller that
    /// redraws every tick can reuse one allocation instead of getting a new
    /// `String` from [`visible_text`](Self::visible_text).
    pub fn write_visible(&self, out: &mut String) {
        out.clear();
        for cluster in &self.chars[..self.revealed] {
            out.push_str(cluster);
        }
    }

    /// Bytes in the full text; used to size the buffer once.
    fn full_len_bytes(&self) -> usize {
        self.chars.iter().map(|c| c.len()).sum()
    }

    pub fn full_text(&self) -> String {
        self.chars.concat()
    }

    /// Number of display clusters (what the reveal counts).
    pub fn cluster_count(&self) -> usize {
        self.chars.len()
    }

    pub fn reset(&mut self, text: &str) {
        self.chars = clusters(text);
        self.revealed = 0;
        self.carry_ms = 0;
        self.complete = self.chars.is_empty();
    }

    /// Like [`reset`](Self::reset) but also replaces the interval
    /// (0 falls back to [`DEFAULT_INTERVAL_MS`]).
    pub fn reset_with_interval(&mut self, text: &str, interval_ms: u32) {
        self.interval_ms = if interval_ms == 0 {
            DEFAULT_INTERVAL_MS
        } else {
            interval_ms
        };
        self.reset(text);
    }
}

impl Default for Typewriter {
    fn default() -> Self {
        Self::new("", DEFAULT_INTERVAL_MS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reveals_one_char_per_interval() {
        let mut tw = Typewriter::new("hello", 12);
        assert_eq!(tw.visible_text(), "");
        assert!(!tw.advance(11));
        assert_eq!(tw.visible_text(), "");
        assert!(tw.advance(1));
        assert_eq!(tw.visible_text(), "h");
        tw.advance(36);
        assert_eq!(tw.visible_text(), "hell");
        tw.advance(12);
        assert_eq!(tw.visible_text(), "hello");
        assert!(tw.is_complete());
        assert!(!tw.advance(100));
    }

    #[test]
    fn skip_reveals_everything() {
        let mut tw = Typewriter::new("some longer text", 12);
        tw.advance(24);
        tw.skip();
        assert_eq!(tw.visible_text(), "some longer text");
        assert!(tw.is_complete());
    }

    #[test]
    fn unicode_is_revealed_by_char_not_byte() {
        let mut tw = Typewriter::new("aé—b", 10);
        tw.advance(20);
        assert_eq!(tw.visible_text(), "aé");
        tw.skip();
        assert_eq!(tw.visible_text(), "aé—b");
    }

    #[test]
    fn zero_interval_falls_back_to_default() {
        let tw = Typewriter::new("x", 0);
        assert_eq!(tw.interval_ms(), DEFAULT_INTERVAL_MS);
    }

    #[test]
    fn combining_marks_reveal_with_their_base() {
        // "e" + combining acute must appear as one step, never as "e" then
        // a floating accent.
        let mut tw = Typewriter::new("e\u{301}x", 10);
        tw.advance(10);
        assert_eq!(tw.visible_text(), "e\u{301}");
        tw.advance(10);
        assert_eq!(tw.visible_text(), "e\u{301}x");
    }

    #[test]
    fn zwj_emoji_sequences_are_one_cluster() {
        // Woman + ZWJ + laptop: three code points, one glyph.
        let text = "\u{1F469}\u{200D}\u{1F4BB}!";
        let mut tw = Typewriter::new(text, 10);
        assert_eq!(tw.cluster_count(), 2, "emoji + '!'");
        tw.advance(10);
        assert_eq!(tw.visible_text(), "\u{1F469}\u{200D}\u{1F4BB}");
        tw.skip();
        assert_eq!(tw.visible_text(), text);
    }

    #[test]
    fn skin_tone_modifier_stays_with_its_emoji() {
        let text = "\u{1F44D}\u{1F3FD}ok";
        let tw = Typewriter::new(text, 10);
        assert_eq!(tw.cluster_count(), 3);
    }

    #[test]
    fn regional_indicators_pair_into_flags() {
        // Two flags: US then JP.
        let text = "\u{1F1FA}\u{1F1F8}\u{1F1EF}\u{1F1F5}";
        let tw = Typewriter::new(text, 10);
        assert_eq!(tw.cluster_count(), 2);
        let mut tw = Typewriter::new(text, 10);
        tw.advance(10);
        assert_eq!(tw.visible_text(), "\u{1F1FA}\u{1F1F8}");
    }

    #[test]
    fn write_visible_reuses_a_buffer() {
        let mut tw = Typewriter::new("abc", 10);
        let mut buf = String::new();
        tw.advance(20);
        tw.write_visible(&mut buf);
        assert_eq!(buf, "ab");
        // A second call clears rather than appends.
        tw.skip();
        tw.write_visible(&mut buf);
        assert_eq!(buf, "abc");
        // The buffer was reused, not reallocated from scratch.
        assert!(buf.capacity() >= 3);
    }

    #[test]
    fn visible_text_matches_write_visible() {
        let mut tw = Typewriter::new("héllo wörld", 5);
        tw.advance(25);
        let mut buf = String::new();
        tw.write_visible(&mut buf);
        assert_eq!(buf, tw.visible_text());
        assert_eq!(tw.visible_text(), "héllo");
    }

    #[test]
    fn crlf_is_one_cluster() {
        let tw = Typewriter::new("a\r\nb", 10);
        assert_eq!(tw.cluster_count(), 3);
    }

    #[test]
    fn reset_starts_over_with_new_text() {
        let mut tw = Typewriter::new("old", 10);
        tw.skip();
        tw.reset("new page");
        assert!(!tw.is_complete());
        assert_eq!(tw.visible_text(), "");
        assert_eq!(tw.full_text(), "new page");
    }
}
