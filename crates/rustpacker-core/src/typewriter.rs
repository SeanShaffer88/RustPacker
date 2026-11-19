//! Pure typewriter state machine for the installer wizard.
//!
//! The stub drives this from a Win32 timer; all timing and skip logic
//! lives here so it can be unit-tested without a window.
//!
//! Text is revealed by Unicode scalar (`char`), not by grapheme
//! cluster: combining sequences and ZWJ emoji may briefly render
//! split. This is a deliberate dependency-free trade-off; wizard
//! copy avoids such sequences.

/// Default reveal interval from the confirmed brief.
pub const DEFAULT_INTERVAL_MS: u32 = 12;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Typewriter {
    chars: Vec<char>,
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
        let chars: Vec<char> = text.chars().collect();
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
        self.chars[..self.revealed].iter().collect()
    }

    pub fn full_text(&self) -> String {
        self.chars.iter().collect()
    }

    pub fn reset(&mut self, text: &str) {
        self.chars = text.chars().collect();
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
    fn reset_starts_over_with_new_text() {
        let mut tw = Typewriter::new("old", 10);
        tw.skip();
        tw.reset("new page");
        assert!(!tw.is_complete());
        assert_eq!(tw.visible_text(), "");
        assert_eq!(tw.full_text(), "new page");
    }
}
