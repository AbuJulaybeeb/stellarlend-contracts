#![cfg(test)]

use crate::bridge::*;
use soroban_sdk::{testutils::Address as _, Address, Env, String};

fn setup() -> (Env, BridgeContractClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let id = env.register(BridgeContract, ());
    let client = BridgeContractClient::new(&env, &id);
    let admin = Address::generate(&env);
    client.init(&admin);
    (env, client, admin)
}

fn s(env: &Env, v: &str) -> String {
    String::from_str(env, v)
}

fn default_bridge(client: &BridgeContractClient, env: &Env, admin: &Address) {
    client.register_bridge(admin, &s(env, "eth-mainnet"), &50u64, &1_000i128);
}

// ── init ──────────────────────────────────────────────────────────────────────

#[test]
fn init_sets_admin() {
    let (_, client, admin) = setup();
    assert_eq!(client.get_admin(), admin);
}

#[test]
#[should_panic(expected = "Error(Contract, #1)")]
fn init_twice_panics() {
    let (env, client, _) = setup();
    client.init(&Address::generate(&env));
}

// ── register_bridge ───────────────────────────────────────────────────────────

#[test]
fn register_bridge_success() {
    let (env, client, admin) = setup();
    default_bridge(&client, &env, &admin);
    let cfg = client.get_bridge_config(&s(&env, "eth-mainnet"));
    assert_eq!(cfg.fee_bps, 50);
    assert_eq!(cfg.min_amount, 1_000);
    assert!(cfg.active);
    assert_eq!(cfg.total_deposited, 0);
    assert_eq!(cfg.total_withdrawn, 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn register_bridge_non_admin_panics() {
    let (env, client, _) = setup();
    let rando = Address::generate(&env);
    client.register_bridge(&rando, &s(&env, "bsc"), &10u64, &100i128);
}

#[test]
#[should_panic(expected = "Error(Contract, #4)")]
fn register_bridge_duplicate_panics() {
    let (env, client, admin) = setup();
    default_bridge(&client, &env, &admin);
    client.register_bridge(&admin, &s(&env, "eth-mainnet"), &50u64, &1_000i128);
}

#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn register_bridge_fee_too_high_panics() {
    let (env, client, admin) = setup();
    client.register_bridge(&admin, &s(&env, "bsc"), &1_001u64, &100i128);
}

#[test]
fn register_bridge_max_fee_ok() {
    let (env, client, admin) = setup();
    client.register_bridge(&admin, &s(&env, "bsc"), &1_000u64, &100i128);
    assert_eq!(client.get_bridge_config(&s(&env, "bsc")).fee_bps, 1_000);
}

#[test]
#[should_panic(expected = "Error(Contract, #8)")]
fn register_bridge_empty_id_panics() {
    let (env, client, admin) = setup();
    client.register_bridge(&admin, &s(&env, ""), &10u64, &100i128);
}

#[test]
#[should_panic(expected = "Error(Contract, #8)")]
fn register_bridge_id_too_long_panics() {
    let (env, client, admin) = setup();
    let long = String::from_str(&env, &"a".repeat(65));
    client.register_bridge(&admin, &long, &10u64, &100i128);
}

#[test]
#[should_panic(expected = "Error(Contract, #10)")]
fn register_bridge_negative_min_amount_panics() {
    let (env, client, admin) = setup();
    client.register_bridge(&admin, &s(&env, "bsc"), &10u64, &-1i128);
}

// ── set_bridge_fee ─────────────────────────────────────────────────────────────

#[test]
fn set_bridge_fee_success() {
    let (env, client, admin) = setup();
    default_bridge(&client, &env, &admin);
    client.set_bridge_fee(&admin, &s(&env, "eth-mainnet"), &200u64);
    assert_eq!(
        client.get_bridge_config(&s(&env, "eth-mainnet")).fee_bps,
        200
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn set_bridge_fee_not_found_panics() {
    let (env, client, admin) = setup();
    client.set_bridge_fee(&admin, &s(&env, "ghost"), &10u64);
}

#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn set_bridge_fee_exceeds_cap_panics() {
    let (env, client, admin) = setup();
    default_bridge(&client, &env, &admin);
    client.set_bridge_fee(&admin, &s(&env, "eth-mainnet"), &9_999u64);
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn set_bridge_fee_unauthorized_panics() {
    let (env, client, admin) = setup();
    default_bridge(&client, &env, &admin);
    let rando = Address::generate(&env);
    client.set_bridge_fee(&rando, &s(&env, "eth-mainnet"), &100u64);
}

// ── set_bridge_active ──────────────────────────────────────────────────────────

#[test]
#[should_panic(expected = "Error(Contract, #6)")]
fn deactivate_bridge_stops_deposits() {
    let (env, client, admin) = setup();
    default_bridge(&client, &env, &admin);
    client.set_bridge_active(&admin, &s(&env, "eth-mainnet"), &false);
    let user = Address::generate(&env);
    client.bridge_deposit(&user, &s(&env, "eth-mainnet"), &10_000i128);
}

#[test]
fn reactivate_bridge_allows_deposits() {
    let (env, client, admin) = setup();
    default_bridge(&client, &env, &admin);
    client.set_bridge_active(&admin, &s(&env, "eth-mainnet"), &false);
    client.set_bridge_active(&admin, &s(&env, "eth-mainnet"), &true);
    let user = Address::generate(&env);
    client.bridge_deposit(&user, &s(&env, "eth-mainnet"), &10_000i128);
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn set_bridge_active_not_found_panics() {
    let (env, client, admin) = setup();
    client.set_bridge_active(&admin, &s(&env, "ghost"), &false);
}

// ── bridge_deposit ─────────────────────────────────────────────────────────────

#[test]
fn deposit_returns_correct_net() {
    let (env, client, admin) = setup();
    default_bridge(&client, &env, &admin); // fee_bps=50, min=1_000
    let user = Address::generate(&env);
    // fee = 100_000 * 50 / 10_000 = 500  →  net = 99_500
    let net = client.bridge_deposit(&user, &s(&env, "eth-mainnet"), &100_000i128);
    assert_eq!(net, 99_500);
}

#[test]
fn deposit_zero_fee_bridge() {
    let (env, client, admin) = setup();
    client.register_bridge(&admin, &s(&env, "free"), &0u64, &1i128);
    let user = Address::generate(&env);
    let net = client.bridge_deposit(&user, &s(&env, "free"), &50_000i128);
    assert_eq!(net, 50_000);
}

#[test]
fn deposit_accumulates_total_deposited() {
    let (env, client, admin) = setup();
    default_bridge(&client, &env, &admin);
    let user = Address::generate(&env);
    client.bridge_deposit(&user, &s(&env, "eth-mainnet"), &20_000i128);
    client.bridge_deposit(&user, &s(&env, "eth-mainnet"), &30_000i128);
    assert_eq!(
        client
            .get_bridge_config(&s(&env, "eth-mainnet"))
            .total_deposited,
        50_000
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #11)")]
fn deposit_zero_amount_panics() {
    let (env, client, admin) = setup();
    default_bridge(&client, &env, &admin);
    let user = Address::generate(&env);
    client.bridge_deposit(&user, &s(&env, "eth-mainnet"), &0i128);
}

#[test]
#[should_panic(expected = "Error(Contract, #11)")]
fn deposit_negative_amount_panics() {
    let (env, client, admin) = setup();
    default_bridge(&client, &env, &admin);
    let user = Address::generate(&env);
    client.bridge_deposit(&user, &s(&env, "eth-mainnet"), &-1i128);
}

#[test]
#[should_panic(expected = "Error(Contract, #12)")]
fn deposit_below_minimum_panics() {
    let (env, client, admin) = setup();
    default_bridge(&client, &env, &admin); // min=1_000
    let user = Address::generate(&env);
    client.bridge_deposit(&user, &s(&env, "eth-mainnet"), &999i128);
}

#[test]
fn deposit_exactly_minimum_succeeds() {
    let (env, client, admin) = setup();
    default_bridge(&client, &env, &admin);
    let user = Address::generate(&env);
    client.bridge_deposit(&user, &s(&env, "eth-mainnet"), &1_000i128);
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn deposit_unknown_bridge_panics() {
    let (env, client, _) = setup();
    let user = Address::generate(&env);
    client.bridge_deposit(&user, &s(&env, "ghost"), &50_000i128);
}

// ── bridge_withdraw ────────────────────────────────────────────────────────────

#[test]
fn withdraw_accumulates_total_withdrawn() {
    let (env, client, admin) = setup();
    default_bridge(&client, &env, &admin);
    let recip = Address::generate(&env);
    client.bridge_withdraw(&admin, &s(&env, "eth-mainnet"), &recip, &40_000i128);
    assert_eq!(
        client
            .get_bridge_config(&s(&env, "eth-mainnet"))
            .total_withdrawn,
        40_000
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn withdraw_non_admin_panics() {
    let (env, client, admin) = setup();
    default_bridge(&client, &env, &admin);
    let rando = Address::generate(&env);
    let recip = Address::generate(&env);
    client.bridge_withdraw(&rando, &s(&env, "eth-mainnet"), &recip, &5_000i128);
}

#[test]
#[should_panic(expected = "Error(Contract, #11)")]
fn withdraw_zero_amount_panics() {
    let (env, client, admin) = setup();
    default_bridge(&client, &env, &admin);
    let recip = Address::generate(&env);
    client.bridge_withdraw(&admin, &s(&env, "eth-mainnet"), &recip, &0i128);
}

#[test]
#[should_panic(expected = "Error(Contract, #12)")]
fn withdraw_below_minimum_panics() {
    let (env, client, admin) = setup();
    default_bridge(&client, &env, &admin); // min=1_000
    let recip = Address::generate(&env);
    client.bridge_withdraw(&admin, &s(&env, "eth-mainnet"), &recip, &500i128);
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn withdraw_unknown_bridge_panics() {
    let (env, client, admin) = setup();
    let recip = Address::generate(&env);
    client.bridge_withdraw(&admin, &s(&env, "ghost"), &recip, &5_000i128);
}

// ── list_bridges ───────────────────────────────────────────────────────────────

#[test]
fn list_bridges_empty() {
    let (_, client, _) = setup();
    assert_eq!(client.list_bridges().len(), 0);
}

#[test]
fn list_bridges_multiple() {
    let (env, client, admin) = setup();
    client.register_bridge(&admin, &s(&env, "bsc"), &10u64, &100i128);
    client.register_bridge(&admin, &s(&env, "polygon"), &20u64, &200i128);
    client.register_bridge(&admin, &s(&env, "avax"), &30u64, &300i128);
    assert_eq!(client.list_bridges().len(), 3);
}

// ── transfer_admin ─────────────────────────────────────────────────────────────

#[test]
fn transfer_admin_success() {
    let (env, client, admin) = setup();
    let new_admin = Address::generate(&env);
    client.transfer_admin(&admin, &new_admin);
    assert_eq!(client.get_admin(), new_admin);
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn transfer_admin_non_admin_panics() {
    let (env, client, _) = setup();
    let rando = Address::generate(&env);
    let new_admin = Address::generate(&env);
    client.transfer_admin(&rando, &new_admin);
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn old_admin_loses_rights_after_transfer() {
    let (env, client, admin) = setup();
    let new_admin = Address::generate(&env);
    client.transfer_admin(&admin, &new_admin);
    client.register_bridge(&admin, &s(&env, "bsc"), &10u64, &100i128);
}

// ── compute_fee ────────────────────────────────────────────────────────────────

#[test]
fn compute_fee_normal() {
    // 1_000_000 * 50 / 10_000 = 5_000
    assert_eq!(BridgeContract::compute_fee(1_000_000, 50), 5_000);
}

#[test]
fn compute_fee_rounds_down() {
    // 999 * 10 / 10_000 = 0
    assert_eq!(BridgeContract::compute_fee(999, 10), 0);
}

#[test]
fn compute_fee_zero_rate() {
    assert_eq!(BridgeContract::compute_fee(1_000_000, 0), 0);
}

#[test]
fn compute_fee_max_rate() {
    // 100_000 * 1_000 / 10_000 = 10_000
    assert_eq!(BridgeContract::compute_fee(100_000, 1_000), 10_000);
}
