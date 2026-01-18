//! Change detection using xxhash
//!
//! Provides fast hashing for screen region comparison.

use xxhash_rust::xxh64::xxh64;

/// Hasher configuration
#[derive(Debug, Clone)]
pub struct HasherConfig {
    /// Hash seed
    pub seed: u64,

    /// Number of hash buckets
    pub bucket_count: usize,
}

impl Default for HasherConfig {
    fn default() -> Self {
        Self {
            seed: 0x123456789ABCDEF0,
            bucket_count: 64,
        }
    }
}

/// xxhash-based hasher for change detection
pub struct Hasher {
    config: HasherConfig,
}

impl Hasher {
    /// Create a new hasher
    pub fn new(config: HasherConfig) -> Self {
        Self { config }
    }

    /// Hash a slice of data
    pub fn hash(&self, data: &[u8]) -> u64 {
        xxh64(data, self.config.seed)
    }

    /// Hash data at specific offset and length
    pub fn hash_region(&self, data: &[u8], offset: usize, len: usize) -> u64 {
        if offset + len <= data.len() {
            xxh64(&data[offset..offset + len], self.config.seed)
        } else {
            0
        }
    }

    /// Compare two regions and return true if different
    pub fn has_changed(&self, data1: &[u8], data2: &[u8]) -> bool {
        if data1.len() != data2.len() {
            return true;
        }
        self.hash(data1) != self.hash(data2)
    }

    /// Hash frame into buckets for region-based change detection
    pub fn hash_frame_buckets(&self, data: &[u8], width: u32, height: u32, bucket_height: u32) -> Vec<u64> {
        let row_size = width * 3;
        let mut hashes = Vec::new();

        let mut y = 0u32;
        while y < height {
            let h = bucket_height.min(height - y);
            let offset = (y * row_size) as usize;
            let size = (row_size * h) as usize;

            if offset + size <= data.len() {
                hashes.push(self.hash_region(data, offset, size));
            } else {
                hashes.push(0);
            }

            y += bucket_height;
        }

        hashes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash() {
        let hasher = Hasher::new(HasherConfig::default());
        let data1 = b"hello world";
        let data2 = b"hello world";
        let data3 = b"hello world!";

        assert_eq!(hasher.hash(data1), hasher.hash(data2));
        assert_ne!(hasher.hash(data1), hasher.hash(data3));
    }

    #[test]
    fn test_has_changed() {
        let hasher = Hasher::new(HasherConfig::default());

        assert!(!hasher.has_changed(b"same", b"same"));
        assert!(hasher.has_changed(b"same", b"diff"));
    }
}
