//! A SPACE TABLE WRITTEN with no page open (R4b): rows of this member's OWN feed of a table — what `storage.js` does
//! for a page — signed with the member's space writer through the identity's fork guard (`identity::upkeep_sign_space`).
//!
//! ONE TAIL STEP per call, its answer awaited before the next (two deltas sent at once may land out of order):
//!   - a feed already there: its rows as VERSIONS (each replacing the version current in the merge), in one step;
//!   - a feed not there yet: listed first — this writer in the space's WRITERS BAG (when it has no catalog there),
//!     its CATALOG made or written (the table at its blinded name) — then the feed made (a PUT);
//!   - a feed with 32 rows waiting: FLUSHED (its new tree blocks put first, then the step naming the root).

use craftworks_data as data;
use craftworks_identity::{self as identity, Host};

use crate::table::{self, Codes, Reading};
use crate::upkeep::{Code, Io};

/// What a write sends, in order: puts first (a bag item, tree blocks), then ONE tail step — `confirm`: the tail whose
/// answer the next call waits for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent {
    pub io: Vec<Io>,
    pub confirm: [u8; 32],
}

/// What is left to do after this step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    /// The rows are written (their step sent).
    Done(Sent),
    /// A step towards them sent (listing, a flush): call again once it is answered.
    More(Sent),
}

fn feed_open(c: &Codes, r: &Reading, owner: &[u8; 32], name: &str, label: &str, held: Option<(Option<Vec<u8>>, Vec<([u8; 32], Vec<u8>)>)>) -> data::Open {
    let space = r.space();
    let t = table::table_name(&space, name);
    let mut o = data::Open::new(c.tail, owner, label);
    o.set_table_key(identity::space_table_key(&space, &t));
    let newest = r.epochs().keys().max().copied();
    for (e, s) in r.epochs() {
        o.epoch_key(*e, Some(identity::epoch_table_key(s, &t)), Some(*e) == newest);
    }
    if let Some((state, blocks)) = held {
        if let Some(st) = state {
            o.absorb(&st);
        }
        for (cid, st) in blocks {
            o.absorb_block(&cid, &st);
        }
    }
    o
}

/// A prepared step signed by the identity and committed: what to send.
fn signed<H: Host>(h: &mut H, member: &[u8; 32], o: &mut data::Open, (seq, hash): (u64, [u8; 32])) -> Result<Io, String> {
    let sig = identity::upkeep_sign_space(h, member, &o.params, seq, &hash)?;
    Ok(match o.commit(sig).ok_or("the tail would not take its own step")? {
        data::Send::Put(state) => Io::Put { code: Code::Tail, params: o.params.clone(), state },
        data::Send::Update(delta) => Io::Update { id: o.id_bytes(), delta },
    })
}

/// The blocks a flush made, as PUTs of their Sealed contracts (at the addresses its key gives).
fn block_puts(f: &data::Flush) -> Result<Vec<Io>, String> {
    let ak = f.address_key.ok_or("a space table's tree is sealed")?;
    Ok(f.blocks.iter().map(|(cid, state)| Io::Put { code: Code::Sealed, params: data::block_address(&ak, cid).to_vec(), state: state.clone() }).collect())
}

/// WRITE `rows` (key, value; `None`: deleted) into this member's feed of table `name` (short), as read by `r`. `nonce`:
/// fresh bytes for a writers-bag item (upkeep's randomness).
pub fn rows<H: Host>(h: &mut H, member: &[u8; 32], c: &Codes, r: &mut Reading, name: &str, rows: &[(Vec<u8>, Option<Vec<u8>>)], nonce: [u8; 12]) -> Result<Next, String> {
    let me = identity::upkeep_space_writer(h, member).ok_or("this node does not hold the member's data key")?;
    let space = r.space();
    let t = table::table_name(&space, name);
    let label = identity::blind_name(&identity::space_table_key(&space, &t), &t);
    let cat_name = table::table_name(&space, "tables");
    let feed = r.own(&me, name);

    // NOT THERE YET: listed first.
    if !matches!(feed, Some((Some(_), _))) {
        let cat = r.own(&me, "tables");
        let mut cat_o = feed_open(c, r, &me, "tables", &cat_name, cat.clone());
        let listed = matches!(cat_o.opened(), Ok(data::Step::Ready(ref rs)) if craftworks_feed::merge(&[(me, rs)]).contains_key(t.as_bytes()));
        if !listed {
            let mut io = Vec::new();
            // No catalog here at all: this writer listed in the writers bag BEFORE its catalog is made.
            if !matches!(cat, Some((Some(_), _))) {
                io.push(bag_item(r, &me, nonce)?);
            }
            let current = match cat_o.opened() {
                Ok(data::Step::Ready(rs)) => craftworks_feed::merge(&[(me, &rs)]).get(t.as_bytes()).map(|row| row.id),
                _ => None,
            };
            let env = craftworks_feed::envelope((me, cat_o.writer.seq() + 1), current, Some(br#"{"b":1}"#));
            let step = cat_o.prepare_row(t.as_bytes(), &env).ok_or("the catalog would not take the row")?;
            io.push(signed(h, member, &mut cat_o, step)?);
            r.took_own(c, &me, "tables", &cat_name, cat_o.state(), cat.map(|(_, b)| b).unwrap_or_default());
            return Ok(Next::More(Sent { io, confirm: cat_o.id_bytes() }));
        }
    }

    // Its tree's blocks as fetched (sealed): what it is read from again.
    let held_blocks = feed.as_ref().map(|(_, b)| b.clone()).unwrap_or_default();
    let mut o = feed_open(c, r, &me, name, &label, feed);
    // DUE A FLUSH first: its blocks, then the step naming its root.
    if o.pending_rows() + rows.len() > data::FLUSH_AT {
        if let Ok(data::Step::Ready(f)) = o.flush() {
            let mut io = block_puts(&f)?;
            io.push(signed(h, member, &mut o, (f.seq, f.hash))?);
            let mut blocks = held_blocks;
            blocks.extend(f.blocks.iter().cloned());
            r.took_own(c, &me, name, &label, o.state(), blocks);
            return Ok(Next::More(Sent { io, confirm: o.id_bytes() }));
        }
    }
    // The rows as VERSIONS over what the merge holds now.
    let merged = r.rows(name);
    let seq = o.writer.seq() + 1;
    let versions: Vec<(Vec<u8>, Vec<u8>)> = rows
        .iter()
        .map(|(k, v)| (k.clone(), craftworks_feed::envelope((me, seq), merged.get(k).map(|row| row.id), v.as_deref())))
        .collect();
    let step = o.prepare_rows(&versions).ok_or("the feed would not take the rows")?;
    let io = signed(h, member, &mut o, step)?;
    r.took_own(c, &me, name, &label, o.state(), held_blocks);
    Ok(Next::Done(Sent { io: vec![io], confirm: o.id_bytes() }))
}

/// This writer's item in the space's writers bag: `{ w }`, sealed with epoch 0's key (`index.spacePoint`).
fn bag_item(r: &Reading, me: &[u8; 32], nonce: [u8; 12]) -> Result<Io, String> {
    use aes_gcm::aead::{Aead, KeyInit};
    let space = r.space();
    let s0 = r.epochs().get(&0).ok_or("this member does not hold the space's first epoch")?;
    let key = identity::epoch_table_key(s0, "bag-writers");
    let cipher = aes_gcm::Aes256Gcm::new_from_slice(&key).map_err(|_| "a bag key")?;
    let ct = cipher
        .encrypt(aes_gcm::Nonce::from_slice(&nonce), serde_json::json!({ "w": table::hex(me) }).to_string().as_bytes())
        .map_err(|_| "sealing the bag item")?;
    let item = [0u32.to_be_bytes().as_slice(), &nonce, &ct].concat();
    let address = table::sealed_bag_address(&space, "writers");
    let state = craftworks_bag_contract::encode(&address, &[craftworks_bag_contract::grind(&address, &item)]);
    Ok(Io::Put { code: Code::Bag, params: address.to_vec(), state })
}
