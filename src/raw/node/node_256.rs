use core::fmt::Debug;
use core::ptr::NonNull;

use ribbit::Atomic;

use crate::raw::edge;
use crate::raw::node;
use crate::raw::node::Edge;
use crate::raw::node::Node;

#[repr(C, align(4096))]
pub(crate) struct Node256<M: ribbit::Pack>([Atomic<Edge<M>>; 256]);

const_assert_size_align!(Node256::<()>, 4096, 4096);

impl<M> Default for Node256<M>
where
    M: ribbit::Pack<Packed: edge::Meta>,
{
    fn default() -> Self {
        Self(core::array::from_fn(|_| Atomic::new_packed(Edge::DEFAULT)))
    }
}

unsafe impl<M> Node<M> for Node256<M>
where
    M: ribbit::Pack<Packed: edge::Meta>,
{
    const TYPE: node::Type = node::Type::Node256;
    const CAPACITY: usize = 256;

    unsafe fn new_unchecked(keys: &[u8], edges: &[ribbit::Packed<Edge<M>>]) -> NonNull<Self> {
        if_validate!(crate::assert_unique(keys));
        validate!(keys.len() == edges.len());
        validate!(keys.len() <= Self::CAPACITY);

        #[cfg(not(feature = "opt-node256-mmap"))]
        let mut node = NonNull::from(Box::leak(Box::new(Self::default())));

        #[cfg(feature = "opt-node256-mmap")]
        let mut node = unsafe {
            match libc::mmap64(
                core::ptr::null_mut(),
                4096,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_ANONYMOUS | libc::MAP_PRIVATE | libc::MAP_POPULATE,
                -1,
                0,
            ) {
                libc::MAP_FAILED => panic!("mmap: {}", std::io::Error::last_os_error()),
                ptr => NonNull::new(ptr)
                    .expect("mmap should not return null")
                    .cast::<Node256<_>>(),
            }
        };

        for (key, edge) in keys.iter().zip(edges) {
            unsafe { node.as_mut() }.0[*key as usize].set_packed(*edge);
        }

        node
    }

    #[inline]
    fn keys<L: node::iter::Lower, U: node::iter::Upper>(
        &self,
        lower: L,
        upper: U,
    ) -> node::KeyIter {
        node::KeyIter::new_256(KeyIter::new(lower, upper))
    }

    #[inline]
    fn edges(&self) -> &[Atomic<Edge<M>>] {
        &self.0
    }

    #[inline]
    fn edges_mut(&mut self) -> &mut [Atomic<Edge<M>>] {
        &mut self.0
    }

    #[inline]
    fn get_key(&self, key: u8) -> Option<u8> {
        Some(key)
    }

    #[inline]
    fn get_or_insert_key(&self, key: u8) -> Option<u8> {
        Some(key)
    }

    #[inline]
    fn insert_key(&mut self, key: u8) -> Option<u8> {
        Some(key)
    }

    #[inline]
    fn freeze_header(&self) -> usize {
        Self::CAPACITY
    }
}

impl<M> Debug for Node256<M>
where
    M: ribbit::Pack<Packed: edge::Meta + Debug>,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Node256").field("edges", &self.0).finish()
    }
}

#[repr(C)]
#[derive(Copy, Clone)]
pub(crate) struct KeyIter {
    #[cfg(target_endian = "big")]
    _tag: Tag,

    head: u16,
    tail: u16,

    #[cfg(target_endian = "little")]
    _tag: Tag,
}

#[repr(u32)]
#[derive(Copy, Clone)]
enum Tag {
    Node256 = (node::Type::Node256 as u32) << 30,
}

impl KeyIter {
    #[inline]
    fn new<L: node::iter::Lower, U: node::iter::Upper>(lower: L, upper: U) -> Self {
        Self {
            head: lower.get() as u16,
            tail: upper.get() as u16 + 1,
            _tag: Tag::Node256,
        }
    }
}

impl Iterator for KeyIter {
    type Item = u8;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.head == self.tail {
            return None;
        }

        let next = self.head as u8;
        self.head += 1;
        Some(next)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = (self.tail - self.head) as usize;
        (len, Some(len))
    }
}

impl ExactSizeIterator for KeyIter {
    #[inline]
    fn len(&self) -> usize {
        let (lower, upper) = self.size_hint();
        validate_eq!(upper, Some(lower));
        lower
    }
}

impl DoubleEndedIterator for KeyIter {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.head == self.tail {
            return None;
        }

        self.tail -= 1;
        Some(self.tail as u8)
    }
}
