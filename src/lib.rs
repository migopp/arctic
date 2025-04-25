use core::sync::atomic::AtomicPtr;
use core::sync::atomic::Ordering;

#[derive(Default)]
pub struct Art {
    root: Ptr,
}

struct Node256([Ptr; 256]);

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
                    Some(Walk::Node(_)) => unreachable!(),
                    Some(Walk::Leaf(old)) => Some(old),
                    None => None,
                };
            }

            match walk.load() {
                Some(Walk::Leaf(_)) => unreachable!(),
                Some(Walk::Node(Header { node })) => {
                    let Some((next, key_)) = key.split_first() else {
                        unreachable!()
                    };

                    key = key_;
                    walk = &node.0[*next as usize];
                }
                None => {
                    let header = Box::<Header>::new(Header {
                        node: Node256(std::array::from_fn(|_| Ptr::default())),
                    });
                    let header = Box::leak(header);
                    walk.store(header);
                }
            }
        }
    }

    pub fn get(&self, mut key: &[u8]) -> Option<u64> {
        let mut walk = self.root.load()?;

        loop {
            match walk {
                Walk::Node(Header { node }) => {
                    let (next, key_) = key.split_first()?;
                    key = key_;
                    walk = node.0[*next as usize].load()?;
                }
                Walk::Leaf(value) if key.is_empty() => break Some(value),
                Walk::Leaf(_) => break None,
            }
        }
    }
}

// enum NodeRef<'a> {
//     N4(&'a ()),
// }

// #[repr(C)]
// struct Node4 {
//     keys: [u8; 4],
//     pointers: [Ptr; 4],
// }

#[derive(Default)]
struct Ptr(AtomicPtr<Header>);

enum Walk<'a> {
    Node(&'a Header),
    Leaf(u64),
}

enum Kind {
    // N4,
    // N16,
    // N48,
    N256,
}

#[repr(C)]
struct Header {
    // kind: Kind,
    node: Node256,
}

impl Ptr {
    const LEAF: u64 = 1 << 63;
    const MASK_KIND: u64 = 0b11 << 61;

    // fn leaf(pointer: *mut T) -> Self {
    //     Self(AtomicPtr::new(
    //         pointer.map_addr(|address| address | Self::LEAF),
    //     ))
    // }

    fn is_null(&self) -> bool {
        self.0.load(Ordering::Relaxed).is_null()
    }

    fn store_leaf(&self, next: u64) {
        self.0
            .store(dbg!((next | Self::LEAF) as *mut _), Ordering::Relaxed)
    }

    fn store(&self, next: *mut Header) {
        self.0.store(next, Ordering::Relaxed)
    }

    fn load(&self) -> Option<Walk> {
        let tagged = self.0.load(Ordering::Relaxed);
        let untagged = Self::untag(tagged);

        match Self::is_leaf(tagged) {
            true => Some(Walk::Leaf(untagged as u64)),
            false => unsafe { untagged.as_ref() }.map(Walk::Node),
        }
    }

    fn untag(address: *mut Header) -> *mut Header {
        address.map_addr(|address| address & !Self::LEAF as usize)
    }

    fn is_leaf(address: *mut Header) -> bool {
        address.addr() as u64 & Self::LEAF > 0
    }
}
