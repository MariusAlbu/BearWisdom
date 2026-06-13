// =============================================================================
// indexer/resolve/engine/symbol_set.rs — SymbolSet return type
//
// The value returned by every multi-symbol `SymbolLookup` method. It spans two
// backing stores: the eager-internal symbol maps (built once, contiguous) and
// the lazily-materialized external store (refs into a stable append-only
// arena). When a lookup has no materialized hit — the common internal-only
// case — it borrows the eager slice directly with zero allocation. Only when a
// materialized symbol also matches does it collect `&SymbolInfo` across stores.
// =============================================================================

use super::SymbolInfo;

/// A set of symbols answering one lookup, spanning the eager-internal and
/// materialized-external stores.
///
/// `Borrowed` is the zero-cost common path: one contiguous slice from an eager
/// map, no materialized match. `Owned` collects references across both stores
/// and is allocated only when a materialized symbol participates.
pub enum SymbolSet<'a> {
    /// A single contiguous eager slice — no materialized hits.
    Borrowed(&'a [SymbolInfo]),
    /// References gathered across the eager and materialized stores.
    Owned(Vec<&'a SymbolInfo>),
}

impl<'a> SymbolSet<'a> {
    /// The empty set.
    pub const fn empty() -> Self {
        SymbolSet::Borrowed(&[])
    }

    pub fn is_empty(&self) -> bool {
        match self {
            SymbolSet::Borrowed(s) => s.is_empty(),
            SymbolSet::Owned(v) => v.is_empty(),
        }
    }

    pub fn len(&self) -> usize {
        match self {
            SymbolSet::Borrowed(s) => s.len(),
            SymbolSet::Owned(v) => v.len(),
        }
    }

    /// First symbol. The returned reference carries the *store* lifetime, not
    /// the borrow of `self`, so it stays valid after a temporary `SymbolSet`
    /// (e.g. `lookup.by_name(x).first()`) is dropped.
    pub fn first(&self) -> Option<&'a SymbolInfo> {
        match self {
            SymbolSet::Borrowed(s) => s.first(),
            SymbolSet::Owned(v) => v.first().copied(),
        }
    }

    pub fn last(&self) -> Option<&'a SymbolInfo> {
        match self {
            SymbolSet::Borrowed(s) => s.last(),
            SymbolSet::Owned(v) => v.last().copied(),
        }
    }

    pub fn get(&self, i: usize) -> Option<&'a SymbolInfo> {
        match self {
            SymbolSet::Borrowed(s) => s.get(i),
            SymbolSet::Owned(v) => v.get(i).copied(),
        }
    }

    /// Iterate the symbols, borrowing from whichever store backs each.
    pub fn iter(&self) -> SymbolSetIter<'_> {
        match self {
            SymbolSet::Borrowed(s) => SymbolSetIter::Slice(s.iter()),
            SymbolSet::Owned(v) => SymbolSetIter::Refs(v.iter()),
        }
    }

    /// Clone the symbols out into an owned `Vec<SymbolInfo>`.
    pub fn to_vec(&self) -> Vec<SymbolInfo> {
        self.iter().cloned().collect()
    }
}

impl<'a> Default for SymbolSet<'a> {
    fn default() -> Self {
        SymbolSet::empty()
    }
}

/// Slice-style indexing. Returns a reference borrowed from `self`; panics on
/// out-of-bounds like a slice. Use `get` for the checked, store-lifetime form.
impl<'a> std::ops::Index<usize> for SymbolSet<'a> {
    type Output = SymbolInfo;

    fn index(&self, i: usize) -> &SymbolInfo {
        match self {
            SymbolSet::Borrowed(s) => &s[i],
            SymbolSet::Owned(v) => v[i],
        }
    }
}

/// Construct from an eager slice — the borrowed common case.
impl<'a> From<&'a [SymbolInfo]> for SymbolSet<'a> {
    fn from(s: &'a [SymbolInfo]) -> Self {
        SymbolSet::Borrowed(s)
    }
}

/// Construct from collected cross-store references.
impl<'a> From<Vec<&'a SymbolInfo>> for SymbolSet<'a> {
    fn from(v: Vec<&'a SymbolInfo>) -> Self {
        SymbolSet::Owned(v)
    }
}

// --- iteration -------------------------------------------------------------

/// Borrowing iterator over a `SymbolSet`, yielding `&SymbolInfo` uniformly
/// across the borrowed-slice and collected-refs backings.
pub enum SymbolSetIter<'a> {
    Slice(std::slice::Iter<'a, SymbolInfo>),
    Refs(std::slice::Iter<'a, &'a SymbolInfo>),
}

impl<'a> Iterator for SymbolSetIter<'a> {
    type Item = &'a SymbolInfo;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            SymbolSetIter::Slice(it) => it.next(),
            SymbolSetIter::Refs(it) => it.next().copied(),
        }
    }
}

/// By-value iteration: `for s in lookup.by_name(x)`.
impl<'a> IntoIterator for SymbolSet<'a> {
    type Item = &'a SymbolInfo;
    type IntoIter = SymbolSetIntoIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        match self {
            // The slice reference is `Copy`, so the iterator carries the `'a`
            // lifetime independently of `self`.
            SymbolSet::Borrowed(s) => SymbolSetIntoIter::Slice(s.iter()),
            SymbolSet::Owned(v) => SymbolSetIntoIter::Vec(v.into_iter()),
        }
    }
}

/// By-reference iteration: `for s in &set`.
impl<'a, 'b> IntoIterator for &'b SymbolSet<'a> {
    type Item = &'b SymbolInfo;
    type IntoIter = SymbolSetIter<'b>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

pub enum SymbolSetIntoIter<'a> {
    Slice(std::slice::Iter<'a, SymbolInfo>),
    Vec(std::vec::IntoIter<&'a SymbolInfo>),
}

impl<'a> Iterator for SymbolSetIntoIter<'a> {
    type Item = &'a SymbolInfo;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            SymbolSetIntoIter::Slice(it) => it.next(),
            SymbolSetIntoIter::Vec(it) => it.next(),
        }
    }
}
