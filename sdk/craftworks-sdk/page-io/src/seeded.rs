//! The machines' model tests' seeded generator, one home for every model (a tiny xorshift, no dependency).

/// A tiny seeded generator (no dependency).
pub(crate) struct Rng(pub(crate) u64);

impl Rng {
    pub(crate) fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub(crate) fn pick(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}
