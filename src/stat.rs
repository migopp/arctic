use core::sync::atomic::AtomicBool;
use core::sync::atomic::Ordering;

use ribbit::Unpack as _;

use crate::raw::edge;
use crate::raw::edge::Key as _;
use crate::raw::edge::Len as _;
use crate::raw::edge::Meta as _;
use crate::raw::iter::Unbound;
use crate::raw::node;
use crate::raw::Smo;
use crate::Key;
use crate::Value;

static RECORD: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "stat")]
thread_local! {
    pub(crate) static THREAD: core::cell::RefCell<Thread> = core::cell::RefCell::new(Thread::default()) ;
}

pub fn process<K: Key, V: Value>(map: &mut crate::concurrent::Map<K, V>) -> Process {
    let mut depth = Histogram::default();
    let mut compression = Histogram::default();
    let mut node_3 = Histogram::default();
    let mut node_15 = Histogram::default();
    let mut node_47 = Histogram::default();
    let mut node_256 = Histogram::default();

    map.as_sequential().postorder().for_each(|edge, depth_| {
        let Some(child) = edge.child() else {
            return;
        };

        let meta = edge.meta();
        let bits = meta.key().len().bits();
        compression.record((bits >> 3) as u64);

        match child {
            edge::Child::Value(_) => {
                depth.record(depth_ as u64);
            }
            edge::Child::Node(node) => {
                let histogram = match node.kind().unpack() {
                    node::Kind::Node3 => &mut node_3,
                    node::Kind::Node15 => &mut node_15,
                    node::Kind::Node47 => &mut node_47,
                    node::Kind::Node256 => &mut node_256,
                };

                let children = unsafe { node.entries(Unbound, Unbound) }
                    .filter(|(_, edge)| !edge.load_packed(Ordering::Relaxed).is_null())
                    .count();

                histogram.record(children as u64);
            }
        }
    });

    Process {
        depth,
        compression,
        node_3,
        node_15,
        node_47,
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
    node_47: Histogram,
    node_256: Histogram,
}

#[cfg_attr(not(feature = "stat"), expect(dead_code))]
#[derive(Copy, Clone)]
pub(crate) enum Counter {
    Op(Smo),
    InsertPessimistic,
    GetOrInsertPessimistic,
    Retire,
    FreeConflict,
    FreeRetire,
    FreeReclaim,
    FreeDrop,
    HazardMatch,

    Node47Consistent,
    Node47CasSuccess,
    Node47CasFailure,

    ScanInsert,
    ScanUpdate,
    ScanScan,

    LockFrozen,
    UnlockFrozen,
}

pub(crate) enum Max {
    RetireCache,
}

pub(crate) enum Record {
    Flush,
    FreezePop,
    ReclaimDepth,
    ReclaimAge0,
    ReclaimAge1,
    ReclaimAge2,
    ReclaimAge3,
}

impl From<Smo> for Counter {
    fn from(op: Smo) -> Self {
        Self::Op(op)
    }
}

#[cfg(feature = "stat")]
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Thread {
    node: Node,
    edge: Edge,
    insert_pessimistic: u64,
    get_or_insert_pessimistic: u64,
    flush: Histogram,
    retire: u64,
    retire_cache: u64,
    free_conflict: u64,
    free_retire: u64,
    free_reclaim: u64,
    free_drop: u64,
    hazard_match: u64,
    scan_insert: u64,
    scan_update: u64,
    scan_scan: u64,
    scan_freeze: u64,
    lock_frozen: u64,
    unlock_frozen: u64,

    node_47_consistent: u64,
    node_47_cas_success: u64,
    node_47_cas_failure: u64,

    freeze_pop: Histogram,

    reclaim_depth: Histogram,

    // Age at reclamation for allocations with n byte prefixes
    reclaim_age_0: Histogram,
    reclaim_age_1: Histogram,
    reclaim_age_2: Histogram,
    reclaim_age_3: Histogram,
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
}

#[cfg(feature = "stat")]
impl Thread {
    fn op(&mut self, op: Smo) -> &mut u64 {
        match op {
            Smo::Node(op) => match op {
                node::Smo::Shrink => &mut self.node.shrink,
                node::Smo::Replace => &mut self.node.replace,
                node::Smo::Grow => &mut self.node.grow,
                node::Smo::Destroy => &mut self.node.destroy,
                node::Smo::Compress => &mut self.node.compress,
            },
            Smo::Edge(op) => match op {
                edge::Smo::Create => &mut self.edge.create,
                edge::Smo::Expand => &mut self.edge.expand,
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
                Counter::GetOrInsertPessimistic => &mut thread.get_or_insert_pessimistic,
                Counter::Retire => &mut thread.retire,
                Counter::FreeConflict => &mut thread.free_conflict,
                Counter::FreeRetire => &mut thread.free_retire,
                Counter::FreeReclaim => &mut thread.free_reclaim,
                Counter::FreeDrop => &mut thread.free_drop,
                Counter::HazardMatch => &mut thread.hazard_match,
                Counter::ScanInsert => &mut thread.scan_insert,
                Counter::ScanUpdate => &mut thread.scan_update,
                Counter::ScanScan => &mut thread.scan_scan,

                Counter::Node47Consistent => &mut thread.node_47_consistent,
                Counter::Node47CasSuccess => &mut thread.node_47_cas_success,
                Counter::Node47CasFailure => &mut thread.node_47_cas_failure,

                Counter::LockFrozen => &mut thread.lock_frozen,
                Counter::UnlockFrozen => &mut thread.unlock_frozen,
            } += 1;
        })
    }
}

#[inline]
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
                Record::Flush => &mut thread.flush,
                Record::FreezePop => &mut thread.freeze_pop,
                Record::ReclaimDepth => &mut thread.reclaim_depth,
                Record::ReclaimAge0 => &mut thread.reclaim_age_0,
                Record::ReclaimAge1 => &mut thread.reclaim_age_1,
                Record::ReclaimAge2 => &mut thread.reclaim_age_2,
                Record::ReclaimAge3 => &mut thread.reclaim_age_3,
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
