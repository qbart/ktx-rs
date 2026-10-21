//! Port of lib/src/hashlist.c — key/value metadata list.
//!
//! The C version is a uthash table iterated in insertion order. Here entries are
//! kept in a `Vec` in insertion order. Lookups return the most recently added
//! entry for a duplicated key, matching uthash bucket-head insertion.

use crate::defs::ktx_pad4;
use crate::error::{ErrorCode, KtxResult};

/// Port of `ktxKVListEntry` / `ktxHashListEntry`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HashListEntry {
    /// Key bytes without the terminating NUL.
    key: Vec<u8>,
    /// Value bytes (may be empty).
    value: Vec<u8>,
}

impl HashListEntry {
    /// Port of `ktxHashListEntry_GetKey`. Returns (keyLen incl. NUL, key without NUL).
    pub fn get_key(&self) -> (u32, &[u8]) {
        (self.key.len() as u32 + 1, &self.key)
    }

    /// Key as UTF-8 if valid.
    pub fn key_str(&self) -> Option<&str> {
        core::str::from_utf8(&self.key).ok()
    }

    /// Port of `ktxHashListEntry_GetValue`. Empty slice corresponds to `NULL`.
    pub fn get_value(&self) -> (u32, &[u8]) {
        (self.value.len() as u32, &self.value)
    }

    /// Mutable access to the value bytes.
    pub fn value_mut(&mut self) -> &mut Vec<u8> {
        &mut self.value
    }
}

/// Port of `ktxHashList`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HashList {
    entries: Vec<HashListEntry>,
}

fn key_bytes(key: &[u8]) -> &[u8] {
    // C uses strlen: anything after an embedded NUL is ignored.
    match key.iter().position(|&c| c == 0) {
        Some(n) => &key[..n],
        None => key,
    }
}

impl HashList {
    /// Port of `ktxHashList_Construct` / `ktxHashList_Create`.
    pub fn new() -> HashList {
        HashList {
            entries: Vec::new(),
        }
    }

    /// Port of `ktxHashList_ConstructCopy` / `ktxHashList_CreateCopy`.
    pub fn construct_copy(orig: &HashList) -> HashList {
        orig.clone()
    }

    /// Port of `ktxHashList_Destruct`: removes all entries.
    pub fn destruct(&mut self) {
        self.entries.clear();
    }

    /// Port of `ktxHashList_Destroy`.
    pub fn destroy(self) {}

    /// Number of entries (`HASH_COUNT`).
    pub fn count(&self) -> usize {
        self.entries.len()
    }

    /// True if the list has no entries (`*pHead == NULL`).
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterates entries in list order (`ktxHashList_Next`).
    pub fn iter(&self) -> core::slice::Iter<'_, HashListEntry> {
        self.entries.iter()
    }

    /// Entry at `index` in list order.
    pub fn entry(&self, index: usize) -> Option<&HashListEntry> {
        self.entries.get(index)
    }

    /// Port of `ktxHashList_AddKVPair`. Key is treated as a NUL-terminated string.
    pub fn add_kv_pair(&mut self, key: impl AsRef<[u8]>, value: &[u8]) -> KtxResult<()> {
        let key = key_bytes(key.as_ref());
        if key.is_empty() {
            return Err(ErrorCode::InvalidValue);
        }
        if value.len() > u32::MAX as usize || key.len() >= u32::MAX as usize {
            return Err(ErrorCode::InvalidValue);
        }
        self.entries.push(HashListEntry {
            key: key.to_vec(),
            value: value.to_vec(),
        });
        Ok(())
    }

    fn find_index(&self, key: &[u8]) -> Option<usize> {
        let key = key_bytes(key);
        self.entries.iter().rposition(|e| e.key == key)
    }

    /// Port of `ktxHashList_DeleteKVPair`. Succeeds even if the key is absent.
    pub fn delete_kv_pair(&mut self, key: impl AsRef<[u8]>) -> KtxResult<()> {
        if let Some(i) = self.find_index(key.as_ref()) {
            self.entries.remove(i);
        }
        Ok(())
    }

    /// Port of `ktxHashList_DeleteEntry`, addressing the entry by list index.
    pub fn delete_entry(&mut self, index: usize) -> KtxResult<()> {
        if index >= self.entries.len() {
            return Err(ErrorCode::InvalidValue);
        }
        self.entries.remove(index);
        Ok(())
    }

    /// Port of `ktxHashList_FindEntry`.
    pub fn find_entry(&self, key: impl AsRef<[u8]>) -> KtxResult<&HashListEntry> {
        match self.find_index(key.as_ref()) {
            Some(i) => self.entries.get(i).ok_or(ErrorCode::NotFound),
            None => Err(ErrorCode::NotFound),
        }
    }

    /// Mutable variant of [`HashList::find_entry`].
    pub fn find_entry_mut(&mut self, key: impl AsRef<[u8]>) -> KtxResult<&mut HashListEntry> {
        match self.find_index(key.as_ref()) {
            Some(i) => self.entries.get_mut(i).ok_or(ErrorCode::NotFound),
            None => Err(ErrorCode::NotFound),
        }
    }

    /// Port of `ktxHashList_FindValue`.
    pub fn find_value(&self, key: impl AsRef<[u8]>) -> KtxResult<&[u8]> {
        self.find_entry(key).map(|e| e.value.as_slice())
    }

    /// Port of `ktxHashList_Serialize`.
    pub fn serialize(&self) -> KtxResult<Vec<u8>> {
        let mut total: u64 = 0;
        for kv in &self.entries {
            let len = kv.key.len() as u64 + 1 + kv.value.len() as u64 + 4;
            total += ktx_pad4(len);
        }
        if total > u32::MAX as u64 {
            return Err(ErrorCode::FileOverflow);
        }
        let mut sd = Vec::with_capacity(total as usize);
        for kv in &self.entries {
            let key_value_len = (kv.key.len() + 1 + kv.value.len()) as u32;
            sd.extend_from_slice(&key_value_len.to_le_bytes());
            sd.extend_from_slice(&kv.key);
            sd.push(0);
            sd.extend_from_slice(&kv.value);
            let pad = ktx_pad4(key_value_len as u64) - key_value_len as u64;
            sd.resize(sd.len() + pad as usize, 0);
        }
        Ok(sd)
    }

    /// Port of `ktxHashList_Sort`: stable sort by key code points (`strcmp`).
    pub fn sort(&mut self) -> KtxResult<()> {
        self.entries.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(())
    }

    /// Port of `ktxHashList_Deserialize`. The list must be empty.
    pub fn deserialize(&mut self, kvd: &[u8]) -> KtxResult<()> {
        if kvd.is_empty() {
            return Err(ErrorCode::InvalidValue);
        }
        if !self.entries.is_empty() {
            return Err(ErrorCode::InvalidOperation);
        }
        let result = self.deserialize_inner(kvd);
        if result.is_err() {
            self.entries.clear();
        }
        result
    }

    fn deserialize_inner(&mut self, kvd: &[u8]) -> KtxResult<()> {
        let end = kvd.len();
        let mut src = 0usize;
        while src < end {
            if src + 6 > end {
                return Err(ErrorCode::FileDataError);
            }
            let size_bytes = kvd.get(src..src + 4).ok_or(ErrorCode::FileDataError)?;
            let key_and_value_byte_size =
                u32::from_le_bytes([size_bytes[0], size_bytes[1], size_bytes[2], size_bytes[3]])
                    as usize;
            if src + 4 + key_and_value_byte_size > end {
                return Err(ErrorCode::FileDataError);
            }
            src += 4;
            let kv = kvd
                .get(src..src + key_and_value_byte_size)
                .ok_or(ErrorCode::FileDataError)?;
            let key_len = match kv.iter().position(|&c| c == 0) {
                Some(n) => n,
                // Missing NUL terminator or no value
                None => return Err(ErrorCode::FileDataError),
            };
            let key = &kv[..key_len];
            if key.starts_with(&[0xEF, 0xBB, 0xBF]) {
                // Forbidden BOM
                return Err(ErrorCode::FileDataError);
            }
            let value = &kv[key_len + 1..];
            self.add_kv_pair(key, value)?;
            src += ktx_pad4(key_and_value_byte_size as u64) as usize;
        }
        Ok(())
    }
}

impl<'a> IntoIterator for &'a HashList {
    type Item = &'a HashListEntry;
    type IntoIter = core::slice::Iter<'a, HashListEntry>;
    fn into_iter(self) -> Self::IntoIter {
        self.entries.iter()
    }
}
