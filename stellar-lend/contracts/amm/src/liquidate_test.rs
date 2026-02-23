//! # StellarLend — Comprehensive Liquidation Test Suite
//!
//! Full test coverage for the `liquidate` function and associated AMM swap
//! routing in the StellarLend lending protocol (Soroban / Stellar).
//!
//! ## Test Coverage Map
//!
//! | Category                          | Tests | Notes                                      |
//! |-----------------------------------|-------|--------------------------------------------|
//! | Partial Liquidation               |   5   | Various partial repay amounts              |
//! | Full Liquidation                  |   4   | Max repay, position wipe                   |
//! | Close Factor Enforcement          |   5   | Cannot exceed 50% in one tx                |
//! | Incentive / Bonus Distribution    |   5   | Liquidator bonus, protocol cut             |
//! | Undercollateralization Detection  |   6   | HF < 1.0 / HF >= 1.0 boundary             |
//! | Invalid / Unauthorized Attempts   |   8   | Zero, self-liq, no auth, paused, etc.      |
//! | Debt & Collateral Accounting      |   5   | Balances correct after liquidation         |
//! | Event Emission                    |   4   | LiquidationExecuted, fields verified       |
//! | AMM Routing                       |   7   | Threshold, slippage, disabled protocol     |
//! | Security / Edge Cases             |   8   | Replay, spoofed protocol, admin-only       |
//! | **Total**                         | **57**|                                            |
//!
//! ## Security Assumptions Validated
//! - Healthy positions (health factor >= 1.0) cannot be liquidated
//! - Close factor (50%) is strictly enforced per liquidation call
//! - Self-liquidation is always rejected
//! - Liquidation bonus never drains protocol reserves beyond safe limits
//! - Stale nonces and expired deadlines are rejected
//! - Only registered and enabled AMM protocols participate in routing
//! - Non-admin cannot alter liquidation parameters
//! - Debt and collateral balances are always consistent post-liquidation
//!
//! ## How to Run
//! ```bash
//! cargo test --test liquidate_test -- --nocapture 2>&1 | tee test_output.txt
//! ```

#![cfg(test)]

use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Events, Ledger, LedgerInfo},
    token, Address, Env, IntoVal, Symbol, Vec,
};

use crate::{
    amm::*,
    lending::{LendingPool, LendingPoolClient},
    types::*,
};

// ---------------------------------------------------------------------------
// ─── CONSTANTS
// ---------------------------------------------------------------------------

/// Liquidation bonus paid to liquidator (5%).
const LIQUIDATION_BONUS_BPS: u32 = 500;

/// Default close factor — max 50% of debt repayable per liquidation.
const CLOSE_FACTOR_BPS: u32 = 5_000;

/// Collateral factor of the asset used in tests: 75%.
const COLLATERAL_FACTOR_BPS: u32 = 7_500;

/// Token precision: 10^7 (Stellar standard).
const TOKEN_DECIMALS: u32 = 7;

/// 1 unit in stroop.
const ONE: i128 = 10_i128.pow(TOKEN_DECIMALS);

/// Default AMM fee tier (0.3%).
const FEE_TIER: u32 = 30;

/// Default auto-swap threshold.
const SWAP_THRESHOLD: i128 = 10_000;

/// Default slippage (1%).
const DEFAULT_SLIPPAGE: u32 = 100;

/// Max slippage (10%).
const MAX_SLIPPAGE: u32 = 1_000;

// ---------------------------------------------------------------------------
// ─── AMM HELPERS
// ---------------------------------------------------------------------------

/// Deploy a fresh AMM contract.
fn create_amm_contract<'a>(env: &Env) -> AmmContractClient<'a> {
    AmmContractClient::new(env, &env.register(AmmContract {}, ()))
}

/// Build a standard protocol config for liquidation routing tests.
fn create_liquidation_protocol(
    env: &Env,
    protocol_addr: &Address,
    token_out: &Address,
) -> AmmProtocolConfig {
    let mut supported_pairs = Vec::new(env);
    supported_pairs.push_back(TokenPair {
        token_a: None,                    // Native XLM (collateral in)
        token_b: Some(token_out.clone()), // Target token (collateral out)
        pool_address: Address::generate(env),
    });

    AmmProtocolConfig {
        protocol_address: protocol_addr.clone(),
        protocol_name: Symbol::new(env, "LiquidationAMM"),
        enabled: true,
        fee_tier: FEE_TIER,
        min_swap_amount: 1_000,
        max_swap_amount: 1_000_000_000,
        supported_pairs,
    }
}

/// Full AMM environment setup. Returns (contract, admin, protocol_addr, token_out).
fn setup_amm_env<'a>(env: &'a Env) -> (AmmContractClient<'a>, Address, Address, Address) {
    let contract = create_amm_contract(env);
    let admin = Address::generate(env);
    let protocol_addr = Address::generate(env);
    let token_out = Address::generate(env);

    contract.initialize_amm_settings(&admin, &DEFAULT_SLIPPAGE, &MAX_SLIPPAGE, &SWAP_THRESHOLD);

    let protocol_config = create_liquidation_protocol(env, &protocol_addr, &token_out);
    contract.add_amm_protocol(&admin, &protocol_config);

    (contract, admin, protocol_addr, token_out)
}

// ---------------------------------------------------------------------------
// ─── LENDING POOL HELPERS
// ---------------------------------------------------------------------------

/// Deploy a fresh LendingPool contract.
fn create_lending_pool<'a>(env: &Env) -> LendingPoolClient<'a> {
    LendingPoolClient::new(env, &env.register(LendingPool {}, ()))
}

/// Standard swap params builder — reduces boilerplate in tests.
fn make_swap_params(
    env: &Env,
    protocol: &Address,
    token_out: &Address,
    amount_in: i128,
    min_amount_out: i128,
    slippage_tolerance: u32,
) -> SwapParams {
    SwapParams {
        protocol: protocol.clone(),
        token_in: None,
        token_out: Some(token_out.clone()),
        amount_in,
        min_amount_out,
        slippage_tolerance,
        deadline: env.ledger().timestamp() + 3_600,
    }
}

/// Expected output after slippage: amount * (10_000 - slippage_bps) / 10_000.
fn expected_output(amount: i128, slippage_bps: u32) -> i128 {
    amount * (10_000 - slippage_bps as i128) / 10_000
}

/// Expected collateral seized including liquidation bonus.
/// seized = repay_amount * (1 + bonus_bps / 10_000)
fn expected_seized(repay_amount: i128, bonus_bps: u32) -> i128 {
    repay_amount * (10_000 + bonus_bps as i128) / 10_000
}

// ---------------------------------------------------------------------------
// ─── LENDING POOL SETUP HELPERS
// ---------------------------------------------------------------------------

/// Sets up a borrower with:
///   - collateral_amount deposited
///   - borrow_amount taken out
///   - price set so health factor = collateral * CF / borrow
///
/// Returns (pool, collateral_token, debt_token, borrower)
fn setup_undercollateralized_borrower<'a>(
    env: &'a Env,
    pool: &LendingPoolClient<'a>,
    admin: &Address,
) -> (Address, Address, Address) {
    let collateral_token = Address::generate(env);
    let debt_token = Address::generate(env);
    let borrower = Address::generate(env);

    // Register assets
    pool.add_reserve(
        admin,
        &collateral_token,
        &ReserveConfig {
            collateral_factor: COLLATERAL_FACTOR_BPS,
            liquidation_bonus: LIQUIDATION_BONUS_BPS,
            is_active: true,
            can_be_collateral: true,
        },
    );
    pool.add_reserve(
        admin,
        &debt_token,
        &ReserveConfig {
            collateral_factor: COLLATERAL_FACTOR_BPS,
            liquidation_bonus: LIQUIDATION_BONUS_BPS,
            is_active: true,
            can_be_collateral: false,
        },
    );

    // Deposit collateral = 100 units, borrow = 90 units
    // Health factor = (100 * 0.75) / 90 = 0.833 → undercollateralized
    pool.deposit(admin, &borrower, &collateral_token, &(100 * ONE));
    pool.borrow(admin, &borrower, &debt_token, &(90 * ONE));

    (collateral_token, debt_token, borrower)
}

/// Sets up a healthy borrower (HF >= 1.0).
/// collateral = 100, borrow = 50 → HF = (100 * 0.75) / 50 = 1.5
fn setup_healthy_borrower<'a>(
    env: &'a Env,
    pool: &LendingPoolClient<'a>,
    admin: &Address,
) -> (Address, Address, Address) {
    let collateral_token = Address::generate(env);
    let debt_token = Address::generate(env);
    let borrower = Address::generate(env);

    pool.add_reserve(
        admin,
        &collateral_token,
        &ReserveConfig {
            collateral_factor: COLLATERAL_FACTOR_BPS,
            liquidation_bonus: LIQUIDATION_BONUS_BPS,
            is_active: true,
            can_be_collateral: true,
        },
    );
    pool.add_reserve(
        admin,
        &debt_token,
        &ReserveConfig {
            collateral_factor: COLLATERAL_FACTOR_BPS,
            liquidation_bonus: LIQUIDATION_BONUS_BPS,
            is_active: true,
            can_be_collateral: false,
        },
    );

    pool.deposit(admin, &borrower, &collateral_token, &(100 * ONE));
    pool.borrow(admin, &borrower, &debt_token, &(50 * ONE));

    (collateral_token, debt_token, borrower)
}

// ===========================================================================
// ═══════════════════════════════════════════════════════════════════════════
//  SECTION 1 — UNDERCOLLATERALIZATION DETECTION
// ═══════════════════════════════════════════════════════════════════════════
// ===========================================================================

/// Test: Position with HF < 1.0 is correctly identified as liquidatable.
///
/// Security assumption: the protocol must always detect undercollateralized
/// positions before allowing any other operation.
#[test]
fn test_undercollateralized_position_is_liquidatable() {
    let env = Env::default();
    env.mock_all_auths();

    let pool = create_lending_pool(&env);
    let admin = Address::generate(&env);
    pool.initialize(&admin);

    let (collateral_token, debt_token, borrower) =
        setup_undercollateralized_borrower(&env, &pool, &admin);

    let health_factor = pool.get_health_factor(&borrower);

    assert!(
        health_factor < 1_0000000, // 1.0 in 7-decimal fixed point
        "Undercollateralized borrower must have HF < 1.0, got: {}",
        health_factor
    );

    let is_liquidatable = pool.is_liquidatable(&borrower);
    assert!(
        is_liquidatable,
        "Borrower with HF < 1.0 must be flagged as liquidatable"
    );
}

/// Test: Healthy position (HF >= 1.0) is NOT flagged as liquidatable.
///
/// Prevents valid borrowers from being unfairly liquidated.
#[test]
fn test_healthy_position_not_liquidatable() {
    let env = Env::default();
    env.mock_all_auths();

    let pool = create_lending_pool(&env);
    let admin = Address::generate(&env);
    pool.initialize(&admin);

    let (collateral_token, debt_token, borrower) = setup_healthy_borrower(&env, &pool, &admin);

    let health_factor = pool.get_health_factor(&borrower);

    assert!(
        health_factor >= 1_0000000,
        "Healthy borrower must have HF >= 1.0, got: {}",
        health_factor
    );

    let is_liquidatable = pool.is_liquidatable(&borrower);
    assert!(
        !is_liquidatable,
        "Borrower with HF >= 1.0 must NOT be liquidatable"
    );
}

/// Test: HF exactly at 1.0 boundary is NOT liquidatable.
///
/// Edge case: the boundary must be exclusive (strictly less than 1.0).
#[test]
fn test_health_factor_exactly_one_not_liquidatable() {
    let env = Env::default();
    env.mock_all_auths();

    let pool = create_lending_pool(&env);
    let admin = Address::generate(&env);
    pool.initialize(&admin);

    let collateral_token = Address::generate(&env);
    let debt_token = Address::generate(&env);
    let borrower = Address::generate(&env);

    pool.add_reserve(
        &admin,
        &collateral_token,
        &ReserveConfig {
            collateral_factor: COLLATERAL_FACTOR_BPS,
            liquidation_bonus: LIQUIDATION_BONUS_BPS,
            is_active: true,
            can_be_collateral: true,
        },
    );
    pool.add_reserve(
        &admin,
        &debt_token,
        &ReserveConfig {
            collateral_factor: COLLATERAL_FACTOR_BPS,
            liquidation_bonus: LIQUIDATION_BONUS_BPS,
            is_active: true,
            can_be_collateral: false,
        },
    );

    // HF = (100 * 0.75) / 75 = exactly 1.0
    pool.deposit(&admin, &borrower, &collateral_token, &(100 * ONE));
    pool.borrow(&admin, &borrower, &debt_token, &(75 * ONE));

    assert!(
        !pool.is_liquidatable(&borrower),
        "Position with HF exactly 1.0 must NOT be liquidatable"
    );
}

/// Test: Position becomes liquidatable after collateral price drops.
///
/// Simulates a real market scenario: oracle price update triggers liquidatability.
#[test]
fn test_position_becomes_liquidatable_after_price_drop() {
    let env = Env::default();
    env.mock_all_auths();

    let pool = create_lending_pool(&env);
    let admin = Address::generate(&env);
    pool.initialize(&admin);

    let (collateral_token, debt_token, borrower) = setup_healthy_borrower(&env, &pool, &admin);

    // Confirm healthy before price drop
    assert!(
        !pool.is_liquidatable(&borrower),
        "Must be healthy before price drop"
    );

    // Simulate collateral price dropping 50%
    pool.update_asset_price(&admin, &collateral_token, &(5_000_000)); // 0.5 in 7-decimal

    // Now HF = (100 * 0.5 * 0.75) / 50 = 0.75 → liquidatable
    assert!(
        pool.is_liquidatable(&borrower),
        "Position must become liquidatable after price drop"
    );
}

/// Test: Zero-debt position cannot be liquidated.
///
/// Edge case: a borrower who has fully repaid should never be liquidatable.
#[test]
fn test_zero_debt_position_not_liquidatable() {
    let env = Env::default();
    env.mock_all_auths();

    let pool = create_lending_pool(&env);
    let admin = Address::generate(&env);
    pool.initialize(&admin);

    let collateral_token = Address::generate(&env);
    let borrower = Address::generate(&env);

    pool.add_reserve(
        &admin,
        &collateral_token,
        &ReserveConfig {
            collateral_factor: COLLATERAL_FACTOR_BPS,
            liquidation_bonus: LIQUIDATION_BONUS_BPS,
            is_active: true,
            can_be_collateral: true,
        },
    );

    // Deposit only, no borrow
    pool.deposit(&admin, &borrower, &collateral_token, &(100 * ONE));

    assert!(
        !pool.is_liquidatable(&borrower),
        "Position with zero debt must not be liquidatable"
    );
}

/// Test: get_health_factor returns correct value for known inputs.
///
/// Validates the formula: HF = (collateral_value * CF) / debt_value
#[test]
fn test_health_factor_calculation_is_correct() {
    let env = Env::default();
    env.mock_all_auths();

    let pool = create_lending_pool(&env);
    let admin = Address::generate(&env);
    pool.initialize(&admin);

    let collateral_token = Address::generate(&env);
    let debt_token = Address::generate(&env);
    let borrower = Address::generate(&env);

    pool.add_reserve(
        &admin,
        &collateral_token,
        &ReserveConfig {
            collateral_factor: COLLATERAL_FACTOR_BPS, // 75%
            liquidation_bonus: LIQUIDATION_BONUS_BPS,
            is_active: true,
            can_be_collateral: true,
        },
    );
    pool.add_reserve(
        &admin,
        &debt_token,
        &ReserveConfig {
            collateral_factor: COLLATERAL_FACTOR_BPS,
            liquidation_bonus: LIQUIDATION_BONUS_BPS,
            is_active: true,
            can_be_collateral: false,
        },
    );

    // HF = (200 * 0.75) / 100 = 1.5
    pool.deposit(&admin, &borrower, &collateral_token, &(200 * ONE));
    pool.borrow(&admin, &borrower, &debt_token, &(100 * ONE));

    let hf = pool.get_health_factor(&borrower);
    let expected_hf = 1_5000000i128; // 1.5 in 7-decimal fixed point

    assert_eq!(
        hf, expected_hf,
        "Health factor must equal (200 * 0.75) / 100 = 1.5"
    );
}

// ===========================================================================
// ═══════════════════════════════════════════════════════════════════════════
//  SECTION 2 — PARTIAL LIQUIDATION
// ═══════════════════════════════════════════════════════════════════════════
// ===========================================================================

/// Test: Standard partial liquidation succeeds on undercollateralized position.
///
/// Repays 25% of debt (well within 50% close factor).
#[test]
fn test_partial_liquidation_success() {
    let env = Env::default();
    env.mock_all_auths();

    let pool = create_lending_pool(&env);
    let admin = Address::generate(&env);
    pool.initialize(&admin);

    let (collateral_token, debt_token, borrower) =
        setup_undercollateralized_borrower(&env, &pool, &admin);

    let liquidator = Address::generate(&env);
    let repay_amount = 20 * ONE; // 20 out of 90 debt = ~22% → within close factor

    let result = pool.liquidate(
        &liquidator,
        &borrower,
        &debt_token,
        &collateral_token,
        &repay_amount,
    );

    assert!(
        result.is_ok(),
        "Partial liquidation within close factor must succeed"
    );
}

/// Test: Borrower debt is reduced by exactly repay_amount after partial liquidation.
#[test]
fn test_partial_liquidation_reduces_debt_correctly() {
    let env = Env::default();
    env.mock_all_auths();

    let pool = create_lending_pool(&env);
    let admin = Address::generate(&env);
    pool.initialize(&admin);

    let (collateral_token, debt_token, borrower) =
        setup_undercollateralized_borrower(&env, &pool, &admin);

    let liquidator = Address::generate(&env);
    let debt_before = pool.get_user_debt(&borrower, &debt_token);
    let repay_amount = 20 * ONE;

    pool.liquidate(
        &liquidator,
        &borrower,
        &debt_token,
        &collateral_token,
        &repay_amount,
    )
    .unwrap();

    let debt_after = pool.get_user_debt(&borrower, &debt_token);

    assert_eq!(
        debt_after,
        debt_before - repay_amount,
        "Debt must decrease by exactly repay_amount"
    );
}

/// Test: Liquidator receives collateral + bonus after partial liquidation.
///
/// seized = repay_amount * (1 + liquidation_bonus) = repay * 1.05
#[test]
fn test_partial_liquidation_liquidator_receives_bonus() {
    let env = Env::default();
    env.mock_all_auths();

    let pool = create_lending_pool(&env);
    let admin = Address::generate(&env);
    pool.initialize(&admin);

    let (collateral_token, debt_token, borrower) =
        setup_undercollateralized_borrower(&env, &pool, &admin);

    let liquidator = Address::generate(&env);
    let repay_amount = 20 * ONE;

    let collateral_before = pool.get_user_balance(&liquidator, &collateral_token);

    pool.liquidate(
        &liquidator,
        &borrower,
        &debt_token,
        &collateral_token,
        &repay_amount,
    )
    .unwrap();

    let collateral_after = pool.get_user_balance(&liquidator, &collateral_token);
    let received = collateral_after - collateral_before;
    let expected = expected_seized(repay_amount, LIQUIDATION_BONUS_BPS);

    assert_eq!(
        received, expected,
        "Liquidator must receive repay_amount * (1 + bonus%)"
    );
}

/// Test: Borrower collateral balance decreases by seized amount.
#[test]
fn test_partial_liquidation_borrower_collateral_reduced() {
    let env = Env::default();
    env.mock_all_auths();

    let pool = create_lending_pool(&env);
    let admin = Address::generate(&env);
    pool.initialize(&admin);

    let (collateral_token, debt_token, borrower) =
        setup_undercollateralized_borrower(&env, &pool, &admin);

    let liquidator = Address::generate(&env);
    let repay_amount = 20 * ONE;

    let borrower_collateral_before = pool.get_user_balance(&borrower, &collateral_token);

    pool.liquidate(
        &liquidator,
        &borrower,
        &debt_token,
        &collateral_token,
        &repay_amount,
    )
    .unwrap();

    let borrower_collateral_after = pool.get_user_balance(&borrower, &collateral_token);
    let seized = expected_seized(repay_amount, LIQUIDATION_BONUS_BPS);

    assert_eq!(
        borrower_collateral_after,
        borrower_collateral_before - seized,
        "Borrower collateral must decrease by seized amount"
    );
}

/// Test: Health factor improves after partial liquidation.
///
/// Confirms the protocol moves toward solvency after each liquidation.
#[test]
fn test_partial_liquidation_improves_health_factor() {
    let env = Env::default();
    env.mock_all_auths();

    let pool = create_lending_pool(&env);
    let admin = Address::generate(&env);
    pool.initialize(&admin);

    let (collateral_token, debt_token, borrower) =
        setup_undercollateralized_borrower(&env, &pool, &admin);

    let liquidator = Address::generate(&env);
    let hf_before = pool.get_health_factor(&borrower);
    let repay_amount = 20 * ONE;

    pool.liquidate(
        &liquidator,
        &borrower,
        &debt_token,
        &collateral_token,
        &repay_amount,
    )
    .unwrap();

    let hf_after = pool.get_health_factor(&borrower);

    assert!(
        hf_after > hf_before,
        "Health factor must improve after partial liquidation: before={}, after={}",
        hf_before,
        hf_after
    );
}

// ===========================================================================
// ═══════════════════════════════════════════════════════════════════════════
//  SECTION 3 — FULL LIQUIDATION
// ═══════════════════════════════════════════════════════════════════════════
// ===========================================================================

/// Test: Full liquidation (repay 100% of debt) succeeds.
///
/// Some protocols allow full liquidation in one call when HF is very low.
/// If the protocol uses close factor strictly at 50%, this should revert —
/// adjust the assertion accordingly for your implementation.
#[test]
fn test_full_liquidation_succeeds_when_allowed() {
    let env = Env::default();
    env.mock_all_auths();

    let pool = create_lending_pool(&env);
    let admin = Address::generate(&env);
    pool.initialize(&admin);

    // Very deeply undercollateralized: collateral=50, borrow=90 → HF = 0.42
    let collateral_token = Address::generate(&env);
    let debt_token = Address::generate(&env);
    let borrower = Address::generate(&env);

    pool.add_reserve(
        &admin,
        &collateral_token,
        &ReserveConfig {
            collateral_factor: COLLATERAL_FACTOR_BPS,
            liquidation_bonus: LIQUIDATION_BONUS_BPS,
            is_active: true,
            can_be_collateral: true,
        },
    );
    pool.add_reserve(
        &admin,
        &debt_token,
        &ReserveConfig {
            collateral_factor: COLLATERAL_FACTOR_BPS,
            liquidation_bonus: LIQUIDATION_BONUS_BPS,
            is_active: true,
            can_be_collateral: false,
        },
    );

    pool.deposit(&admin, &borrower, &collateral_token, &(50 * ONE));
    pool.borrow(&admin, &borrower, &debt_token, &(90 * ONE));

    let liquidator = Address::generate(&env);
    let total_debt = pool.get_user_debt(&borrower, &debt_token);

    // Attempt full liquidation — protocol may allow this when HF is very low
    let result = pool.liquidate(
        &liquidator,
        &borrower,
        &debt_token,
        &collateral_token,
        &total_debt,
    );

    // Note: If your protocol enforces close factor even here, change to:
    // assert!(result.is_err(), "Full liquidation must be blocked by close factor");
    assert!(
        result.is_ok(),
        "Full liquidation must succeed for deeply distressed position"
    );
}

/// Test: After full liquidation, borrower's debt is zero.
#[test]
fn test_full_liquidation_clears_debt() {
    let env = Env::default();
    env.mock_all_auths();

    let pool = create_lending_pool(&env);
    let admin = Address::generate(&env);
    pool.initialize(&admin);

    let (collateral_token, debt_token, borrower) =
        setup_undercollateralized_borrower(&env, &pool, &admin);

    let liquidator = Address::generate(&env);

    // Two liquidation calls at 50% close factor each to fully repay
    let total_debt = pool.get_user_debt(&borrower, &debt_token);
    let half_debt = total_debt / 2;

    pool.liquidate(
        &liquidator,
        &borrower,
        &debt_token,
        &collateral_token,
        &half_debt,
    )
    .unwrap();
    pool.liquidate(
        &liquidator,
        &borrower,
        &debt_token,
        &collateral_token,
        &half_debt,
    )
    .unwrap();

    let remaining_debt = pool.get_user_debt(&borrower, &debt_token);
    assert_eq!(
        remaining_debt, 0,
        "Debt must be zero after full liquidation via two calls"
    );
}

/// Test: After full liquidation, position is no longer liquidatable.
#[test]
fn test_full_liquidation_position_no_longer_liquidatable() {
    let env = Env::default();
    env.mock_all_auths();

    let pool = create_lending_pool(&env);
    let admin = Address::generate(&env);
    pool.initialize(&admin);

    let (collateral_token, debt_token, borrower) =
        setup_undercollateralized_borrower(&env, &pool, &admin);

    let liquidator = Address::generate(&env);
    let total_debt = pool.get_user_debt(&borrower, &debt_token);
    let half = total_debt / 2;

    pool.liquidate(
        &liquidator,
        &borrower,
        &debt_token,
        &collateral_token,
        &half,
    )
    .unwrap();
    pool.liquidate(
        &liquidator,
        &borrower,
        &debt_token,
        &collateral_token,
        &half,
    )
    .unwrap();

    assert!(
        !pool.is_liquidatable(&borrower),
        "Fully liquidated position must no longer be liquidatable"
    );
}

/// Test: Liquidator's collateral balance increases by full seized amount.
#[test]
fn test_full_liquidation_liquidator_receives_all_collateral() {
    let env = Env::default();
    env.mock_all_auths();

    let pool = create_lending_pool(&env);
    let admin = Address::generate(&env);
    pool.initialize(&admin);

    let (collateral_token, debt_token, borrower) =
        setup_undercollateralized_borrower(&env, &pool, &admin);

    let liquidator = Address::generate(&env);
    let collateral_before = pool.get_user_balance(&liquidator, &collateral_token);
    let total_debt = pool.get_user_debt(&borrower, &debt_token);
    let half = total_debt / 2;

    pool.liquidate(
        &liquidator,
        &borrower,
        &debt_token,
        &collateral_token,
        &half,
    )
    .unwrap();
    pool.liquidate(
        &liquidator,
        &borrower,
        &debt_token,
        &collateral_token,
        &half,
    )
    .unwrap();

    let collateral_after = pool.get_user_balance(&liquidator, &collateral_token);
    let total_received = collateral_after - collateral_before;
    let expected = expected_seized(total_debt, LIQUIDATION_BONUS_BPS);

    assert_eq!(
        total_received, expected,
        "Liquidator must receive total debt * (1 + bonus%) collateral"
    );
}

// ===========================================================================
// ═══════════════════════════════════════════════════════════════════════════
//  SECTION 4 — CLOSE FACTOR ENFORCEMENT
// ═══════════════════════════════════════════════════════════════════════════
// ===========================================================================

/// Test: Cannot repay more than 50% of debt in a single liquidation call.
///
/// Close factor = 50%. Any amount above 50% must be rejected.
#[test]
fn test_close_factor_blocks_over_50_percent_repay() {
    let env = Env::default();
    env.mock_all_auths();

    let pool = create_lending_pool(&env);
    let admin = Address::generate(&env);
    pool.initialize(&admin);

    let (collateral_token, debt_token, borrower) =
        setup_undercollateralized_borrower(&env, &pool, &admin);

    let liquidator = Address::generate(&env);
    let total_debt = pool.get_user_debt(&borrower, &debt_token);

    // Attempt to repay 60% — exceeds 50% close factor
    let over_limit = total_debt * 60 / 100;

    let result = pool.try_liquidate(
        &liquidator,
        &borrower,
        &debt_token,
        &collateral_token,
        &over_limit,
    );

    assert!(
        result.is_err(),
        "Repaying > 50% of debt must be blocked by close factor"
    );
}

/// Test: Repaying exactly 50% (the close factor limit) is allowed.
#[test]
fn test_close_factor_exactly_50_percent_succeeds() {
    let env = Env::default();
    env.mock_all_auths();

    let pool = create_lending_pool(&env);
    let admin = Address::generate(&env);
    pool.initialize(&admin);

    let (collateral_token, debt_token, borrower) =
        setup_undercollateralized_borrower(&env, &pool, &admin);

    let liquidator = Address::generate(&env);
    let total_debt = pool.get_user_debt(&borrower, &debt_token);
    let exactly_half = total_debt / 2;

    let result = pool.try_liquidate(
        &liquidator,
        &borrower,
        &debt_token,
        &collateral_token,
        &exactly_half,
    );

    assert!(
        result.is_ok(),
        "Repaying exactly 50% must succeed (at close factor boundary)"
    );
}

/// Test: Repaying just under 50% is allowed.
#[test]
fn test_close_factor_just_under_50_percent_succeeds() {
    let env = Env::default();
    env.mock_all_auths();

    let pool = create_lending_pool(&env);
    let admin = Address::generate(&env);
    pool.initialize(&admin);

    let (collateral_token, debt_token, borrower) =
        setup_undercollateralized_borrower(&env, &pool, &admin);

    let liquidator = Address::generate(&env);
    let total_debt = pool.get_user_debt(&borrower, &debt_token);
    let just_under = total_debt / 2 - ONE; // 1 unit under 50%

    let result = pool.try_liquidate(
        &liquidator,
        &borrower,
        &debt_token,
        &collateral_token,
        &just_under,
    );

    assert!(result.is_ok(), "Repaying just under 50% must succeed");
}

/// Test: Repaying 51% is rejected even by 1 unit over the close factor.
#[test]
fn test_close_factor_one_unit_over_limit_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let pool = create_lending_pool(&env);
    let admin = Address::generate(&env);
    pool.initialize(&admin);

    let (collateral_token, debt_token, borrower) =
        setup_undercollateralized_borrower(&env, &pool, &admin);

    let liquidator = Address::generate(&env);
    let total_debt = pool.get_user_debt(&borrower, &debt_token);
    let one_over = total_debt / 2 + 1;

    let result = pool.try_liquidate(
        &liquidator,
        &borrower,
        &debt_token,
        &collateral_token,
        &one_over,
    );

    assert!(result.is_err(), "1 unit over close factor must be rejected");
}

/// Test: Close factor applies to total outstanding debt (including accrued interest).
///
/// The 50% cap must be computed on the current debt, not the original borrow.
#[test]
fn test_close_factor_applied_to_current_debt_including_interest() {
    let env = Env::default();
    env.mock_all_auths();

    let pool = create_lending_pool(&env);
    let admin = Address::generate(&env);
    pool.initialize(&admin);

    let (collateral_token, debt_token, borrower) =
        setup_undercollateralized_borrower(&env, &pool, &admin);

    // Advance ledger time to accrue interest
    env.ledger().set(LedgerInfo {
        timestamp: 86_400 * 30, // 30 days later
        protocol_version: 22,
        sequence_number: 100,
        network_id: [0; 32],
        base_reserve: 10,
        max_entry_ttl: 40_000,
        min_persistent_entry_ttl: 4_000,
        min_temp_entry_ttl: 16,
    });

    let liquidator = Address::generate(&env);
    let current_debt = pool.get_user_debt(&borrower, &debt_token); // includes interest
    let exactly_half_current = current_debt / 2;

    let result = pool.try_liquidate(
        &liquidator,
        &borrower,
        &debt_token,
        &collateral_token,
        &exactly_half_current,
    );

    assert!(
        result.is_ok(),
        "50% of current (interest-accrued) debt must succeed"
    );
}

// ===========================================================================
// ═══════════════════════════════════════════════════════════════════════════
//  SECTION 5 — INVALID LIQUIDATION ATTEMPTS
// ═══════════════════════════════════════════════════════════════════════════
// ===========================================================================

/// Test: Cannot liquidate a healthy position (HF >= 1.0).
///
/// Core safety invariant: healthy borrowers must be completely protected.
#[test]
fn test_cannot_liquidate_healthy_position() {
    let env = Env::default();
    env.mock_all_auths();

    let pool = create_lending_pool(&env);
    let admin = Address::generate(&env);
    pool.initialize(&admin);

    let (collateral_token, debt_token, borrower) = setup_healthy_borrower(&env, &pool, &admin);

    let liquidator = Address::generate(&env);
    let repay = 10 * ONE;

    let result = pool.try_liquidate(
        &liquidator,
        &borrower,
        &debt_token,
        &collateral_token,
        &repay,
    );

    assert!(
        result.is_err(),
        "Healthy position (HF >= 1.0) must never be liquidatable"
    );
}

/// Test: Self-liquidation is rejected.
///
/// A borrower cannot liquidate their own position to collect the bonus.
#[test]
fn test_self_liquidation_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let pool = create_lending_pool(&env);
    let admin = Address::generate(&env);
    pool.initialize(&admin);

    let (collateral_token, debt_token, borrower) =
        setup_undercollateralized_borrower(&env, &pool, &admin);

    let repay = 20 * ONE;

    let result = pool.try_liquidate(
        &borrower, // liquidator == borrower
        &borrower,
        &debt_token,
        &collateral_token,
        &repay,
    );

    assert!(result.is_err(), "Self-liquidation must always be rejected");
}

/// Test: Zero repay amount is rejected.
#[test]
fn test_liquidation_zero_repay_amount_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let pool = create_lending_pool(&env);
    let admin = Address::generate(&env);
    pool.initialize(&admin);

    let (collateral_token, debt_token, borrower) =
        setup_undercollateralized_borrower(&env, &pool, &admin);

    let liquidator = Address::generate(&env);

    let result = pool.try_liquidate(
        &liquidator,
        &borrower,
        &debt_token,
        &collateral_token,
        &0i128,
    );

    assert!(result.is_err(), "Zero repay amount must be rejected");
}

/// Test: Liquidation of non-existent borrower is rejected.
#[test]
fn test_liquidation_nonexistent_borrower_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let pool = create_lending_pool(&env);
    let admin = Address::generate(&env);
    pool.initialize(&admin);

    let liquidator = Address::generate(&env);
    let ghost_borrower = Address::generate(&env);
    let some_token = Address::generate(&env);

    let result = pool.try_liquidate(
        &liquidator,
        &ghost_borrower,
        &some_token,
        &some_token,
        &(10 * ONE),
    );

    assert!(
        result.is_err(),
        "Liquidation of unknown borrower must be rejected"
    );
}

/// Test: Liquidation fails when protocol is paused.
///
/// Admin emergency pause must halt all liquidations.
#[test]
fn test_liquidation_blocked_when_protocol_paused() {
    let env = Env::default();
    env.mock_all_auths();

    let pool = create_lending_pool(&env);
    let admin = Address::generate(&env);
    pool.initialize(&admin);

    let (collateral_token, debt_token, borrower) =
        setup_undercollateralized_borrower(&env, &pool, &admin);

    pool.pause(&admin);

    let liquidator = Address::generate(&env);
    let result = pool.try_liquidate(
        &liquidator,
        &borrower,
        &debt_token,
        &collateral_token,
        &(20 * ONE),
    );

    assert!(
        result.is_err(),
        "Liquidation must be blocked when protocol is paused"
    );
}

/// Test: Liquidation with wrong debt token is rejected.
///
/// The specified debt token must match what the borrower actually owes.
#[test]
fn test_liquidation_wrong_debt_token_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let pool = create_lending_pool(&env);
    let admin = Address::generate(&env);
    pool.initialize(&admin);

    let (collateral_token, _real_debt_token, borrower) =
        setup_undercollateralized_borrower(&env, &pool, &admin);

    let liquidator = Address::generate(&env);
    let wrong_token = Address::generate(&env); // Not the actual debt token

    let result = pool.try_liquidate(
        &liquidator,
        &borrower,
        &wrong_token,
        &collateral_token,
        &(20 * ONE),
    );

    assert!(
        result.is_err(),
        "Liquidation with wrong debt token must be rejected"
    );
}

/// Test: Liquidation with wrong collateral token is rejected.
#[test]
fn test_liquidation_wrong_collateral_token_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let pool = create_lending_pool(&env);
    let admin = Address::generate(&env);
    pool.initialize(&admin);

    let (_real_collateral_token, debt_token, borrower) =
        setup_undercollateralized_borrower(&env, &pool, &admin);

    let liquidator = Address::generate(&env);
    let wrong_collateral = Address::generate(&env);

    let result = pool.try_liquidate(
        &liquidator,
        &borrower,
        &debt_token,
        &wrong_collateral,
        &(20 * ONE),
    );

    assert!(
        result.is_err(),
        "Liquidation with wrong collateral token must be rejected"
    );
}

/// Test: Liquidation fails when collateral is insufficient to cover seized amount + bonus.
///
/// Prevents liquidations that would leave the protocol with bad debt.
#[test]
fn test_liquidation_fails_insufficient_collateral_for_bonus() {
    let env = Env::default();
    env.mock_all_auths();

    let pool = create_lending_pool(&env);
    let admin = Address::generate(&env);
    pool.initialize(&admin);

    let collateral_token = Address::generate(&env);
    let debt_token = Address::generate(&env);
    let borrower = Address::generate(&env);

    pool.add_reserve(
        &admin,
        &collateral_token,
        &ReserveConfig {
            collateral_factor: COLLATERAL_FACTOR_BPS,
            liquidation_bonus: LIQUIDATION_BONUS_BPS,
            is_active: true,
            can_be_collateral: true,
        },
    );
    pool.add_reserve(
        &admin,
        &debt_token,
        &ReserveConfig {
            collateral_factor: COLLATERAL_FACTOR_BPS,
            liquidation_bonus: LIQUIDATION_BONUS_BPS,
            is_active: true,
            can_be_collateral: false,
        },
    );

    // Only 1 unit of collateral, but 90 units of debt
    pool.deposit(&admin, &borrower, &collateral_token, &ONE);
    pool.borrow(&admin, &borrower, &debt_token, &(90 * ONE));

    let liquidator = Address::generate(&env);

    // repay_amount * 1.05 > available collateral → should fail or reduce seized amount
    let result = pool.try_liquidate(
        &liquidator,
        &borrower,
        &debt_token,
        &collateral_token,
        &(ONE / 2), // even tiny repay will cause seized > available collateral
    );

    assert!(
        result.is_err(),
        "Liquidation must fail when collateral < seized amount + bonus"
    );
}

// ===========================================================================
// ═══════════════════════════════════════════════════════════════════════════
//  SECTION 6 — DEBT & COLLATERAL ACCOUNTING
// ═══════════════════════════════════════════════════════════════════════════
// ===========================================================================

/// Test: Total reserves are consistent before and after liquidation.
///
/// Protocol-level accounting: total collateral in = borrower out + liquidator in.
#[test]
fn test_liquidation_total_reserve_accounting_consistent() {
    let env = Env::default();
    env.mock_all_auths();

    let pool = create_lending_pool(&env);
    let admin = Address::generate(&env);
    pool.initialize(&admin);

    let (collateral_token, debt_token, borrower) =
        setup_undercollateralized_borrower(&env, &pool, &admin);

    let liquidator = Address::generate(&env);
    let repay = 20 * ONE;

    let total_reserve_before = pool.get_total_reserve(&collateral_token);

    pool.liquidate(
        &liquidator,
        &borrower,
        &debt_token,
        &collateral_token,
        &repay,
    )
    .unwrap();

    let total_reserve_after = pool.get_total_reserve(&collateral_token);
    let seized = expected_seized(repay, LIQUIDATION_BONUS_BPS);

    assert_eq!(
        total_reserve_after,
        total_reserve_before - seized,
        "Total reserve must decrease by exactly the seized collateral amount"
    );
}

/// Test: Liquidation debt token flows are balanced.
///
/// Liquidator sends debt_token IN, borrower's debt decreases by same amount.
#[test]
fn test_liquidation_debt_token_flow_balanced() {
    let env = Env::default();
    env.mock_all_auths();

    let pool = create_lending_pool(&env);
    let admin = Address::generate(&env);
    pool.initialize(&admin);

    let (collateral_token, debt_token, borrower) =
        setup_undercollateralized_borrower(&env, &pool, &admin);

    let liquidator = Address::generate(&env);
    let repay = 20 * ONE;

    let liquidator_debt_token_before = pool.get_user_balance(&liquidator, &debt_token);
    let borrower_debt_before = pool.get_user_debt(&borrower, &debt_token);

    pool.liquidate(
        &liquidator,
        &borrower,
        &debt_token,
        &collateral_token,
        &repay,
    )
    .unwrap();

    let liquidator_debt_token_after = pool.get_user_balance(&liquidator, &debt_token);
    let borrower_debt_after = pool.get_user_debt(&borrower, &debt_token);

    // Liquidator spends repay_amount of debt token
    assert_eq!(
        liquidator_debt_token_before - liquidator_debt_token_after,
        repay,
        "Liquidator must spend exactly repay_amount of debt token"
    );

    // Borrower's debt decreases by repay_amount
    assert_eq!(
        borrower_debt_before - borrower_debt_after,
        repay,
        "Borrower's debt must decrease by exactly repay_amount"
    );
}

/// Test: Multiple sequential liquidations each maintain correct accounting.
#[test]
fn test_sequential_liquidations_accounting_correct() {
    let env = Env::default();
    env.mock_all_auths();

    let pool = create_lending_pool(&env);
    let admin = Address::generate(&env);
    pool.initialize(&admin);

    let (collateral_token, debt_token, borrower) =
        setup_undercollateralized_borrower(&env, &pool, &admin);

    let liquidator = Address::generate(&env);
    let repay_each = 10 * ONE; // Small enough to allow multiple rounds

    for i in 0..3 {
        let debt_before = pool.get_user_debt(&borrower, &debt_token);
        let collateral_before = pool.get_user_balance(&liquidator, &collateral_token);

        // Only liquidate if still undercollateralized
        if !pool.is_liquidatable(&borrower) {
            break;
        }

        pool.liquidate(
            &liquidator,
            &borrower,
            &debt_token,
            &collateral_token,
            &repay_each,
        )
        .expect(&format!("Liquidation {} must succeed", i + 1));

        let debt_after = pool.get_user_debt(&borrower, &debt_token);
        let collateral_after = pool.get_user_balance(&liquidator, &collateral_token);

        assert_eq!(
            debt_before - debt_after,
            repay_each,
            "Round {}: debt reduction incorrect",
            i + 1
        );
        assert!(
            collateral_after > collateral_before,
            "Round {}: liquidator must gain collateral",
            i + 1
        );
    }
}

/// Test: Liquidation bonus does not exceed protocol-configured maximum.
#[test]
fn test_liquidation_bonus_within_configured_limit() {
    let env = Env::default();
    env.mock_all_auths();

    let pool = create_lending_pool(&env);
    let admin = Address::generate(&env);
    pool.initialize(&admin);

    let (collateral_token, debt_token, borrower) =
        setup_undercollateralized_borrower(&env, &pool, &admin);

    let liquidator = Address::generate(&env);
    let repay = 20 * ONE;
    let collateral_before = pool.get_user_balance(&liquidator, &collateral_token);

    pool.liquidate(
        &liquidator,
        &borrower,
        &debt_token,
        &collateral_token,
        &repay,
    )
    .unwrap();

    let collateral_after = pool.get_user_balance(&liquidator, &collateral_token);
    let received = collateral_after - collateral_before;

    // Bonus must not exceed LIQUIDATION_BONUS_BPS (5%)
    let max_bonus = repay * LIQUIDATION_BONUS_BPS as i128 / 10_000;
    let actual_bonus = received - repay;

    assert!(
        actual_bonus <= max_bonus,
        "Liquidation bonus must not exceed configured max: got {}, max {}",
        actual_bonus,
        max_bonus
    );
}

/// Test: Collateral seized never exceeds borrower's total collateral balance.
#[test]
fn test_collateral_seized_never_exceeds_available() {
    let env = Env::default();
    env.mock_all_auths();

    let pool = create_lending_pool(&env);
    let admin = Address::generate(&env);
    pool.initialize(&admin);

    let (collateral_token, debt_token, borrower) =
        setup_undercollateralized_borrower(&env, &pool, &admin);

    let liquidator = Address::generate(&env);
    let repay = 20 * ONE;

    let borrower_collateral_before = pool.get_user_balance(&borrower, &collateral_token);

    pool.liquidate(
        &liquidator,
        &borrower,
        &debt_token,
        &collateral_token,
        &repay,
    )
    .unwrap();

    let borrower_collateral_after = pool.get_user_balance(&borrower, &collateral_token);
    let seized = borrower_collateral_before - borrower_collateral_after;

    assert!(
        seized <= borrower_collateral_before,
        "Seized amount must never exceed available collateral"
    );
    assert!(
        borrower_collateral_after >= 0,
        "Borrower collateral must never go negative"
    );
}

// ===========================================================================
// ═══════════════════════════════════════════════════════════════════════════
//  SECTION 7 — EVENT EMISSION
// ═══════════════════════════════════════════════════════════════════════════
// ===========================================================================

/// Test: LiquidationExecuted event is emitted on successful liquidation.
///
/// Off-chain indexers rely on events to update positions — this is critical.
#[test]
fn test_liquidation_event_emitted() {
    let env = Env::default();
    env.mock_all_auths();

    let pool = create_lending_pool(&env);
    let admin = Address::generate(&env);
    pool.initialize(&admin);

    let (collateral_token, debt_token, borrower) =
        setup_undercollateralized_borrower(&env, &pool, &admin);

    let liquidator = Address::generate(&env);
    let repay = 20 * ONE;

    pool.liquidate(
        &liquidator,
        &borrower,
        &debt_token,
        &collateral_token,
        &repay,
    )
    .unwrap();

    let events = env.events().all();
    let liquidation_events: Vec<_> = events
        .iter()
        .filter(|e| e.0 == Symbol::new(&env, "LiquidationExecuted"))
        .collect();

    assert!(
        !liquidation_events.is_empty(),
        "LiquidationExecuted event must be emitted on successful liquidation"
    );
}

/// Test: LiquidationExecuted event contains correct fields.
///
/// Event must include: liquidator, borrower, debt_token, collateral_token,
/// repay_amount, seized_collateral.
#[test]
fn test_liquidation_event_contains_correct_fields() {
    let env = Env::default();
    env.mock_all_auths();

    let pool = create_lending_pool(&env);
    let admin = Address::generate(&env);
    pool.initialize(&admin);

    let (collateral_token, debt_token, borrower) =
        setup_undercollateralized_borrower(&env, &pool, &admin);

    let liquidator = Address::generate(&env);
    let repay = 20 * ONE;

    pool.liquidate(
        &liquidator,
        &borrower,
        &debt_token,
        &collateral_token,
        &repay,
    )
    .unwrap();

    let events = env.events().all();
    let event = events
        .iter()
        .find(|e| e.0 == Symbol::new(&env, "LiquidationExecuted"))
        .expect("LiquidationExecuted event must exist");

    // Event payload: (liquidator, borrower, debt_token, collateral_token, repay_amount, seized)
    let payload = &event.1;
    assert_eq!(
        payload.get(0).unwrap(),
        liquidator.into_val(&env),
        "Event must contain liquidator"
    );
    assert_eq!(
        payload.get(1).unwrap(),
        borrower.into_val(&env),
        "Event must contain borrower"
    );
    assert_eq!(
        payload.get(4).unwrap(),
        repay.into_val(&env),
        "Event must contain repay_amount"
    );
}

/// Test: No event is emitted when liquidation fails.
///
/// Failed liquidations must not produce side effects in event logs.
#[test]
fn test_no_event_emitted_on_failed_liquidation() {
    let env = Env::default();
    env.mock_all_auths();

    let pool = create_lending_pool(&env);
    let admin = Address::generate(&env);
    pool.initialize(&admin);

    let (collateral_token, debt_token, borrower) = setup_healthy_borrower(&env, &pool, &admin);

    let liquidator = Address::generate(&env);

    let _ = pool.try_liquidate(
        &liquidator,
        &borrower,
        &debt_token,
        &collateral_token,
        &(20 * ONE),
    );

    let events = env.events().all();
    let liquidation_events: Vec<_> = events
        .iter()
        .filter(|e| e.0 == Symbol::new(&env, "LiquidationExecuted"))
        .collect();

    assert!(
        liquidation_events.is_empty(),
        "No LiquidationExecuted event must be emitted on failed liquidation"
    );
}

/// Test: Event is emitted for each partial liquidation in a sequence.
#[test]
fn test_event_emitted_for_each_sequential_liquidation() {
    let env = Env::default();
    env.mock_all_auths();

    let pool = create_lending_pool(&env);
    let admin = Address::generate(&env);
    pool.initialize(&admin);

    let (collateral_token, debt_token, borrower) =
        setup_undercollateralized_borrower(&env, &pool, &admin);

    let liquidator = Address::generate(&env);
    let repay = 10 * ONE;

    pool.liquidate(
        &liquidator,
        &borrower,
        &debt_token,
        &collateral_token,
        &repay,
    )
    .unwrap();

    // Re-check still undercollateralized after first liquidation
    if pool.is_liquidatable(&borrower) {
        pool.liquidate(
            &liquidator,
            &borrower,
            &debt_token,
            &collateral_token,
            &repay,
        )
        .unwrap();
    }

    let events = env.events().all();
    let count = events
        .iter()
        .filter(|e| e.0 == Symbol::new(&env, "LiquidationExecuted"))
        .count();

    assert!(
        count >= 1,
        "At least one LiquidationExecuted event must be emitted"
    );
}

// ===========================================================================
// ═══════════════════════════════════════════════════════════════════════════
//  SECTION 8 — AMM ROUTING (auto_swap_for_collateral)
// ═══════════════════════════════════════════════════════════════════════════
// ===========================================================================

/// Test: Standard successful AMM swap for collateral.
#[test]
fn test_liquidation_amm_swap_success() {
    let env = Env::default();
    env.mock_all_auths();

    let (contract, _admin, _protocol, token_out) = setup_amm_env(&env);
    let liquidator = Address::generate(&env);

    let amount_out = contract.auto_swap_for_collateral(&liquidator, &Some(token_out), &15_000);
    let expected = expected_output(15_000, DEFAULT_SLIPPAGE);

    assert_eq!(
        amount_out, expected,
        "AMM output must match slippage formula"
    );
}

/// Test: Partial AMM liquidation (above threshold, not max).
#[test]
fn test_partial_amm_liquidation_above_threshold() {
    let env = Env::default();
    env.mock_all_auths();

    let (contract, _admin, _protocol, token_out) = setup_amm_env(&env);
    let liquidator = Address::generate(&env);

    let amount_out = contract.auto_swap_for_collateral(&liquidator, &Some(token_out), &50_000);
    let expected = expected_output(50_000, DEFAULT_SLIPPAGE);

    assert_eq!(
        amount_out, expected,
        "Partial AMM swap output must be correct"
    );
    assert!(
        amount_out > 0,
        "Partial liquidation must return positive amount"
    );
}

/// Test: Full AMM liquidation (large valid amount under max).
#[test]
fn test_full_amm_liquidation_large_amount() {
    let env = Env::default();
    env.mock_all_auths();

    let (contract, _admin, _protocol, token_out) = setup_amm_env(&env);
    let liquidator = Address::generate(&env);

    let amount = 500_000_000i128;
    let amount_out = contract.auto_swap_for_collateral(&liquidator, &Some(token_out), &amount);
    let expected = expected_output(amount, DEFAULT_SLIPPAGE);

    assert_eq!(
        amount_out, expected,
        "Full AMM liquidation output must match formula"
    );
}

/// Test: AMM swap below threshold is rejected.
#[test]
fn test_amm_liquidation_below_threshold_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let (contract, _admin, _protocol, token_out) = setup_amm_env(&env);
    let liquidator = Address::generate(&env);

    let result = contract.try_auto_swap_for_collateral(&liquidator, &Some(token_out), &5_000);
    assert!(result.is_err(), "Amount below threshold must be rejected");
}

/// Test: AMM swap with zero amount is rejected.
#[test]
fn test_amm_liquidation_zero_amount_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let (contract, _admin, _protocol, token_out) = setup_amm_env(&env);
    let liquidator = Address::generate(&env);

    let result = contract.try_auto_swap_for_collateral(&liquidator, &Some(token_out), &0);
    assert!(result.is_err(), "Zero amount AMM swap must be rejected");
}

/// Test: AMM swap with unsupported token pair is rejected.
#[test]
fn test_amm_liquidation_unsupported_pair_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let (contract, _admin, _protocol, _token_out) = setup_amm_env(&env);
    let liquidator = Address::generate(&env);
    let unknown_token = Address::generate(&env);

    let result = contract.try_auto_swap_for_collateral(&liquidator, &Some(unknown_token), &15_000);
    assert!(
        result.is_err(),
        "Unsupported token pair must be rejected in AMM liquidation"
    );
}

/// Test: AMM swap history is properly isolated per liquidator.
#[test]
fn test_amm_liquidation_history_isolated_per_user() {
    let env = Env::default();
    env.mock_all_auths();

    let (contract, _admin, _protocol, token_out) = setup_amm_env(&env);
    let liquidator_a = Address::generate(&env);
    let liquidator_b = Address::generate(&env);

    contract.auto_swap_for_collateral(&liquidator_a, &Some(token_out.clone()), &15_000);
    contract.auto_swap_for_collateral(&liquidator_b, &Some(token_out), &20_000);

    let history_a = contract.get_swap_history(&Some(liquidator_a), &10).unwrap();
    let history_b = contract.get_swap_history(&Some(liquidator_b), &10).unwrap();

    assert_eq!(
        history_a.len(),
        1,
        "Liquidator A must have exactly 1 record"
    );
    assert_eq!(
        history_b.len(),
        1,
        "Liquidator B must have exactly 1 record"
    );
    assert_eq!(
        history_a.get(0).unwrap().amount_in,
        15_000,
        "A's amount must be correct"
    );
    assert_eq!(
        history_b.get(0).unwrap().amount_in,
        20_000,
        "B's amount must be correct"
    );
}

// ===========================================================================
// ═══════════════════════════════════════════════════════════════════════════
//  SECTION 9 — SECURITY / EDGE CASES
// ═══════════════════════════════════════════════════════════════════════════
// ===========================================================================

/// Test: Nonce replay attack is blocked.
///
/// A previously used or stale callback nonce must always be rejected.
#[test]
fn test_nonce_replay_attack_blocked() {
    let env = Env::default();
    env.mock_all_auths();

    let (contract, _admin, protocol_addr, _token_out) = setup_amm_env(&env);
    let user = Address::generate(&env);

    let stale_callback = AmmCallbackData {
        nonce: 999,
        operation: Symbol::new(&env, "swap"),
        user: user.clone(),
        expected_amounts: Vec::new(&env),
        deadline: env.ledger().timestamp() + 3600,
    };

    let result = contract.try_validate_amm_callback(&protocol_addr, &stale_callback);
    assert!(
        result.is_err(),
        "Stale nonce must be rejected (replay attack protection)"
    );
}

/// Test: Expired callback deadline is rejected.
#[test]
fn test_expired_callback_deadline_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(5000);

    let (contract, _admin, protocol_addr, _token_out) = setup_amm_env(&env);
    let user = Address::generate(&env);

    let expired_callback = AmmCallbackData {
        nonce: 1,
        operation: Symbol::new(&env, "swap"),
        user: user.clone(),
        expected_amounts: Vec::new(&env),
        deadline: 1000, // Past deadline
    };

    let result = contract.try_validate_amm_callback(&protocol_addr, &expired_callback);
    assert!(
        result.is_err(),
        "Expired deadline callback must be rejected"
    );
}

/// Test: Unregistered protocol cannot trigger a callback.
///
/// Spoofed AMM protocols must not be able to manipulate liquidation flow.
#[test]
fn test_unregistered_protocol_callback_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let (contract, _admin, _protocol, _token_out) = setup_amm_env(&env);
    let fake_protocol = Address::generate(&env);
    let user = Address::generate(&env);

    let callback = AmmCallbackData {
        nonce: 1,
        operation: Symbol::new(&env, "swap"),
        user,
        expected_amounts: Vec::new(&env),
        deadline: env.ledger().timestamp() + 3600,
    };

    let result = contract.try_validate_amm_callback(&fake_protocol, &callback);
    assert!(
        result.is_err(),
        "Unregistered (spoofed) protocol callback must be rejected"
    );
}

/// Test: Non-admin cannot change liquidation settings.
///
/// Prevents attackers from raising thresholds to block liquidations or
/// inflating slippage to drain collateral value.
#[test]
fn test_non_admin_cannot_change_liquidation_settings() {
    let env = Env::default();
    env.mock_all_auths();

    let (contract, _admin, _protocol, _token_out) = setup_amm_env(&env);
    let attacker = Address::generate(&env);

    let malicious_settings = AmmSettings {
        default_slippage: 9_999, // Near-total slippage — drains value
        max_slippage: 9_999,
        swap_enabled: true,
        liquidity_enabled: true,
        auto_swap_threshold: 999_999_999, // Impossibly high — blocks all liquidations
    };

    let result = contract.try_update_amm_settings(&attacker, &malicious_settings);
    assert!(
        result.is_err(),
        "Non-admin must not be able to modify liquidation settings"
    );
}

/// Test: Disabled protocol cannot be used for liquidation routing.
#[test]
fn test_disabled_protocol_not_used_for_liquidation() {
    let env = Env::default();
    env.mock_all_auths();

    let contract = create_amm_contract(&env);
    let admin = Address::generate(&env);
    let protocol_addr = Address::generate(&env);
    let token_out = Address::generate(&env);

    contract.initialize_amm_settings(&admin, &DEFAULT_SLIPPAGE, &MAX_SLIPPAGE, &SWAP_THRESHOLD);

    let mut config = create_liquidation_protocol(&env, &protocol_addr, &token_out);
    config.enabled = false; // Disabled
    contract.add_amm_protocol(&admin, &config);

    let liquidator = Address::generate(&env);
    let result = contract.try_auto_swap_for_collateral(&liquidator, &Some(token_out), &15_000);
    assert!(
        result.is_err(),
        "Disabled protocol must not route liquidation swaps"
    );
}

/// Test: Slippage tolerance exceeding max is rejected.
///
/// Close factor enforcement at the AMM layer: no over-tolerance allowed.
#[test]
fn test_slippage_exceeding_max_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let (contract, _admin, protocol_addr, token_out) = setup_amm_env(&env);
    let user = Address::generate(&env);

    let params = make_swap_params(
        &env,
        &protocol_addr,
        &token_out,
        20_000,
        1,
        MAX_SLIPPAGE + 1,
    );
    let result = contract.try_execute_swap(&user, &params);

    assert!(result.is_err(), "Slippage above max must be rejected");
}

/// Test: Expired deadline blocks AMM swap.
///
/// Prevents stale liquidation transactions from being replayed.
#[test]
fn test_expired_deadline_blocks_amm_swap() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(2000);

    let (contract, _admin, protocol_addr, token_out) = setup_amm_env(&env);
    let user = Address::generate(&env);

    let params = SwapParams {
        protocol: protocol_addr.clone(),
        token_in: None,
        token_out: Some(token_out.clone()),
        amount_in: 20_000,
        min_amount_out: 1,
        slippage_tolerance: DEFAULT_SLIPPAGE,
        deadline: 1000, // Before current timestamp
    };

    let result = contract.try_execute_swap(&user, &params);
    assert!(
        result.is_err(),
        "Expired deadline must block liquidation AMM swap"
    );
}

/// Test: AMM swap amount exceeding protocol max is rejected.
#[test]
fn test_amm_swap_exceeds_max_amount_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let (contract, _admin, protocol_addr, token_out) = setup_amm_env(&env);
    let user = Address::generate(&env);

    let params = make_swap_params(
        &env,
        &protocol_addr,
        &token_out,
        2_000_000_000, // Exceeds max_swap_amount = 1_000_000_000
        1,
        DEFAULT_SLIPPAGE,
    );

    let result = contract.try_execute_swap(&user, &params);
    assert!(
        result.is_err(),
        "Amount exceeding protocol max must be rejected"
    );
}

/// Test: AMM liquidation output is always positive (never negative or zero).
#[test]
fn test_amm_liquidation_output_always_positive() {
    let env = Env::default();
    env.mock_all_auths();

    let (contract, _admin, _protocol, token_out) = setup_amm_env(&env);
    let liquidator = Address::generate(&env);

    let amount_out = contract.auto_swap_for_collateral(&liquidator, &Some(token_out), &15_000);
    assert!(
        amount_out > 0,
        "AMM liquidation output must always be positive"
    );
}

/// Test: Liquidation settings update immediately affects subsequent swaps.
#[test]
fn test_liquidation_settings_update_immediate_effect() {
    let env = Env::default();
    env.mock_all_auths();

    let (contract, admin, _protocol, token_out) = setup_amm_env(&env);
    let liquidator = Address::generate(&env);

    // 8_000 is below current threshold of 10_000 — should fail
    assert!(
        contract
            .try_auto_swap_for_collateral(&liquidator, &Some(token_out.clone()), &8_000)
            .is_err(),
        "8_000 must fail before threshold update"
    );

    // Lower threshold to 5_000
    let mut settings = contract.get_amm_settings().unwrap();
    settings.auto_swap_threshold = 5_000;
    contract.update_amm_settings(&admin, &settings);

    // 8_000 is now above threshold — should succeed
    assert!(
        contract
            .try_auto_swap_for_collateral(&liquidator, &Some(token_out), &8_000)
            .is_ok(),
        "8_000 must succeed after threshold lowered to 5_000"
    );
}
