#![cfg(test)]

use super::*;
use soroban_sdk::xdr::{ScErrorCode, ScErrorType};
use soroban_sdk::{
    testutils::{Address as _, AuthorizedFunction, AuthorizedInvocation},
    Address, Env, IntoVal, Symbol, Val,
};

// Import the GovernanceContractClient for direct interaction
use governance::GovernanceContractClient;
use governance::{Action, GovernanceError, Proposal, ProposalStatus};

/// Helper function to create a test environment
fn create_test_env() -> Env {
    let env = Env::default();
    env.mock_all_auths();
    env
}

/// Helper function to deploy the GovernanceContract
fn deploy_governance_contract(env: &Env) -> (Address, GovernanceContractClient<'static>) {
    let governance_id = env.register_contract(None, GovernanceContract);
    let governance_client = GovernanceContractClient::new(env, &governance_id);
    (governance_id, governance_client)
}

/// Helper function to deploy and initialize both contracts
fn setup_contracts(
    env: &Env,
    admin: &Address,
    signers: &soroban_sdk::Vec<Address>,
    threshold: u32,
) -> (
    Address,
    HelloContractClient<'static>,
    Address,
    GovernanceContractClient<'static>,
) {
    let (governance_id, governance_client) = deploy_governance_contract(env);

    // Initialize the Governance contract
    governance_client
        .initialize(&admin, &signers, &threshold)
        .unwrap();

    let hello_contract_id = env.register_contract(None, HelloContract);
    let hello_contract_client = HelloContractClient::new(env, &hello_contract_id);

    // Initialize the HelloContract with the GovernanceContract ID
    hello_contract_client
        .initialize(&admin, &governance_id)
        .unwrap();

    (
        hello_contract_id,
        hello_contract_client,
        governance_id,
        governance_client,
    )
}

#[test]
fn test_governance_initialize() {
    let env = create_test_env();
    let admin = Address::generate(&env);
    let signer1 = Address::generate(&env);
    let signer2 = Address::generate(&env);
    let signers = soroban_sdk::Vec::from_array(&env, [signer1.clone(), signer2.clone()]);
    let threshold = 2;

    let (_hello_id, _hello_client, _governance_id, governance_client) =
        setup_contracts(&env, &admin, &signers, threshold);

    // Verify initialization
    assert_eq!(governance_client.get_admin(), admin);
    assert_eq!(governance_client.get_signers(), signers);
    assert_eq!(governance_client.get_threshold(), threshold);

    // Try to initialize again, should panic
    let res = governance_client
        .try_initialize(&admin, &signers, &threshold)
        .unwrap_err()
        .code;
    assert_eq!(res, ScErrorCode::ContractError);
    assert_eq!(GovernanceError::AlreadyInitialized as u32, 100);
}

#[test]
fn test_governance_propose() {
    let env = create_test_env();
    let admin = Address::generate(&env);
    let signer1 = Address::generate(&env);
    let signers = soroban_sdk::Vec::from_array(&env, [signer1.clone()]);
    let threshold = 1;

    let (_hello_id, _hello_client, _governance_id, governance_client) =
        setup_contracts(&env, &admin, &signers, threshold);

    let description = soroban_sdk::String::from_str(&env, "Test Proposal");
    let action = Action::SetEmergencyPause(true);
    let voting_period = 100;
    let grace_period = 50;

    env.as_contract(&admin, || {
        let proposal_id = governance_client
            .propose(&admin, &description, &action, &voting_period, &grace_period)
            .unwrap();
        assert_eq!(proposal_id, 1);

        let proposal = governance_client.get_proposal(&proposal_id).unwrap();
        assert_eq!(proposal.proposer, admin);
        assert_eq!(proposal.description, description);
        assert_eq!(proposal.action, action);
        assert_eq!(
            proposal.voting_ends_at,
            env.ledger().timestamp() + voting_period
        );
        assert_eq!(
            proposal.grace_period_ends_at,
            env.ledger().timestamp() + voting_period + grace_period
        );
        assert_eq!(proposal.status, ProposalStatus::Pending);
        assert_eq!(proposal.votes, 0);
        assert_eq!(proposal.executed, false);
        assert!(proposal.voters.is_empty());
    });
}

#[test]
fn test_governance_vote_success() {
    let env = create_test_env();
    let admin = Address::generate(&env);
    let signer1 = Address::generate(&env);
    let signer2 = Address::generate(&env);
    let signers = soroban_sdk::Vec::from_array(&env, [signer1.clone(), signer2.clone()]);
    let threshold = 2;

    let (_hello_id, _hello_client, _governance_id, governance_client) =
        setup_contracts(&env, &admin, &signers, threshold);

    let description = soroban_sdk::String::from_str(&env, "Test Proposal");
    let action = Action::SetEmergencyPause(true);
    let voting_period = 100;
    let grace_period = 50;

    let proposal_id = env.as_contract(&admin, || {
        governance_client
            .propose(&admin, &description, &action, &voting_period, &grace_period)
            .unwrap()
    });

    // Signer1 votes
    env.as_contract(&signer1, || {
        governance_client.vote(&signer1, &proposal_id).unwrap();
    });

    let proposal_after_vote1 = governance_client.get_proposal(&proposal_id).unwrap();
    assert_eq!(proposal_after_vote1.votes, 1);
    assert_eq!(proposal_after_vote1.status, ProposalStatus::Active);
    assert!(proposal_after_vote1.voters.contains(&signer1));
    assert!(!proposal_after_vote1.voters.contains(&signer2));

    // Signer2 votes, reaching threshold
    env.as_contract(&signer2, || {
        governance_client.vote(&signer2, &proposal_id).unwrap();
    });

    let proposal_after_vote2 = governance_client.get_proposal(&proposal_id).unwrap();
    assert_eq!(proposal_after_vote2.votes, 2);
    assert_eq!(proposal_after_vote2.status, ProposalStatus::Approved);
    assert!(proposal_after_vote2.voters.contains(&signer1));
    assert!(proposal_after_vote2.voters.contains(&signer2));
}

#[test]
#[should_panic(expected = "Error(Contract, #108)")] // GovernanceError::VoteAlreadyCast
fn test_governance_vote_already_cast() {
    let env = create_test_env();
    let admin = Address::generate(&env);
    let signer1 = Address::generate(&env);
    let signers = soroban_sdk::Vec::from_array(&env, [signer1.clone()]);
    let threshold = 1;

    let (_hello_id, _hello_client, _governance_id, governance_client) =
        setup_contracts(&env, &admin, &signers, threshold);

    let description = soroban_sdk::String::from_str(&env, "Test Proposal");
    let action = Action::SetEmergencyPause(true);
    let voting_period = 100;
    let grace_period = 50;

    let proposal_id = env.as_contract(&admin, || {
        governance_client
            .propose(&admin, &description, &action, &voting_period, &grace_period)
            .unwrap()
    });

    env.as_contract(&signer1, || {
        governance_client.vote(&signer1, &proposal_id).unwrap();
    });

    // Try to vote again with the same signer
    env.as_contract(&signer1, || {
        governance_client.vote(&signer1, &proposal_id).unwrap();
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #102)")] // GovernanceError::Unauthorized
fn test_governance_vote_unauthorized() {
    let env = create_test_env();
    let admin = Address::generate(&env);
    let signer1 = Address::generate(&env);
    let unauthorized_voter = Address::generate(&env);
    let signers = soroban_sdk::Vec::from_array(&env, [signer1.clone()]);
    let threshold = 1;

    let (_hello_id, _hello_client, _governance_id, governance_client) =
        setup_contracts(&env, &admin, &signers, threshold);

    let description = soroban_sdk::String::from_str(&env, "Test Proposal");
    let action = Action::SetEmergencyPause(true);
    let voting_period = 100;
    let grace_period = 50;

    let proposal_id = env.as_contract(&admin, || {
        governance_client
            .propose(&admin, &description, &action, &voting_period, &grace_period)
            .unwrap()
    });

    // Unauthorized voter tries to vote
    env.as_contract(&unauthorized_voter, || {
        governance_client
            .vote(&unauthorized_voter, &proposal_id)
            .unwrap();
    });
}

#[test]
fn test_governance_execute_success() {
    let env = create_test_env();
    let admin = Address::generate(&env);
    let signer1 = Address::generate(&env);
    let signer2 = Address::generate(&env);
    let signers = soroban_sdk::Vec::from_array(&env, [signer1.clone(), signer2.clone()]);
    let threshold = 2;

    let (hello_id, hello_client, _governance_id, governance_client) =
        setup_contracts(&env, &admin, &signers, threshold);

    let description = soroban_sdk::String::from_str(&env, "Enable Emergency Pause");
    let action = Action::SetEmergencyPause(true);
    let voting_period = 100; // Small voting period
    let grace_period = 50; // Small grace period

    let proposal_id = env.as_contract(&admin, || {
        governance_client
            .propose(&admin, &description, &action, &voting_period, &grace_period)
            .unwrap()
    });

    // Advance time past voting period for testing execution
    env.ledger().set(soroban_sdk::testutils::LedgerInfo {
        timestamp: env.ledger().timestamp() + voting_period + 1,
        sequence_number: env.ledger().sequence() + 1,
        ..env.ledger()
    });

    // Signer1 votes
    env.as_contract(&signer1, || {
        governance_client.vote(&signer1, &proposal_id).unwrap();
    });

    // Signer2 votes
    env.as_contract(&signer2, || {
        governance_client.vote(&signer2, &proposal_id).unwrap();
    });

    let proposal_after_votes = governance_client.get_proposal(&proposal_id).unwrap();
    assert_eq!(proposal_after_votes.status, ProposalStatus::Approved);

    // Execute the proposal
    env.as_contract(&admin, || {
        governance_client.execute(&proposal_id).unwrap();
    });

    let proposal_after_exec = governance_client.get_proposal(&proposal_id).unwrap();
    assert_eq!(proposal_after_exec.status, ProposalStatus::Executed);
    assert_eq!(proposal_after_exec.executed, true);

    // Verify the action was performed (Emergency Pause enabled)
    assert!(hello_client.is_emergency_paused());
}

#[test]
#[should_panic(expected = "Error(Contract, #107)")] // GovernanceError::ProposalExpired
fn test_governance_execute_expired() {
    let env = create_test_env();
    let admin = Address::generate(&env);
    let signer1 = Address::generate(&env);
    let signer2 = Address::generate(&env);
    let signers = soroban_sdk::Vec::from_array(&env, [signer1.clone(), signer2.clone()]);
    let threshold = 2;

    let (_hello_id, _hello_client, _governance_id, governance_client) =
        setup_contracts(&env, &admin, &signers, threshold);

    let description = soroban_sdk::String::from_str(&env, "Test Proposal Expired");
    let action = Action::SetEmergencyPause(true);
    let voting_period = 100;
    let grace_period = 50;

    let proposal_id = env.as_contract(&admin, || {
        governance_client
            .propose(&admin, &description, &action, &voting_period, &grace_period)
            .unwrap()
    });

    // Advance time past voting period AND grace period
    env.ledger().set(soroban_sdk::testutils::LedgerInfo {
        timestamp: env.ledger().timestamp() + voting_period + grace_period + 1,
        sequence_number: env.ledger().sequence() + 1,
        ..env.ledger()
    });

    // Signer1 votes (still possible, but proposal should be expired for execution)
    env.as_contract(&signer1, || {
        governance_client.vote(&signer1, &proposal_id).unwrap();
    });

    // Signer2 votes
    env.as_contract(&signer2, || {
        governance_client.vote(&signer2, &proposal_id).unwrap();
    });

    // Try to execute the proposal (should panic due to expiration)
    env.as_contract(&admin, || {
        governance_client.execute(&proposal_id).unwrap();
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #109)")] // GovernanceError::InsufficientVotes
fn test_governance_execute_insufficient_votes_defeats() {
    let env = create_test_env();
    let admin = Address::generate(&env);
    let signer1 = Address::generate(&env);
    let signer2 = Address::generate(&env);
    let signers = soroban_sdk::Vec::from_array(&env, [signer1.clone(), signer2.clone()]);
    let threshold = 2; // Requires 2 votes

    let (_hello_id, _hello_client, _governance_id, governance_client) =
        setup_contracts(&env, &admin, &signers, threshold);

    let description = soroban_sdk::String::from_str(&env, "Test Proposal Insufficient Votes");
    let action = Action::SetEmergencyPause(true);
    let voting_period = 100;
    let grace_period = 50;

    let proposal_id = env.as_contract(&admin, || {
        governance_client
            .propose(&admin, &description, &action, &voting_period, &grace_period)
            .unwrap()
    });

    // Advance time past voting period
    env.ledger().set(soroban_sdk::testutils::LedgerInfo {
        timestamp: env.ledger().timestamp() + voting_period + 1,
        sequence_number: env.ledger().sequence() + 1,
        ..env.ledger()
    });

    // Only one signer votes (insufficient votes)
    env.as_contract(&signer1, || {
        governance_client.vote(&signer1, &proposal_id).unwrap();
    });

    let proposal_after_vote = governance_client.get_proposal(&proposal_id).unwrap();
    assert_eq!(proposal_after_vote.votes, 1);
    assert_eq!(proposal_after_vote.status, ProposalStatus::Active); // Still active after vote

    // Try to execute (should panic due to insufficient votes, leading to Defeated)
    env.as_contract(&admin, || {
        governance_client.execute(&proposal_id).unwrap();
    });

    let proposal_after_exec = governance_client.get_proposal(&proposal_id).unwrap();
    assert_eq!(proposal_after_exec.status, ProposalStatus::Defeated);
}

#[test]
fn test_governance_add_signer() {
    let env = create_test_env();
    let admin = Address::generate(&env);
    let signer1 = Address::generate(&env);
    let signers = soroban_sdk::Vec::from_array(&env, [signer1.clone()]);
    let threshold = 1;

    let (_hello_id, _hello_client, _governance_id, governance_client) =
        setup_contracts(&env, &admin, &signers, threshold);

    let new_signer = Address::generate(&env);

    env.as_contract(&admin, || {
        governance_client.add_signer(&admin, &new_signer).unwrap();
    });

    let updated_signers = governance_client.get_signers();
    assert!(updated_signers.contains(&new_signer));
    assert_eq!(updated_signers.len(), 2);
}

#[test]
#[should_panic(expected = "Error(Contract, #102)")] // GovernanceError::Unauthorized
fn test_governance_add_signer_unauthorized() {
    let env = create_test_env();
    let admin = Address::generate(&env);
    let signer1 = Address::generate(&env);
    let unauthorized_caller = Address::generate(&env);
    let signers = soroban_sdk::Vec::from_array(&env, [signer1.clone()]);
    let threshold = 1;

    let (_hello_id, _hello_client, _governance_id, governance_client) =
        setup_contracts(&env, &admin, &signers, threshold);

    let new_signer = Address::generate(&env);

    // Unauthorized caller tries to add signer
    env.as_contract(&unauthorized_caller, || {
        governance_client
            .add_signer(&unauthorized_caller, &new_signer)
            .unwrap();
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #111)")] // GovernanceError::SignerAlreadyExists
fn test_governance_add_signer_already_exists() {
    let env = create_test_env();
    let admin = Address::generate(&env);
    let signer1 = Address::generate(&env);
    let signers = soroban_sdk::Vec::from_array(&env, [signer1.clone()]);
    let threshold = 1;

    let (_hello_id, _hello_client, _governance_id, governance_client) =
        setup_contracts(&env, &admin, &signers, threshold);

    // Try to add existing signer
    env.as_contract(&admin, || {
        governance_client.add_signer(&admin, &signer1).unwrap();
    });
}

#[test]
fn test_governance_remove_signer() {
    let env = create_test_env();
    let admin = Address::generate(&env);
    let signer1 = Address::generate(&env);
    let signer2 = Address::generate(&env);
    let signers = soroban_sdk::Vec::from_array(&env, [signer1.clone(), signer2.clone()]);
    let threshold = 2;

    let (_hello_id, _hello_client, _governance_id, governance_client) =
        setup_contracts(&env, &admin, &signers, threshold);

    env.as_contract(&admin, || {
        governance_client.remove_signer(&admin, &signer2).unwrap();
    });

    let updated_signers = governance_client.get_signers();
    assert!(!updated_signers.contains(&signer2));
    assert!(updated_signers.contains(&signer1));
    assert_eq!(updated_signers.len(), 1);

    // Threshold should ideally be adjusted, but current contract doesn't do it automatically
    // This is a manual check for now
    assert_eq!(governance_client.get_threshold(), threshold);
}

#[test]
#[should_panic(expected = "Error(Contract, #102)")] // GovernanceError::Unauthorized
fn test_governance_remove_signer_unauthorized() {
    let env = create_test_env();
    let admin = Address::generate(&env);
    let signer1 = Address::generate(&env);
    let signer2 = Address::generate(&env);
    let unauthorized_caller = Address::generate(&env);
    let signers = soroban_sdk::Vec::from_array(&env, [signer1.clone(), signer2.clone()]);
    let threshold = 2;

    let (_hello_id, _hello_client, _governance_id, governance_client) =
        setup_contracts(&env, &admin, &signers, threshold);

    // Unauthorized caller tries to remove signer
    env.as_contract(&unauthorized_caller, || {
        governance_client
            .remove_signer(&unauthorized_caller, &signer2)
            .unwrap();
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #110)")] // GovernanceError::CannotSelfRemove
fn test_governance_remove_signer_self_remove() {
    let env = create_test_env();
    let admin = Address::generate(&env);
    let signer1 = Address::generate(&env);
    let signers = soroban_sdk::Vec::from_array(&env, [signer1.clone()]);
    let threshold = 1;

    let (_hello_id, _hello_client, _governance_id, governance_client) =
        setup_contracts(&env, &admin, &signers, threshold);

    // Admin tries to remove self (which is a signer)
    env.as_contract(&admin, || {
        governance_client.remove_signer(&admin, &admin).unwrap();
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #112)")] // GovernanceError::SignerNotFound
fn test_governance_remove_signer_not_found() {
    let env = create_test_env();
    let admin = Address::generate(&env);
    let signer1 = Address::generate(&env);
    let non_existent_signer = Address::generate(&env);
    let signers = soroban_sdk::Vec::from_array(&env, [signer1.clone()]);
    let threshold = 1;

    let (_hello_id, _hello_client, _governance_id, governance_client) =
        setup_contracts(&env, &admin, &signers, threshold);

    // Admin tries to remove non-existent signer
    env.as_contract(&admin, || {
        governance_client
            .remove_signer(&admin, &non_existent_signer)
            .unwrap();
    });
}

#[test]
fn test_governance_set_threshold() {
    let env = create_test_env();
    let admin = Address::generate(&env);
    let signer1 = Address::generate(&env);
    let signer2 = Address::generate(&env);
    let signer3 = Address::generate(&env);
    let signers =
        soroban_sdk::Vec::from_array(&env, [signer1.clone(), signer2.clone(), signer3.clone()]);
    let threshold = 2;

    let (_hello_id, _hello_client, _governance_id, governance_client) =
        setup_contracts(&env, &admin, &signers, threshold);

    let new_threshold = 3;
    env.as_contract(&admin, || {
        governance_client
            .set_threshold(&admin, &new_threshold)
            .unwrap();
    });

    assert_eq!(governance_client.get_threshold(), new_threshold);
}

#[test]
#[should_panic(expected = "Error(Contract, #102)")] // GovernanceError::Unauthorized
fn test_governance_set_threshold_unauthorized() {
    let env = create_test_env();
    let admin = Address::generate(&env);
    let signer1 = Address::generate(&env);
    let unauthorized_caller = Address::generate(&env);
    let signers = soroban_sdk::Vec::from_array(&env, [signer1.clone()]);
    let threshold = 1;

    let (_hello_id, _hello_client, _governance_id, governance_client) =
        setup_contracts(&env, &admin, &signers, threshold);

    // Unauthorized caller tries to set threshold
    env.as_contract(&unauthorized_caller, || {
        governance_client
            .set_threshold(&unauthorized_caller, &1)
            .unwrap();
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #104)")] // GovernanceError::InvalidArguments
fn test_governance_set_threshold_invalid() {
    let env = create_test_env();
    let admin = Address::generate(&env);
    let signer1 = Address::generate(&env);
    let signer2 = Address::generate(&env);
    let signers = soroban_sdk::Vec::from_array(&env, [signer1.clone(), signer2.clone()]);
    let threshold = 2;

    let (_hello_id, _hello_client, _governance_id, governance_client) =
        setup_contracts(&env, &admin, &signers, threshold);

    // Try to set threshold greater than number of signers
    env.as_contract(&admin, || {
        governance_client
            .set_threshold(&admin, &(signers.len() + 1))
            .unwrap();
    });
}

#[test]
fn test_governance_transfer_admin() {
    let env = create_test_env();
    let admin = Address::generate(&env);
    let signer1 = Address::generate(&env);
    let signers = soroban_sdk::Vec::from_array(&env, [signer1.clone()]);
    let threshold = 1;

    let (_hello_id, _hello_client, _governance_id, governance_client) =
        setup_contracts(&env, &admin, &signers, threshold);

    let new_admin = Address::generate(&env);

    env.as_contract(&admin, || {
        governance_client
            .transfer_admin(&admin, &new_admin)
            .unwrap();
    });

    assert_eq!(governance_client.get_admin(), new_admin);
}

#[test]
#[should_panic(expected = "Error(Contract, #102)")] // GovernanceError::Unauthorized
fn test_governance_transfer_admin_unauthorized() {
    let env = create_test_env();
    let admin = Address::generate(&env);
    let signer1 = Address::generate(&env);
    let unauthorized_caller = Address::generate(&env);
    let signers = soroban_sdk::Vec::from_array(&env, [signer1.clone()]);
    let threshold = 1;

    let (_hello_id, _hello_client, _governance_id, governance_client) =
        setup_contracts(&env, &admin, &signers, threshold);

    let new_admin = Address::generate(&env);

    // Unauthorized caller tries to transfer admin
    env.as_contract(&unauthorized_caller, || {
        governance_client
            .transfer_admin(&unauthorized_caller, &new_admin)
            .unwrap();
    });
}
