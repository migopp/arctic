use core::sync::atomic::AtomicBool;
use core::sync::atomic::Ordering;

use crate::edge;
use crate::node;
use crate::raw;

static RECORD: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "stat")]
thread_local! {
    pub(crate) static THREAD: core::cell::RefCell<Thread> = core::cell::RefCell::new(Thread::default()) ;
}

pub fn process<K: crate::Key, V>(map: &mut crate::concurrent::Map<K, V>) -> Process {
    let mut depth = Histogram::default();
    let mut compression = Histogram::default();
    let mut node_3 = Histogram::default();
    let mut node_15 = Histogram::default();
    let mut node_256 = Histogram::default();

    for (edge, depth_) in map
        .as_sequential()
        .as_raw()
        .postorder::<raw::iter::postorder::SelectAll>()
    {
        let meta = edge.meta();
        let data = edge.data();

        compression.record(meta.key().len().bits() as u64);

        if meta.leaf() {
            depth.record(depth_ as u64);
        } else {
            let node = unsafe { data.into_node_unchecked() };
            let histogram = match node {
                node::Ref::Node3(_) => &mut node_3,
                node::Ref::Node15(_) => &mut node_15,
                node::Ref::Node256(_) => &mut node_256,
            };

            let children = unsafe { node.iter_unsorted() }
                .filter(|(_, edge)| !edge.load_packed(Ordering::Relaxed).is_null())
                .count();

            histogram.record(children as u64);
        }
    }

    Process {
        depth,
        compression,
        node_3,
        node_15,
        node_256,
    }
}

#[inline]
pub fn thread() -> Thread {
    #[cfg(feature = "stat")]
    {
        THREAD.with_borrow(|thread| thread.clone())
    }

    #[cfg(not(feature = "stat"))]
    {
        Thread
    }
}

#[inline]
pub fn start() {
    if cfg!(feature = "stat") {
        RECORD.store(true, Ordering::Relaxed);
    }
}

#[inline]
pub fn stop() {
    if cfg!(feature = "stat") {
        RECORD.store(false, Ordering::Relaxed);
    }
}

#[inline]
pub fn reset() {
    #[cfg(feature = "stat")]
    THREAD.with_borrow_mut(|thread| *thread = Thread::default());
}

#[derive(Default)]
#[cfg_attr(feature = "stat", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(not(feature = "stat"), expect(unused))]
pub struct Process {
    depth: Histogram,
    compression: Histogram,
    node_3: Histogram,
    node_15: Histogram,
    node_256: Histogram,
}

#[cfg_attr(not(feature = "stat"), expect(dead_code))]
pub(crate) enum Counter {
    Op(raw::Op),
    InsertPessimistic,
    Retire,
    #[cfg_attr(not(feature = "smr-hazard"), expect(dead_code))]
    Flush,
    FreeConflict,
    #[cfg_attr(not(feature = "smr-hazard"), expect(dead_code))]
    FreeRetire,
    FreeDrop,
    #[cfg_attr(not(feature = "smr-hazard"), expect(dead_code))]
    HazardMatch,
}

#[cfg_attr(not(feature = "smr-hazard"), expect(dead_code))]
pub(crate) enum Max {
    RetireCache,
}

pub(crate) enum Record {
    RangeConflict,
}

impl From<raw::Op> for Counter {
    fn from(op: raw::Op) -> Self {
        Self::Op(op)
    }
}

impl From<edge::Op> for Counter {
    fn from(op: edge::Op) -> Self {
        Self::Op(raw::Op::Edge(op))
    }
}

impl From<node::Op> for Counter {
    fn from(op: node::Op) -> Self {
        Self::Op(raw::Op::Node(op))
    }
}

#[cfg(feature = "stat")]
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Thread {
    node: Node,
    edge: Edge,
    insert_pessimistic: u64,
    retire: u64,
    flush: u64,
    retire_cache: u64,
    free_conflict: u64,
    free_retire: u64,
    free_drop: u64,
    hazard_match: u64,
    range_retry: Histogram,
}

#[cfg(not(feature = "stat"))]
pub struct Thread;

#[cfg(feature = "stat")]
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
struct Node {
    replace: u64,
    shrink: u64,
    grow: u64,
    destroy: u64,
    compress: u64,
}

#[cfg(feature = "stat")]
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
struct Edge {
    create: u64,
    expand: u64,
    insert: u64,
    remove: u64,
}

#[cfg(feature = "stat")]
impl Thread {
    fn op(&mut self, op: raw::Op) -> &mut u64 {
        match op {
            raw::Op::Node(op) => match op {
                crate::node::Op::Shrink => &mut self.node.shrink,
                crate::node::Op::Replace => &mut self.node.replace,
                crate::node::Op::Grow => &mut self.node.grow,
                crate::node::Op::Destroy => &mut self.node.destroy,
                crate::node::Op::Compress => &mut self.node.compress,
            },
            raw::Op::Edge(op) => match op {
                crate::edge::Op::Create => &mut self.edge.create,
                crate::edge::Op::Expand => &mut self.edge.expand,
                crate::edge::Op::Insert => &mut self.edge.insert,
                crate::edge::Op::Remove => &mut self.edge.remove,
            },
        }
    }
}

#[inline]
pub(crate) fn increment<C: Into<Counter>>(_counter: C) {
    #[cfg(feature = "stat")]
    if RECORD.load(Ordering::Relaxed) {
        THREAD.with_borrow_mut(|thread| {
            *match _counter.into() {
                Counter::Op(op) => thread.op(op),
                Counter::InsertPessimistic => &mut thread.insert_pessimistic,
                Counter::Retire => &mut thread.retire,
                Counter::Flush => &mut thread.flush,
                Counter::FreeConflict => &mut thread.free_conflict,
                Counter::FreeRetire => &mut thread.free_retire,
                Counter::FreeDrop => &mut thread.free_drop,
                Counter::HazardMatch => &mut thread.hazard_match,
            } += 1;
        })
    }
}

#[inline]
#[cfg_attr(not(feature = "smr-hazard"), expect(dead_code))]
pub(crate) fn max(_max: Max, _value: u64) {
    #[cfg(feature = "stat")]
    if RECORD.load(Ordering::Relaxed) {
        THREAD.with_borrow_mut(|thread| {
            let old = match _max {
                Max::RetireCache => &mut thread.retire_cache,
            };
            *old = (*old).max(_value);
        })
    }
}

#[inline]
pub(crate) fn record(_record: Record, _value: u64) {
    #[cfg(feature = "stat")]
    if RECORD.load(Ordering::Relaxed) {
        THREAD.with_borrow_mut(|thread| {
            let old = match _record {
                Record::RangeConflict => &mut thread.range_retry,
            };
            old.record(_value);
        })
    }
}

#[derive(Clone)]
struct Histogram {
    #[cfg(feature = "stat")]
    inner: hdrhistogram::Histogram<u64>,
}

impl Histogram {
    fn record(&mut self, _value: u64) {
        #[cfg(feature = "stat")]
        self.inner.record(_value).unwrap();
    }
}

impl Default for Histogram {
    fn default() -> Self {
        Self {
            #[cfg(feature = "stat")]
            inner: hdrhistogram::Histogram::new(3).unwrap(),
        }
    }
}

#[cfg(feature = "stat")]
impl serde::Serialize for Histogram {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use hdrhistogram::serialization::Serializer as _;
        use hdrhistogram::serialization::V2DeflateSerializer;
        use serde::ser::Error as _;

        let mut buffer = Vec::new();

        {
            let mut encoder = base64::write::EncoderWriter::new(
                &mut buffer,
                &base64::engine::general_purpose::STANDARD,
            );

            V2DeflateSerializer::new()
                .serialize(&self.inner, &mut encoder)
                .map_err(S::Error::custom)?;
        }

        serializer.serialize_str(str::from_utf8(&buffer).map_err(S::Error::custom)?)
    }
}

#[cfg(feature = "stat")]
impl<'de> serde::Deserialize<'de> for Histogram {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use hdrhistogram::serialization::Deserializer;
        use serde::de::Error as _;

        let mut string = <&'de str>::deserialize(deserializer).map(std::io::Cursor::new)?;
        let mut decoder = base64::read::DecoderReader::new(
            &mut string,
            &base64::engine::general_purpose::STANDARD,
        );

        Ok(Histogram {
            inner: Deserializer::new()
                .deserialize(&mut decoder)
                .map_err(D::Error::custom)?,
        })
    }
}
