use std::{
    collections::HashMap,
    ptr,
    rc::Rc,
    sync::{
        atomic::{AtomicPtr, AtomicU64, Ordering},
        Mutex,
    },
};

use crate::entry::Entry;

// The actual inner map.
type Map<K, V> = HashMap<K, Rc<Entry<V>>>;

// ReadOnly means the read map.
struct ReadOnly<K, V>
where
    K: std::cmp::Eq + std::hash::Hash,
{
    m: Map<K, V>,
    amended: bool,
}

impl<K, V> Default for ReadOnly<K, V>
where
    K: std::cmp::Eq + std::hash::Hash + Clone,
{
    fn default() -> Self {
        ReadOnly::new()
    }
}

impl<K, V> ReadOnly<K, V>
where
    K: std::cmp::Eq + std::hash::Hash + Clone,
{
    fn new() -> Self {
        ReadOnly {
            m: HashMap::new(),
            amended: false,
        }
    }
}

pub struct SyncMap<K, V>
where
    K: std::cmp::Eq + std::hash::Hash,
{
    // Held when store the read map.
    mu: Mutex<()>,

    // read contains the portion of the map's contents that are safe for
    // concurrent access (with or without mu held).
    //
    // The read field itself is always safe to load, but must only be stored with
    // mu held.
    //
    // Entries stored in read may be updated concurrently without mu, but updating
    // a previously-expunged entry requires that the entry be copied to the dirty
    // map and unexpunged with mu held.
    read: AtomicPtr<ReadOnly<K, V>>,

    // dirty contains the portion of the map's contents that require mutex to be
    // held. To ensure that the dirty map can be promoted to the read map quickly,
    // it also includes all of the non-expunged entries in the read map.
    //
    // Expunged entries are not stored in the dirty map. An expunged entry in the
    // clean map must be unexpunged and added to the dirty map before a new value
    // can be stored to it.
    //
    // If the dirty map is nil, the next write to the map will initialize it by
    // making a shallow copy of the clean map, omitting stale entries.
    dirty: Mutex<Option<Map<K, V>>>,

    misses: AtomicU64,
}

impl<K, V> Default for SyncMap<K, V>
where
    K: std::cmp::Eq + std::hash::Hash + Clone,
{
    fn default() -> Self {
        SyncMap::new()
    }
}

impl<K, V> SyncMap<K, V>
where
    K: std::cmp::Eq + std::hash::Hash + Clone,
{
    pub fn new() -> SyncMap<K, V> {
        SyncMap {
            mu: Mutex::new(()),
            read: AtomicPtr::new(ptr::null_mut()),
            dirty: Mutex::new(Some(HashMap::new())),
            misses: AtomicU64::new(0),
        }
    }

    #[inline]
    fn load_readonly(&self) -> Option<&ReadOnly<K, V>> {
        let read_map = self.read.load(Ordering::Acquire);
        if read_map.is_null() {
            return None;
        }

        unsafe { Some(&*read_map) }
    }

    // TODO: reduce the logic.
    // The whole serach logic is like this:
    // First check the key in the read map, this don't need the lock.
    // Then try to find it in the dirty map, note this need the lock
    pub fn load(&self, key: &K) -> Option<&V> {
        let read_only = self.load_readonly();

        if let Some(read) = read_only {
            let present = read.m.contains_key(key);
            // Maybe the KV is in the dirty map.
            if !present && read.amended {
                let _lock = self.mu.lock().unwrap();
                return self.load_dirty_locked(key);
            }

            // Never insert this key before.
            if !present {
                return None;
            }

            // Find in the read map.
            read.m.get(key).as_ref().unwrap().load();
        }

        None
    }

    #[inline(always)]
    fn load_dirty_locked(&self, key: &K) -> Option<&V> {
        let read_only = self.load_readonly();
        if let Some(read) = read_only {
            let present = read.m.contains_key(key);
            if !present && read.amended {}
        }

        None
    }

    // If misses hit the threshold, flip
    fn miss_locked(&self) {
        let num = self.misses.fetch_add(1, Ordering::Release) as usize;
        if num + 1 < self.dirty.as_ref().unwrap().len() {
            return;
        }

        let new = Box::into_raw(Box::new(ReadOnly {
            amended: false,
            m: self.dirty.take().unwrap(),
        }));
        let old = self.read.swap(new, Ordering::Release);

        // m.read.Store(&readOnly{m: m.dirty})
        // m.dirty = nil
        // m.misses = 0
        self.dirty = None;
        self.misses.store(0, Ordering::Release);
    }
}

impl<K, V> Drop for SyncMap<K, V>
where
    K: std::cmp::Eq + std::hash::Hash,
{
    fn drop(&mut self) {
        let read_ptr = self.read.load(Ordering::Acquire);
        if !read_ptr.is_null() {
            unsafe {
                let _ = Box::from_raw(read_ptr);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load() {}

    #[test]
    fn drop() {
        let s = String::from("this will put on the heap");
        let e = super::Entry::new(s);

        let mut map = HashMap::new();
        map.insert(1, Rc::new(e));
    }
}
