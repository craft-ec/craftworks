//! Record ids: 16 bytes — 8 ms timestamp ‖ 4 device ‖ 4 tail — big-endian, so
//! byte order is time order. Ids from one generator strictly increase, even
//! within one millisecond or if the clock steps back.

pub type RKey = [u8; 16];

/// Time and randomness, injected so tests are deterministic.
pub trait Env {
    fn now_ms(&mut self) -> u64;
    fn rand32(&mut self) -> u32;
}

pub struct SystemEnv;

impl Env for SystemEnv {
    #[cfg(not(target_arch = "wasm32"))]
    fn now_ms(&mut self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
    #[cfg(target_arch = "wasm32")]
    fn now_ms(&mut self) -> u64 {
        js_sys::Date::now() as u64
    }
    fn rand32(&mut self) -> u32 {
        let mut b = [0u8; 4];
        // A failed RNG only weakens uniqueness across devices, which the device
        // field already separates; never fatal.
        let _ = getrandom::getrandom(&mut b);
        u32::from_le_bytes(b)
    }
}

pub struct IdGen {
    device: [u8; 4],
    last_ms: u64,
    last_tail: u32,
}

impl IdGen {
    pub fn new(device: [u8; 4]) -> Self {
        IdGen {
            device,
            last_ms: 0,
            last_tail: 0,
        }
    }

    pub fn next(&mut self, env: &mut dyn Env) -> RKey {
        let now = env.now_ms();
        if now > self.last_ms {
            self.last_ms = now;
            // Leave headroom so same-millisecond ids can count up without wrapping.
            self.last_tail = env.rand32() >> 1;
        } else if self.last_tail == u32::MAX {
            self.last_ms += 1;
            self.last_tail = 0;
        } else {
            self.last_tail += 1;
        }
        let mut id = [0u8; 16];
        id[..8].copy_from_slice(&self.last_ms.to_be_bytes());
        id[8..12].copy_from_slice(&self.device);
        id[12..].copy_from_slice(&self.last_tail.to_be_bytes());
        id
    }
}

/// The slot a record derived from a source takes: a deterministic rkey.
///
/// `created_ms (8 bytes, big-endian) ‖ BLAKE3(namespace, source_id)[..8]`.
/// The same source always gives the same slot — on any run, device or
/// platform (the vector is pinned in `tests/create_at.rs`) — and the time
/// prefix means derived slots sort among minted ids by creation time, exactly
/// as minted ones do (craftworks-sdk#149).
///
/// **`created_ms` is the source record's `created` FIELD**, never a time read
/// back out of its id: in LocalDb the two are two clock reads and disagree
/// about once in 5,000 records, and two paths that derived the slot from the
/// two different times would copy that record twice.
///
/// The namespace is length-prefixed in the hash, so no split of one string
/// between `namespace` and `source_id` can collide with another split.
pub fn slot_from(created_ms: u64, namespace: &str, source_id: &str) -> RKey {
    let mut h = blake3::Hasher::new();
    h.update(&(namespace.len() as u64).to_be_bytes());
    h.update(namespace.as_bytes());
    h.update(source_id.as_bytes());
    let mut slot = [0u8; 16];
    slot[..8].copy_from_slice(&created_ms.to_be_bytes());
    slot[8..].copy_from_slice(&h.finalize().as_bytes()[..8]);
    slot
}

pub fn created_ms(id: &RKey) -> u64 {
    u64::from_be_bytes(id[..8].try_into().unwrap())
}

pub fn to_hex(id: &RKey) -> String {
    crate::hex(id)
}

pub fn from_hex(s: &str) -> Option<RKey> {
    core_types::hex::decode_array(s)
}

/// WHERE a record is, which is not the same as WHAT it is.
///
/// A record's own id is its `rkey` and always has been. In a domain that
/// declares a parent, the key is `<parent>‖<rkey>`, so addressing the record
/// needs both — the rkey alone names it but cannot find it.
///
/// Kept as a separate type rather than widening `RKey` because the rkey is
/// load-bearing on its own: it is time-ordered, and `created_ms` reads the
/// timestamp straight out of it (see craftworks-sdk#117 on why that ordering
/// matters). A parent prepended to it would break both.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Loc {
    pub parent: Option<RKey>,
    pub rkey: RKey,
}

impl Loc {
    pub fn bare(rkey: RKey) -> Self {
        Loc { parent: None, rkey }
    }
    pub fn under(parent: RKey, rkey: RKey) -> Self {
        Loc { parent: Some(parent), rkey }
    }
}

/// A bare rkey addresses a record in a domain with no parent. Present so the
/// common call reads as it always did.
impl From<RKey> for Loc {
    fn from(rkey: RKey) -> Self {
        Loc::bare(rkey)
    }
}

impl From<&RKey> for Loc {
    fn from(rkey: &RKey) -> Self {
        Loc::bare(*rkey)
    }
}

impl From<&Loc> for Loc {
    fn from(l: &Loc) -> Self {
        *l
    }
}

/// The id an app holds: 32 hex for a bare record, 64 for one under a parent.
///
/// SELF-DESCRIBING BY LENGTH, so an app never has to be told which kind it
/// has and never parses one. That is what keeps `Session::preload`'s rule
/// intact — the app names a parent, and the SDK still owns what a range IS.
pub fn loc_to_hex(loc: &Loc) -> String {
    match loc.parent {
        Some(p) => format!("{}{}", crate::hex(&p), crate::hex(&loc.rkey)),
        None => crate::hex(&loc.rkey),
    }
}

pub fn loc_from_hex(s: &str) -> Option<Loc> {
    match s.len() {
        32 => Some(Loc::bare(from_hex(s)?)),
        64 => Some(Loc::under(from_hex(&s[..32])?, from_hex(&s[32..])?)),
        _ => None,
    }
}
