use crate::{HelloContract, HelloContractClient, deposit::DepositDataKey, deposit::AssetParams};
use soroban_sdk::{testutils::{Address as _, Ledger}, Address, Env};

/// Helper function to create a test environment
fn create_test_env() -> Env {
    let env = Env::default();
    env.mock_all_auths();
    env
}

#[test]
fn test_borrow_fee_collection() {
    let env = create_test_env();
    let contract_id = env.register(HelloContract, ());
    let client = HelloContractClient::new(&env, &contract_id);
    
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let asset = Address::generate(&env);
    
    client.initialize(&admin);
    
    // Setup asset params with 2% borrow fee (200 bps)
    env.as_contract(&contract_id, || {
        let params = AssetParams {
            deposit_enabled: true,
            collateral_factor: 7000,
            max_deposit: 0,
            borrow_fee_bps: 200,
        };
        env.storage().persistent().set(&DepositDataKey::AssetParams(asset.clone()), &params);
        
        let position = crate::deposit::Position {
            collateral: 10000,
            debt: 0,
            borrow_interest: 0,
            last_accrual_time: env.ledger().timestamp(),
        };
        env.storage().persistent().set(&DepositDataKey::Position(user.clone()), &position);
        env.storage().persistent().set(&DepositDataKey::CollateralBalance(user.clone()), &10000i128);
    });
    
    client.borrow_asset(&user, &Some(asset.clone()), &1000);
    
    let reserve_balance = client.get_reserve_balance(&Some(asset.clone()));
    assert_eq!(reserve_balance, 20);
}

#[test]
fn test_interest_reserve_factor() {
    let env = create_test_env();
    let contract_id = env.register(HelloContract, ());
    let client = HelloContractClient::new(&env, &contract_id);
    
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let asset = Address::generate(&env);
    
    client.initialize(&admin);
    
    env.as_contract(&contract_id, || {
        let params = AssetParams {
            deposit_enabled: true,
            collateral_factor: 7000,
            max_deposit: 0,
            borrow_fee_bps: 0,
        };
        env.storage().persistent().set(&DepositDataKey::AssetParams(asset.clone()), &params);
        
        let position = crate::deposit::Position {
            collateral: 10000,
            debt: 1000,
            borrow_interest: 100,
            last_accrual_time: env.ledger().timestamp(),
        };
        env.storage().persistent().set(&DepositDataKey::Position(user.clone()), &position);
    });
    
    client.repay_debt(&user, &Some(asset.clone()), &100);
    
    let reserve_balance = client.get_reserve_balance(&Some(asset.clone()));
    assert_eq!(reserve_balance, 10);
}

#[test]
fn test_admin_claim_reserves() {
    let env = create_test_env();
    let contract_id = env.register(HelloContract, ());
    let client = HelloContractClient::new(&env, &contract_id);
    
    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let asset = Address::generate(&env);
    
    client.initialize(&admin);
    
    // Mock reserve balance
    env.as_contract(&contract_id, || {
        env.storage().persistent().set(&DepositDataKey::ProtocolReserve(Some(asset.clone())), &500i128);
    });
    
    assert_eq!(client.get_reserve_balance(&Some(asset.clone())), 500);
    
    // Claim 200
    // Note: claim_reserves also calls token.transfer which we skip in tests
    client.claim_reserves(&admin, &Some(asset.clone()), &treasury, &200);
    
    assert_eq!(client.get_reserve_balance(&Some(asset.clone())), 300);
}
