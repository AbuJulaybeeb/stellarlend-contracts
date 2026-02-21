//! # Risk Management Parameters Test Suite
//!
//! Comprehensive tests for risk parameter configuration and enforcement (#290).
//!
//! ## Test scenarios
//!
//! - **Set/Get params**: Initialize, set risk params (full and partial), verify get_risk_config and individual getters.
//! - **Bounds**: Min/max for min_collateral_ratio, liquidation_threshold, close_factor, liquidation_incentive.
//! - **Validation**: min_cr >= liquidation_threshold, 10% max change per update, InvalidParameter / ParameterChangeTooLarge.
//! - **Enforcement**: require_min_collateral_ratio, can_be_liquidated, get_max_liquidatable_amount, get_liquidation_incentive_amount.
//! - **Admin-only**: set_risk_params, set_pause_switch, set_emergency_pause reject non-admin (Unauthorized).
//! - **Edge values**: Boundary values (exactly at min/max), zero debt, partial updates.
//! - **Pause**: Operation pause switches and emergency pause; emergency pause blocks set_risk_params.
//!
//! ## Security assumptions validated
//!
//! - Only admin can change risk params and pause state.
//! - Parameter changes are capped at ±10% per update.
//! - Min collateral ratio must be >= liquidation threshold.
//! - Close factor in [0, 100%], liquidation incentive in [0, 50%].

use crate::{HelloContract, HelloContractClient};
use soroban_sdk::{testutils::Address as _, Address, Env, Symbol};

// =============================================================================
// HELPERS
// =============================================================================

/// Creates a test environment with all auths mocked.
fn create_test_env() -> Env {
    let env = Env::default();
    env.mock_all_auths();
    env
}

/// Sets up contract and admin, initializes. Pass env from create_test_env() so client outlives env.
fn setup(env: &Env) -> (Address, Address, HelloContractClient<'_>) {
    let contract_id = env.register(HelloContract, ());
    let client = HelloContractClient::new(env, &contract_id);
    let admin = Address::generate(env);
    client.initialize(&admin);
    (contract_id, admin, client)
}

// =============================================================================
// SET / GET PARAMS
// =============================================================================

/// After initialize, get_risk_config returns defaults and getters match.
#[test]
fn risk_params_get_after_initialize() {
    let env = create_test_env();
    let (_cid, _admin, client) = setup(&env);

    let config = client.get_risk_config().expect("config should exist");
    assert_eq!(
        config.min_collateral_ratio, 11_000,
        "min_collateral_ratio 110%"
    );
    assert_eq!(
        config.liquidation_threshold, 10_500,
        "liquidation_threshold 105%"
    );
    assert_eq!(config.close_factor, 5_000, "close_factor 50%");
    assert_eq!(
        config.liquidation_incentive, 1_000,
        "liquidation_incentive 10%"
    );

    assert_eq!(client.get_min_collateral_ratio(), 11_000);
    assert_eq!(client.get_liquidation_threshold(), 10_500);
    assert_eq!(client.get_close_factor(), 5_000);
    assert_eq!(client.get_liquidation_incentive(), 1_000);
}

/// Set all risk params within 10% change limit; get_risk_config and getters reflect new values.
#[test]
fn risk_params_set_all_and_get() {
    let env = create_test_env();
    let (_cid, admin, client) = setup(&env);

    client.set_risk_params(
        &admin,
        &Some(12_000),
        &Some(11_000),
        &Some(5_500),
        &Some(1_100),
    );

    let config = client.get_risk_config().unwrap();
    assert_eq!(config.min_collateral_ratio, 12_000);
    assert_eq!(config.liquidation_threshold, 11_000);
    assert_eq!(config.close_factor, 5_500);
    assert_eq!(config.liquidation_incentive, 1_100);

    assert_eq!(client.get_min_collateral_ratio(), 12_000);
    assert_eq!(client.get_liquidation_threshold(), 11_000);
    assert_eq!(client.get_close_factor(), 5_500);
    assert_eq!(client.get_liquidation_incentive(), 1_100);
}

/// Partial update: only min_collateral_ratio set; others unchanged.
#[test]
fn risk_params_set_partial_min_cr_only() {
    let env = create_test_env();
    let (_cid, admin, client) = setup(&env);

    client.set_risk_params(&admin, &Some(12_000), &None, &None, &None);

    assert_eq!(client.get_min_collateral_ratio(), 12_000);
    assert_eq!(client.get_liquidation_threshold(), 10_500);
    assert_eq!(client.get_close_factor(), 5_000);
    assert_eq!(client.get_liquidation_incentive(), 1_000);
}

/// Partial update: only liquidation_threshold set.
#[test]
fn risk_params_set_partial_liq_threshold_only() {
    let env = create_test_env();
    let (_cid, admin, client) = setup(&env);

    client.set_risk_params(&admin, &None, &Some(11_000), &None, &None);

    assert_eq!(client.get_min_collateral_ratio(), 11_000);
    assert_eq!(client.get_liquidation_threshold(), 11_000);
}

/// Partial update: only close_factor and liquidation_incentive set.
#[test]
fn risk_params_set_partial_close_factor_and_incentive() {
    let env = create_test_env();
    let (_cid, admin, client) = setup(&env);

    client.set_risk_params(&admin, &None, &None, &Some(4_500), &Some(900));

    assert_eq!(client.get_close_factor(), 4_500);
    assert_eq!(client.get_liquidation_incentive(), 900);
}

// =============================================================================
// BOUNDS AND VALIDATION
// =============================================================================

/// set_risk_params by non-admin returns Unauthorized (Contract #1).
#[test]
#[should_panic(expected = "Error(Contract, #1)")]
fn risk_params_set_unauthorized() {
    let env = create_test_env();
    let (_cid, _admin, client) = setup(&env);
    let non_admin = Address::generate(&env);
    client.set_risk_params(&non_admin, &Some(12_000), &None, &None, &None);
}

/// Min collateral ratio below allowed minimum (10_000) or change too large leads to error.
/// Here we use a large increase to hit ParameterChangeTooLarge (#3).
#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn risk_params_set_change_too_large_min_cr() {
    let env = create_test_env();
    let (_cid, admin, client) = setup(&env);
    // Default 11_000; 10% max change = 1_100; 15_000 is +4_000
    client.set_risk_params(&admin, &Some(15_000), &None, &None, &None);
}

/// Min collateral ratio below liquidation threshold returns InvalidCollateralRatio (#7).
#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn risk_params_set_min_cr_below_liquidation_threshold() {
    let env = create_test_env();
    let (_cid, admin, client) = setup(&env);
    client.set_risk_params(
        &admin,
        &Some(10_000),
        &Some(10_500), // threshold > min_cr
        &None,
        &None,
    );
}

/// Close factor above 100% (10_001 bps) fails validation. Change from 5_000 is 5_001 > 500 (10%), so ParameterChangeTooLarge first.
#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn risk_params_set_close_factor_over_max() {
    let env = create_test_env();
    let (_cid, admin, client) = setup(&env);
    client.set_risk_params(&admin, &None, &None, &Some(10_001), &None);
}

/// Liquidation incentive above 50% (5_001 bps) fails; large change triggers ParameterChangeTooLarge.
#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn risk_params_set_liquidation_incentive_over_max() {
    let env = create_test_env();
    let (_cid, admin, client) = setup(&env);
    client.set_risk_params(&admin, &None, &None, &None, &Some(5_001));
}

/// Multiple steps within 10% each can reach new target (e.g. min_cr from 11_000 to 13_000 in two steps).
#[test]
fn risk_params_multiple_steps_within_change_limit() {
    let env = create_test_env();
    let (_cid, admin, client) = setup(&env);

    // 11_000 -> 12_100 (10% increase)
    client.set_risk_params(&admin, &Some(12_100), &None, &None, &None);
    assert_eq!(client.get_min_collateral_ratio(), 12_100);

    // 12_100 -> 13_310 (10% increase)
    client.set_risk_params(&admin, &Some(13_310), &None, &None, &None);
    assert_eq!(client.get_min_collateral_ratio(), 13_310);
}

// =============================================================================
// ENFORCEMENT: require_min_collateral_ratio, can_be_liquidated, close_factor, incentive
// =============================================================================

/// require_min_collateral_ratio: passes when ratio >= min_cr, fails when below.
#[test]
fn risk_params_enforcement_require_min_collateral_ratio() {
    let env = create_test_env();
    let (_cid, _admin, client) = setup(&env);

    // Default min_cr 110%. collateral=1100, debt=1000 -> 110% ok
    client.require_min_collateral_ratio(&1_100, &1_000);
    // No debt always ok
    client.require_min_collateral_ratio(&1_000, &0);
}

/// require_min_collateral_ratio: fails with InsufficientCollateralRatio (#4) when ratio below min_cr.
#[test]
#[should_panic(expected = "Error(Contract, #4)")]
fn risk_params_enforcement_require_min_collateral_ratio_fail() {
    let env = create_test_env();
    let (_cid, _admin, client) = setup(&env);
    // 100% < 110%
    client.require_min_collateral_ratio(&1_000, &1_000);
}

/// After raising min_cr to 120%, require_min_collateral_ratio(1150, 1000) fails (115% < 120%).
#[test]
#[should_panic(expected = "Error(Contract, #4)")]
fn risk_params_enforcement_require_min_cr_after_param_change() {
    let env = create_test_env();
    let (_cid, admin, client) = setup(&env);
    client.set_risk_params(&admin, &Some(12_000), &None, &None, &None);
    client.require_min_collateral_ratio(&1_150, &1_000);
}

/// can_be_liquidated: true when ratio < liquidation_threshold, false when >= or debt zero.
#[test]
fn risk_params_enforcement_can_be_liquidated() {
    let env = create_test_env();
    let (_cid, _admin, client) = setup(&env);

    assert!(client.can_be_liquidated(&1_000, &1_000));
    assert!(client.can_be_liquidated(&1_040, &1_000));
    assert!(!client.can_be_liquidated(&1_050, &1_000));
    assert!(!client.can_be_liquidated(&1_100, &1_000));
    assert!(!client.can_be_liquidated(&1_000, &0));
}

/// After raising liquidation_threshold to 115%, can_be_liquidated(1100, 1000) becomes true.
#[test]
fn risk_params_enforcement_can_be_liquidated_after_threshold_change() {
    let env = create_test_env();
    let (_cid, admin, client) = setup(&env);
    client.set_risk_params(&admin, &Some(12_000), &Some(11_500), &None, &None);
    // 110% < 115% threshold
    assert!(client.can_be_liquidated(&1_100, &1_000));
}

/// get_max_liquidatable_amount respects close_factor (default 50%).
#[test]
fn risk_params_enforcement_max_liquidatable_amount() {
    let env = create_test_env();
    let (_cid, admin, client) = setup(&env);

    assert_eq!(client.get_max_liquidatable_amount(&1_000), 500);
    // 50% -> 55% (10% increase)
    client.set_risk_params(&admin, &None, &None, &Some(5_500), &None);
    assert_eq!(client.get_max_liquidatable_amount(&1_000), 550);
    // 55% -> 49.5% (10% decrease from 5_500 = 550, so 4_950)
    client.set_risk_params(&admin, &None, &None, &Some(4_950), &None);
    assert_eq!(client.get_max_liquidatable_amount(&1_000), 495);
}

/// get_liquidation_incentive_amount respects liquidation_incentive (default 10%).
#[test]
fn risk_params_enforcement_liquidation_incentive_amount() {
    let env = create_test_env();
    let (_cid, admin, client) = setup(&env);

    assert_eq!(client.get_liquidation_incentive_amount(&1_000), 100);
    client.set_risk_params(&admin, &None, &None, &None, &Some(1_100));
    assert_eq!(client.get_liquidation_incentive_amount(&1_000), 110);
}

// =============================================================================
// ADMIN-ONLY: PAUSE SWITCHES AND EMERGENCY PAUSE
// =============================================================================

/// set_pause_switch as admin succeeds; is_operation_paused reflects state.
#[test]
fn risk_params_pause_switch_admin_success() {
    let env = create_test_env();
    let (_cid, admin, client) = setup(&env);
    let sym = Symbol::new(&env, "pause_borrow");

    assert!(!client.is_operation_paused(&sym));
    client.set_pause_switch(&admin, &sym, &true);
    assert!(client.is_operation_paused(&sym));
    client.set_pause_switch(&admin, &sym, &false);
    assert!(!client.is_operation_paused(&sym));
}

/// set_pause_switch by non-admin panics with Unauthorized (#1).
#[test]
#[should_panic(expected = "Error(Contract, #1)")]
fn risk_params_pause_switch_unauthorized() {
    let env = create_test_env();
    let (_cid, _admin, client) = setup(&env);
    let non_admin = Address::generate(&env);
    client.set_pause_switch(&non_admin, &Symbol::new(&env, "pause_deposit"), &true);
}

/// set_emergency_pause as admin succeeds; is_emergency_paused reflects state.
#[test]
fn risk_params_emergency_pause_admin_success() {
    let env = create_test_env();
    let (_cid, admin, client) = setup(&env);

    assert!(!client.is_emergency_paused());
    client.set_emergency_pause(&admin, &true);
    assert!(client.is_emergency_paused());
    client.set_emergency_pause(&admin, &false);
    assert!(!client.is_emergency_paused());
}

/// set_emergency_pause by non-admin panics with Unauthorized (#1).
#[test]
#[should_panic(expected = "Error(Contract, #1)")]
fn risk_params_emergency_pause_unauthorized() {
    let env = create_test_env();
    let (_cid, _admin, client) = setup(&env);
    let non_admin = Address::generate(&env);
    client.set_emergency_pause(&non_admin, &true);
}

/// When emergency pause is active, set_risk_params panics with EmergencyPaused (#6).
#[test]
#[should_panic(expected = "Error(Contract, #6)")]
fn risk_params_emergency_pause_blocks_set_risk_params() {
    let env = create_test_env();
    let (_cid, admin, client) = setup(&env);
    client.set_emergency_pause(&admin, &true);
    client.set_risk_params(&admin, &Some(12_000), &None, &None, &None);
}

// =============================================================================
// EDGE VALUES AND BOUNDARIES
// =============================================================================

/// Edge: set min_cr and liquidation_threshold to allowed minimum (10_000) within 10% decrease.
#[test]
fn risk_params_edge_at_minimum_bounds() {
    let env = create_test_env();
    let (_cid, admin, client) = setup(&env);

    client.set_risk_params(
        &admin,
        &Some(10_000),
        &Some(10_000),
        &Some(4_500),
        &Some(900),
    );
    assert_eq!(client.get_min_collateral_ratio(), 10_000);
    assert_eq!(client.get_liquidation_threshold(), 10_000);
    assert_eq!(client.get_close_factor(), 4_500);
    assert_eq!(client.get_liquidation_incentive(), 900);
}

/// Edge: close_factor 0 and 100% (within change limit from default 50% we can do 45% or 55% in one step; 0 and 100 need steps).
#[test]
fn risk_params_edge_close_factor_boundaries() {
    let env = create_test_env();
    let (_cid, admin, client) = setup(&env);

    // 50% -> 45% (10% decrease)
    client.set_risk_params(&admin, &None, &None, &Some(4_500), &None);
    assert_eq!(client.get_max_liquidatable_amount(&1_000), 450);

    // 45% -> 40.5% -> ... we can step down; 0% requires multiple steps
    client.set_risk_params(&admin, &None, &None, &Some(4_050), &None);
    assert_eq!(client.get_max_liquidatable_amount(&1_000), 405);
}

/// Edge: require_min_collateral_ratio at exact min_cr boundary (110% with default).
#[test]
fn risk_params_edge_require_ratio_at_boundary() {
    let env = create_test_env();
    let (_cid, _admin, client) = setup(&env);
    client.require_min_collateral_ratio(&1_100, &1_000);
}

/// Edge: can_be_liquidated at exact liquidation threshold (105%) is false (must be below).
#[test]
fn risk_params_edge_can_be_liquidated_at_threshold() {
    let env = create_test_env();
    let (_cid, _admin, client) = setup(&env);
    assert!(!client.can_be_liquidated(&1_050, &1_000));
}

/// Edge: get_max_liquidatable_amount with zero debt returns 0.
#[test]
fn risk_params_edge_max_liquidatable_zero_debt() {
    let env = create_test_env();
    let (_cid, _admin, client) = setup(&env);
    assert_eq!(client.get_max_liquidatable_amount(&0), 0);
}

/// Edge: get_liquidation_incentive_amount with zero amount returns 0.
#[test]
fn risk_params_edge_liquidation_incentive_zero_amount() {
    let env = create_test_env();
    let (_cid, _admin, client) = setup(&env);
    assert_eq!(client.get_liquidation_incentive_amount(&0), 0);
}

// =============================================================================
// PAUSE SWITCHES: ALL OPERATIONS
// =============================================================================

/// All operation pause symbols can be set and read.
#[test]
fn risk_params_pause_all_operations() {
    let env = create_test_env();
    let (_cid, admin, client) = setup(&env);
    let ops = [
        "pause_deposit",
        "pause_withdraw",
        "pause_borrow",
        "pause_repay",
        "pause_liquidate",
    ];

    for op in ops.iter() {
        let sym = Symbol::new(&env, op);
        client.set_pause_switch(&admin, &sym, &true);
        assert!(client.is_operation_paused(&sym));
        client.set_pause_switch(&admin, &sym, &false);
        assert!(!client.is_operation_paused(&sym));
    }
}
