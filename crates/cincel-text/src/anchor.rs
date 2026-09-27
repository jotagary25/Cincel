//! Anchors: byte positions that survive edits.
//!
//! # Design
//!
//! [`Anchor`] is a plain value (`{ offset, bias }`), exactly as the module spec
//! describes it. A value cannot update itself when the buffer changes, so
//! surviving edits is the job of a *container*: [`AnchorMap<T>`] (and its alias
//! [`AnchorSet`]). Both the buffer and any consumer crate (editor, review) can
//! own one.
//!
//! The container keeps its anchors in a slot arena plus an index sorted by
//! offset. Transforming it for one edit is a binary search for the first anchor
//! at or after the edit start plus a linear pass over the anchors from there:
//! `O(log n + k)`, within the `O(k log n)` per edit the spec asks for. The
//! transform is monotone non-decreasing, so the sorted index stays sorted and
//! never has to be re-sorted.
//!
//! A container owned outside the buffer is fed with
//! [`AnchorMap::apply_event`], which derives the same transforms from a
//! [`BufferEvent::Edited`](crate::BufferEvent::Edited).

use std::ops::Range;

use crate::BufferEvent;

/// Which side of an edit at the exact anchor offset the anchor sticks to.
///
/// `Before` keeps the anchor to the left of text inserted at its offset,
/// `After` pushes it to the right.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Bias {
    /// Stick to the text on the left.
    #[default]
    Before,
    /// Stick to the text on the right.
    After,
}

/// A byte position with a bias.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Anchor {
    /// Byte offset in the buffer.
    pub offset: usize,
    /// Side the anchor sticks to when text is inserted at `offset`.
    pub bias: Bias,
}

impl Anchor {
    /// Builds an anchor.
    pub fn new(offset: usize, bias: Bias) -> Self {
        Self { offset, bias }
    }

    /// Builds a `Before`-biased anchor.
    pub fn before(offset: usize) -> Self {
        Self::new(offset, Bias::Before)
    }

    /// Builds an `After`-biased anchor.
    pub fn after(offset: usize) -> Self {
        Self::new(offset, Bias::After)
    }

    /// Transforms this anchor across one edit that replaced `range` with
    /// `new_len` bytes.
    ///
    /// - before the edit: unchanged,
    /// - after the edit: shifted by the length delta,
    /// - inside (or at either end of) the edit: collapsed to the left edge for
    ///   `Before` and to the right edge of the inserted text for `After`.
    pub fn transform(&mut self, range: &Range<usize>, new_len: usize) {
        self.offset = transform_offset(self.offset, self.bias, range, new_len);
    }

    /// [`Anchor::transform`] by value.
    #[must_use]
    pub fn transformed(mut self, range: &Range<usize>, new_len: usize) -> Self {
        self.transform(range, new_len);
        self
    }
}

/// The transform rule, shared by [`Anchor`] and [`AnchorMap`].
pub(crate) fn transform_offset(
    offset: usize,
    bias: Bias,
    range: &Range<usize>,
    new_len: usize,
) -> usize {
    let start = range.start;
    let end = range.end.max(start);
    if offset < start {
        offset
    } else if offset > end {
        offset + new_len - (end - start)
    } else {
        match bias {
            Bias::Before => start,
            Bias::After => start + new_len,
        }
    }
}

/// A stable handle into an [`AnchorMap`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AnchorId(u32);

impl AnchorId {
    /// The raw slot index. Only meaningful inside the map that produced it.
    pub fn as_u32(self) -> u32 {
        self.0
    }
}

/// A set of anchors, each carrying a value, transformed as a whole on every
/// edit. See the [module docs](self) for the complexity argument.
#[derive(Clone, Debug)]
pub struct AnchorMap<T> {
    /// Slot arena. The index of a slot is its [`AnchorId`].
    slots: Vec<Option<(Anchor, T)>>,
    /// Free slot indices, reused by `insert`.
    free: Vec<u32>,
    /// Slot indices sorted by anchor offset (ties broken by bias then slot).
    order: Vec<u32>,
}

/// An [`AnchorMap`] without payload.
pub type AnchorSet = AnchorMap<()>;

impl<T> Default for AnchorMap<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> AnchorMap<T> {
    /// An empty map.
    pub fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
            order: Vec::new(),
        }
    }

    /// Number of live anchors.
    pub fn len(&self) -> usize {
        self.order.len()
    }

    /// Whether the map holds no anchors.
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// Drops every anchor.
    pub fn clear(&mut self) {
        self.slots.clear();
        self.free.clear();
        self.order.clear();
    }

    /// Registers `anchor` with `value` and returns its stable handle.
    pub fn insert(&mut self, anchor: Anchor, value: T) -> AnchorId {
        let slot = match self.free.pop() {
            Some(slot) => {
                self.slots[slot as usize] = Some((anchor, value));
                slot
            }
            None => {
                self.slots.push(Some((anchor, value)));
                (self.slots.len() - 1) as u32
            }
        };
        let key = (anchor.offset, anchor.bias, slot);
        let at = self.order.partition_point(|&s| self.sort_key(s) < key);
        self.order.insert(at, slot);
        AnchorId(slot)
    }

    /// Removes an anchor, returning what it held.
    pub fn remove(&mut self, id: AnchorId) -> Option<(Anchor, T)> {
        let slot = id.0;
        let entry = self.slots.get_mut(slot as usize)?.take()?;
        if let Some(at) = self.order.iter().position(|&s| s == slot) {
            self.order.remove(at);
        }
        self.free.push(slot);
        Some(entry)
    }

    /// The current anchor behind `id`.
    pub fn anchor(&self, id: AnchorId) -> Option<Anchor> {
        self.slots
            .get(id.0 as usize)
            .and_then(|slot| slot.as_ref())
            .map(|(anchor, _)| *anchor)
    }

    /// The current offset of `id`.
    pub fn resolve(&self, id: AnchorId) -> Option<usize> {
        self.anchor(id).map(|anchor| anchor.offset)
    }

    /// The value stored with `id`.
    pub fn value(&self, id: AnchorId) -> Option<&T> {
        self.slots
            .get(id.0 as usize)
            .and_then(|slot| slot.as_ref())
            .map(|(_, value)| value)
    }

    /// The value stored with `id`, mutably.
    pub fn value_mut(&mut self, id: AnchorId) -> Option<&mut T> {
        self.slots
            .get_mut(id.0 as usize)
            .and_then(|slot| slot.as_mut())
            .map(|(_, value)| value)
    }

    /// Every live anchor, in increasing offset order.
    pub fn iter(&self) -> impl Iterator<Item = (AnchorId, Anchor, &T)> {
        self.order.iter().map(move |&slot| {
            let (anchor, value) = self.slots[slot as usize]
                .as_ref()
                .expect("ordered slot is live");
            (AnchorId(slot), *anchor, value)
        })
    }

    /// Transforms every anchor across one edit that replaced `range` with
    /// `new_len` bytes.
    pub fn apply_edit(&mut self, range: Range<usize>, new_len: usize) {
        if self.order.is_empty() {
            return;
        }
        // Anchors strictly before the edit never move: skip them with a binary
        // search over the offset-ordered index. `collapsed` is the end of the
        // group that falls inside the edit; everything past it only shifts.
        let first = self
            .order
            .partition_point(|&slot| self.offset_of(slot) < range.start);
        let collapsed = self
            .order
            .partition_point(|&slot| self.offset_of(slot) <= range.end.max(range.start));
        for i in first..self.order.len() {
            let slot = self.order[i] as usize;
            if let Some((anchor, _)) = self.slots[slot].as_mut() {
                anchor.transform(&range, new_len);
            }
        }
        // Anchors that fell inside the edit collapse to either edge depending
        // on their bias, which is the one case where the index can go out of
        // order. Everything else shifts uniformly and stays sorted.
        if collapsed > first + 1 {
            let keys: Vec<(usize, Bias, u32)> = self.order[first..collapsed]
                .iter()
                .map(|&slot| self.sort_key(slot))
                .collect();
            let mut pairs: Vec<((usize, Bias, u32), u32)> = keys
                .into_iter()
                .zip(self.order[first..collapsed].iter().copied())
                .collect();
            pairs.sort_by_key(|(key, _)| *key);
            for (dst, (_, slot)) in self.order[first..collapsed].iter_mut().zip(pairs) {
                *dst = slot;
            }
        }
    }

    /// Transforms every anchor across a whole [`BufferEvent`].
    ///
    /// The event carries the replaced ranges in pre-edit coordinates and the
    /// resulting ranges in post-edit coordinates; applying them in ascending
    /// order lets each step work in the coordinate space the previous steps
    /// already produced.
    pub fn apply_event(&mut self, event: &BufferEvent) {
        let BufferEvent::Edited {
            old_ranges,
            new_ranges,
            ..
        } = event;
        for (old, new) in old_ranges.iter().zip(new_ranges.iter()) {
            let old_len = old.end.saturating_sub(old.start);
            self.apply_edit(new.start..new.start + old_len, new.end - new.start);
        }
    }

    fn offset_of(&self, slot: u32) -> usize {
        self.slots[slot as usize]
            .as_ref()
            .expect("ordered slot is live")
            .0
            .offset
    }

    fn sort_key(&self, slot: u32) -> (usize, Bias, u32) {
        let anchor = self.slots[slot as usize]
            .as_ref()
            .expect("ordered slot is live")
            .0;
        (anchor.offset, anchor.bias, slot)
    }
}

impl AnchorMap<()> {
    /// Convenience insert for an [`AnchorSet`].
    pub fn add(&mut self, anchor: Anchor) -> AnchorId {
        self.insert(anchor, ())
    }
}
