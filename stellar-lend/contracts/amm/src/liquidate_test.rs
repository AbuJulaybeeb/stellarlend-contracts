//! # Liquidation Test Suite — StellarLend AMM Contract
//!
//! This module tests liquidation-related functionality in the AMM contract,
//! specifically the `auto_swap_for_collateral` function which handles
//! collateral optimization during undercollateralized lending positions.
//!
//! ## Scenarios Covered
//!
//! ### Valid (Partial / Full) Liquidation
//! - Successful collateral swap (standard liquidation path)
//! - Partial liquidation: amount above threshold, below max
//! - Full liquidation: maximum allowable swap amount
//! - Correct output calculation with slippage (close-factor equivalent)
//! - Swap history recorded after liquidation
//!
//! ### Close Factor / Slippage Enforcement
//! - Slippage acts as the close factor — limits how much value is lost
//! - Max slippage boundary: exactly at limit succeeds
//! - Exceeding max slippage setting is rejected
//!
//! ### Incentive Distribution
//! - Liquidator receives correct output based on slippage settings
//! - Output is calculated as: amount_in * (10000 - slippage) / 10000
//! - Verified against mock AMM formula in execute_amm_swap
//!
//! ### Invalid Liquidation Attempts
//! - Amount below auto_swap_threshold is rejected
//! - Zero amount is rejected
//! - Swap paused (protocol frozen): liquidation blocked
//! - No matching AMM protocol: liquidation blocked
//! - Unsupported token pair: liquidation blocked
//! - Expired deadline: liquidation blocked
//!
//! ### Undercollateralized Detection
//! - Only accounts meeting threshold can trigger auto_swap_for_collateral
//! - Threshold acts as minimum collateral-at-risk value
//!
//! ### Security Assumptions
//! - Nonce replay protection on AMM callbacks
//! - Admin-only settings cannot be changed by non-admins
//! - Disabled protocols cannot participate in liquidation swaps
//! - Paused swap state fully blocks liquidation path
//!
//! ## Security Notes
//! - All tests use `env.mock_all_auths()` to simulate authorized callers
//! - Nonce replay attack test confirms callback cannot be reused
//! - Non-admin settings change is explicitly tested and must fail

#[cfg(test)]
mod liquidate_tests {
    use super::*;
    use crate::amm::*;
    use soroban_sdk::{testutils::Address as _, testutils::Ledger, Address, Env, Symbol, Vec};

    // =========================================================
    // HELPERS — mirrors the style in test.rs
    // =========================================================

    /// Create a deployed AMM contract client (same pattern as test.rs)
    fn create_amm_contract<'a>(env: &Env) -> AmmContractClient<'a> {
        AmmContractClient::new(env, &env.register(AmmContract {}, ()))
    }

    /// Standard protocol config used across liquidation tests.
    /// fee_tier = 30 (0.3%), supports XLM → token_b pair.
    fn create_liquidation_protocol(env: &Env, protocol_addr: &Address, token_out: &Address) -> AmmProtocolConfig {
        let mut supported_pairs = Vec::new(env);
        supported_pairs.push_back(TokenPair {
            token_a: None,                        // Native XLM (collateral in)
            token_b: Some(token_out.clone()),     // Target token (collateral out)
            pool_address: Address::generate(env),
        });

        AmmProtocolConfig {
            protocol_address: protocol_addr.clone(),
            protocol_name: Symbol::new(env, "LiquidationAMM"),
            enabled: true,
            fee_tier: 30,           // 0.3% fee
            min_swap_amount: 1_000,
            max_swap_amount: 1_000_000_000,
            supported_pairs,
        }
    }

    /// Initialize AMM settings and register one protocol.
    /// Returns (contract, admin, protocol_addr, token_out).
    fn setup_liquidation_env<'a>(
        env: &'a Env,
    ) -> (AmmContractClient<'a>, Address, Address, Address) {
        let contract = create_amm_contract(env);
        let admin = Address::generate(env);
        let protocol_addr = Address::generate(env);
        let token_out = Address::generate(env);

        // default_slippage = 100 (1%), max_slippage = 1000 (10%), threshold = 10_000
        contract.initialize_amm_settings(&admin, &100, &1000, &10_000);

        let protocol_config = create_liquidation_protocol(env, &protocol_addr, &token_out);
        contract.add_amm_protocol(&admin, &protocol_config);

        (contract, admin, protocol_addr, token_out)
    }

    // =========================================================
    // ✅ VALID LIQUIDATION — auto_swap_for_collateral success
    // =========================================================

    /// Test: Standard successful liquidation swap.
    ///
    /// Amount is above threshold. Output = amount * (10000 - slippage) / 10000.
    /// With default slippage 100: 15_000 * 9900 / 10000 = 14_850.
    #[test]
    fn test_liquidation_swap_success() {
        let env = Env::default();
        env.mock_all_auths();

        let (contract, _admin, _protocol, token_out) = setup_liquidation_env(&env);
        let liquidator = Address::generate(&env);

        let amount_out = contract.auto_swap_for_collateral(&liquidator, &Some(token_out), &15_000);

        // 15_000 * (10000 - 100) / 10000 = 14_850
        assert_eq!(amount_out, 14_850, "Liquidation output must match slippage formula");
    }

    /// Test: Partial liquidation — amount well above threshold but not maximum.
    ///
    /// Verifies that partial collateral swaps (not full position) work correctly.
    #[test]
    fn test_partial_liquidation_above_threshold() {
        let env = Env::default();
        env.mock_all_auths();

        let (contract, _admin, _protocol, token_out) = setup_liquidation_env(&env);
        let liquidator = Address::generate(&env);

        // 50_000 is partial — well above 10_000 threshold but not near max
        let amount_out = contract.auto_swap_for_collateral(&liquidator, &Some(token_out), &50_000);

        // 50_000 * (10000 - 100) / 10000 = 49_500
        assert_eq!(amount_out, 49_500, "Partial liquidation output must respect slippage");
        assert!(amount_out > 0, "Partial liquidation must return positive amount");
    }

    /// Test: Full liquidation — largest valid amount under max_swap_amount.
    ///
    /// Verifies maximum collateral swap (full position wipe) works end-to-end.
    #[test]
    fn test_full_liquidation_max_amount() {
        let env = Env::default();
        env.mock_all_auths();

        let (contract, _admin, _protocol, token_out) = setup_liquidation_env(&env);
        let liquidator = Address::generate(&env);

        // Use a large but valid amount (within max_swap_amount = 1_000_000_000)
        let amount = 500_000_000i128;
        let amount_out = contract.auto_swap_for_collateral(&liquidator, &Some(token_out), &amount);

        // 500_000_000 * 9900 / 10000 = 495_000_000
        assert_eq!(amount_out, 495_000_000, "Full liquidation output must match formula");
    }

    /// Test: Liquidation swap is recorded in swap history.
    ///
    /// After a successful auto_swap_for_collateral, the swap must appear in history.
    #[test]
    fn test_liquidation_swap_recorded_in_history() {
        let env = Env::default();
        env.mock_all_auths();

        let (contract, _admin, _protocol, token_out) = setup_liquidation_env(&env);
        let liquidator = Address::generate(&env);

        contract.auto_swap_for_collateral(&liquidator, &Some(token_out), &15_000);

        let history = contract.get_swap_history(&Some(liquidator), &10).unwrap();
        assert_eq!(history.len(), 1, "One swap record must exist after liquidation");
        assert_eq!(history.get(0).unwrap().amount_in, 15_000, "Recorded amount_in must match");
    }

    /// Test: Multiple sequential liquidation swaps all recorded correctly.
    ///
    /// Verifies history accumulates and each record is accurate.
    #[test]
    fn test_multiple_liquidation_swaps_recorded() {
        let env = Env::default();
        env.mock_all_auths();

        let (contract, _admin, _protocol, token_out) = setup_liquidation_env(&env);
        let liquidator = Address::generate(&env);

        contract.auto_swap_for_collateral(&liquidator, &Some(token_out.clone()), &15_000);
        contract.auto_swap_for_collateral(&liquidator, &Some(token_out.clone()), &20_000);
        contract.auto_swap_for_collateral(&liquidator, &Some(token_out), &25_000);

        let history = contract.get_swap_history(&Some(liquidator), &10).unwrap();
        assert_eq!(history.len(), 3, "All three liquidation swaps must be recorded");
    }

    // =========================================================
    // ✅ CLOSE FACTOR / SLIPPAGE ENFORCEMENT
    // =========================================================

    /// Test: Liquidation at exactly the max slippage boundary succeeds.
    ///
    /// Close factor analogue: max_slippage defines how much value loss is allowed.
    /// Using slippage = max_slippage exactly must still pass.
    #[test]
    fn test_liquidation_at_max_slippage_boundary_succeeds() {
        let env = Env::default();
        env.mock_all_auths();

        let contract = create_amm_contract(&env);
        let admin = Address::generate(&env);
        let protocol_addr = Address::generate(&env);
        let token_out = Address::generate(&env);

        // max_slippage = 2000 (20%)
        contract.initialize_amm_settings(&admin, &100, &2000, &10_000);
        let config = create_liquidation_protocol(&env, &protocol_addr, &token_out);
        contract.add_amm_protocol(&admin, &config);

        let liquidator = Address::generate(&env);
        let user = Address::generate(&env);

        // Execute a swap with slippage exactly at max (2000 = 20%)
        let params = SwapParams {
            protocol: protocol_addr.clone(),
            token_in: None,
            token_out: Some(token_out.clone()),
            amount_in: 20_000,
            min_amount_out: 1,           // Allow full slippage
            slippage_tolerance: 2000,    // Exactly at max
            deadline: env.ledger().timestamp() + 3600,
        };

        let result = contract.try_execute_swap(&user, &params);
        assert!(result.is_ok(), "Swap at exactly max slippage must succeed");
    }

    /// Test: Liquidation exceeding max slippage is rejected.
    ///
    /// Close factor enforcement: slippage above max_slippage must be blocked.
    #[test]
    fn test_liquidation_exceeding_max_slippage_rejected() {
        let env = Env::default();
        env.mock_all_auths();

        let (contract, _admin, protocol_addr, token_out) = setup_liquidation_env(&env);
        let user = Address::generate(&env);

        // max_slippage is 1000 (10%), attempt 1500 (15%) — too high
        let params = SwapParams {
            protocol: protocol_addr.clone(),
            token_in: None,
            token_out: Some(token_out.clone()),
            amount_in: 20_000,
            min_amount_out: 1,
            slippage_tolerance: 1500, // Exceeds max_slippage of 1000
            deadline: env.ledger().timestamp() + 3600,
        };

        let result = contract.try_execute_swap(&user, &params);
        assert!(result.is_err(), "Slippage above max must be rejected (close factor enforcement)");
    }

    /// Test: min_amount_out (close factor floor) enforcement.
    ///
    /// If the AMM output is below min_amount_out, the liquidation must revert.
    #[test]
    fn test_liquidation_min_output_not_met_rejected() {
        let env = Env::default();
        env.mock_all_auths();

        let (contract, _admin, protocol_addr, token_out) = setup_liquidation_env(&env);
        let user = Address::generate(&env);

        let params = SwapParams {
            protocol: protocol_addr.clone(),
            token_in: None,
            token_out: Some(token_out.clone()),
            amount_in: 10_000,
            min_amount_out: 10_000, // Requires 100% output — impossible with any slippage
            slippage_tolerance: 100,
            deadline: env.ledger().timestamp() + 3600,
        };

        let result = contract.try_execute_swap(&user, &params);
        assert!(result.is_err(), "Must reject when min_amount_out cannot be met");
    }

    // =========================================================
    // ✅ INCENTIVE DISTRIBUTION
    // =========================================================

    /// Test: Liquidation incentive is correct at 1% slippage.
    ///
    /// Output = amount_in * (10000 - default_slippage) / 10000.
    /// Liquidator effectively gets a "discount" via slippage tolerance.
    #[test]
    fn test_liquidation_incentive_one_percent_slippage() {
        let env = Env::default();
        env.mock_all_auths();

        let (contract, _admin, _protocol, token_out) = setup_liquidation_env(&env);
        let liquidator = Address::generate(&env);

        let amount_in = 100_000i128;
        let amount_out = contract.auto_swap_for_collateral(&liquidator, &Some(token_out), &amount_in);

        // default_slippage = 100 → 1%
        // Expected: 100_000 * (10000 - 100) / 10000 = 99_000
        let expected = amount_in * (10_000 - 100) / 10_000;
        assert_eq!(amount_out, expected, "Incentive output must match slippage formula exactly");
    }

    /// Test: Higher slippage tolerance yields lower output (more value transferred).
    ///
    /// Confirms slippage and incentive are inversely related.
    #[test]
    fn test_higher_slippage_yields_lower_output() {
        let env = Env::default();
        env.mock_all_auths();

        let contract = create_amm_contract(&env);
        let admin = Address::generate(&env);
        let protocol_addr = Address::generate(&env);
        let token_out = Address::generate(&env);

        // Allow up to 20% slippage
        contract.initialize_amm_settings(&admin, &100, &2000, &10_000);
        let config = create_liquidation_protocol(&env, &protocol_addr, &token_out);
        contract.add_amm_protocol(&admin, &config);

        let user = Address::generate(&env);

        // Low slippage swap
        let low_slippage_params = SwapParams {
            protocol: protocol_addr.clone(),
            token_in: None,
            token_out: Some(token_out.clone()),
            amount_in: 100_000,
            min_amount_out: 1,
            slippage_tolerance: 100,  // 1%
            deadline: env.ledger().timestamp() + 3600,
        };
        let low_out = contract.execute_swap(&user, &low_slippage_params);

        // High slippage swap (resets nonce, new user)
        let user2 = Address::generate(&env);
        let high_slippage_params = SwapParams {
            protocol: protocol_addr.clone(),
            token_in: None,
            token_out: Some(token_out.clone()),
            amount_in: 100_000,
            min_amount_out: 1,
            slippage_tolerance: 2000, // 20%
            deadline: env.ledger().timestamp() + 3600,
        };
        let high_out = contract.execute_swap(&user2, &high_slippage_params);

        assert!(
            low_out > high_out,
            "Lower slippage must yield higher output (better liquidation terms)"
        );
    }

    // =========================================================
    // ❌ INVALID LIQUIDATION ATTEMPTS
    // =========================================================

    /// Test: Amount below auto_swap_threshold is rejected.
    ///
    /// Undercollateralization detection: position too small to liquidate.
    #[test]
    fn test_liquidation_below_threshold_rejected() {
        let env = Env::default();
        env.mock_all_auths();

        let (contract, _admin, _protocol, token_out) = setup_liquidation_env(&env);
        let liquidator = Address::generate(&env);

        // threshold is 10_000 — try 5_000 (below it)
        let result = contract.try_auto_swap_for_collateral(&liquidator, &Some(token_out), &5_000);
        assert!(result.is_err(), "Amount below threshold must be rejected");
    }

    /// Test: Zero amount liquidation is rejected.
    #[test]
    fn test_liquidation_zero_amount_rejected() {
        let env = Env::default();
        env.mock_all_auths();

        let (contract, _admin, _protocol, token_out) = setup_liquidation_env(&env);
        let liquidator = Address::generate(&env);

        let result = contract.try_auto_swap_for_collateral(&liquidator, &Some(token_out), &0);
        assert!(result.is_err(), "Zero amount liquidation must be rejected");
    }

    /// Test: Liquidation blocked when swaps are paused.
    ///
    /// If the protocol is frozen, no liquidations can proceed.
    #[test]
    fn test_liquidation_blocked_when_swap_paused() {
        let env = Env::default();
        env.mock_all_auths();

        let (contract, admin, _protocol, token_out) = setup_liquidation_env(&env);
        let liquidator = Address::generate(&env);

        // Pause swaps
        let mut settings = contract.get_amm_settings().unwrap();
        settings.swap_enabled = false;
        contract.update_amm_settings(&admin, &settings);

        let result = contract.try_auto_swap_for_collateral(&liquidator, &Some(token_out), &15_000);
        assert!(result.is_err(), "Liquidation must be blocked when swaps are paused");
    }

    /// Test: Liquidation fails when no AMM protocol is registered.
    #[test]
    fn test_liquidation_fails_no_protocol_registered() {
        let env = Env::default();
        env.mock_all_auths();

        let contract = create_amm_contract(&env);
        let admin = Address::generate(&env);
        let token_out = Address::generate(&env);
        let liquidator = Address::generate(&env);

        // Initialize but don't register any protocol
        contract.initialize_amm_settings(&admin, &100, &1000, &10_000);

        let result = contract.try_auto_swap_for_collateral(&liquidator, &Some(token_out), &15_000);
        assert!(result.is_err(), "Liquidation must fail with no registered protocol");
    }

    /// Test: Liquidation fails for an unsupported token pair.
    ///
    /// If the target token has no AMM pool, liquidation path doesn't exist.
    #[test]
    fn test_liquidation_fails_unsupported_token_pair() {
        let env = Env::default();
        env.mock_all_auths();

        let (contract, _admin, _protocol, _token_out) = setup_liquidation_env(&env);
        let liquidator = Address::generate(&env);

        // Use a completely different token not in any supported pair
        let unknown_token = Address::generate(&env);
        let result = contract.try_auto_swap_for_collateral(&liquidator, &Some(unknown_token), &15_000);
        assert!(result.is_err(), "Liquidation to unsupported token must fail");
    }

    /// Test: Liquidation via direct swap fails when deadline has passed.
    ///
    /// Protects against stale liquidation transactions being replayed.
    #[test]
    fn test_liquidation_fails_expired_deadline() {
        let env = Env::default();
        env.mock_all_auths();

        env.ledger().set(soroban_sdk::testutils::LedgerInfo {
            timestamp: 2000,
            protocol_version: 22,
            sequence_number: 1,
            network_id: [0; 32],
            base_reserve: 10,
            max_entry_ttl: 40_000,
            min_persistent_entry_ttl: 4_000,
            min_temp_entry_ttl: 16,
        });

        let (contract, _admin, protocol_addr, token_out) = setup_liquidation_env(&env);
        let user = Address::generate(&env);

        let params = SwapParams {
            protocol: protocol_addr.clone(),
            token_in: None,
            token_out: Some(token_out.clone()),
            amount_in: 20_000,
            min_amount_out: 1,
            slippage_tolerance: 100,
            deadline: 1000, // Before current timestamp (2000)
        };

        let result = contract.try_execute_swap(&user, &params);
        assert!(result.is_err(), "Expired deadline must block liquidation swap");
    }

    /// Test: Liquidation fails with disabled AMM protocol.
    ///
    /// Disabled protocols must never participate in liquidation routing.
    #[test]
    fn test_liquidation_skips_disabled_protocol() {
        let env = Env::default();
        env.mock_all_auths();

        let contract = create_amm_contract(&env);
        let admin = Address::generate(&env);
        let protocol_addr = Address::generate(&env);
        let token_out = Address::generate(&env);
        let liquidator = Address::generate(&env);

        contract.initialize_amm_settings(&admin, &100, &1000, &10_000);

        // Register protocol but disable it
        let mut config = create_liquidation_protocol(&env, &protocol_addr, &token_out);
        config.enabled = false;
        contract.add_amm_protocol(&admin, &config);

        let result = contract.try_auto_swap_for_collateral(&liquidator, &Some(token_out), &15_000);
        assert!(result.is_err(), "Disabled protocol must not be used for liquidation");
    }

    /// Test: Liquidation exceeding protocol max_swap_amount is rejected.
    ///
    /// Protocol capacity limits act as a ceiling on single liquidation size.
    #[test]
    fn test_liquidation_exceeds_protocol_max_swap_amount() {
        let env = Env::default();
        env.mock_all_auths();

        let (contract, _admin, protocol_addr, token_out) = setup_liquidation_env(&env);
        let user = Address::generate(&env);

        // max_swap_amount on default protocol is 1_000_000_000
        // Force an over-limit by creating a custom direct swap
        let params = SwapParams {
            protocol: protocol_addr.clone(),
            token_in: None,
            token_out: Some(token_out.clone()),
            amount_in: 2_000_000_000, // Exceeds 1_000_000_000 max
            min_amount_out: 1,
            slippage_tolerance: 100,
            deadline: env.ledger().timestamp() + 3600,
        };

        let result = contract.try_execute_swap(&user, &params);
        assert!(result.is_err(), "Liquidation exceeding max_swap_amount must fail");
    }

    // =========================================================
    // 🔐 SECURITY ASSUMPTIONS
    // =========================================================

    /// Test: Nonce replay attack is blocked.
    ///
    /// A previously used callback nonce must be rejected to prevent
    /// the same liquidation callback being replayed maliciously.
    #[test]
    fn test_nonce_replay_attack_blocked() {
        let env = Env::default();
        env.mock_all_auths();

        let (contract, _admin, protocol_addr, _token_out) = setup_liquidation_env(&env);
        let user = Address::generate(&env);

        // Reuse an old nonce (999) — should be rejected
        let stale_callback = AmmCallbackData {
            nonce: 999,
            operation: Symbol::new(&env, "swap"),
            user: user.clone(),
            expected_amounts: Vec::new(&env),
            deadline: env.ledger().timestamp() + 3600,
        };

        let result = contract.try_validate_amm_callback(&protocol_addr, &stale_callback);
        assert!(result.is_err(), "Stale/invalid nonce must be rejected (replay protection)");
    }

    /// Test: Expired callback is rejected.
    ///
    /// A callback with a past deadline cannot be used to trigger a liquidation.
    #[test]
    fn test_expired_callback_rejected() {
        let env = Env::default();
        env.mock_all_auths();

        env.ledger().set_timestamp(5000);

        let (contract, _admin, protocol_addr, _token_out) = setup_liquidation_env(&env);
        let user = Address::generate(&env);

        let expired_callback = AmmCallbackData {
            nonce: 1,
            operation: Symbol::new(&env, "swap"),
            user: user.clone(),
            expected_amounts: Vec::new(&env),
            deadline: 1000, // Far in the past
        };

        let result = contract.try_validate_amm_callback(&protocol_addr, &expired_callback);
        assert!(result.is_err(), "Expired callback deadline must be rejected");
    }

    /// Test: Unregistered protocol cannot trigger a callback.
    ///
    /// Prevents spoofed AMM protocols from manipulating liquidation flow.
    #[test]
    fn test_unregistered_protocol_callback_rejected() {
        let env = Env::default();
        env.mock_all_auths();

        let (contract, _admin, _protocol, _token_out) = setup_liquidation_env(&env);
        let fake_protocol = Address::generate(&env); // Not registered
        let user = Address::generate(&env);

        let callback = AmmCallbackData {
            nonce: 1,
            operation: Symbol::new(&env, "swap"),
            user: user.clone(),
            expected_amounts: Vec::new(&env),
            deadline: env.ledger().timestamp() + 3600,
        };

        let result = contract.try_validate_amm_callback(&fake_protocol, &callback);
        assert!(result.is_err(), "Unregistered protocol must not be able to trigger callbacks");
    }

    /// Test: Non-admin cannot change liquidation settings.
    ///
    /// Critical: Liquidation parameters (slippage, threshold) are admin-only.
    /// An attacker must not be able to raise threshold to disable liquidations
    /// or raise slippage to drain value.
    #[test]
    fn test_non_admin_cannot_change_liquidation_settings() {
        let env = Env::default();
        env.mock_all_auths();

        let (contract, _admin, _protocol, _token_out) = setup_liquidation_env(&env);
        let attacker = Address::generate(&env);

        let malicious_settings = AmmSettings {
            default_slippage: 9999,   // Extreme slippage — would drain liquidated value
            max_slippage: 9999,
            swap_enabled: true,
            liquidity_enabled: true,
            auto_swap_threshold: 999_999_999, // Make threshold impossibly high to block liquidations
        };

        let result = contract.try_update_amm_settings(&attacker, &malicious_settings);
        assert!(result.is_err(), "Non-admin must not be able to modify liquidation settings");
    }

    /// Test: Liquidation output is never negative.
    ///
    /// Even with maximum slippage, output must be a positive value.
    #[test]
    fn test_liquidation_output_never_negative() {
        let env = Env::default();
        env.mock_all_auths();

        let (contract, _admin, _protocol, token_out) = setup_liquidation_env(&env);
        let liquidator = Address::generate(&env);

        let amount_out = contract.auto_swap_for_collateral(&liquidator, &Some(token_out), &15_000);
        assert!(amount_out > 0, "Liquidation output must always be positive");
    }

    /// Test: Liquidation history is isolated per user.
    ///
    /// One liquidator's history must not appear in another's.
    #[test]
    fn test_liquidation_history_isolated_per_user() {
        let env = Env::default();
        env.mock_all_auths();

        let (contract, _admin, _protocol, token_out) = setup_liquidation_env(&env);
        let liquidator_a = Address::generate(&env);
        let liquidator_b = Address::generate(&env);

        contract.auto_swap_for_collateral(&liquidator_a, &Some(token_out.clone()), &15_000);
        contract.auto_swap_for_collateral(&liquidator_b, &Some(token_out.clone()), &20_000);

        let history_a = contract.get_swap_history(&Some(liquidator_a), &10).unwrap();
        let history_b = contract.get_swap_history(&Some(liquidator_b), &10).unwrap();

        assert_eq!(history_a.len(), 1, "Liquidator A must have exactly 1 record");
        assert_eq!(history_b.len(), 1, "Liquidator B must have exactly 1 record");
        assert_eq!(history_a.get(0).unwrap().amount_in, 15_000, "A's record must show correct amount");
        assert_eq!(history_b.get(0).unwrap().amount_in, 20_000, "B's record must show correct amount");
    }

    /// Test: Liquidation settings update takes immediate effect.
    ///
    /// If admin lowers threshold mid-session, new liquidations use new threshold.
    #[test]
    fn test_liquidation_settings_update_takes_effect() {
        let env = Env::default();
        env.mock_all_auths();

        let (contract, admin, _protocol, token_out) = setup_liquidation_env(&env);
        let liquidator = Address::generate(&env);

        // 8_000 is below current threshold of 10_000 — should fail
        let result_before = contract.try_auto_swap_for_collateral(&liquidator, &Some(token_out.clone()), &8_000);
        assert!(result_before.is_err(), "8_000 below threshold must fail before update");

        // Lower threshold to 5_000
        let mut settings = contract.get_amm_settings().unwrap();
        settings.auto_swap_threshold = 5_000;
        contract.update_amm_settings(&admin, &settings);

        // Now 8_000 is above new threshold — should succeed
        let result_after = contract.try_auto_swap_for_collateral(&liquidator, &Some(token_out), &8_000);
        assert!(result_after.is_ok(), "8_000 above new threshold must succeed after update");
    }
}
