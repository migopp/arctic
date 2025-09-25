use core::sync::atomic::AtomicBool;
use core::sync::atomic::Ordering;

use crate::byte;
use crate::cursor;
use crate::edge;
use crate::node;

static RECORD: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "stat")]
thread_local! {
    pub(crate) static THREAD: Thread = const { Thread::new() };
}

pub fn process<K: crate::Key, V>(map: &mut crate::Map<K, V>) -> Process {
    let mut depth = Histogram::new();
    let mut compression = Histogram::new();
    let mut node_3 = Histogram::new();
    let mut node_15 = Histogram::new();
    let mut node_256 = Histogram::new();

    let mut entries = map.raw.preorder::<byte::Ignore>();
    while let Some((depth_, _, edge)) = entries.next() {
        let meta = edge.meta();
        let kind = meta.kind();

        if kind == node::Kind::NONE {
            continue;
        }

        compression.record(byte::Array::len(meta.key()) as u64);

        if kind == node::Kind::LEAF {
            depth.record(depth_ as u64);
        } else {
            let node = unsafe { edge::Edge::next_node_unchecked(edge.data(), kind) };
            let histogram = match node {
                node::Ref::Node3(_) => &mut node_3,
                node::Ref::Node15(_) => &mut node_15,
                node::Ref::Node256(_) => &mut node_256,
            };

            let children = unsafe { node.iter() }
                .filter(|(_, edge)| {
                    let edge = edge.load(Ordering::Relaxed);
                    !matches!(edge.meta.kind, node::Kind::None)
                })
                .count();

            histogram.record(children as u64);
        }
    }

    Process {
        depth: depth.into(),
        compression: compression.into(),
        node_3: node_3.into(),
        node_15: node_15.into(),
        node_256: node_256.into(),
    }
}

#[inline]
pub fn thread() -> Thread {
    #[cfg(feature = "stat")]
    {
        THREAD.with(|thread| thread.clone())
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
    THREAD.with(|thread| thread.reset());
}

#[derive(Default)]
#[cfg_attr(feature = "stat", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(not(feature = "stat"), expect(unused))]
pub struct Process {
    depth: Distribution,
    compression: Distribution,
    node_3: Distribution,
    node_15: Distribution,
    node_256: Distribution,
}

#[derive(Default)]
#[cfg_attr(feature = "stat", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(not(feature = "stat"), expect(unused))]
struct Distribution {
    min: u64,
    max: u64,
    mean: f64,
    p50: u64,
    p75: u64,
    p90: u64,
    p99: u64,
}

struct Histogram {
    #[cfg(feature = "stat")]
    inner: hdrhistogram::Histogram<u64>,
}

impl Histogram {
    fn new() -> Self {
        Self {
            #[cfg(feature = "stat")]
            inner: hdrhistogram::Histogram::new(3).unwrap(),
        }
    }

    fn record(&mut self, _value: u64) {
        #[cfg(feature = "stat")]
        self.inner.record(_value).unwrap();
    }
}

impl From<Histogram> for Distribution {
    fn from(_histogram: Histogram) -> Self {
        #[cfg(not(feature = "stat"))]
        {
            Self::default()
        }

        #[cfg(feature = "stat")]
        {
            let histogram = _histogram.inner;
            Self {
                min: histogram.min(),
                max: histogram.max(),
                mean: histogram.mean(),
                p50: histogram.value_at_quantile(0.5),
                p75: histogram.value_at_quantile(0.75),
                p90: histogram.value_at_quantile(0.9),
                p99: histogram.value_at_quantile(0.99),
            }
        }
    }
}

#[cfg_attr(not(feature = "stat"), expect(dead_code))]
pub(crate) enum Counter {
    Op(cursor::Op),
    InsertPessimistic,
    Retire,
    #[cfg_attr(not(feature = "smr-hazard"), expect(dead_code))]
    Flush,
    FreeConflict,
    #[cfg_attr(not(feature = "smr-hazard"), expect(dead_code))]
    FreeRetire,
    #[cfg_attr(not(feature = "smr-hazard"), expect(dead_code))]
    FreeDrop,
    #[cfg_attr(not(feature = "smr-hazard"), expect(dead_code))]
    HazardMatch,
}

#[cfg_attr(not(feature = "smr-hazard"), expect(dead_code))]
pub(crate) enum Max {
    RetireCache,
}

impl From<cursor::Op> for Counter {
    fn from(op: cursor::Op) -> Self {
        Self::Op(op)
    }
}

#[cfg(feature = "stat")]
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Thread {
    node: Node,
    edge: Edge,
    insert_pessimistic: core::cell::Cell<u64>,
    retire: core::cell::Cell<u64>,
    flush: core::cell::Cell<u64>,
    retire_cache: core::cell::Cell<u64>,
    free_conflict: core::cell::Cell<u64>,
    free_retire: core::cell::Cell<u64>,
    free_drop: core::cell::Cell<u64>,
    hazard_match: core::cell::Cell<u64>,
}

#[cfg(not(feature = "stat"))]
pub struct Thread;

#[cfg(feature = "stat")]
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Node {
    replace: core::cell::Cell<u64>,
    shrink: core::cell::Cell<u64>,
    grow: core::cell::Cell<u64>,
    destroy: core::cell::Cell<u64>,
    compress: core::cell::Cell<u64>,
}

#[cfg(feature = "stat")]
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Edge {
    create: core::cell::Cell<u64>,
    expand: core::cell::Cell<u64>,
    insert: core::cell::Cell<u64>,
    remove: core::cell::Cell<u64>,
}

#[cfg(feature = "stat")]
impl Thread {
    const fn new() -> Self {
        Self {
            node: Node {
                replace: core::cell::Cell::new(0),
                shrink: core::cell::Cell::new(0),
                grow: core::cell::Cell::new(0),
                destroy: core::cell::Cell::new(0),
                compress: core::cell::Cell::new(0),
            },
            edge: Edge {
                create: core::cell::Cell::new(0),
                expand: core::cell::Cell::new(0),
                insert: core::cell::Cell::new(0),
                remove: core::cell::Cell::new(0),
            },
            insert_pessimistic: core::cell::Cell::new(0),
            retire: core::cell::Cell::new(0),
            retire_cache: core::cell::Cell::new(0),
            flush: core::cell::Cell::new(0),
            free_conflict: core::cell::Cell::new(0),
            free_retire: core::cell::Cell::new(0),
            free_drop: core::cell::Cell::new(0),
            hazard_match: core::cell::Cell::new(0),
        }
    }

    fn reset(&self) {
        self.node.replace.set(0);
        self.node.shrink.set(0);
        self.node.grow.set(0);
        self.node.destroy.set(0);
        self.node.compress.set(0);

        self.edge.create.set(0);
        self.edge.expand.set(0);
        self.edge.insert.set(0);
        self.edge.remove.set(0);
    }

    fn op(&self, op: cursor::Op) -> &core::cell::Cell<u64> {
        match op {
            cursor::Op::Node(op) => match op {
                crate::node::Op::Shrink => &self.node.shrink,
                crate::node::Op::Replace => &self.node.replace,
                crate::node::Op::Grow => &self.node.grow,
                crate::node::Op::Destroy => &self.node.destroy,
                crate::node::Op::Compress => &self.node.compress,
            },
            cursor::Op::Edge(op) => match op {
                crate::edge::Op::Create => &self.edge.create,
                crate::edge::Op::Expand => &self.edge.expand,
                crate::edge::Op::Insert => &self.edge.insert,
                crate::edge::Op::Remove => &self.edge.remove,
            },
        }
    }
}

#[inline]
pub(crate) fn increment<C: Into<Counter>>(_counter: C) {
    #[cfg(feature = "stat")]
    if RECORD.load(Ordering::Relaxed) {
        THREAD.with(|thread| {
            match _counter.into() {
                Counter::Op(op) => thread.op(op),
                Counter::InsertPessimistic => &thread.insert_pessimistic,
                Counter::Retire => &thread.retire,
                Counter::Flush => &thread.flush,
                Counter::FreeConflict => &thread.free_conflict,
                Counter::FreeRetire => &thread.free_retire,
                Counter::FreeDrop => &thread.free_drop,
                Counter::HazardMatch => &thread.hazard_match,
            }
            .update(|count| count + 1)
        })
    }
}

#[inline]
#[cfg_attr(not(feature = "smr-hazard"), expect(dead_code))]
pub(crate) fn max(_max: Max, _value: u64) {
    #[cfg(feature = "stat")]
    if RECORD.load(Ordering::Relaxed) {
        THREAD.with(|thread| {
            match _max {
                Max::RetireCache => &thread.retire_cache,
            }
            .update(|count| count.max(_value))
        })
    }
}
