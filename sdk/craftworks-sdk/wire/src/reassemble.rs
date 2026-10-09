//! Putting a chunked reply back together, with the bounds a stranger's input
//! needs.
//!
//! # Why this exists rather than stdlib's buffer alone
//!
//! stdlib's `ReassemblyBuffer` already bounds what matters most — it refuses a
//! `total` over its cap BEFORE allocating the slot vector, which is the
//! allocation a hostile sender would otherwise choose the size of. This wraps
//! it rather than replacing it, so that bound is theirs and stays theirs.
//!
//! What it adds is the thing that is missing **on wasm specifically**:
//! `ReassemblyBuffer::evict_stale` is `#[cfg(not(target_family = "wasm"))]`.
//! In a browser nothing ages a half-finished stream out, and the buffer holds
//! at most `MAX_CONCURRENT_STREAMS` (8) of them — so eight abandoned streams
//! wedge reassembly permanently, and the symptom is replies that simply stop
//! arriving. A node that started eight messages and finished none would do it
//! on purpose; a flaky connection would do it by accident.
//!
//! So this counts streams itself and drops the oldest when it must, which
//! turns a permanent wedge into a lost message — and a lost message is
//! something the layers above already handle, because delivery was never
//! guaranteed.

use crate::Unusable;
use freenet_stdlib::client_api::streaming::MAX_CONCURRENT_STREAMS;
use freenet_stdlib::client_api::HostResponse;

/// Reassembles chunked replies, bounded.
pub struct Reassembler {
    /// `(stream_id, total, arrived_count, parts)`, oldest first.
    streams: Vec<Stream>,
    /// Bytes held across all in-flight streams.
    bytes: usize,
    /// Beyond this, the oldest unfinished stream is dropped.
    ///
    /// A SECOND bound, in bytes, because a count is not a byte budget: eight
    /// streams of 64 MiB each is inside stdlib's chunk cap and nowhere near
    /// anything a browser tab should hold.
    pub max_bytes: usize,
}

struct Stream {
    id: u32,
    total: u32,
    parts: Vec<Option<Vec<u8>>>,
    bytes: usize,
}

impl Default for Reassembler {
    fn default() -> Self {
        Reassembler::new()
    }
}

impl Reassembler {
    pub fn new() -> Reassembler {
        Reassembler {
            streams: Vec::new(),
            bytes: 0,
            // ONE whole message may be held (the one bound): a lone large answer is never evicted mid-stream.
            max_bytes: crate::MAX_REASSEMBLED,
        }
    }

    pub fn in_flight(&self) -> usize {
        self.streams.len()
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Take a chunk. `Ok(Some(bytes))` when the message is complete.
    ///
    /// Every bound is checked before anything is allocated for it, because
    /// `total` and `index` are the sender's numbers.
    pub fn chunk(
        &mut self,
        id: u32,
        index: u32,
        total: u32,
        data: Vec<u8>,
    ) -> Result<Option<Vec<u8>>, Unusable> {
        use freenet_stdlib::client_api::streaming::MAX_TOTAL_CHUNKS;
        // stdlib's own bounds, applied here because this is the door.
        if total == 0 || total > MAX_TOTAL_CHUNKS || index >= total {
            return Err(Unusable::BadStream);
        }
        // A chunk larger than the sender's OWN chunk size is not a chunk.
        // Bounding it by `MAX_FRAME` instead — sixteen times larger — is what
        // let one stream hold about a gigabyte.
        if data.len() > crate::MAX_CHUNK {
            return Err(Unusable::BadStream);
        }
        // Refuse on the DECLARATION, before any of it arrives. `total` is the
        // sender's number and it says how big the message will be.
        if (total as usize).saturating_mul(crate::MAX_CHUNK) > crate::MAX_REASSEMBLED {
            return Err(Unusable::BadStream);
        }

        let at = match self.streams.iter().position(|s| s.id == id) {
            Some(i) => i,
            None => {
                // A NEW stream. Make room rather than refusing: a browser has
                // no stale eviction underneath, so refusing here is how eight
                // abandoned streams become a permanent wedge.
                while self.streams.len() >= MAX_CONCURRENT_STREAMS {
                    self.drop_oldest();
                }
                self.streams.push(Stream {
                    id,
                    total,
                    // Allocated only AFTER `total` has been bounded.
                    parts: vec![None; total as usize],
                    bytes: 0,
                });
                self.streams.len() - 1
            }
        };

        // A `total` that disagrees with the one this stream started with is
        // not a chunk of it. Believing the second one would let a sender
        // re-shape a stream in flight.
        if self.streams[at].total != total {
            return Err(Unusable::BadStream);
        }

        let n = data.len();
        // A DUPLICATE chunk replaces rather than accumulates, so a sender
        // cannot grow a stream past its own declared size by re-sending.
        if let Some(old) = self.streams[at].parts[index as usize].replace(data) {
            self.streams[at].bytes -= old.len();
            self.bytes -= old.len();
        }
        self.streams[at].bytes += n;
        self.bytes += n;

        // Evicting SHIFTS the vector, so `at` is stale the moment this runs.
        // The first version kept using it and indexed past the end — a panic,
        // in the one crate whose claim is that a stranger's bytes can never
        // cause one. Found by the byte-cap test, which is the only case where
        // an eviction and an insert happen in the same call.
        // A stream that has grown past what this client will ever reassemble
        // is dropped AT THIS CHUNK rather than when it completes — the point
        // of a bound is that the bytes are never held.
        if self.streams[at].bytes > crate::MAX_REASSEMBLED {
            let s = self.streams.remove(at);
            self.bytes -= s.bytes;
            return Err(Unusable::BadStream);
        }

        // And the whole buffer is bounded, INCLUDING when there is only one
        // stream in it. The first version said `streams.len() > 1`, so a lone
        // sender met no bound at all — and the test that was supposed to
        // cover this used four streams, which is exactly why it passed.
        while self.bytes > self.max_bytes && !self.streams.is_empty() {
            self.drop_oldest();
        }

        // Re-find by ID. The stream may itself have been the one evicted —
        // which is not an error: its bytes are gone and the message will not
        // complete, and that is what a bound MEANS.
        let Some(at) = self.streams.iter().position(|s| s.id == id) else {
            return Ok(None);
        };
        if self.streams[at].parts.iter().any(|p| p.is_none()) {
            return Ok(None);
        }
        let s = self.streams.remove(at);
        self.bytes -= s.bytes;
        Ok(Some(s.parts.into_iter().flatten().flatten().collect()))
    }

    /// Decode a complete message. Never panics on anything.
    pub fn decode(bytes: &[u8]) -> Result<HostResponse, Unusable> {
        // A WHOLE message's bound, reassembled or not (a single frame met MAX_FRAME at its door already).
        if bytes.len() > crate::MAX_REASSEMBLED {
            return Err(Unusable::TooLarge);
        }
        let decoded: Result<Result<HostResponse, freenet_stdlib::client_api::ClientError>, _> =
            bincode::deserialize(bytes);
        match decoded {
            Ok(Ok(r)) => Ok(r),
            // The node said no. That is a message, not a failure to read one —
            // the caller turns it into `Refused`.
            Ok(Err(_)) => Err(Unusable::NodeSaidNo),
            Err(_) => Err(Unusable::Unparseable),
        }
    }

    fn drop_oldest(&mut self) {
        if self.streams.is_empty() {
            return;
        }
        let s = self.streams.remove(0);
        self.bytes -= s.bytes;
    }

    /// Everything in flight goes. Called on reconnect: stream ids restart at
    /// zero on a new connection (stdlib's `next_stream_id` lives on the
    /// client), so a half-finished stream from the old one would be
    /// reassembled with chunks from a different message under the same id.
    pub fn reset(&mut self) {
        self.streams.clear();
        self.bytes = 0;
    }
}
