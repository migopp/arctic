use core::sync::atomic::AtomicPtr;
use core::sync::atomic::AtomicU8;
use core::sync::atomic::Ordering;

#[derive(Default)]
pub struct Art {
    root: Ptr,
}

impl Art {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, mut key: &[u8], value: u64) -> Option<u64> {
        let mut walk = &self.root;

        loop {
            if key.is_empty() {
                let old = walk.load();
                walk.store_leaf(value);
                return match old {
                    Some(Walk::Node { .. }) => unreachable!(),
                    Some(Walk::Leaf(old)) => Some(old),
                    None => None,
                };
            }

            match walk.load() {
                Some(Walk::Leaf(_)) => unreachable!(),
                Some(Walk::Node { header: _, node }) => {
                    let Some((next, key_)) = key.split_first() else {
                        unreachable!()
                    };

                    match node.get_or_insert(*next) {
                        None => {
                            walk.store(node.expand());
                        }
                        Some(walk_) => {
                            key = key_;
                            walk = walk_;
                        }
                    }
                }
                None => {
                    let node = Box::new((
                        Header {
                            kind: Kind::N4,
                            _pad: [0; 7],
                        },
                        Node4::default(),
                    ));

                    let node = Box::leak(node);
                    walk.store(node as *mut _ as _);
                }
            }
        }
    }

    pub fn get(&self, mut key: &[u8]) -> Option<u64> {
        let mut walk = self.root.load()?;

        loop {
            match walk {
                Walk::Node { header: _, node } => {
                    let (next, key_) = key.split_first()?;
                    key = key_;
                    walk = node.get(*next)?.load()?;
                }
                Walk::Leaf(value) if key.is_empty() => break Some(value),
                Walk::Leaf(_) => break None,
            }
        }
    }
}

enum NodeRef<'a> {
    N4(&'a Node4),
    N256(&'a Node256),
}

impl<'a> NodeRef<'a> {
    fn get_or_insert(&self, byte: u8) -> Option<&'a Ptr> {
        match self {
            NodeRef::N4(node) => node.get_or_insert(byte),
            NodeRef::N256(node) => node.get_or_insert(byte),
        }
    }

    fn get(&self, byte: u8) -> Option<&'a Ptr> {
        match self {
            NodeRef::N4(node) => node.get(byte),
            NodeRef::N256(node) => node.get(byte),
        }
    }

    fn expand(&self) -> *mut Header {
        match self {
            NodeRef::N4(node) => node.expand(),
            NodeRef::N256(_) => unreachable!(),
        }
    }
}

#[repr(C)]
#[derive(Default)]
struct Node4 {
    keys: [AtomicU8; 4],
    pointers: [Ptr; 4],
}

impl Node4 {
    fn get_or_insert(&self, byte: u8) -> Option<&Ptr> {
        if let Some(pointer) = self.get(byte) {
            return Some(pointer);
        }

        self.keys
            .iter()
            .zip(&self.pointers)
            .find(|(_, pointer)| pointer.is_null())
            .map(|(key, pointer)| {
                key.store(byte, Ordering::Relaxed);
                pointer
            })
    }

    fn get(&self, byte: u8) -> Option<&Ptr> {
        self.keys
            .iter()
            .zip(&self.pointers)
            .find_map(|(key, pointer)| match key.load(Ordering::Relaxed) == byte {
                true => Some(pointer),
                false => None,
            })
    }

    fn expand(&self) -> *mut Header {
        let node = Box::new((
            Header {
                kind: Kind::N256,
                _pad: [0; 7],
            },
            Node256::default(),
        ));

        self.keys
            .iter()
            .zip(&self.pointers)
            .filter(|(_, pointer)| !pointer.is_null())
            .for_each(|(key, pointer)| {
                Ptr::copy(pointer, &node.1 .0[key.load(Ordering::Relaxed) as usize])
            });

        let node = Box::leak(node);
        node as *mut _ as _
    }
}

#[repr(C)]
struct Node256([Ptr; 256]);

impl Default for Node256 {
    fn default() -> Self {
        Self(std::array::from_fn(|_| Ptr::default()))
    }
}

impl Node256 {
    fn get_or_insert(&self, byte: u8) -> Option<&Ptr> {
        self.get(byte)
    }

    fn get(&self, byte: u8) -> Option<&Ptr> {
        Some(&self.0[byte as usize])
    }
}

#[derive(Default)]
struct Ptr(AtomicPtr<Header>);

enum Walk<'a> {
    Node {
        header: &'a Header,
        node: NodeRef<'a>,
    },
    Leaf(u64),
}

enum Kind {
    N4,
    // N16,
    // N48,
    N256,
}

#[repr(C)]
struct Header {
    kind: Kind,
    _pad: [u8; 7],
}

const _: () = assert!(size_of::<Header>() == 8);

impl Ptr {
    const LEAF: u64 = 1 << 63;
    // const MASK_KIND: u64 = 0b11 << 61;

    fn is_null(&self) -> bool {
        self.0.load(Ordering::Relaxed).is_null()
    }

    fn store_leaf(&self, next: u64) {
        self.0
            .store((next | Self::LEAF) as *mut _, Ordering::Relaxed)
    }

    fn copy(source: &Ptr, dest: &Ptr) {
        dest.0
            .store(source.0.load(Ordering::Relaxed), Ordering::Relaxed);
    }

    fn store(&self, next: *mut Header) {
        self.0.store(next, Ordering::Relaxed)
    }

    fn load(&self) -> Option<Walk> {
        let tagged = self.0.load(Ordering::Relaxed);
        let untagged = Self::untag(tagged);

        if Self::is_leaf(tagged) {
            return Some(Walk::Leaf(untagged as u64));
        }

        let header = unsafe { untagged.as_ref() }?;
        let node = match header.kind {
            Kind::N4 => NodeRef::N4(unsafe { untagged.add(1).cast::<Node4>().as_ref().unwrap() }),
            Kind::N256 => {
                NodeRef::N256(unsafe { untagged.add(1).cast::<Node256>().as_ref().unwrap() })
            }
        };

        Some(Walk::Node { header, node })
    }

    fn untag(address: *mut Header) -> *mut Header {
        address.map_addr(|address| address & !Self::LEAF as usize)
    }

    fn is_leaf(address: *mut Header) -> bool {
        address.addr() as u64 & Self::LEAF > 0
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
