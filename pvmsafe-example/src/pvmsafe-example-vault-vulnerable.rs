#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

use pallet_revive_uapi::{HostFnImpl as api, StorageFlags};
use ruint::aliases::U256;

#[pvm_contract_macros::contract("Vault.sol", allocator = "pico")]
mod vault_vulnerable {
    use super::*;
    use alloc::vec;
    use pvm_contract_types::Address;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Error {
        InsufficientShares,
    }

    impl AsRef<[u8]> for Error {
        fn as_ref(&self) -> &[u8] {
            match *self {
                Error::InsufficientShares => b"InsufficientShares",
            }
        }
    }

    #[pvm_contract_macros::constructor]
    pub fn new() -> Result<(), Error> {
        Ok(())
    }

    #[pvm_contract_macros::method]
    pub fn total_assets() -> U256 {
        load_u256(&total_assets_key())
    }

    #[pvm_contract_macros::method]
    pub fn total_shares() -> U256 {
        load_u256(&total_shares_key())
    }

    #[pvm_contract_macros::method]
    pub fn shares_of(user: Address) -> U256 {
        let user: [u8; 20] = user.into();
        load_u256(&shares_key(&user))
    }

    #[pvm_contract_macros::method]
    pub fn deposit(amount: U256) -> Result<(), Error> {
        let assets = total_assets();
        let supply = total_shares();

        let new_shares = if supply == U256::ZERO {
            amount
        } else {
            amount * supply / supply
        };

        let caller = get_caller();
        let user_shares = load_u256(&shares_key(&caller));

        store_u256(&total_assets_key(), assets + amount);
        store_u256(&total_shares_key(), supply + new_shares);
        store_u256(&shares_key(&caller), user_shares + new_shares);

        Ok(())
    }

    #[pvm_contract_macros::method]
    pub fn withdraw(shares: U256) -> Result<(), Error> {
        let supply = total_shares();
        let assets = total_assets();

        let payout = if supply == U256::ZERO {
            U256::ZERO
        } else {
            shares * assets / supply
        };

        store_u256(&total_shares_key(), supply - shares);
        store_u256(&total_assets_key(), assets - payout);

        Ok(())
    }

    #[pvm_contract_macros::fallback]
    pub fn fallback() -> Result<(), Error> {
        Ok(())
    }

    fn load_u256(key: &[u8; 32]) -> U256 {
        let mut bytes = vec![0u8; 32];
        let mut output = bytes.as_mut_slice();
        match api::get_storage(StorageFlags::empty(), key, &mut output) {
            Ok(_) => U256::from_be_bytes::<32>(output[0..32].try_into().unwrap()),
            Err(_) => U256::ZERO,
        }
    }

    fn store_u256(key: &[u8; 32], value: U256) {
        api::set_storage(StorageFlags::empty(), key, &value.to_be_bytes::<32>());
    }

    fn total_assets_key() -> [u8; 32] {
        let mut key = [0u8; 32];
        key[31] = 1;
        key
    }

    fn total_shares_key() -> [u8; 32] {
        let mut key = [0u8; 32];
        key[31] = 2;
        key
    }

    fn shares_key(addr: &[u8; 20]) -> [u8; 32] {
        let mut input = [0u8; 64];
        input[12..32].copy_from_slice(addr);
        input[63] = 3;
        let mut key = [0u8; 32];
        api::hash_keccak_256(&input, &mut key);
        key
    }

    fn get_caller() -> [u8; 20] {
        let mut caller = [0u8; 20];
        api::caller(&mut caller);
        caller
    }
}
