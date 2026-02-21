#![cfg(test)]

use crate::admin::{
    get_admin, grant_role, has_admin, has_role, require_admin, require_role_or_admin, revoke_role,
    set_admin, AdminError,
};
use crate::HelloContract;
use soroban_sdk::{
    testutils::{Address as _, Events},
    Address, Env, IntoVal, Symbol,
};

fn setup_env() -> (Env, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(HelloContract, ());
    (env, contract_id)
}

#[test]
fn test_set_and_get_admin() {
    let (env, contract_id) = setup_env();
    let admin = Address::generate(&env);

    env.as_contract(&contract_id, || {
        assert!(!has_admin(&env));
        assert!(get_admin(&env).is_none());

        // First time setting admin
        let result = set_admin(&env, admin.clone(), None);
        assert_eq!(result, Ok(()));

        assert!(has_admin(&env));
        assert_eq!(get_admin(&env), Some(admin.clone()));
    });

    // Verify event
    let events = env.events().all();
    assert_eq!(events.len(), 1);
    let event = events.last().unwrap();
    let topics = event.1;
    let expected_topic: Symbol = Symbol::new(&env, "admin_changed");
    let actual_topic: Symbol = topics.first().unwrap().into_val(&env);
    assert_eq!(actual_topic, expected_topic);
}

#[test]
fn test_transfer_admin_authorized() {
    let (env, contract_id) = setup_env();
    let admin1 = Address::generate(&env);
    let admin2 = Address::generate(&env);

    env.as_contract(&contract_id, || {
        set_admin(&env, admin1.clone(), None).unwrap();

        // Authorized transfer
        let result = set_admin(&env, admin2.clone(), Some(admin1.clone()));
        assert_eq!(result, Ok(()));
        assert_eq!(get_admin(&env), Some(admin2));
    });
}

#[test]
fn test_transfer_admin_unauthorized() {
    let (env, contract_id) = setup_env();
    let admin = Address::generate(&env);
    let unauthorized = Address::generate(&env);
    let new_admin = Address::generate(&env);

    env.as_contract(&contract_id, || {
        set_admin(&env, admin.clone(), None).unwrap();

        // Unauthorized transfer
        let result = set_admin(&env, new_admin, Some(unauthorized));
        assert_eq!(result, Err(AdminError::Unauthorized));
        assert_eq!(get_admin(&env), Some(admin));

        // No caller specified when admin already exists
        let result2 = set_admin(&env, Address::generate(&env), None);
        assert_eq!(result2, Err(AdminError::Unauthorized));
    });
}

#[test]
fn test_require_admin() {
    let (env, contract_id) = setup_env();
    let admin = Address::generate(&env);
    let non_admin = Address::generate(&env);

    env.as_contract(&contract_id, || {
        set_admin(&env, admin.clone(), None).unwrap();

        assert_eq!(require_admin(&env, &admin), Ok(()));
        assert_eq!(
            require_admin(&env, &non_admin),
            Err(AdminError::Unauthorized)
        );
    });
}

#[test]
fn test_role_management() {
    let (env, contract_id) = setup_env();
    let admin = Address::generate(&env);
    let account = Address::generate(&env);
    let role = Symbol::new(&env, "minter");

    env.as_contract(&contract_id, || {
        set_admin(&env, admin.clone(), None).unwrap();

        // Account doesn't have role initially
        assert!(!has_role(&env, role.clone(), account.clone()));

        // Grant role
        let result = grant_role(&env, admin.clone(), role.clone(), account.clone());
        assert_eq!(result, Ok(()));
        assert!(has_role(&env, role.clone(), account.clone()));
    });

    // Verify role_granted event
    {
        let events = env.events().all();
        let event = events.last().unwrap();
        let topics = event.1;
        let expected_topic: Symbol = Symbol::new(&env, "role_granted");
        let actual_topic: Symbol = topics.first().unwrap().into_val(&env);
        assert_eq!(actual_topic, expected_topic);
    }

    env.as_contract(&contract_id, || {
        // Revoke role
        let result = revoke_role(&env, admin.clone(), role.clone(), account.clone());
        assert_eq!(result, Ok(()));
        assert!(!has_role(&env, role.clone(), account.clone()));
    });

    // Verify role_revoked event
    {
        let events = env.events().all();
        let event = events.last().unwrap();
        let topics = event.1;
        let expected_topic: Symbol = Symbol::new(&env, "role_revoked");
        let actual_topic: Symbol = topics.first().unwrap().into_val(&env);
        assert_eq!(actual_topic, expected_topic);
    }
}

#[test]
fn test_grant_role_unauthorized() {
    let (env, contract_id) = setup_env();
    let admin = Address::generate(&env);
    let unauthorized = Address::generate(&env);
    let account = Address::generate(&env);
    let role = Symbol::new(&env, "minter");

    env.as_contract(&contract_id, || {
        set_admin(&env, admin.clone(), None).unwrap();

        // Grant role fails if not admin
        let result = grant_role(&env, unauthorized.clone(), role.clone(), account.clone());
        assert_eq!(result, Err(AdminError::Unauthorized));
        assert!(!has_role(&env, role.clone(), account.clone()));
    });
}

#[test]
fn test_require_role_or_admin() {
    let (env, contract_id) = setup_env();
    let admin = Address::generate(&env);
    let roled_account = Address::generate(&env);
    let unroled_account = Address::generate(&env);
    let role = Symbol::new(&env, "oracle_admin");

    env.as_contract(&contract_id, || {
        set_admin(&env, admin.clone(), None).unwrap();
        grant_role(&env, admin.clone(), role.clone(), roled_account.clone()).unwrap();

        // Admin should pass
        assert_eq!(require_role_or_admin(&env, &admin, role.clone()), Ok(()));

        // Roled account should pass
        assert_eq!(
            require_role_or_admin(&env, &roled_account, role.clone()),
            Ok(())
        );

        // Unroled account should fail
        assert_eq!(
            require_role_or_admin(&env, &unroled_account, role.clone()),
            Err(AdminError::Unauthorized)
        );
    });
}
