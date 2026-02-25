#![cfg(test)]
extern crate std;

use super::*;
use soroban_sdk::{testutils::{Address as _, Events}, Address, Env, Vec, symbol_short, IntoVal};
use crate::{HelloContract, HelloContractClient};
use crate::bridge::{BridgeError};
use crate::cross_asset::{AssetConfig, initialize as init_cross_asset, initialize_asset};

fn setup_test_env() -> (Env, HelloContractClient<'static>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    
    let contract_id = env.register_contract(None, HelloContract);
    let client = HelloContractClient::new(&env, &contract_id);
    
    // Initialize cross_asset (admin state)
    env.as_contract(&contract_id, || {
        init_cross_asset(&env, admin.clone()).unwrap();
    });
    
    (env, client, admin, user)
}

#[test]
fn test_register_bridge() {
    let (env, client, admin, _user) = setup_test_env();
    let bridge_addr = Address::generate(&env);
    
    client.register_bridge(&admin, &1u32, &bridge_addr, &100i128);
    
    let config = client.get_bridge_config(&1u32);
    assert_eq!(config.bridge_address, bridge_addr);
    assert_eq!(config.fee_bps, 100);
    assert!(config.is_active);
    
    let bridges = client.list_bridges();
    assert_eq!(bridges.len(), 1);
}

#[test]
#[should_panic(expected = "Error(Contract, #2)")] // BridgeAlreadyExists
fn test_register_duplicate_bridge() {
    let (env, client, admin, _user) = setup_test_env();
    let bridge_addr = Address::generate(&env);
    
    client.register_bridge(&admin, &1u32, &bridge_addr, &100i128);
    client.register_bridge(&admin, &1u32, &bridge_addr, &100i128);
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")] // InvalidFee
fn test_register_bridge_invalid_fee() {
    let (env, client, admin, _user) = setup_test_env();
    let bridge_addr = Address::generate(&env);
    
    client.register_bridge(&admin, &1u32, &bridge_addr, &10001i128);
}

#[test]
#[should_panic(expected = "Error(Contract, #1)")] // NotAuthorized
fn test_register_bridge_unauthorized() {
    let (env, client, _admin, user) = setup_test_env();
    let bridge_addr = Address::generate(&env);
    
    client.register_bridge(&user, &1u32, &bridge_addr, &100i128);
}

#[test]
fn test_set_bridge_fee() {
    let (env, client, admin, _user) = setup_test_env();
    let bridge_addr = Address::generate(&env);
    
    client.register_bridge(&admin, &1u32, &bridge_addr, &100i128);
    client.set_bridge_fee(&admin, &1u32, &200i128);
    
    let config = client.get_bridge_config(&1u32);
    assert_eq!(config.fee_bps, 200);
}

#[test]
fn test_bridge_deposit_withdraw() {
    let (env, client, admin, user) = setup_test_env();
    let bridge_addr = Address::generate(&env);
    let asset = Address::generate(&env);
    
    // Configure an asset
    env.as_contract(&client.address, || {
        let config = AssetConfig {
            asset: Some(asset.clone()),
            collateral_factor: 7500,
            borrow_factor: 8000,
            reserve_factor: 1000,
            max_supply: 1_000_000,
            max_borrow: 1_000_000,
            can_collateralize: true,
            can_borrow: true,
            price: 1_000_000,
            price_updated_at: env.ledger().timestamp(),
        };
        initialize_asset(&env, Some(asset.clone()), config).unwrap();
    });
    
    client.register_bridge(&admin, &1u32, &bridge_addr, &100i128); // 1% fee
    
    // Deposit 10,000, fee is 100, deposit amount 9900
    let deposited = client.bridge_deposit(&user, &1u32, &Some(asset.clone()), &10000i128);
    assert_eq!(deposited, 9900);
    
    // Withdraw 5000, fee is 50, withdraw amount 4950
    let withdrawn = client.bridge_withdraw(&user, &1u32, &Some(asset.clone()), &5000i128);
    assert_eq!(withdrawn, 4950);
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")] // BridgeNotFound
fn test_deposit_unknown_bridge() {
    let (env, client, _admin, user) = setup_test_env();
    let asset = Address::generate(&env);
    
    client.bridge_deposit(&user, &99u32, &Some(asset), &10000i128);
}
