#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]
#![feature(stmt_expr_attributes)]

use pallet_revive_uapi::{HostFnImpl as api, StorageFlags};
use ruint::aliases::U256;

#[pvmsafe_macros::contract]
#[pvmsafe::invariant(conserves(shares))]
#[pvm_contract_macros::contract("Vault.sol", allocator = "pico")]
mod vault {
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
    pub fn shares_of(#[pvmsafe::unchecked] user: Address) -> U256 {
        let user: [u8; 20] = user.into();
        load_u256(&shares_key(&user))
    }

    #[pvm_contract_macros::method]
    #[allow(unused_braces)]
    pub fn deposit(
        #[pvmsafe::refine(amount > 0)] amount: U256,
    ) -> Result<(), Error> {
        let assets = total_assets();
        let supply = total_shares();

        let new_shares = if supply == U256::ZERO {
            amount
        } else {
            #[pvmsafe::given(amount > 0)]
            { amount.saturating_mul(supply) / supply }
        };

        let caller = get_caller();
        let user_shares = load_u256(&shares_key(&caller));

        store_u256(&total_assets_key(), assets.saturating_add(amount));
        #[pvmsafe::delta(shares = -new_shares)]
        store_u256(&total_shares_key(), supply.saturating_add(new_shares));
        #[pvmsafe::delta(shares = new_shares)]
        store_u256(&shares_key(&caller), user_shares.saturating_add(new_shares));

        emit_deposit(&caller, amount, new_shares);

        Ok(())
    }

    #[pvm_contract_macros::method]
    #[allow(unused_braces)]
    pub fn withdraw(
        #[pvmsafe::refine(shares > 0)] shares: U256,
    ) -> Result<(), Error> {
        let caller = get_caller();
        let user_shares = load_u256(&shares_key(&caller));

        if user_shares < shares {
            return Err(Error::InsufficientShares);
        }

        let supply = total_shares();
        let assets = total_assets();

        let payout = if supply == U256::ZERO {
            U256::ZERO
        } else {
            #[pvmsafe::given(shares > 0)]
            { shares.saturating_mul(assets) / supply }
        };

        let new_user_shares = user_shares - shares;

        #[pvmsafe::delta(shares = -shares)]
        store_u256(&shares_key(&caller), new_user_shares);
        #[pvmsafe::delta(shares = shares)]
        store_u256(&total_shares_key(), supply.saturating_sub(shares));
        store_u256(&total_assets_key(), assets.saturating_sub(payout));

        emit_withdraw(&caller, payout, shares);

        Ok(())
    }

    #[pvm_contract_macros::fallback]
    pub fn fallback() -> Result<(), Error> {
        Ok(())
    }

    #[pvmsafe::ensures(v >= 0)]
    #[pvmsafe::effect(read)]
    fn load_u256(key: &[u8; 32]) -> U256 {
        let mut bytes = vec![0u8; 32];
        let mut output = bytes.as_mut_slice();
        match api::get_storage(StorageFlags::empty(), key, &mut output) {
            Ok(_) => U256::from_be_bytes::<32>(output[0..32].try_into().unwrap()),
            Err(_) => U256::ZERO,
        }
    }

    #[pvmsafe::effect(write)]
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

    #[pvmsafe::effect(read)]
    fn get_caller() -> [u8; 20] {
        let mut caller = [0u8; 20];
        api::caller(&mut caller);
        caller
    }

    const DEPOSIT_EVENT_SIGNATURE: [u8; 32] = [
        0xe1, 0xff, 0xfc, 0xc4, 0x92, 0x3d, 0x04, 0xb5,
        0x59, 0xf4, 0xd2, 0x9a, 0x8b, 0xfc, 0x6c, 0xda,
        0x04, 0xeb, 0x5b, 0x0d, 0x3c, 0x46, 0x0c, 0x57,
        0xa5, 0x40, 0xf2, 0x7a, 0xd0, 0x16, 0x45, 0xa7,
    ];

    const WITHDRAW_EVENT_SIGNATURE: [u8; 32] = [
        0xfb, 0xde, 0x79, 0x71, 0x76, 0x55, 0x38, 0x0c,
        0x61, 0xb2, 0xac, 0x3c, 0x17, 0x2d, 0x82, 0xa3,
        0x78, 0xf4, 0x7f, 0x86, 0x89, 0xe3, 0x0d, 0x43,
        0xf4, 0xa1, 0xb5, 0x56, 0xc4, 0x05, 0x24, 0x1a,
    ];

    #[pvmsafe::effect(emit)]
    fn emit_deposit(user: &[u8; 20], amount: U256, shares: U256) {
        let mut user_topic = [0u8; 32];
        user_topic[12..32].copy_from_slice(user);
        let topics = [DEPOSIT_EVENT_SIGNATURE, user_topic];
        let mut data = [0u8; 64];
        data[0..32].copy_from_slice(&amount.to_be_bytes::<32>());
        data[32..64].copy_from_slice(&shares.to_be_bytes::<32>());
        api::deposit_event(&topics, &data);
    }

    #[pvmsafe::effect(emit)]
    fn emit_withdraw(user: &[u8; 20], amount: U256, shares: U256) {
        let mut user_topic = [0u8; 32];
        user_topic[12..32].copy_from_slice(user);
        let topics = [WITHDRAW_EVENT_SIGNATURE, user_topic];
        let mut data = [0u8; 64];
        data[0..32].copy_from_slice(&amount.to_be_bytes::<32>());
        data[32..64].copy_from_slice(&shares.to_be_bytes::<32>());
        api::deposit_event(&topics, &data);
    }
}
