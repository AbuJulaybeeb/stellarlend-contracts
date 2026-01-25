use super::*;
use crate::amm::*;
use soroban_sdk::{testutils::Address as _, Address, Env, Symbol, Vec};

fn create_amm_contract<'a>(env: &Env) -> AmmContractClient<'a> {
    AmmContractClient::new(env, &env.register(AmmContract {}, ()))
}

fn create_test_protocol_config(env: &Env, protocol_addr: &Address) -> AmmProtocolConfig {
    let mut supported_pairs = Vec::new(env);
    supported_pairs.push_back(TokenPair {
        token_a: None,                         // Native XLM
        token_b: Some(Address::generate(env)), // Mock USDC
        pool_address: Address::generate(env),
    });

    AmmProtocolConfig {
        protocol_address: protocol_addr.clone(),
        protocol_name: Symbol::new(env, "TestAMM"),
        enabled: true,
        fee_tier: 30, // 0.3%
        min_swap_amount: 1000,
        max_swap_amount: 1_000_000_000,
        supported_pairs,
    }
}

#[test]
fn test_initialize_amm_settings() {
    let env = Env::default();
    env.mock_all_auths();

    let contract = create_amm_contract(&env);
    let admin = Address::generate(&env);

    // Initialize AMM settings - this should not panic
    contract.initialize_amm_settings(
        &admin, &100,   // 1% default slippage
        &1000,  // 10% max slippage
        &10000, // 10000 auto-swap threshold
    );

    // Verify settings were stored
    let settings = contract.get_amm_settings();
    assert!(settings.is_some());
    let settings = settings.unwrap();
    assert_eq!(settings.default_slippage, 100);
    assert_eq!(settings.max_slippage, 1000);
    assert_eq!(settings.auto_swap_threshold, 10000);
    assert!(settings.swap_enabled);
    assert!(settings.liquidity_enabled);
}

#[test]
fn test_add_amm_protocol() {
    let env = Env::default();
    env.mock_all_auths();

    let contract = create_amm_contract(&env);
    let admin = Address::generate(&env);
    let protocol_addr = Address::generate(&env);

    // Initialize first
    contract.initialize_amm_settings(&admin, &100, &1000, &10000);

    // Create protocol config
    let protocol_config = create_test_protocol_config(&env, &protocol_addr);

    // Add protocol - this should not panic
    contract.add_amm_protocol(&admin, &protocol_config);

    // Verify protocol was added
    let protocols = contract.get_amm_protocols();
    assert!(protocols.is_some());
    let protocols = protocols.unwrap();
    assert!(protocols.contains_key(protocol_addr.clone()));
}

#[test]
fn test_update_amm_settings() {
    let env = Env::default();
    env.mock_all_auths();

    let contract = create_amm_contract(&env);
    let admin = Address::generate(&env);

    // Initialize
    contract.initialize_amm_settings(&admin, &100, &1000, &10000);

    // Update settings
    let new_settings = AmmSettings {
        default_slippage: 200,
        max_slippage: 2000,
        swap_enabled: false,
        liquidity_enabled: true,
        auto_swap_threshold: 20000,
    };

    contract.update_amm_settings(&admin, &new_settings);

    // Verify settings were updated
    let settings = contract.get_amm_settings().unwrap();
    assert_eq!(settings.default_slippage, 200);
    assert_eq!(settings.max_slippage, 2000);
    assert!(!settings.swap_enabled);
    assert!(settings.liquidity_enabled);
    assert_eq!(settings.auto_swap_threshold, 20000);
}

// Simplified tests that don't rely on complex swap logic
#[test]
fn test_get_amm_settings_empty() {
    let env = Env::default();
    env.mock_all_auths();

    let contract = create_amm_contract(&env);

    // Should return None when not initialized
    let settings = contract.get_amm_settings();
    assert!(settings.is_none());
}

#[test]
fn test_get_amm_protocols_empty() {
    let env = Env::default();
    env.mock_all_auths();

    let contract = create_amm_contract(&env);
    let admin = Address::generate(&env);

    // Initialize first
    contract.initialize_amm_settings(&admin, &100, &1000, &10000);

    // Should return empty map when no protocols added
    let protocols = contract.get_amm_protocols();
    assert!(protocols.is_some());
    let protocols = protocols.unwrap();
    assert_eq!(protocols.len(), 0);
}

#[test]
fn test_get_swap_history_empty() {
    let env = Env::default();
    env.mock_all_auths();

    let contract = create_amm_contract(&env);
    let user = Address::generate(&env);

    // Should return empty history when no swaps performed
    let history = contract.get_swap_history(&Some(user), &10);
    assert!(history.is_some());
    let history = history.unwrap();
    assert_eq!(history.len(), 0);
}

#[test]
fn test_get_liquidity_history_empty() {
    let env = Env::default();
    env.mock_all_auths();

    let contract = create_amm_contract(&env);
    let user = Address::generate(&env);

    // Should return empty history when no liquidity operations performed
    let history = contract.get_liquidity_history(&Some(user), &10);
    assert!(history.is_some());
    let history = history.unwrap();
    assert_eq!(history.len(), 0);
}

// Test basic contract functionality without complex operations
#[test]
fn test_contract_deployment() {
    let env = Env::default();
    env.mock_all_auths();

    // Should be able to create contract without errors
    let _contract = create_amm_contract(&env);
}

#[test]
fn test_multiple_protocol_registration() {
    let env = Env::default();
    env.mock_all_auths();

    let contract = create_amm_contract(&env);
    let admin = Address::generate(&env);

    // Initialize
    contract.initialize_amm_settings(&admin, &100, &1000, &10000);

    // Add multiple protocols
    let protocol1 = Address::generate(&env);
    let protocol2 = Address::generate(&env);

    let config1 = create_test_protocol_config(&env, &protocol1);
    let config2 = create_test_protocol_config(&env, &protocol2);

    contract.add_amm_protocol(&admin, &config1);
    contract.add_amm_protocol(&admin, &config2);

    // Verify both protocols were added
    let protocols = contract.get_amm_protocols().unwrap();
    assert_eq!(protocols.len(), 2);
    assert!(protocols.contains_key(protocol1));
    assert!(protocols.contains_key(protocol2));
}

#[test]
fn test_settings_persistence() {
    let env = Env::default();
    env.mock_all_auths();

    let contract = create_amm_contract(&env);
    let admin = Address::generate(&env);

    // Initialize with specific values
    contract.initialize_amm_settings(&admin, &150, &1500, &25000);

    // Verify values persist
    let settings = contract.get_amm_settings().unwrap();
    assert_eq!(settings.default_slippage, 150);
    assert_eq!(settings.max_slippage, 1500);
    assert_eq!(settings.auto_swap_threshold, 25000);
    assert!(settings.swap_enabled);
    assert!(settings.liquidity_enabled);
}
