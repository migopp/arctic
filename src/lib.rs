mod node;

pub(crate) use node::Node;

#[derive(Default)]
pub struct Art {
    // root: Ptr,
}

impl Art {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, mut key: &[u8], value: u64) -> Option<u64> {
        todo!()
    }

    pub fn get(&self, mut key: &[u8]) -> Option<u64> {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use crate::Art;

    #[test]
    fn smoke() {
        let art = Art::default();
        art.insert(b"abcd", 1);
        assert_eq!(art.get(b"abcd"), Some(1));
    }

    #[test]
    fn node4_overwrite() {
        let art = Art::default();

        for value in [1, 2, 3, 4] {
            art.insert(&[1], value as u64);
            assert_eq!(art.get(&[1]), Some(value as u64));
        }
    }

    #[test]
    fn node4_full() {
        let art = Art::default();

        const KEYS: [u8; 4] = [1, 2, 3, 4];

        for key in KEYS {
            art.insert(&[key], key as u64);
            assert_eq!(art.get(&[key]), Some(key as u64));
        }

        for key in KEYS {
            assert_eq!(art.get(&[key]), Some(key as u64));
        }
    }

    #[test]
    fn node4_expand() {
        let art = Art::default();

        const KEYS: [u8; 5] = [1, 2, 3, 4, 5];

        for key in KEYS {
            art.insert(&[key], key as u64);
            assert_eq!(art.get(&[key]), Some(key as u64));
        }

        for key in KEYS {
            assert_eq!(art.get(&[key]), Some(key as u64));
        }
    }

    #[test]
    fn node256_full() {
        let art = Art::default();

        for key in 0..=255 {
            art.insert(&[key], key as u64);
            assert_eq!(art.get(&[key]), Some(key as u64));
        }

        for key in 0..=255 {
            assert_eq!(art.get(&[key]), Some(key as u64));
        }
    }
}
