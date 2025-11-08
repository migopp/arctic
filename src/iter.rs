pub struct Sorted;

pub struct Unsorted;

pub trait Order {
    const REVERSE: bool;
    const SORTED: bool;
}

impl Order for Sorted {
    const REVERSE: bool = false;
    const SORTED: bool = true;
}

impl Order for core::iter::Rev<Sorted> {
    const REVERSE: bool = true;
    const SORTED: bool = true;
}

impl Order for Unsorted {
    const REVERSE: bool = false;
    const SORTED: bool = false;
}
