//! This module contains the [Storage] type and its supporting types, which
//! provide the [Host](crate::Host) with access to durable ledger entries.
//!
//! For more details, see the [Env](crate::Env) data access functions:
//!   - [Env::has_contract_data](crate::Env::has_contract_data)
//!   - [Env::get_contract_data](crate::Env::get_contract_data)
//!   - [Env::put_contract_data](crate::Env::put_contract_data)
//!   - [Env::del_contract_data](crate::Env::del_contract_data)

use std::rc::Rc;

use soroban_env_common::{Compare, RawVal};

use crate::budget::Budget;
use crate::xdr::{Hash, LedgerEntry, LedgerKey, ScHostStorageErrorCode};
use crate::Host;
use crate::{host::metered_map::MeteredOrdMap, HostError};

pub type FootprintMap = MeteredOrdMap<Rc<LedgerKey>, AccessType, Budget>;
pub type StorageMap = MeteredOrdMap<Rc<LedgerKey>, Option<Rc<LedgerEntry>>, Budget>;
pub type TempStorageMap = MeteredOrdMap<Rc<(Hash, RawVal)>, RawVal, Host>;

/// A helper type used by [Footprint] to designate which ways
/// a given [LedgerKey] is accessed, or is allowed to be accessed,
/// in a given transaction.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum AccessType {
    /// When in [FootprintMode::Recording], indicates that the [LedgerKey] is only read.
    /// When in [FootprintMode::Enforcing], indicates that the [LedgerKey] is only _allowed_ to be read.
    ReadOnly,
    /// When in [FootprintMode::Recording], indicates that the [LedgerKey] is written (and also possibly read)
    /// When in [FootprintMode::Enforcing], indicates that the [LedgerKey] is _allowed_ to be written (and also allowed to be read).
    ReadWrite,
}

impl Compare<AccessType> for Host {
    type Error = HostError;

    fn compare(&self, a: &AccessType, b: &AccessType) -> Result<core::cmp::Ordering, Self::Error> {
        Ok(a.cmp(b))
    }
}

impl Compare<AccessType> for Budget {
    type Error = HostError;

    fn compare(&self, a: &AccessType, b: &AccessType) -> Result<core::cmp::Ordering, Self::Error> {
        Ok(a.cmp(b))
    }
}

/// A helper type used by [FootprintMode::Recording] to provide access
/// to a stable read-snapshot of a ledger.
pub trait SnapshotSource {
    fn get(&self, key: &Rc<LedgerKey>) -> Result<Rc<LedgerEntry>, HostError>;
    fn has(&self, key: &Rc<LedgerKey>) -> Result<bool, HostError>;
}

/// Describes the total set of [LedgerKey]s that a given transaction
/// will access, as well as the [AccessType] governing each key.
///
/// A [Footprint] must be provided in order to run a transaction that
/// accesses any [LedgerKey]s in [FootprintMode::Enforcing]. If a
/// transaction has an unknown [Footprint] it can be calculated by
/// running a "preflight" execution in [FootprintMode::Recording],
/// against a suitably fresh [SnapshotSource].
// Notes on metering: covered by the underneath `MeteredOrdMap`.
#[derive(Clone, Default)]
pub struct Footprint(pub FootprintMap);

impl Footprint {
    pub fn record_access(
        &mut self,
        key: &Rc<LedgerKey>,
        ty: AccessType,
        budget: &Budget,
    ) -> Result<(), HostError> {
        if let Some(existing) = self.0.get::<Rc<LedgerKey>>(key, budget)? {
            match (existing, ty.clone()) {
                (AccessType::ReadOnly, AccessType::ReadOnly) => Ok(()),
                (AccessType::ReadOnly, AccessType::ReadWrite) => {
                    // The only interesting case is an upgrade
                    // from previously-read-only to read-write.
                    self.0 = self.0.insert(Rc::clone(key), ty, budget)?;
                    Ok(())
                }
                (AccessType::ReadWrite, AccessType::ReadOnly) => Ok(()),
                (AccessType::ReadWrite, AccessType::ReadWrite) => Ok(()),
            }
        } else {
            self.0 = self.0.insert(Rc::clone(key), ty, budget)?;
            Ok(())
        }
    }

    pub fn enforce_access(
        &mut self,
        key: &Rc<LedgerKey>,
        ty: AccessType,
        budget: &Budget,
    ) -> Result<(), HostError> {
        if let Some(existing) = self.0.get::<Rc<LedgerKey>>(key, budget)? {
            match (existing, ty) {
                (AccessType::ReadOnly, AccessType::ReadOnly) => Ok(()),
                (AccessType::ReadOnly, AccessType::ReadWrite) => {
                    Err(ScHostStorageErrorCode::ReadwriteAccessToReadonlyEntry.into())
                }
                (AccessType::ReadWrite, AccessType::ReadOnly) => Ok(()),
                (AccessType::ReadWrite, AccessType::ReadWrite) => Ok(()),
            }
        } else {
            Err(ScHostStorageErrorCode::AccessToUnknownEntry.into())
        }
    }
}

#[derive(Clone)]
pub enum FootprintMode {
    Recording(Rc<dyn SnapshotSource>),
    Enforcing,
}

impl Default for FootprintMode {
    fn default() -> Self {
        FootprintMode::Enforcing
    }
}

/// A special-purpose map from [LedgerKey]s to [LedgerEntry]s. Represents a
/// transactional batch of contract IO from and to durable storage, while
/// partitioning that IO between concurrently executing groups of contracts
/// through the use of IO [Footprint]s.
///
/// Specifically: access to each [LedgerKey] is mediated by the [Footprint],
/// which may be in either [FootprintMode::Recording] or
/// [FootprintMode::Enforcing] mode.
///
/// [FootprintMode::Recording] mode is used to calculate [Footprint]s during
/// "preflight" execution of a contract. Once calculated, a recorded [Footprint]
/// can be provided to "real" execution, which always runs in
/// [FootprintMode::Enforcing] mode and enforces partitioned access.
#[derive(Clone, Default)]
pub struct Storage {
    pub footprint: Footprint,
    pub mode: FootprintMode,
    pub map: StorageMap,
}

// Notes on metering: all storage operations: `put`, `get`, `del`, `has` are
// covered by the underneath `MeteredOrdMap` and the `Footprint`'s own map.
impl Storage {
    /// Constructs a new [Storage] in [FootprintMode::Enforcing] using a
    /// given [Footprint] and a storage map populated with all the keys
    /// listed in the [Footprint].
    pub fn with_enforcing_footprint_and_map(footprint: Footprint, map: StorageMap) -> Self {
        Self {
            mode: FootprintMode::Enforcing,
            footprint,
            map,
        }
    }

    /// Constructs a new [Storage] in [FootprintMode::Recording] using a
    /// given [SnapshotSource].
    pub fn with_recording_footprint(src: Rc<dyn SnapshotSource>) -> Self {
        Self {
            mode: FootprintMode::Recording(src),
            footprint: Footprint::default(),
            map: Default::default(),
        }
    }

    /// Attempts to retrieve the [LedgerEntry] associated with a given
    /// [LedgerKey] in the [Storage], returning an error if the key is not
    /// found.
    ///
    /// In [FootprintMode::Recording] mode, records the read [LedgerKey] in the
    /// [Footprint] as [AccessType::ReadOnly] (unless already recorded as
    /// [AccessType::ReadWrite]) and reads through to the underlying
    /// [SnapshotSource], if the [LedgerKey] has not yet been loaded.
    ///
    /// In [FootprintMode::Enforcing] mode, succeeds only if the read
    /// [LedgerKey] has been declared in the [Footprint].
    pub fn get(
        &mut self,
        key: &Rc<LedgerKey>,
        budget: &Budget,
    ) -> Result<Rc<LedgerEntry>, HostError> {
        let ty = AccessType::ReadOnly;
        match self.mode {
            FootprintMode::Recording(ref src) => {
                self.footprint.record_access(key, ty, budget)?;
                // In recording mode we treat the map as a cache
                // that misses read-through to the underlying src.
                if !self.map.contains_key::<Rc<LedgerKey>>(key, budget)? {
                    self.map = self
                        .map
                        .insert(Rc::clone(key), Some(src.get(key)?), budget)?;
                }
            }
            FootprintMode::Enforcing => {
                self.footprint.enforce_access(key, ty, budget)?;
            }
        };
        match self.map.get::<Rc<LedgerKey>>(key, budget)? {
            None => Err(ScHostStorageErrorCode::MissingKeyInGet.into()),
            Some(None) => Err(ScHostStorageErrorCode::GetOnDeletedKey.into()),
            Some(Some(val)) => Ok(Rc::clone(&val)),
        }
    }

    fn put_opt(
        &mut self,
        key: &Rc<LedgerKey>,
        val: Option<&Rc<LedgerEntry>>,
        budget: &Budget,
    ) -> Result<(), HostError> {
        let ty = AccessType::ReadWrite;
        match self.mode {
            FootprintMode::Recording(_) => {
                self.footprint.record_access(key, ty, budget)?;
            }
            FootprintMode::Enforcing => {
                self.footprint.enforce_access(key, ty, budget)?;
            }
        };
        self.map = self
            .map
            .insert(Rc::clone(key), val.map(|v| Rc::clone(v)), budget)?;
        Ok(())
    }

    /// Attempts to write to the [LedgerEntry] associated with a given
    /// [LedgerKey] in the [Storage].
    ///
    /// In [FootprintMode::Recording] mode, records the written [LedgerKey] in
    /// the [Footprint] as [AccessType::ReadWrite].
    ///
    /// In [FootprintMode::Enforcing] mode, succeeds only if the written
    /// [LedgerKey] has been declared in the [Footprint] as
    /// [AccessType::ReadWrite].
    pub fn put(
        &mut self,
        key: &Rc<LedgerKey>,
        val: &Rc<LedgerEntry>,
        budget: &Budget,
    ) -> Result<(), HostError> {
        self.put_opt(key, Some(val), budget)
    }

    /// Attempts to delete the [LedgerEntry] associated with a given [LedgerKey]
    /// in the [Storage].
    ///
    /// In [FootprintMode::Recording] mode, records the deleted [LedgerKey] in
    /// the [Footprint] as [AccessType::ReadWrite].
    ///
    /// In [FootprintMode::Enforcing] mode, succeeds only if the deleted
    /// [LedgerKey] has been declared in the [Footprint] as
    /// [AccessType::ReadWrite].
    pub fn del(&mut self, key: &Rc<LedgerKey>, budget: &Budget) -> Result<(), HostError> {
        self.put_opt(key, None, budget)
    }

    /// Attempts to determine the presence of a [LedgerEntry] associated with a
    /// given [LedgerKey] in the [Storage], returning `Ok(true)` if an entry
    /// with the key exists and `Ok(false)` if it does not.
    ///
    /// In [FootprintMode::Recording] mode, records the access and reads-through
    /// to the underlying [SnapshotSource].
    ///
    /// In [FootprintMode::Enforcing] mode, succeeds only if the access has been
    /// declared in the [Footprint].
    pub fn has(&mut self, key: &Rc<LedgerKey>, budget: &Budget) -> Result<bool, HostError> {
        let ty = AccessType::ReadOnly;
        match self.mode {
            FootprintMode::Recording(ref src) => {
                self.footprint.record_access(key, ty, budget)?;
                // We don't cache has() calls but we do
                // consult the cache before answering them.
                match self.map.get::<Rc<LedgerKey>>(key, budget)? {
                    Some(None) => Ok(false),
                    Some(Some(_)) => Ok(true),
                    None => src.has(key),
                }
            }
            FootprintMode::Enforcing => {
                self.footprint.enforce_access(key, ty, budget)?;
                match self.map.get::<Rc<LedgerKey>>(key, budget)? {
                    Some(None) => Ok(false),
                    Some(Some(_)) => Ok(true),
                    None => Ok(false),
                }
            }
        }
    }
}

/// A special-purpose map from arbitrary contract-owned values to arbitrary
/// values.
///
/// Since `TempStorage` stores `RawVal`s, it has to be attributed to the host
/// instance and can't be transferred between host instances.
///
/// Semantically, this is similar to the `Storage`, but it is never persisted
/// and hence doesn't need to support the footprints and convert keys to the
/// ledger-compatible types.
#[derive(Clone, Default)]
pub struct TempStorage {
    pub map: TempStorageMap,
}

impl TempStorage {
    pub fn get(&self, contract_id: Hash, key: RawVal, host: &Host) -> Result<RawVal, HostError> {
        match self.map.get(&(contract_id, key), host)? {
            None => Err(ScHostStorageErrorCode::MissingKeyInGet.into()),
            Some(val) => Ok(*val),
        }
    }

    pub fn put(
        &mut self,
        contract_id: Hash,
        key: RawVal,
        val: RawVal,
        host: &Host,
    ) -> Result<(), HostError> {
        self.map = self.map.insert(Rc::new((contract_id, key)), val, host)?;
        Ok(())
    }

    pub fn del(&mut self, contract_id: Hash, key: RawVal, host: &Host) -> Result<(), HostError> {
        match self.map.remove(&(contract_id, key), host)? {
            Some((new_self, _)) => {
                self.map = new_self;
            }
            None => (),
        };
        Ok(())
    }

    pub fn has(&mut self, contract_id: Hash, key: RawVal, host: &Host) -> Result<bool, HostError> {
        self.map.contains_key(&(contract_id, key), host)
    }
}

#[cfg(test)]
mod test_footprint {

    use super::*;
    use crate::budget::Budget;
    use crate::xdr::{LedgerKeyContractData, ScVal};

    #[test]
    fn footprint_record_access() -> Result<(), HostError> {
        let budget = Budget::default();
        budget.reset_unlimited();
        let mut fp = Footprint::default();
        // record when key not exist
        let contract_id = [0; 32].into();
        let key = Rc::new(LedgerKey::ContractData(LedgerKeyContractData {
            contract_id,
            key: ScVal::I32(0),
        }));
        fp.record_access(&key, AccessType::ReadOnly, &budget)?;
        assert_eq!(fp.0.contains_key::<LedgerKey>(&key, &budget)?, true);
        assert_eq!(
            fp.0.get::<LedgerKey>(&key, &budget)?,
            Some(&AccessType::ReadOnly)
        );
        // record and change access
        fp.record_access(&key, AccessType::ReadWrite, &budget)?;
        assert_eq!(
            fp.0.get::<LedgerKey>(&key, &budget)?,
            Some(&AccessType::ReadWrite)
        );
        fp.record_access(&key, AccessType::ReadOnly, &budget)?;
        assert_eq!(
            fp.0.get::<LedgerKey>(&key, &budget)?,
            Some(&AccessType::ReadWrite)
        );
        Ok(())
    }

    #[test]
    fn footprint_enforce_access() -> Result<(), HostError> {
        let budget = Budget::default();
        let contract_id = [0; 32].into();
        let key = Rc::new(LedgerKey::ContractData(LedgerKeyContractData {
            contract_id,
            key: ScVal::I32(0),
        }));
        let om = [(Rc::clone(&key), AccessType::ReadOnly)].into();
        let mom = MeteredOrdMap::from_map(om, &budget)?;
        let mut fp = Footprint(mom);
        fp.enforce_access(&key, AccessType::ReadOnly, &budget)?;
        fp.0 =
            fp.0.insert(Rc::clone(&key), AccessType::ReadWrite, &budget)?;
        fp.enforce_access(&key, AccessType::ReadOnly, &budget)?;
        fp.enforce_access(&key, AccessType::ReadWrite, &budget)?;
        Ok(())
    }

    #[test]
    fn footprint_enforce_access_not_exist() -> Result<(), HostError> {
        let budget = Budget::default();
        let mut fp = Footprint::default();
        let contract_id = [0; 32].into();
        let key = Rc::new(LedgerKey::ContractData(LedgerKeyContractData {
            contract_id,
            key: ScVal::I32(0),
        }));
        let res = fp.enforce_access(&key, AccessType::ReadOnly, &budget);
        assert!(HostError::result_matches_err_status(
            res,
            ScHostStorageErrorCode::AccessToUnknownEntry
        ));
        Ok(())
    }

    #[test]
    fn footprint_attempt_to_write_readonly_entry() -> Result<(), HostError> {
        let budget = Budget::default();
        let contract_id = [0; 32].into();
        let key = Rc::new(LedgerKey::ContractData(LedgerKeyContractData {
            contract_id,
            key: ScVal::I32(0),
        }));
        let om = [(Rc::clone(&key), AccessType::ReadOnly)].into();
        let mom = MeteredOrdMap::from_map(om, &budget)?;
        let mut fp = Footprint(mom);
        let res = fp.enforce_access(&key, AccessType::ReadWrite, &budget);
        assert!(HostError::result_matches_err_status(
            res,
            ScHostStorageErrorCode::ReadwriteAccessToReadonlyEntry
        ));
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod test_storage {
    use std::collections::BTreeMap;

    use soroban_env_common::xdr::ScUnknownErrorCode;

    use super::*;
    #[allow(dead_code)]
    pub(crate) struct MockSnapshotSource(BTreeMap<Rc<LedgerKey>, Rc<LedgerEntry>>);
    #[allow(dead_code)]
    impl MockSnapshotSource {
        pub(crate) fn new() -> Self {
            Self(BTreeMap::<Rc<LedgerKey>, Rc<LedgerEntry>>::new())
        }
    }
    impl SnapshotSource for MockSnapshotSource {
        fn get(&self, key: &Rc<LedgerKey>) -> Result<Rc<LedgerEntry>, HostError> {
            if let Some(val) = self.0.get(key) {
                Ok(Rc::clone(&val))
            } else {
                Err(ScUnknownErrorCode::General.into())
            }
        }

        fn has(&self, key: &Rc<LedgerKey>) -> Result<bool, HostError> {
            Ok(self.0.contains_key(key))
        }
    }
}
