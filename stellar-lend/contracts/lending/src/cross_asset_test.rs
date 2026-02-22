#![cfg(test)]

use super::*;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env};

fn setup_test(env: &Env) -> (LendingContractClient<'static>, Address, Address, Address) {
    let admin = Address::generate(env);
    let user = Address::generate(env);
    let asset1 = Address::generate(env);
    let _asset2 = Address::generate(env);

    let contract_id = env.register_contract(None, LendingContract);
    let client = LendingContractClient::new(env, &contract_id);

    client.initialize_admin(&admin);

    (client, admin, user, asset1)
}

#[test]
fn test_set_asset_params() {
    let env = Env::default();
    let (client, _admin, _, asset1) = setup_test(&env);

    let params = AssetParams {
        ltv: 8000,
        liquidation_threshold: 8500,
        price_feed: Address::generate(&env),
        debt_ceiling: 1000000,
        is_active: true,
    };

    env.mock_all_auths();
    client.set_asset_params(&asset1, &params);
}

#[test]
fn test_deposit_and_summary() {
    let env = Env::default();
    let (client, _admin, user, asset1) = setup_test(&env);

    let params = AssetParams {
        ltv: 8000,
        liquidation_threshold: 8500,
        price_feed: Address::generate(&env),
        debt_ceiling: 1000000,
        is_active: true,
    };

    env.mock_all_auths();
    client.set_asset_params(&asset1, &params);

    client.deposit_collateral_asset(&user, &asset1, &1000);

    let summary = client.get_cross_position_summary(&user);
    assert_eq!(summary.total_collateral_usd, 1000);
    assert_eq!(summary.total_debt_usd, 0);
    assert!(summary.health_factor >= 10000);
}

#[test]
fn test_borrow_success() {
    let env = Env::default();
    let (client, _admin, user, asset1) = setup_test(&env);

    let params = AssetParams {
        ltv: 8000, // 80%
        liquidation_threshold: 8500,
        price_feed: Address::generate(&env),
        debt_ceiling: 1000000,
        is_active: true,
    };

    env.mock_all_auths();
    client.set_asset_params(&asset1, &params);

    client.deposit_collateral_asset(&user, &asset1, &2000); // $2000 collateral
    // Max borrow = 2000 * 0.8 = 1600
    
    client.borrow_asset(&user, &asset1, &1000); // $1000 borrow

    let summary = client.get_cross_position_summary(&user);
    assert_eq!(summary.total_collateral_usd, 2000);
    assert_eq!(summary.total_debt_usd, 1000);
    // Health factor = (2000 * 0.8) / 1000 = 1.6 (16000)
    assert_eq!(summary.health_factor, 16000);
}

#[test]
#[should_panic(expected = "InsufficientCollateral")]
fn test_borrow_insufficient_collateral() {
    let env = Env::default();
    let (client, _admin, user, asset1) = setup_test(&env);

    let params = AssetParams {
        ltv: 5000, // 50%
        liquidation_threshold: 6000,
        price_feed: Address::generate(&env),
        debt_ceiling: 1000000,
        is_active: true,
    };

    env.mock_all_auths();
    client.set_asset_params(&asset1, &params);

    client.deposit_collateral_asset(&user, &asset1, &1000); // $1000 collateral
    // Max borrow = 1000 * 0.5 = 500
    
    client.borrow_asset(&user, &asset1, &600); // Should panic
}

#[test]
fn test_repay_and_withdraw() {
    let env = Env::default();
    let (client, _admin, user, asset1) = setup_test(&env);

    let params = AssetParams {
        ltv: 8000,
        liquidation_threshold: 8500,
        price_feed: Address::generate(&env),
        debt_ceiling: 1000000,
        is_active: true,
    };

    env.mock_all_auths();
    client.set_asset_params(&asset1, &params);

    client.deposit_collateral_asset(&user, &asset1, &1000);
    client.borrow_asset(&user, &asset1, &500);
    
    client.repay_asset(&user, &asset1, &500);
    
    let summary = client.get_cross_position_summary(&user);
    assert_eq!(summary.total_debt_usd, 0);

    client.withdraw_asset(&user, &asset1, &1000);
    let summary2 = client.get_cross_position_summary(&user);
    assert_eq!(summary2.total_collateral_usd, 0);
}
