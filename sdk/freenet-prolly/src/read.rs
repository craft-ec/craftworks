//! Point lookup.

use crate::node::Value;
use crate::store::{load, Blocks, Held, ReadError};
use crate::Cid;

/// Levels in the tree at `root`; 1 is a single leaf.
pub fn height(blocks: &impl Blocks, root: &Cid) -> Result<usize, ReadError> {
    Ok(load(blocks, root)?.level() as usize + 1)
}

/// The value stored under `key` in the tree at `root`, if any.
pub fn get<'a>(
    blocks: &'a impl Blocks,
    root: &Cid,
    key: &[u8],
) -> Result<Option<Value<'a>>, ReadError> {
    let mut node = Held::root(blocks, root)?;
    while !node.is_leaf() {
        // A branch key is its child's smallest key: the child that can hold
        // `key` is the last one whose key is ≤ `key`.
        let i = match node.search(key) {
            Ok(i) => i,
            Err(0) => return Ok(None),
            Err(i) => i - 1,
        };
        node = node.open(blocks, i)?;
    }
    Ok(node.search(key).ok().map(|i| node.value(i)))
}
