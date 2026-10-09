//! LIVE, as the tree's own diff (READ-STATE inv. 5): each bound watch key keeps
//! the root its binding was last told about — its `RenderedAt`, the one
//! head-shaped fact a binding legitimately owns, since only it knows what it
//! rendered — and a head move is answered by diffing from there to the store's
//! root over the key's range.
//!
//! In the SDK, not in `web::Session`, so something native can run it: the
//! session is a `#[wasm_bindgen]` type in a `cdylib`, and a decision that
//! lives there is one no test can reach (the lesson of `parking` and
//! `Refresh`, whose files this replaces).

use crate::app::StoredName;
use crate::store::{Delta, Reads};
use freenet_prolly::Cid;
use std::collections::BTreeMap;

/// What a LIVE binding watches — a domain, or one parent's band of it
/// (`Db::watch_key`) — as the TREE names it. Built ONLY from a
/// [`StoredName`]: a name as JavaScript holds it (an app-relative
/// `AppName`, or any string) is not a watch key, so a binding keyed by the
/// wrong form does not compile (sdk#287's two types, carried through):
///
/// ```compile_fail
/// use craftworks_sdk::LiveBindings;
/// let mut live = LiveBindings::default();
/// // A JavaScript string where a watch key is expected:
/// live.bind("notes".to_string());
/// ```
///
/// The control, which DOES compile — the same name through `app::read`:
///
/// ```
/// use craftworks_sdk::{app, LiveBindings};
/// use craftworks_sdk::live_bindings::WatchKey;
/// let mut live = LiveBindings::default();
/// live.bind(WatchKey::of(app::read(Some("notes-app"), "notes").unwrap()));
/// assert_eq!(live.len(), 1);
/// ```
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WatchKey(StoredName);

impl WatchKey {
    pub fn of(stored: StoredName) -> WatchKey {
        WatchKey(stored)
    }

    /// The stored name it is: what `app::own` turns back into what the app
    /// calls it.
    pub fn stored(&self) -> &StoredName {
        &self.0
    }
}

/// The LIVE watch keys a page has bound, each with its `RenderedAt`.
#[derive(Debug, Default)]
pub struct LiveBindings {
    /// `None`: bound before the head was recovered, so the first root counts
    /// as a change.
    at: BTreeMap<WatchKey, Option<Cid>>,
}

impl LiveBindings {
    /// Bind `key` at `head`: its binding's first read is at this root or a
    /// newer one. Binding a key already bound keeps its `RenderedAt`.
    /// Bind `key`. It has rendered NOTHING yet, so until its first read
    /// completes ([`LiveBindings::rendered`]) every head counts as a change
    /// to it — never the head at bind time, which its read may not reach.
    pub fn bind(&mut self, key: WatchKey) {
        self.at.entry(key).or_insert(None);
    }

    /// The binding's read of `key` COMPLETED, answered at `root` (the pinned
    /// root it resumed at, or the head): what it shows now is that tree. Set
    /// on completion, never at report time (the architect on sdk#289): a
    /// re-read that parks and ends UNAVAILABLE, or answers at
    /// an older pinned root, leaves `RenderedAt` where the binding really is,
    /// so the change is reported again.
    pub fn rendered(&mut self, key: &WatchKey, root: Option<Cid>) {
        if let (Some(at), Some(root)) = (self.at.get_mut(key), root) {
            *at = Some(root);
        }
    }

    pub fn unbind(&mut self, key: &WatchKey) {
        self.at.remove(key);
    }

    pub fn len(&self) -> usize {
        self.at.len()
    }

    pub fn is_empty(&self) -> bool {
        self.at.is_empty()
    }

    /// Which bound keys' ranges changed between their `RenderedAt` and
    /// `head`, in key order. It only REPORTS: a changed key keeps its
    /// `RenderedAt` until its binding's re-read completes
    /// ([`LiveBindings::rendered`]), so a re-read that fails is reported
    /// again. An UNCHANGED key moves to `head` — its range is the same tree at
    /// both roots. A diff that cannot be walked (a block this page does not
    /// hold, a root with no range) counts as a change, and the re-read waits
    /// on the fetch.
    pub fn take_changed<S: Reads>(&mut self, store: &mut S, head: Option<Cid>, range_of: impl Fn(&str) -> Option<(Vec<u8>, Vec<u8>)>) -> Vec<WatchKey> {
        let Some(head) = head else { return Vec::new() };
        let mut changed = Vec::new();
        for (key, at) in self.at.iter_mut() {
            if *at == Some(head) {
                continue;
            }
            let moved = match (*at, range_of(key.stored().as_str())) {
                (Some(from), Some((lo, hi))) => match store.changes_since(from, &lo, &hi, 1) {
                    Ok(Delta::Changes { changes, .. }) => !changes.is_empty(),
                    _ => true,
                },
                _ => true,
            };
            if moved {
                changed.push(key.clone());
            } else {
                *at = Some(head);
            }
        }
        changed
    }
}
