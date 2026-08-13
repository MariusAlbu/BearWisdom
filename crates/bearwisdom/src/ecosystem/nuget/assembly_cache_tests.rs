use std::cell::Cell;
use std::path::Path;

use super::AssemblyCache;

fn counting_load<'a>(
    calls: &'a Cell<usize>,
    value: Option<(u32, u64)>,
) -> impl FnOnce(&Path) -> Option<(u32, u64)> + 'a {
    move |_| {
        calls.set(calls.get() + 1);
        value
    }
}

#[test]
fn hit_does_not_reload() {
    let calls = Cell::new(0);
    let mut cache: AssemblyCache<u32> = AssemblyCache::new(100);
    assert_eq!(cache.get_or_load(Path::new("a.dll"), counting_load(&calls, Some((1, 10)))), Some(1));
    assert_eq!(cache.get_or_load(Path::new("a.dll"), counting_load(&calls, Some((1, 10)))), Some(1));
    assert_eq!(calls.get(), 1);
}

#[test]
fn failed_load_is_not_retried_until_clear() {
    let calls = Cell::new(0);
    let mut cache: AssemblyCache<u32> = AssemblyCache::new(100);
    assert_eq!(cache.get_or_load(Path::new("bad.dll"), counting_load(&calls, None)), None);
    assert_eq!(cache.get_or_load(Path::new("bad.dll"), counting_load(&calls, Some((9, 1)))), None);
    assert_eq!(calls.get(), 1);
}

#[test]
fn over_budget_evicts_least_recently_used() {
    let calls = Cell::new(0);
    let mut cache: AssemblyCache<u32> = AssemblyCache::new(10);
    cache.get_or_load(Path::new("a.dll"), counting_load(&calls, Some((1, 6))));
    cache.get_or_load(Path::new("b.dll"), counting_load(&calls, Some((2, 5))));
    assert_eq!(calls.get(), 2);
    // 6 + 5 > 10 → "a" (LRU) evicted; demanding it re-loads.
    assert_eq!(cache.get_or_load(Path::new("a.dll"), counting_load(&calls, Some((1, 6)))), Some(1));
    assert_eq!(calls.get(), 3);
}

#[test]
fn recent_use_protects_from_eviction() {
    let calls = Cell::new(0);
    let mut cache: AssemblyCache<u32> = AssemblyCache::new(10);
    cache.get_or_load(Path::new("a.dll"), counting_load(&calls, Some((1, 4))));
    cache.get_or_load(Path::new("b.dll"), counting_load(&calls, Some((2, 4))));
    cache.get_or_load(Path::new("a.dll"), counting_load(&calls, Some((1, 4)))); // promote "a"
    cache.get_or_load(Path::new("c.dll"), counting_load(&calls, Some((3, 4)))); // evicts "b"
    assert_eq!(calls.get(), 3);
    assert_eq!(cache.get_or_load(Path::new("a.dll"), counting_load(&calls, Some((1, 4)))), Some(1));
    assert_eq!(calls.get(), 3, "promoted entry must still be resident");
}

#[test]
fn sole_entry_over_budget_stays_resident() {
    let calls = Cell::new(0);
    let mut cache: AssemblyCache<u32> = AssemblyCache::new(10);
    assert_eq!(
        cache.get_or_load(Path::new("huge.dll"), counting_load(&calls, Some((1, 50)))),
        Some(1)
    );
    assert_eq!(
        cache.get_or_load(Path::new("huge.dll"), counting_load(&calls, Some((1, 50)))),
        Some(1)
    );
    assert_eq!(calls.get(), 1, "in-use entry must not evict itself");
}

#[test]
fn clear_forgets_successes_failures_and_budget_use() {
    let calls = Cell::new(0);
    let mut cache: AssemblyCache<u32> = AssemblyCache::new(10);
    cache.get_or_load(Path::new("a.dll"), counting_load(&calls, Some((1, 8))));
    cache.get_or_load(Path::new("bad.dll"), counting_load(&calls, None));
    cache.clear();
    assert_eq!(cache.get_or_load(Path::new("a.dll"), counting_load(&calls, Some((2, 8)))), Some(2));
    assert_eq!(cache.get_or_load(Path::new("bad.dll"), counting_load(&calls, Some((3, 1)))), Some(3));
    assert_eq!(calls.get(), 4, "both paths must reload after clear");
}
