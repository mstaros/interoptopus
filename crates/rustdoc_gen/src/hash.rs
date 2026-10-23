//! FNV-1a (64 bit): a stable, dependency-free hash for cache keys and signature hashes, not for
//! security.

/// An FNV-1a hasher.
pub struct Fnv(u64);

impl Fnv {
    /// A hasher in its initial state.
    #[must_use]
    pub const fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    /// Hashes a length-prefixed field so that adjacent fields cannot run together.
    pub fn field(&mut self, data: &[u8]) {
        let length = u64::try_from(data.len()).unwrap_or(u64::MAX);
        for byte in length.to_le_bytes().iter().chain(data) {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    /// The hash as 16 lowercase hexadecimal digits.
    #[must_use]
    pub fn hex(&self) -> String {
        format!("{:016x}", self.0)
    }
}

impl Default for Fnv {
    fn default() -> Self {
        Self::new()
    }
}