//! In-process identity of groups, motion tracks, prepared backdrops and atlases: small copyable index-plus-generation handles.
//!
//! A handle is an index into a table and the generation the slot had when the object was made. Freeing a slot raises its
//! generation, so a handle to a dropped object never names a later one that took its slot (the table answers `None`), and the
//! pair is never reused. The compositor never sees these: wire ids come from counters in `Scene` that only rise.

/// Makes a handle type: `index` into its table and the `generation` of the slot then.
macro_rules! handle {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name {
            index: u16,
            generation: u32,
        }

        impl $name {
            pub(crate) const fn new(index: u16, generation: u32) -> Self {
                Self { index, generation }
            }

            /// The key that finds this object in its table.
            pub(crate) const fn key(self) -> (u16, u32) {
                (self.index, self.generation)
            }

            pub(crate) const fn from_key(key: (u16, u32)) -> Self {
                Self::new(key.0, key.1)
            }

            /// The slot this handle indexes (small: below the ceiling of its kind).
            pub const fn index(self) -> usize {
                self.index as usize
            }

            /// How many objects have had this slot before this one.
            pub const fn generation(self) -> u32 {
                self.generation
            }
        }
    };
}

pub(crate) use handle;

struct Slot<T> {
    /// Starts at 1, so a handle made from zeros never matches.
    generation: u32,
    value: Option<T>,
}

/// A fixed-ceiling table of objects addressed by (index, generation).
pub(crate) struct Slots<T> {
    slots: Vec<Slot<T>>,
    free: Vec<u16>,
    live: usize,
    limit: usize,
}

impl<T> Slots<T> {
    /// A table for at most `limit` live objects; it never allocates after this.
    pub fn with_limit(limit: usize) -> Self {
        Self {
            slots: Vec::with_capacity(limit),
            free: Vec::with_capacity(limit),
            live: 0,
            limit,
        }
    }

    pub fn live(&self) -> usize {
        self.live
    }

    pub fn is_full(&self) -> bool {
        self.live >= self.limit
    }

    /// Stores `value`; `(index, generation)` of its slot, or `None` when the table holds its ceiling already.
    pub fn insert(&mut self, value: T) -> Option<(u16, u32)> {
        if self.is_full() {
            return None;
        }
        self.live += 1;
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[usize::from(index)];
            slot.value = Some(value);
            return Some((index, slot.generation));
        }
        let index = u16::try_from(self.slots.len()).expect("a table of at most 65536 slots");
        self.slots.push(Slot {
            generation: 1,
            value: Some(value),
        });
        Some((index, 1))
    }

    pub fn get(&self, (index, generation): (u16, u32)) -> Option<&T> {
        let slot = self.slots.get(usize::from(index))?;
        if slot.generation == generation {
            slot.value.as_ref()
        } else {
            None
        }
    }

    pub fn get_mut(&mut self, (index, generation): (u16, u32)) -> Option<&mut T> {
        let slot = self.slots.get_mut(usize::from(index))?;
        if slot.generation == generation {
            slot.value.as_mut()
        } else {
            None
        }
    }

    /// Takes the object out and raises the slot's generation.
    pub fn remove(&mut self, (index, generation): (u16, u32)) -> Option<T> {
        let slot = self.slots.get_mut(usize::from(index))?;
        if slot.generation != generation || slot.value.is_none() {
            return None;
        }
        let value = slot.value.take();
        slot.generation = slot.generation.wrapping_add(1).max(1);
        self.free.push(index);
        self.live -= 1;
        value
    }

    /// Every live object with the (index, generation) that names it.
    pub fn iter(&self) -> impl Iterator<Item = (u16, u32, &T)> {
        self.slots.iter().enumerate().filter_map(|(i, s)| {
            s.value
                .as_ref()
                .map(|v| (u16::try_from(i).expect("small table"), s.generation, v))
        })
    }

    /// The same, mutably.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (u16, u32, &mut T)> {
        self.slots.iter_mut().enumerate().filter_map(|(i, s)| {
            let generation = s.generation;
            s.value
                .as_mut()
                .map(|v| (u16::try_from(i).expect("small table"), generation, v))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    handle!(
        /// A test handle.
        Probe
    );

    #[test]
    fn a_dropped_objects_handle_never_names_the_one_that_takes_its_slot() {
        let mut table = Slots::with_limit(2);
        let (i, g) = table.insert("first").unwrap();
        let first = Probe::from_key((i, g));
        assert_eq!((first.index(), first.generation()), (usize::from(i), g));
        assert_eq!(table.remove((i, g)), Some("first"));
        let (j, h) = table.insert("second").unwrap();
        assert_eq!(j, i, "the slot is reused");
        assert_ne!(h, g, "under a new generation");
        assert_eq!(table.get(first.key()), None);
        assert_eq!(table.get((j, h)), Some(&"second"));
        assert_eq!(table.remove(first.key()), None);
        assert_eq!(table.live(), 1);
    }

    #[test]
    fn a_table_is_full_at_its_ceiling_and_has_room_again_after_a_removal() {
        let mut table = Slots::with_limit(2);
        let a = table.insert(1).unwrap();
        table.insert(2).unwrap();
        assert!(table.insert(3).is_none());
        table.remove(a).unwrap();
        assert!(table.insert(3).is_some());
        assert_eq!(table.iter().map(|(_, _, v)| *v).sum::<i32>(), 5);
    }
}
