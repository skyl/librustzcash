//! Helper functions for managing light client key material.
use zcash_primitives::{
    consensus,
    zip32::{AccountId, DiversifierIndex},
};

use crate::address::UnifiedAddress;

#[cfg(feature = "transparent-inputs")]
use std::convert::TryInto;

#[cfg(feature = "transparent-inputs")]
use zcash_primitives::legacy::keys::{self as legacy, IncomingViewingKey};

pub mod sapling {
    use zcash_primitives::zip32::{AccountId, ChildIndex};
    pub use zcash_primitives::zip32::{ExtendedFullViewingKey, ExtendedSpendingKey};

    /// Derives the ZIP 32 [`ExtendedSpendingKey`] for a given coin type and account from the
    /// given seed.
    ///
    /// # Panics
    ///
    /// Panics if `seed` is shorter than 32 bytes.
    ///
    /// # Examples
    ///
    /// ```
    /// use zcash_primitives::{
    ///     constants::testnet::COIN_TYPE,
    ///     zip32::AccountId,
    /// };
    /// use zcash_client_backend::{
    ///     keys::sapling,
    /// };
    ///
    /// let extsk = sapling::spending_key(&[0; 32][..], COIN_TYPE, AccountId::from(0));
    /// ```
    /// [`ExtendedSpendingKey`]: zcash_primitives::zip32::ExtendedSpendingKey
    pub fn spending_key(seed: &[u8], coin_type: u32, account: AccountId) -> ExtendedSpendingKey {
        if seed.len() < 32 {
            panic!("ZIP 32 seeds MUST be at least 32 bytes");
        }

        ExtendedSpendingKey::from_path(
            &ExtendedSpendingKey::master(seed),
            &[
                ChildIndex::Hardened(32),
                ChildIndex::Hardened(coin_type),
                ChildIndex::Hardened(account.into()),
            ],
        )
    }
}

#[cfg(feature = "transparent-inputs")]
fn to_transparent_child_index(j: DiversifierIndex) -> Option<u32> {
    let (low_4_bytes, rest) = j.0.split_at(4);
    let transparent_j = u32::from_le_bytes(low_4_bytes.try_into().unwrap());
    if transparent_j > (0x7FFFFFFF) || rest.iter().any(|b| b != &0) {
        None
    } else {
        Some(transparent_j)
    }
}

#[derive(Debug)]
#[doc(hidden)]
pub enum DerivationError {
    #[cfg(feature = "transparent-inputs")]
    Transparent(hdwallet::error::Error),
}

/// A set of viewing keys that are all associated with a single
/// ZIP-0032 account identifier.
#[derive(Clone, Debug)]
#[doc(hidden)]
pub struct UnifiedSpendingKey {
    account: AccountId,
    #[cfg(feature = "transparent-inputs")]
    transparent: legacy::AccountPrivKey,
    sapling: sapling::ExtendedSpendingKey,
}

#[doc(hidden)]
impl UnifiedSpendingKey {
    pub fn from_seed<P: consensus::Parameters>(
        params: &P,
        seed: &[u8],
        account: AccountId,
    ) -> Result<UnifiedSpendingKey, DerivationError> {
        if seed.len() < 32 {
            panic!("ZIP 32 seeds MUST be at least 32 bytes");
        }

        #[cfg(feature = "transparent-inputs")]
        let transparent = legacy::AccountPrivKey::from_seed(params, seed, account)
            .map_err(DerivationError::Transparent)?;

        Ok(UnifiedSpendingKey {
            account,
            #[cfg(feature = "transparent-inputs")]
            transparent,
            sapling: sapling::spending_key(seed, params.coin_type(), account),
        })
    }

    pub fn to_unified_full_viewing_key(&self) -> UnifiedFullViewingKey {
        UnifiedFullViewingKey {
            account: self.account,
            #[cfg(feature = "transparent-inputs")]
            transparent: Some(self.transparent.to_account_pubkey()),
            sapling: Some(sapling::ExtendedFullViewingKey::from(&self.sapling)),
        }
    }

    pub fn account(&self) -> AccountId {
        self.account
    }

    /// Returns the transparent component of the unified key at the
    /// BIP44 path `m/44'/<coin_type>'/<account>'`.
    #[cfg(feature = "transparent-inputs")]
    pub fn transparent(&self) -> &legacy::AccountPrivKey {
        &self.transparent
    }

    /// Returns the Sapling extended full viewing key component of this
    /// unified key.
    pub fn sapling(&self) -> &sapling::ExtendedSpendingKey {
        &self.sapling
    }
}

/// A set of viewing keys that are all associated with a single
/// ZIP-0032 account identifier.
#[derive(Clone, Debug)]
#[doc(hidden)]
pub struct UnifiedFullViewingKey {
    account: AccountId,
    #[cfg(feature = "transparent-inputs")]
    transparent: Option<legacy::AccountPubKey>,
    // TODO: This type is invalid for a UFVK; create a `sapling::DiversifiableFullViewingKey`
    // to replace it.
    sapling: Option<sapling::ExtendedFullViewingKey>,
}

#[doc(hidden)]
impl UnifiedFullViewingKey {
    /// Construct a new unified full viewing key, if the required components are present.
    pub fn new(
        account: AccountId,
        #[cfg(feature = "transparent-inputs")] transparent: Option<legacy::AccountPubKey>,
        sapling: Option<sapling::ExtendedFullViewingKey>,
    ) -> Option<UnifiedFullViewingKey> {
        if sapling.is_none() {
            None
        } else {
            Some(UnifiedFullViewingKey {
                account,
                #[cfg(feature = "transparent-inputs")]
                transparent,
                sapling,
            })
        }
    }

    /// Returns the ZIP32 account identifier to which all component
    /// keys are related.
    pub fn account(&self) -> AccountId {
        self.account
    }

    /// Returns the transparent component of the unified key at the
    /// BIP44 path `m/44'/<coin_type>'/<account>'`.
    #[cfg(feature = "transparent-inputs")]
    pub fn transparent(&self) -> Option<&legacy::AccountPubKey> {
        self.transparent.as_ref()
    }

    /// Returns the Sapling extended full viewing key component of this
    /// unified key.
    pub fn sapling(&self) -> Option<&sapling::ExtendedFullViewingKey> {
        self.sapling.as_ref()
    }

    /// Attempts to derive the Unified Address for the given diversifier index.
    ///
    /// Returns `None` if the specified index does not produce a valid diversifier.
    // TODO: Allow filtering down by receiver types?
    pub fn address(&self, j: DiversifierIndex) -> Option<UnifiedAddress> {
        let sapling = if let Some(extfvk) = self.sapling.as_ref() {
            Some(extfvk.address(j)?)
        } else {
            None
        };

        #[cfg(feature = "transparent-inputs")]
        let transparent = if let Some(tfvk) = self.transparent.as_ref() {
            match to_transparent_child_index(j) {
                Some(transparent_j) => match tfvk
                    .derive_external_ivk()
                    .and_then(|tivk| tivk.derive_address(transparent_j))
                {
                    Ok(taddr) => Some(taddr),
                    Err(_) => return None,
                },
                // Diversifier doesn't generate a valid transparent child index.
                None => return None,
            }
        } else {
            None
        };
        #[cfg(not(feature = "transparent-inputs"))]
        let transparent = None;

        UnifiedAddress::from_receivers(None, sapling, transparent)
    }

    /// Searches the diversifier space starting at diversifier index `j` for one which will
    /// produce a valid diversifier, and return the Unified Address constructed using that
    /// diversifier along with the index at which the valid diversifier was found.
    ///
    /// Returns `None` if no valid diversifier exists
    pub fn find_address(
        &self,
        mut j: DiversifierIndex,
    ) -> Option<(UnifiedAddress, DiversifierIndex)> {
        // If we need to generate a transparent receiver, check that the user has not
        // specified an invalid transparent child index, from which we can never search to
        // find a valid index.
        #[cfg(feature = "transparent-inputs")]
        if self.transparent.is_some() && to_transparent_child_index(j).is_none() {
            return None;
        }

        // Find a working diversifier and construct the associated address.
        loop {
            let res = self.address(j);
            if let Some(ua) = res {
                break Some((ua, j));
            }
            if j.increment().is_err() {
                break None;
            }
        }
    }

    /// Returns the Unified Address corresponding to the smallest valid diversifier index,
    /// along with that index.
    pub fn default_address(&self) -> (UnifiedAddress, DiversifierIndex) {
        self.find_address(DiversifierIndex::new())
            .expect("UFVK should have at least one valid diversifier")
    }
}

#[cfg(test)]
mod tests {
    use super::sapling;
    use zcash_primitives::zip32::AccountId;

    #[cfg(feature = "transparent-inputs")]
    use {
        crate::encoding::AddressCodec,
        zcash_primitives::{consensus::MAIN_NETWORK, legacy, legacy::keys::IncomingViewingKey},
    };

    #[cfg(feature = "transparent-inputs")]
    fn seed() -> Vec<u8> {
        let seed_hex = "6ef5f84def6f4b9d38f466586a8380a38593bd47c8cda77f091856176da47f26b5bd1c8d097486e5635df5a66e820d28e1d73346f499801c86228d43f390304f";
        hex::decode(&seed_hex).unwrap()
    }

    #[test]
    #[should_panic]
    fn spending_key_panics_on_short_seed() {
        let _ = sapling::spending_key(&[0; 31][..], 0, AccountId::from(0));
    }

    #[cfg(feature = "transparent-inputs")]
    #[test]
    fn pk_to_taddr() {
        let taddr =
            legacy::keys::AccountPrivKey::from_seed(&MAIN_NETWORK, &seed(), AccountId::from(0))
                .unwrap()
                .to_account_pubkey()
                .derive_external_ivk()
                .unwrap()
                .derive_address(0)
                .unwrap()
                .encode(&MAIN_NETWORK);
        assert_eq!(taddr, "t1PKtYdJJHhc3Pxowmznkg7vdTwnhEsCvR4".to_string());
    }
}
