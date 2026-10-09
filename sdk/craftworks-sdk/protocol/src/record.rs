//! A record whose unknown fields survive being rewritten.
//!
//! ARCHITECTURE §19: every published version keeps working. The hard case is
//! not reading a newer record — it is WRITING one back. An app built against
//! v1 opens a record a v2 app created, changes one field, and saves it. If
//! the v1 encoder writes out only the fields it knows, the v2 fields are
//! gone, and nothing anywhere reports an error: the v2 app simply finds its
//! data missing, some time later, with no way to tell what removed it.
//!
//! So a record is not a struct with known fields. It is an ORDERED list of
//! `(tag, bytes)`, and a field this build does not understand is carried
//! through **byte for byte, in its original position**.
//!
//! Position matters as much as content. Re-ordering fields on a rewrite
//! would make two encodings of the same record differ, and the record's
//! bytes are what it is addressed by — so a reorder is a different block,
//! and every reader holding the old id can no longer find it.

/// A field's tag. Small numbers are this build's; anything else is a
/// stranger's and is kept as it was.
pub type Tag = u32;

/// One field: its tag, and its bytes exactly as they arrived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub tag: Tag,
    pub bytes: Vec<u8>,
}

/// A record, in wire order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Record {
    fields: Vec<Field>,
}

/// Why a record could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordError {
    Truncated,
    /// A field claims more bytes than the record holds.
    BadLength,
    /// Two fields share a tag, so "the" value of it is not a question with
    /// one answer.
    DuplicateTag(Tag),
}

impl Record {
    pub fn new() -> Record {
        Record::default()
    }

    /// `[tag varint-free u32 LE][len u32 LE][bytes]`, repeated.
    ///
    /// Deliberately dull and self-describing: a reader that cannot interpret
    /// a field can still measure it, which is exactly what preserving one
    /// requires.
    pub fn decode(mut b: &[u8]) -> Result<Record, RecordError> {
        let mut fields: Vec<Field> = Vec::new();
        while !b.is_empty() {
            if b.len() < 8 {
                return Err(RecordError::Truncated);
            }
            let tag = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            let len = u32::from_le_bytes([b[4], b[5], b[6], b[7]]) as usize;
            let rest = &b[8..];
            if rest.len() < len {
                return Err(RecordError::BadLength);
            }
            if fields.iter().any(|f| f.tag == tag) {
                return Err(RecordError::DuplicateTag(tag));
            }
            fields.push(Field {
                tag,
                bytes: rest[..len].to_vec(),
            });
            b = &rest[len..];
        }
        Ok(Record { fields })
    }

    /// Write it back out.
    ///
    /// Every field, in the order it arrived — including the ones this build
    /// has never heard of.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for f in &self.fields {
            out.extend_from_slice(&f.tag.to_le_bytes());
            out.extend_from_slice(&(f.bytes.len() as u32).to_le_bytes());
            out.extend_from_slice(&f.bytes);
        }
        out
    }

    pub fn get(&self, tag: Tag) -> Option<&[u8]> {
        self.fields
            .iter()
            .find(|f| f.tag == tag)
            .map(|f| f.bytes.as_slice())
    }

    /// Set a field, KEEPING ITS POSITION if it is already there.
    ///
    /// Appending instead would move the field to the end, changing the
    /// encoding of a record whose content did not change — and a record is
    /// addressed by its bytes, so that is a different block and every reader
    /// holding the old id loses it.
    pub fn set(&mut self, tag: Tag, bytes: Vec<u8>) {
        match self.fields.iter_mut().find(|f| f.tag == tag) {
            Some(f) => f.bytes = bytes,
            None => self.fields.push(Field { tag, bytes }),
        }
    }

    /// Tags this record carries, in order.
    pub fn tags(&self) -> Vec<Tag> {
        self.fields.iter().map(|f| f.tag).collect()
    }

    /// Fields this build does not know about.
    pub fn unknown(&self, known: &[Tag]) -> Vec<Tag> {
        self.fields
            .iter()
            .map(|f| f.tag)
            .filter(|t| !known.contains(t))
            .collect()
    }
}
