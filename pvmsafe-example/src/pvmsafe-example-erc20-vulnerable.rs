#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

use pallet_revive_uapi::{HostFnImpl as api, StorageFlags};
use ruint::aliases::U256;

#[pvm_contract_macros::contract("ERC20Full.sol", allocator = "pico")]
mod erc20_vulnerable {
    use super::*;
    use alloc::vec;
    use pvm_contract_types::Address;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Error {
        InsufficientBalance,
        InsufficientAllowance,
    }

    impl AsRef<[u8]> for Error {
        fn as_ref(&self) -> &[u8] {
            match *self {
                Error::InsufficientBalance => b"InsufficientBalance",
                Error::InsufficientAllowance => b"InsufficientAllowance",
            }
        }
    }

    #[pvm_contract_macros::constructor]
    pub fn new() -> Result<(), Error> {
        Ok(())
    }

    #[pvm_contract_macros::method]
    pub fn total_supply() -> U256 {
        load_u256(&total_supply_key())
    }

    #[pvm_contract_macros::method]
    pub fn balance_of(account: Address) -> U256 {
        let account: [u8; 20] = account.into();
        load_u256(&balance_key(&account))
    }

    #[pvm_contract_macros::method]
    pub fn allowance(owner: Address, spender: Address) -> U256 {
        let owner: [u8; 20] = owner.into();
        let spender: [u8; 20] = spender.into();
        load_u256(&allowance_key(&owner, &spender))
    }

    #[pvm_contract_macros::method]
    pub fn transfer(to: Address, amount: U256) -> Result<(), Error> {
        let caller = get_caller();
        let sender_balance = load_u256(&balance_key(&caller));

        let new_sender_balance = sender_balance - amount;
        let to: [u8; 20] = to.into();
        let recipient_balance = load_u256(&balance_key(&to));
        let new_recipient_balance = recipient_balance.saturating_add(amount);

        store_u256(&balance_key(&caller), new_sender_balance);
        store_u256(&balance_key(&to), new_recipient_balance);

        emit_transfer(&caller, &to, amount);

        Ok(())
    }

    #[pvm_contract_macros::method]
    pub fn approve(spender: Address, amount: U256) -> Result<(), Error> {
        let caller = get_caller();
        let spender: [u8; 20] = spender.into();

        store_u256(&allowance_key(&caller, &spender), amount);
        emit_approval(&caller, &spender, amount);

        Ok(())
    }

    #[pvm_contract_macros::method]
    pub fn transfer_from(from: Address, to: Address, amount: U256) -> Result<(), Error> {
        let caller = get_caller();
        let from_arr: [u8; 20] = from.into();

        let current_allowance = load_u256(&allowance_key(&from_arr, &caller));
        let from_balance = load_u256(&balance_key(&from_arr));

        let new_allowance = current_allowance - amount;
        let new_from_balance = from_balance - amount;
        let to_arr: [u8; 20] = to.into();
        let to_balance = load_u256(&balance_key(&to_arr));
        let new_to_balance = to_balance.saturating_add(amount);

        store_u256(&allowance_key(&from_arr, &caller), new_allowance);
        store_u256(&balance_key(&from_arr), new_from_balance);
        store_u256(&balance_key(&to_arr), new_to_balance);

        emit_transfer(&from_arr, &to_arr, amount);

        Ok(())
    }

    #[pvm_contract_macros::method]
    pub fn mint(to: Address, amount: U256) -> Result<(), Error> {
        let to_arr: [u8; 20] = to.into();
        let new_recipient_balance = load_u256(&balance_key(&to_arr)).saturating_add(amount);
        let new_supply = load_u256(&total_supply_key()).saturating_add(amount);

        store_u256(&balance_key(&to_arr), new_recipient_balance);
        store_u256(&total_supply_key(), new_supply);

        emit_transfer(&[0u8; 20], &to_arr, amount);

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

    fn total_supply_key() -> [u8; 32] {
        [0u8; 32]
    }

    fn balance_key(addr: &[u8; 20]) -> [u8; 32] {
        let mut input = [0u8; 64];
        input[12..32].copy_from_slice(addr);
        input[63] = 1;

        let mut key = [0u8; 32];
        api::hash_keccak_256(&input, &mut key);
        key
    }

    fn allowance_key(owner: &[u8; 20], spender: &[u8; 20]) -> [u8; 32] {
        let mut input = [0u8; 96];
        input[12..32].copy_from_slice(owner);
        input[44..64].copy_from_slice(spender);
        input[95] = 2;

        let mut key = [0u8; 32];
        api::hash_keccak_256(&input, &mut key);
        key
    }

    fn get_caller() -> [u8; 20] {
        let mut caller = [0u8; 20];
        api::caller(&mut caller);
        caller
    }

    const TRANSFER_EVENT_SIGNATURE: [u8; 32] = [
        0xdd, 0xf2, 0x52, 0xad, 0x1b, 0xe2, 0xc8, 0x9b, 0x69, 0xc2, 0xb0, 0x68, 0xfc, 0x37, 0x8d,
        0xaa, 0x95, 0x2b, 0xa7, 0xf1, 0x63, 0xc4, 0xa1, 0x16, 0x28, 0xf5, 0x5a, 0x4d, 0xf5, 0x23,
        0xb3, 0xef,
    ];

    const APPROVAL_EVENT_SIGNATURE: [u8; 32] = [
        0x8c, 0x5b, 0xe1, 0xe5, 0xeb, 0xec, 0x7d, 0x5b, 0xd1, 0x4f, 0x71, 0x42, 0x7d, 0x1e, 0x84,
        0xf3, 0xdd, 0x03, 0x14, 0xc0, 0xf7, 0x92, 0x14, 0x58, 0xb2, 0x08, 0x45, 0x8c, 0xc2, 0xfc,
        0xe9, 0x25,
    ];

    fn emit_transfer(from: &[u8; 20], to: &[u8; 20], value: U256) {
        let mut from_topic = [0u8; 32];
        from_topic[12..32].copy_from_slice(from);

        let mut to_topic = [0u8; 32];
        to_topic[12..32].copy_from_slice(to);

        let topics = [TRANSFER_EVENT_SIGNATURE, from_topic, to_topic];
        let data = value.to_be_bytes::<32>();
        api::deposit_event(&topics, &data);
    }

    fn emit_approval(owner: &[u8; 20], spender: &[u8; 20], value: U256) {
        let mut owner_topic = [0u8; 32];
        owner_topic[12..32].copy_from_slice(owner);

        let mut spender_topic = [0u8; 32];
        spender_topic[12..32].copy_from_slice(spender);

        let topics = [APPROVAL_EVENT_SIGNATURE, owner_topic, spender_topic];
        let data = value.to_be_bytes::<32>();
        api::deposit_event(&topics, &data);
    }
}
