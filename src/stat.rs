use core::cell::Cell;

thread_local! {
    pub(crate) static THREAD: Thread = const { Thread::new() };
}

pub fn thread() -> Thread {
    THREAD.with(|thread| thread.clone())
}

#[derive(Clone)]
#[cfg_attr(feature = "stat", derive(serde::Serialize, serde::Deserialize))]
pub struct Thread {
    node: Node,
    edge: Edge,
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
        }
    }

    fn get(&self, op: &crate::cursor::Op) -> &Cell<u64> {
        match op {
            crate::cursor::Op::Node(op) => match op {
                crate::node::Op::Shrink => &self.node.shrink,
                crate::node::Op::Replace => &self.node.replace,
                crate::node::Op::Grow => &self.node.grow,
                crate::node::Op::Destroy => &self.node.destroy,
                crate::node::Op::Compress => &self.node.compress,
            },
            crate::cursor::Op::Edge(op) => match op {
                crate::edge::Op::Create => &self.edge.create,
                crate::edge::Op::Expand => &self.edge.expand,
                crate::edge::Op::Insert => &self.edge.insert,
                crate::edge::Op::Remove => &self.edge.remove,
            },
        }
    }
}

#[inline]
pub(crate) fn increment(op: &crate::cursor::Op) {
    if cfg!(feature = "stat") {
        THREAD.with(|thread| thread.get(op).update(|count| count + 1))
    }
}
