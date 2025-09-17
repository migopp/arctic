use core::cell::Cell;
use core::sync::atomic::AtomicBool;
use core::sync::atomic::Ordering;

use crate::cursor;
use crate::node;
use crate::Or;

static RECORD: AtomicBool = AtomicBool::new(false);

thread_local! {
    pub(crate) static THREAD: Thread = const { Thread::new() };
}

pub fn process<K, V>(map: &mut crate::Map<K, V>) -> Process {
    let mut depth = Histogram::new();
    let mut compression = Histogram::new();
    let mut node_3 = Histogram::new();
    let mut node_15 = Histogram::new();
    let mut node_256 = Histogram::new();

    map.raw.preorder().for_each(|(depth_, _, meta, data)| {
        let Some(child) = (unsafe { data.to_node(meta.kind) }) else {
            return;
        };

        compression.record(meta.key.len.to_usize() as u64);

        match child {
            Or::L(_) => {
                depth.record(depth_ as u64);
            }
            Or::R(node) => {
                let histogram = match node {
                    node::Ref::Node3(_) => &mut node_3,
                    node::Ref::Node15(_) => &mut node_15,
                    node::Ref::Node256(_) => &mut node_256,
                };

                let children = unsafe { node.iter() }
                    .filter(|(_, edge)| {
                        let meta = edge.load_low(Ordering::Relaxed);
                        !matches!(meta.kind, node::Kind::None)
                    })
                    .count();

                histogram.record(children as u64);
            }
        }
    });

    Process {
        depth: depth.into(),
        compression: compression.into(),
        node_3: node_3.into(),
        node_15: node_15.into(),
        node_256: node_256.into(),
    }
}

pub fn thread() -> Thread {
    THREAD.with(|thread| thread.clone())
}

pub fn start() {
    if cfg!(feature = "stat") {
        RECORD.store(true, Ordering::Relaxed);
    }
}

pub fn stop() {
    if cfg!(feature = "stat") {
        RECORD.store(false, Ordering::Relaxed);
    }
}

pub fn reset() {
    if cfg!(feature = "stat") {
        THREAD.with(|thread| thread.reset());
    }
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

pub(crate) enum Counter {
    Op(cursor::Op),
    InsertPessimistic,
}

impl From<cursor::Op> for Counter {
    fn from(op: cursor::Op) -> Self {
        Self::Op(op)
    }
}

#[derive(Clone)]
#[cfg_attr(feature = "stat", derive(serde::Serialize, serde::Deserialize))]
pub struct Thread {
    node: Node,
    edge: Edge,
    insert_pessimistic: Cell<u64>,
}

#[derive(Clone)]
#[cfg_attr(feature = "stat", derive(serde::Serialize, serde::Deserialize))]
struct Node {
    replace: Cell<u64>,
    shrink: Cell<u64>,
    grow: Cell<u64>,
    destroy: Cell<u64>,
    compress: Cell<u64>,
}

#[derive(Clone)]
#[cfg_attr(feature = "stat", derive(serde::Serialize, serde::Deserialize))]
struct Edge {
    create: Cell<u64>,
    expand: Cell<u64>,
    insert: Cell<u64>,
    remove: Cell<u64>,
}

impl Thread {
    const fn new() -> Self {
        Self {
            node: Node {
                replace: Cell::new(0),
                shrink: Cell::new(0),
                grow: Cell::new(0),
                destroy: Cell::new(0),
                compress: Cell::new(0),
            },
            edge: Edge {
                create: Cell::new(0),
                expand: Cell::new(0),
                insert: Cell::new(0),
                remove: Cell::new(0),
            },
            insert_pessimistic: Cell::new(0),
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

    fn op(&self, op: cursor::Op) -> &Cell<u64> {
        match op {
            cursor::Op::Node(op) => match op {
                node::Op::Shrink => &self.node.shrink,
                node::Op::Replace => &self.node.replace,
                node::Op::Grow => &self.node.grow,
                node::Op::Destroy => &self.node.destroy,
                node::Op::Compress => &self.node.compress,
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
pub(crate) fn increment<C: Into<Counter>>(counter: C) {
    if cfg!(feature = "stat") && RECORD.load(Ordering::Relaxed) {
        match counter.into() {
            Counter::Op(op) => THREAD.with(|thread| thread.op(op).update(|count| count + 1)),
            Counter::InsertPessimistic => {
                THREAD.with(|thread| thread.insert_pessimistic.update(|count| count + 1))
            }
        }
    }
}
