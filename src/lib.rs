pub mod iter;
pub mod mapref;
mod t;
mod util;

use dashmap_shard::HashMap;
use fxhash::FxBuildHasher;
use iter::{Iter, IterMut};
use mapref::entry::{Entry, OccupiedEntry, VacantEntry};
use mapref::one::{Ref, RefMut};
use parking_lot::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::borrow::Borrow;
use std::hash::{BuildHasher, Hash, Hasher};
use t::Map;
use crossbeam_utils::CachePadded;

fn shard_amount() -> usize {
    (num_cpus::get() * 4).next_power_of_two()
}

fn ncb(shard_amount: usize) -> usize {
    (shard_amount as f32).log2() as usize
}

#[derive(Default)]
pub struct DashMap<K, V>
where
    K: Eq + Hash,
{
    ncb: usize,
    shards: Box<[CachePadded<RwLock<HashMap<K, V, FxBuildHasher>>>]>,
    hash_builder: FxBuildHasher,
}

impl<'a, K: 'a + Eq + Hash, V: 'a> DashMap<K, V> {
    #[inline]
    pub fn new() -> Self {
        let shard_amount = shard_amount();
        let shards = (0..shard_amount)
            .map(|_| CachePadded::new(RwLock::new(HashMap::with_hasher(FxBuildHasher::default()))))
            .collect::<Vec<_>>()
            .into_boxed_slice();

        Self {
            ncb: ncb(shard_amount),
            shards,
            hash_builder: FxBuildHasher::default(),
        }
    }

    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        let shard_amount = shard_amount();
        let cps = capacity / shard_amount;
        let shards = (0..shard_amount)
            .map(|_| {
                CachePadded::new(RwLock::new(HashMap::with_capacity_and_hasher(
                    cps,
                    FxBuildHasher::default(),
                )))
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();

        Self {
            ncb: ncb(shard_amount),
            shards,
            hash_builder: FxBuildHasher::default(),
        }
    }

    #[inline]
    pub fn shards(&self) -> &[CachePadded<RwLock<HashMap<K, V, FxBuildHasher>>>] {
        &self.shards
    }

    #[inline]
    pub fn determine_map<Q>(&self, key: &Q) -> (usize, u64)
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let mut hash_state = self.hash_builder.build_hasher();
        key.hash(&mut hash_state);

        let hash = hash_state.finish();
        let shift = util::ptr_size_bits() - self.ncb;

        ((hash >> shift) as usize, hash)
    }

    #[inline]
    pub fn insert(&self, key: K, value: V) -> Option<V> {
        self._insert(key, value)
    }

    #[inline]
    pub fn remove<Q>(&self, key: &Q) -> Option<(K, V)>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self._remove(key)
    }

    #[inline]
    pub fn iter(&'a self) -> Iter<'a, K, V, DashMap<K, V>> {
        self._iter()
    }

    #[inline]
    pub fn iter_mut(&'a self) -> IterMut<'a, K, V, DashMap<K, V>> {
        self._iter_mut()
    }

    #[inline]
    pub fn get<Q>(&'a self, key: &Q) -> Option<Ref<'a, K, V>>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self._get(key)
    }

    #[inline]
    pub fn get_mut<Q>(&'a self, key: &Q) -> Option<RefMut<'a, K, V>>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self._get_mut(key)
    }

    #[inline]
    pub fn shrink_to_fit(&self) {
        self._shrink_to_fit();
    }

    #[inline]
    pub fn retain(&self, f: impl FnMut(&K, &mut V) -> bool) {
        self._retain(f);
    }

    #[inline]
    pub fn len(&self) -> usize {
        self._len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self._is_empty()
    }

    #[inline]
    pub fn clear(&self) {
        self._clear();
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self._capacity()
    }

    #[inline]
    pub fn alter<Q>(&self, key: &Q, f: impl FnOnce(&K, V) -> V)
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self._alter(key, f);
    }

    #[inline]
    pub fn alter_all(&self, f: impl FnMut(&K, V) -> V) {
        self._alter_all(f);
    }

    #[inline]
    pub fn contains_key<Q>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self._contains_key(key)
    }

    #[inline]
    pub fn entry(&'a self, key: K) -> Entry<'a, K, V> {
        self._entry(key)
    }
}

impl<'a, K: 'a + Eq + Hash, V: 'a> Map<'a, K, V> for DashMap<K, V> {
    #[inline(always)]
    fn _shard_count(&self) -> usize {
        self.shards.len()
    }

    #[inline(always)]
    unsafe fn _yield_read_shard(&'a self, i: usize) -> RwLockReadGuard<'a, HashMap<K, V, FxBuildHasher>> {
        self.shards.get_unchecked(i).read()
    }

    #[inline(always)]
    unsafe fn _yield_write_shard(
        &'a self,
        i: usize,
    ) -> RwLockWriteGuard<'a, HashMap<K, V, FxBuildHasher>> {
        self.shards.get_unchecked(i).write()
    }

    #[inline(always)]
    fn _insert(&self, key: K, value: V) -> Option<V> {
        let (shard, hash) = self.determine_map(&key);
        let mut shard = unsafe { self._yield_write_shard(shard) };
        shard.insert_with_hash_nocheck(key, value, hash)
    }

    #[inline(always)]
    fn _remove<Q>(&self, key: &Q) -> Option<(K, V)>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let (shard, _) = self.determine_map(&key);
        let mut shard = unsafe { self._yield_write_shard(shard) };
        shard.remove_entry(key)
    }

    #[inline(always)]
    fn _iter(&'a self) -> Iter<'a, K, V, DashMap<K, V>> {
        Iter::new(self)
    }

    #[inline(always)]
    fn _iter_mut(&'a self) -> IterMut<'a, K, V, DashMap<K, V>> {
        IterMut::new(self)
    }

    #[inline(always)]
    fn _get<Q>(&'a self, key: &Q) -> Option<Ref<'a, K, V>>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let (shard, hash) = self.determine_map(&key);
        let shard = unsafe { self._yield_read_shard(shard) };
        if let Some((kptr, vptr)) = shard.get_hash_nocheck_key_value(hash, key) {
            unsafe {
                let kptr = util::change_lifetime_const(kptr);
                let vptr = util::change_lifetime_const(vptr);
                Some(Ref::new(shard, kptr, vptr))
            }
        } else {
            None
        }
    }

    #[inline(always)]
    fn _get_mut<Q>(&'a self, key: &Q) -> Option<RefMut<'a, K, V>>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let (shard, hash) = self.determine_map(&key);
        let shard = unsafe { self._yield_write_shard(shard) };
        if let Some((kptr, vptr)) = shard.get_hash_nocheck_key_value(hash, key) {
            unsafe {
                let kptr = util::change_lifetime_const(kptr);
                let vptr = util::change_lifetime_mut(util::to_mut(vptr));
                Some(RefMut::new(shard, kptr, vptr))
            }
        } else {
            None
        }
    }

    #[inline(always)]
    fn _shrink_to_fit(&self) {
        self.shards.iter().for_each(|s| s.write().shrink_to_fit());
    }

    #[inline(always)]
    fn _retain(&self, mut f: impl FnMut(&K, &mut V) -> bool) {
        self.shards.iter().for_each(|s| s.write().retain(&mut f));
    }

    #[inline(always)]
    fn _len(&self) -> usize {
        self.shards.iter().map(|s| s.read().len()).sum()
    }

    #[inline(always)]
    fn _capacity(&self) -> usize {
        self.shards.iter().map(|s| s.read().capacity()).sum()
    }

    #[inline(always)]
    fn _alter<Q>(&self, key: &Q, f: impl FnOnce(&K, V) -> V)
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        if let Some(mut r) = self.get_mut(key) {
            util::map_in_place_2(r.pair_mut(), f);
        }
    }

    #[inline(always)]
    fn _alter_all(&self, mut f: impl FnMut(&K, V) -> V) {
        self.shards.iter().for_each(|s| {
            s.write()
                .iter_mut()
                .for_each(|pair| util::map_in_place_2(pair, &mut f));
        });
    }

    #[inline(always)]
    fn _entry(&'a self, key: K) -> Entry<'a, K, V> {
        let (shard, hash) = self.determine_map(&key);
        let shard = unsafe { self._yield_write_shard(shard) };
        if let Some((kptr, vptr)) = shard.get_hash_nocheck_key_value(hash, &key) {
            unsafe {
                let kptr = util::change_lifetime_const(kptr);
                let vptr = util::change_lifetime_mut(util::to_mut(vptr));
                Entry::Occupied(OccupiedEntry::new(shard, Some(key), (kptr, vptr)))
            }
        } else {
            Entry::Vacant(VacantEntry::new(shard, key))
        }
    }
}
