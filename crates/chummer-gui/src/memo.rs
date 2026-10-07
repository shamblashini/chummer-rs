//! Values worked out once per revision of a character.
//!
//! A page asks the engine the same questions every frame (upgrade costs,
//! weapon stats, ammunition); some of those compute a whole sheet. A
//! [`Memo`] keeps the answers until the character's revision
//! (`Doc::revision`) changes.

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::Hash;

pub struct Memo<K, V> {
    inner: RefCell<(Option<u64>, HashMap<K, V>)>,
}

impl<K, V> Default for Memo<K, V> {
    fn default() -> Self {
        Memo { inner: RefCell::new((None, HashMap::new())) }
    }
}

impl<K: Eq + Hash, V: Clone> Memo<K, V> {
    /// The value for `key` at revision `rev`, from `f` the first time.
    pub fn get(&self, rev: u64, key: K, f: impl FnOnce() -> V) -> V {
        {
            let m = self.inner.borrow();
            if m.0 == Some(rev) {
                if let Some(v) = m.1.get(&key) {
                    return v.clone();
                }
            }
        }
        let v = f();
        let mut m = self.inner.borrow_mut();
        if m.0 != Some(rev) {
            m.1.clear();
            m.0 = Some(rev);
        }
        m.1.insert(key, v.clone());
        v
    }

    /// Forget everything (the document was replaced).
    pub fn clear(&self) {
        let mut m = self.inner.borrow_mut();
        m.0 = None;
        m.1.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_values_until_the_revision_changes() {
        let m: Memo<&str, i32> = Memo::default();
        let mut calls = 0;
        let mut ask = |rev| {
            m.get(rev, "a", || {
                calls += 1;
                calls
            })
        };
        assert_eq!(ask(1), 1);
        assert_eq!(ask(1), 1);
        assert_eq!(ask(2), 2);
        m.clear();
        assert_eq!(m.get(2, "a", || 9), 9);
    }
}
