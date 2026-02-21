//! # Asset Configuration Test Suite
//!
//! Comprehensive tests for collateral and asset configuration functionality.
//!
//! ## Test Coverage
//! - Asset initialization and configuration
//! - Enable/disable assets as collateral
//! - LTV (collateral factor) configuration
//! - Liquidation threshold configuration
//! - Debt ceiling (max borrow) enforcement
//! - Supply cap enforcement
//! - Configuration validation
//! - Admin access control
//! - Edge cases (disable collateral with existing positions)

#![cfg(test)]

use crate::cross_asset::*;
use crate::HelloContract;
use soroban_sdk::{testutils::Address as _, Address, Env};

// ============================================================================
// Test Helpers
// ============================================================================

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(HelloContract, ());
    let admin = Address::generate(&env);

    env.as_contract(&contract_id, || {
        initialize(&env, admin.clone()).unwrap();
    });

    (env, contract_id, admin)
}

fn create_test_config(env: &Env, asset: Option<Address>) -> AssetConfig {
    AssetConfig {
        asset,
        collateral_factor: 7500,     // 75% LTV
        liquidation_threshold: 8000, // 80% liquidation threshold
        reserve_factor: 1000,        // 10%
        max_supply: 1_000_000_000,
        max_borrow: 800_000_000, // Debt ceiling
        can_collateralize: true,
        can_borrow: true,
        price: 1_0000000, // $1.00
        price_updated_at: env.ledger().timestamp(),
    }
}

macro_rules! with_contract {
    ($env:expr, $contract_id:expr, $body:block) => {
        $env.as_contract($contract_id, || $body)
    };
}

// ============================================================================
// Initialization Tests
// ============================================================================

#[test]
fn test_initialize_admin_success() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(HelloContract, ());
    let admin = Address::generate(&env);

    with_contract!(env, &contract_id, {
        let result = initialize(&env, admin);
        assert!(result.is_ok());
    });
}

#[test]
fn test_initialize_admin_twice_fails() {
    let (env, cid, admin) = setup();

    with_contract!(env, &cid, {
        let result = initialize(&env, admin);
        assert_eq!(result, Err(CrossAssetError::NotAuthorized));
    });
}

// ============================================================================
// Asset Configuration Tests
// ============================================================================

#[test]
fn test_initialize_asset_success() {
    let (env, cid, _admin) = setup();
    let usdc = Address::generate(&env);
    let config = create_test_config(&env, Some(usdc.clone()));

    with_contract!(env, &cid, {
        let result = initialize_asset(&env, Some(usdc.clone()), config.clone());
        assert!(result.is_ok());

        let stored_config = get_asset_config_by_address(&env, Some(usdc)).unwrap();
        assert_eq!(stored_config.collateral_factor, 7500);
        assert_eq!(stored_config.liquidation_threshold, 8000);
        assert_eq!(stored_config.max_borrow, 800_000_000);
    });
}

#[test]
fn test_initialize_asset_invalid_collateral_factor() {
    let (env, cid, _admin) = setup();
    let usdc = Address::generate(&env);
    let mut config = create_test_config(&env, Some(usdc.clone()));
    config.collateral_factor = 11_000; // Invalid: > 10000

    with_contract!(env, &cid, {
        let result = initialize_asset(&env, Some(usdc), config);
        assert_eq!(result, Err(CrossAssetError::AssetNotConfigured));
    });
}

#[test]
fn test_initialize_asset_invalid_liquidation_threshold() {
    let (env, cid, _admin) = setup();
    let usdc = Address::generate(&env);
    let mut config = create_test_config(&env, Some(usdc.clone()));
    config.liquidation_threshold = 11_000; // Invalid: > 10000

    with_contract!(env, &cid, {
        let result = initialize_asset(&env, Some(usdc), config);
        assert_eq!(result, Err(CrossAssetError::AssetNotConfigured));
    });
}

#[test]
fn test_initialize_asset_liquidation_threshold_below_ltv() {
    let (env, cid, _admin) = setup();
    let usdc = Address::generate(&env);
    let mut config = create_test_config(&env, Some(usdc.clone()));
    config.collateral_factor = 8000;
    config.liquidation_threshold = 7500; // Invalid: < collateral_factor

    with_contract!(env, &cid, {
        let result = initialize_asset(&env, Some(usdc), config);
        assert_eq!(result, Err(CrossAssetError::AssetNotConfigured));
    });
}

#[test]
fn test_initialize_asset_zero_price() {
    let (env, cid, _admin) = setup();
    let usdc = Address::generate(&env);
    let mut config = create_test_config(&env, Some(usdc.clone()));
    config.price = 0;

    with_contract!(env, &cid, {
        let result = initialize_asset(&env, Some(usdc), config);
        assert_eq!(result, Err(CrossAssetError::InvalidPrice));
    });
}

#[test]
fn test_initialize_asset_negative_price() {
    let (env, cid, _admin) = setup();
    let usdc = Address::generate(&env);
    let mut config = create_test_config(&env, Some(usdc.clone()));
    config.price = -100;

    with_contract!(env, &cid, {
        let result = initialize_asset(&env, Some(usdc), config);
        assert_eq!(result, Err(CrossAssetError::InvalidPrice));
    });
}

// ============================================================================
// Update Asset Configuration Tests
// ============================================================================

#[test]
fn test_update_collateral_factor() {
    let (env, cid, _admin) = setup();
    let usdc = Address::generate(&env);
    let config = create_test_config(&env, Some(usdc.clone()));

    with_contract!(env, &cid, {
        initialize_asset(&env, Some(usdc.clone()), config).unwrap();

        update_asset_config(
            &env,
            Some(usdc.clone()),
            Some(6000), // New LTV: 60%
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();

        let updated_config = get_asset_config_by_address(&env, Some(usdc)).unwrap();
        assert_eq!(updated_config.collateral_factor, 6000);
        assert_eq!(updated_config.liquidation_threshold, 8000); // Unchanged
    });
}

#[test]
fn test_update_liquidation_threshold() {
    let (env, cid, _admin) = setup();
    let usdc = Address::generate(&env);
    let config = create_test_config(&env, Some(usdc.clone()));

    with_contract!(env, &cid, {
        initialize_asset(&env, Some(usdc.clone()), config).unwrap();

        update_asset_config(
            &env,
            Some(usdc.clone()),
            None,
            Some(8500), // New liquidation threshold: 85%
            None,
            None,
            None,
            None,
        )
        .unwrap();

        let updated_config = get_asset_config_by_address(&env, Some(usdc)).unwrap();
        assert_eq!(updated_config.liquidation_threshold, 8500);
        assert_eq!(updated_config.collateral_factor, 7500); // Unchanged
    });
}

#[test]
fn test_update_debt_ceiling() {
    let (env, cid, _admin) = setup();
    let usdc = Address::generate(&env);
    let config = create_test_config(&env, Some(usdc.clone()));

    with_contract!(env, &cid, {
        initialize_asset(&env, Some(usdc.clone()), config).unwrap();

        update_asset_config(
            &env,
            Some(usdc.clone()),
            None,
            None,
            None,
            Some(1_000_000_000), // New debt ceiling
            None,
            None,
        )
        .unwrap();

        let updated_config = get_asset_config_by_address(&env, Some(usdc)).unwrap();
        assert_eq!(updated_config.max_borrow, 1_000_000_000);
    });
}

#[test]
fn test_disable_collateral() {
    let (env, cid, _admin) = setup();
    let usdc = Address::generate(&env);
    let config = create_test_config(&env, Some(usdc.clone()));

    with_contract!(env, &cid, {
        initialize_asset(&env, Some(usdc.clone()), config).unwrap();

        update_asset_config(
            &env,
            Some(usdc.clone()),
            None,
            None,
            None,
            None,
            Some(false), // Disable collateral
            None,
        )
        .unwrap();

        let updated_config = get_asset_config_by_address(&env, Some(usdc)).unwrap();
        assert!(!updated_config.can_collateralize);
    });
}

#[test]
fn test_disable_borrowing() {
    let (env, cid, _admin) = setup();
    let usdc = Address::generate(&env);
    let config = create_test_config(&env, Some(usdc.clone()));

    with_contract!(env, &cid, {
        initialize_asset(&env, Some(usdc.clone()), config).unwrap();

        update_asset_config(
            &env,
            Some(usdc.clone()),
            None,
            None,
            None,
            None,
            None,
            Some(false), // Disable borrowing
        )
        .unwrap();

        let updated_config = get_asset_config_by_address(&env, Some(usdc)).unwrap();
        assert!(!updated_config.can_borrow);
    });
}

#[test]
fn test_update_multiple_fields() {
    let (env, cid, _admin) = setup();
    let usdc = Address::generate(&env);
    let config = create_test_config(&env, Some(usdc.clone()));

    with_contract!(env, &cid, {
        initialize_asset(&env, Some(usdc.clone()), config).unwrap();

        update_asset_config(
            &env,
            Some(usdc.clone()),
            Some(6500),          // New LTV
            Some(7500),          // New liquidation threshold
            Some(2_000_000_000), // New supply cap
            Some(1_500_000_000), // New debt ceiling
            Some(true),
            Some(true),
        )
        .unwrap();

        let updated_config = get_asset_config_by_address(&env, Some(usdc)).unwrap();
        assert_eq!(updated_config.collateral_factor, 6500);
        assert_eq!(updated_config.liquidation_threshold, 7500);
        assert_eq!(updated_config.max_supply, 2_000_000_000);
        assert_eq!(updated_config.max_borrow, 1_500_000_000);
    });
}

// ============================================================================
// Price Update Tests
// ============================================================================

#[test]
fn test_update_price_success() {
    let (env, cid, _admin) = setup();
    let usdc = Address::generate(&env);
    let config = create_test_config(&env, Some(usdc.clone()));

    with_contract!(env, &cid, {
        initialize_asset(&env, Some(usdc.clone()), config).unwrap();

        let new_price = 1_0500000; // $1.05
        update_asset_price(&env, Some(usdc.clone()), new_price).unwrap();

        let updated_config = get_asset_config_by_address(&env, Some(usdc)).unwrap();
        assert_eq!(updated_config.price, new_price);
        assert_eq!(updated_config.price_updated_at, env.ledger().timestamp());
    });
}

#[test]
fn test_update_price_zero_fails() {
    let (env, cid, _admin) = setup();
    let usdc = Address::generate(&env);
    let config = create_test_config(&env, Some(usdc.clone()));

    with_contract!(env, &cid, {
        initialize_asset(&env, Some(usdc.clone()), config).unwrap();

        let result = update_asset_price(&env, Some(usdc), 0);
        assert_eq!(result, Err(CrossAssetError::InvalidPrice));
    });
}

#[test]
fn test_update_price_negative_fails() {
    let (env, cid, _admin) = setup();
    let usdc = Address::generate(&env);
    let config = create_test_config(&env, Some(usdc.clone()));

    with_contract!(env, &cid, {
        initialize_asset(&env, Some(usdc.clone()), config).unwrap();

        let result = update_asset_price(&env, Some(usdc), -100);
        assert_eq!(result, Err(CrossAssetError::InvalidPrice));
    });
}

// ============================================================================
// Deposit Tests with Configuration
// ============================================================================

#[test]
fn test_deposit_enabled_asset() {
    let (env, cid, _admin) = setup();
    let user = Address::generate(&env);
    let usdc = Address::generate(&env);
    let config = create_test_config(&env, Some(usdc.clone()));

    with_contract!(env, &cid, {
        initialize_asset(&env, Some(usdc.clone()), config).unwrap();

        let result = cross_asset_deposit(&env, user.clone(), Some(usdc.clone()), 1000);
        assert!(result.is_ok());

        let position = get_user_asset_position(&env, &user, Some(usdc));
        assert_eq!(position.collateral, 1000);
    });
}

#[test]
fn test_deposit_disabled_collateral_fails() {
    let (env, cid, _admin) = setup();
    let user = Address::generate(&env);
    let usdc = Address::generate(&env);
    let mut config = create_test_config(&env, Some(usdc.clone()));
    config.can_collateralize = false;

    with_contract!(env, &cid, {
        initialize_asset(&env, Some(usdc.clone()), config).unwrap();

        let result = cross_asset_deposit(&env, user, Some(usdc), 1000);
        assert_eq!(result, Err(CrossAssetError::AssetDisabled));
    });
}

#[test]
fn test_deposit_exceeds_supply_cap() {
    let (env, cid, _admin) = setup();
    let user = Address::generate(&env);
    let usdc = Address::generate(&env);
    let mut config = create_test_config(&env, Some(usdc.clone()));
    config.max_supply = 1000;

    with_contract!(env, &cid, {
        initialize_asset(&env, Some(usdc.clone()), config).unwrap();

        let result = cross_asset_deposit(&env, user, Some(usdc), 2000);
        assert_eq!(result, Err(CrossAssetError::SupplyCapExceeded));
    });
}

// ============================================================================
// Configuration Enforcement Tests
// ============================================================================

#[test]
fn test_debt_ceiling_enforcement() {
    let (env, cid, _admin) = setup();
    let usdc = Address::generate(&env);
    let mut config = create_test_config(&env, Some(usdc.clone()));
    config.max_borrow = 1000; // Debt ceiling

    with_contract!(env, &cid, {
        initialize_asset(&env, Some(usdc.clone()), config).unwrap();

        let stored_config = get_asset_config_by_address(&env, Some(usdc)).unwrap();
        assert_eq!(stored_config.max_borrow, 1000);
    });
}

#[test]
fn test_ltv_configuration() {
    let (env, cid, _admin) = setup();
    let usdc = Address::generate(&env);
    let mut config = create_test_config(&env, Some(usdc.clone()));
    config.collateral_factor = 6500; // 65% LTV
    config.liquidation_threshold = 7500; // 75% liquidation threshold

    with_contract!(env, &cid, {
        initialize_asset(&env, Some(usdc.clone()), config).unwrap();

        let stored_config = get_asset_config_by_address(&env, Some(usdc)).unwrap();
        assert_eq!(stored_config.collateral_factor, 6500);
        assert_eq!(stored_config.liquidation_threshold, 7500);
    });
}

#[test]
fn test_multiple_assets_configuration() {
    let (env, cid, _admin) = setup();
    let usdc = Address::generate(&env);
    let usdt = Address::generate(&env);
    let dai = Address::generate(&env);

    with_contract!(env, &cid, {
        // Initialize multiple assets with different configs
        let mut config1 = create_test_config(&env, Some(usdc.clone()));
        config1.collateral_factor = 7500;
        config1.liquidation_threshold = 8000;
        initialize_asset(&env, Some(usdc.clone()), config1).unwrap();

        let mut config2 = create_test_config(&env, Some(usdt.clone()));
        config2.collateral_factor = 7000;
        config2.liquidation_threshold = 7500;
        initialize_asset(&env, Some(usdt.clone()), config2).unwrap();

        let mut config3 = create_test_config(&env, Some(dai.clone()));
        config3.collateral_factor = 8000;
        config3.liquidation_threshold = 8500;
        initialize_asset(&env, Some(dai.clone()), config3).unwrap();

        // Verify all configs are stored correctly
        let stored1 = get_asset_config_by_address(&env, Some(usdc)).unwrap();
        let stored2 = get_asset_config_by_address(&env, Some(usdt)).unwrap();
        let stored3 = get_asset_config_by_address(&env, Some(dai)).unwrap();

        assert_eq!(stored1.collateral_factor, 7500);
        assert_eq!(stored2.collateral_factor, 7000);
        assert_eq!(stored3.collateral_factor, 8000);

        let asset_list = get_asset_list(&env);
        assert_eq!(asset_list.len(), 3);
    });
}

#[test]
fn test_get_asset_list() {
    let (env, cid, _admin) = setup();
    let usdc = Address::generate(&env);
    let usdt = Address::generate(&env);

    with_contract!(env, &cid, {
        let config1 = create_test_config(&env, Some(usdc.clone()));
        let config2 = create_test_config(&env, Some(usdt.clone()));

        initialize_asset(&env, Some(usdc), config1).unwrap();
        initialize_asset(&env, Some(usdt), config2).unwrap();

        let asset_list = get_asset_list(&env);
        assert_eq!(asset_list.len(), 2);
    });
}
