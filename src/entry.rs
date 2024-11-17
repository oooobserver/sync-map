use std::{
    marker::PhantomData,
    sync::atomic::{AtomicPtr, Ordering},
};

enum EntryState<V> {
    // The pointer can't be null, because
    // `SoftDelete` represent pointer null situation
    Present(AtomicPtr<V>, PhantomData<V>),

    SoftDelete,

    // Expunged
    HardDelete,
}

/// The container of the value, controls the lifetime of the value and
/// is responsible for value deallocation.
pub struct Entry<V> {
    state: EntryState<V>,
}

impl<V> Entry<V> {
    pub fn new(val: V) -> Self {
        let boxed_val = Box::new(val);
        let ptr = Box::into_raw(boxed_val);
        Self {
            state: EntryState::Present(AtomicPtr::new(ptr), PhantomData),
        }
    }

    pub fn new_null_entry() -> Self {
        Self {
            state: EntryState::HardDelete,
        }
    }

    /// Loads a reference to the value if present.
    pub fn load(&self) -> Option<&V> {
        match &self.state {
            EntryState::Present(atomic_ptr, _) => {
                let ptr = atomic_ptr.load(Ordering::Acquire);
                unsafe { Some(&*ptr) }
            }
            EntryState::SoftDelete | EntryState::HardDelete => None,
        }
    }

    /// Swaps a value if the entry has not been expunged
    ///
    /// If the entry is expunged, trySwap returns the value and leaves the entry unchanged
    pub fn try_swap(&self, val: V) -> Option<V> {
        if let EntryState::Present(ref ptr, _) = self.state {
            let new_ptr = Box::into_raw(Box::new(val));
            loop {
                let old_ptr = ptr.load(Ordering::Acquire);

                match ptr.compare_exchange_weak(
                    old_ptr,
                    new_ptr,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(ptr) => {
                        // Convert the old pointer back to a box and return the value
                        return Some(unsafe { *Box::from_raw(ptr) });
                    }
                    // Swap failed; retry the loop with the current `old_ptr`
                    Err(_) => continue,
                }
            }
        }

        Some(val)
    }

    /// Ensures that the entry is not marked as expunged. Return if the entry was previously expunged
    //
    /// If the entry was previously expunged, it must be added to the dirty map before mu is unlocked.
    pub fn unexpunge_locked(&mut self) -> bool {
        match &self.state {
            EntryState::Present(_, _) | EntryState::SoftDelete => false,
            EntryState::HardDelete => {
                self.state = EntryState::SoftDelete;
                true
            }
        }
    }

    // Unconditionally swaps a value into the entry.
    //
    // The entry must be known not to be expunged.
    pub fn swap_locked(&mut self, val: V) -> Option<V> {
        match &self.state {
            EntryState::Present(atomic_ptr, _) => {
                let ptr = Box::into_raw(Box::new(val));
                let old = atomic_ptr.swap(ptr, Ordering::Acquire);
                Some(unsafe { *Box::from_raw(old) })
            }
            EntryState::SoftDelete => {
                let ptr = Box::into_raw(Box::new(val));
                self.state = EntryState::Present(AtomicPtr::new(ptr), PhantomData);
                None
            }
            EntryState::HardDelete => unreachable!(),
        }
    }
}

impl<V> Drop for Entry<V> {
    fn drop(&mut self) {
        dbg!("Drop entry");
        if let EntryState::Present(atomic_ptr, _) = &self.state {
            let ptr = atomic_ptr.load(Ordering::Acquire);
            unsafe { drop(Box::from_raw(ptr)) };
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn load() {
        let s = String::from("this will put on the heap");
        let e = super::Entry::new(s);
        let res = e.load();
        assert!(res.is_some());
        assert_eq!(res.unwrap(), "this will put on the heap")
    }

    #[test]
    fn try_swap() {
        let s = String::from("this will put on the heap");
        let e = super::Entry::new(s);
        let new_s = String::from("try swap");
        assert!(e.try_swap(new_s).is_some());
        assert_eq!(e.load().unwrap(), "try swap")
    }

    #[test]
    fn drop() {
        let s = String::from("this will put on the heap");
        let _ = super::Entry::new(s);
    }

    #[test]
    fn drop_box() {
        let s = String::from("this will put on the heap");
        let e = super::Entry::new(s);
        let _ = Box::new(e);
    }
}
