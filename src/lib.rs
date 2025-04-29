#[cfg(not(feature = "loom"))]
mod sync {
    pub(crate) use core::sync::*;
}

#[cfg(feature = "loom")]
mod sync {
    pub(crate) use loom::sync::*;
}

use sync::atomic::AtomicPtr;
use sync::atomic::AtomicU8;
use sync::atomic::Ordering;

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
                return walk.compare_exchange_leaf(value);
            }

            match walk.load() {
                Some(Walk::Leaf(_)) => unreachable!(),
                Some(Walk::Node { header: _, node }) => {
                    let Some((next, key_)) = key.split_first() else {
                        unreachable!()
                    };

                    match node.get_or_insert(*next) {
                        None => {
                            let expand = node.expand();
                            match walk.compare_exchange_n256(expand) {
                                Ok(()) => (),
                                Err(_) => unsafe {
                                    drop(Box::from_raw(expand.cast::<(Header, Node256)>()));
                                },
                            }
                        }
                        Some(walk_) => {
                            key = key_;
                            walk = walk_;
                        }
                    }
                }
                None => {
                    let node = Box::new((Header::new_4(), Node4::default()));

                    let node = Box::leak(node);
                    match walk.compare_exchange_n4(node as *mut _ as _) {
                        Ok(()) => (),
                        Err(_) => unsafe {
                            drop(Box::from_raw(node as *mut (Header, Node4)));
                        },
                    }
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
        let node = Box::new((Header::new_256(), Node256::default()));

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
    prefix: Prefix,
    level: u8,
    kind: Kind,
    _pad: [u8; 6],
}

impl Header {
    pub fn new_4() -> Self {
        Self {
            prefix: Prefix::default(),
            level: 0,
            kind: Kind::N4,
            _pad: [0; 6],
        }
    }
    pub fn new_256() -> Self {
        Self {
            prefix: Prefix::default(),
            level: 0,
            kind: Kind::N256,
            _pad: [0; 6],
        }
    }
}

#[ribbit::pack(size = 64)]
#[derive(Default)]
struct Prefix {
    len: u8,
    bytes: u56,
}

const _: () = assert!(size_of::<Header>() == 16);

impl Ptr {
    const LEAF: u64 = 1 << 63;
    // const MASK_KIND: u64 = 0b11 << 61;

    fn is_null(&self) -> bool {
        self.0.load(Ordering::Relaxed).is_null()
    }

    fn copy(source: &Ptr, dest: &Ptr) {
        dest.0
            .store(source.0.load(Ordering::Relaxed), Ordering::Relaxed);
    }

    fn store(&self, next: *mut Header) {
        self.0.store(next, Ordering::Release)
    }

    fn load(&self) -> Option<Walk> {
        let tagged = self.0.load(Ordering::Acquire);
        self.translate(tagged)
    }

    fn compare_exchange_n4(&self, new: *mut Header) -> Result<(), (&Header, NodeRef)> {
        let old = match self.load() {
            None => core::ptr::null_mut(),
            Some(Walk::Leaf(_)) => unreachable!(),
            Some(Walk::Node { header, node }) => return Err((header, node)),
        };

        loop {
            match dbg!(self
                .0
                .compare_exchange(old, new, Ordering::AcqRel, Ordering::Acquire))
            {
                Ok(_) => return Ok(()),
                Err(conflict) => match self.translate(conflict) {
                    None => todo!(),
                    Some(Walk::Leaf(_)) => unreachable!(),
                    Some(Walk::Node { header, node }) => return Err((header, node)),
                },
            }
        }
    }

    fn compare_exchange_n256(&self, new: *mut Header) -> Result<(), (&Header, NodeRef)> {
        let old = match self.load() {
            None | Some(Walk::Leaf(_)) => unreachable!(),
            Some(Walk::Node { header, node }) => match header.kind {
                Kind::N4 => header as *const _ as *mut _,
                Kind::N256 => return Err((header, node)),
            },
        };

        loop {
            match dbg!(self
                .0
                .compare_exchange(old, new, Ordering::AcqRel, Ordering::Acquire))
            {
                Ok(_) => return Ok(()),
                Err(conflict) => match self.translate(conflict) {
                    None => todo!(),
                    Some(Walk::Leaf(_)) => unreachable!(),
                    Some(Walk::Node { header, node }) => return Err((header, node)),
                },
            }
        }
    }

    fn compare_exchange_leaf(&self, new: u64) -> Option<u64> {
        let mut old = match self.load() {
            None => 0,
            Some(Walk::Leaf(old)) => old | Self::LEAF,
            Some(Walk::Node { .. }) => unreachable!(),
        };

        let new = (new | Self::LEAF) as *mut _;

        loop {
            match self
                .0
                .compare_exchange(old as *mut _, new, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(old) if old.is_null() => return None,
                Ok(old) => return Some(Self::untag(old) as u64),
                Err(conflict) => match self.translate(conflict) {
                    None => old = 0,
                    Some(Walk::Leaf(conflict)) => old = conflict | Self::LEAF,
                    Some(Walk::Node { .. }) => unreachable!(),
                },
            }
        }
    }

    fn translate(&self, tagged: *mut Header) -> Option<Walk> {
        if tagged.is_null() {
            return None;
        }

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
