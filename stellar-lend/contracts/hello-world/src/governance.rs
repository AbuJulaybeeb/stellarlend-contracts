use soroban_sdk::xdr::{ScErrorCode, ScErrorType};
use soroban_sdk::{
    contract, contractimpl, contracttype, Address, Env, Error as SdkError, Symbol, Val,
};

impl From<&GovernanceError> for SdkError {
    fn from(error: &GovernanceError) -> Self {
        SdkError::from_type_and_code(ScErrorType::Contract, unsafe {
            ScErrorCode::try_from(*error as i32).unwrap_unchecked()
        })
    }
}

impl From<SdkError> for GovernanceError {
    fn from(error: SdkError) -> Self {
        // Attempt to get the ScError from the SdkError
        let error_code_val = error.get_code();
        let error_code: u32 = error_code_val;

        match error_code {
            100 => GovernanceError::AlreadyInitialized,
            101 => GovernanceError::NotInitialized,
            102 => GovernanceError::Unauthorized,
            103 => GovernanceError::NotFound,
            104 => GovernanceError::InvalidArguments,
            105 => GovernanceError::NotYetTime,
            106 => GovernanceError::ProposalAlreadyExecuted,
            107 => GovernanceError::ProposalExpired,
            108 => GovernanceError::VoteAlreadyCast,
            109 => GovernanceError::InsufficientVotes,
            110 => GovernanceError::CannotSelfRemove,
            111 => GovernanceError::SignerAlreadyExists,
            112 => GovernanceError::SignerNotFound,
            _ => GovernanceError::InvalidArguments, // Generic fallback
        }
    }
}

// Define errors for the governance contract
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum GovernanceError {
    AlreadyInitialized = 100,
    NotInitialized = 101,
    Unauthorized = 102,
    NotFound = 103,
    InvalidArguments = 104,
    NotYetTime = 105,
    ProposalAlreadyExecuted = 106,
    ProposalExpired = 107,
    VoteAlreadyCast = 108,
    InsufficientVotes = 109,
    CannotSelfRemove = 110,
    SignerAlreadyExists = 111,
    SignerNotFound = 112,
}

// Define storage keys
#[contracttype]
pub enum GovernanceDataKey {
    Initialized,
    Admin,
    Signers,
    Threshold,
    ProposalCount,
    Proposal(u32),
}

// Define proposal status
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ProposalStatus {
    Pending,
    Active,
    Approved,
    Executed,
    Cancelled,
    Expired,
    Defeated,
}

// Define action types for proposals
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action {
    // Example: SetRiskParams in risk_management
    SetRiskParams(
        Option<i128>, // min_collateral_ratio
        Option<i128>, // liquidation_threshold
        Option<i128>, // close_factor
        Option<i128>, // liquidation_incentive
    ),
    // Example: SetPauseSwitch
    SetPauseSwitch(
        Symbol, // operation
        bool,   // paused
    ),
    // Example: SetEmergencyPause
    SetEmergencyPause(
        bool, // paused
    ),
    // Generic action for calling other contracts or functions
    Call(
        Address,               // contract_id
        Symbol,                // function
        soroban_sdk::Vec<Val>, // args
    ),
    // Upgrade contract
    Upgrade(
        soroban_sdk::BytesN<32>, // new_wasm_hash
    ),
}

// Define proposal structure
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Proposal {
    pub proposer: Address,
    pub description: soroban_sdk::String,
    pub action: Action,
    pub created_at: u64,
    pub voting_ends_at: u64,
    pub grace_period_ends_at: u64,
    pub status: ProposalStatus,
    pub votes: u32,
    pub executed: bool,
    pub voters: soroban_sdk::Vec<Address>,
}

#[contract]
#[allow(dead_code)]
pub struct GovernanceContract;

#[contractimpl]
#[allow(dead_code)]
impl GovernanceContract {
    /// Initializes the governance contract.
    ///
    /// # Arguments
    /// * `env` - The contract environment.
    /// * `admin` - The initial admin address.
    /// * `signers` - A vector of initial multisig signers.
    /// * `threshold` - The number of required signatures for multisig.
    pub fn initialize(
        env: Env,
        admin: Address,
        signers: soroban_sdk::Vec<Address>,
        threshold: u32,
    ) -> Result<(), GovernanceError> {
        if env
            .storage()
            .instance()
            .has(&GovernanceDataKey::Initialized)
        {
            return Err(GovernanceError::AlreadyInitialized);
        }

        env.storage()
            .instance()
            .set(&GovernanceDataKey::Initialized, &true);
        env.storage()
            .instance()
            .set(&GovernanceDataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&GovernanceDataKey::Signers, &signers);
        env.storage()
            .instance()
            .set(&GovernanceDataKey::Threshold, &threshold);
        env.storage()
            .instance()
            .set(&GovernanceDataKey::ProposalCount, &0u32);

        Ok(())
    }

    /// Creates a new governance proposal.
    ///
    /// Only admin or existing signers can create proposals.
    ///
    /// # Arguments
    /// * `env` - The contract environment.
    /// * `proposer` - The address proposing the action.
    /// * `description` - A description of the proposal.
    /// * `action` - The action to be performed if the proposal passes.
    /// * `voting_period_seconds` - The duration for voting in seconds.
    /// * `grace_period_seconds` - The duration after voting ends during which the proposal can be executed.
    pub fn propose(
        env: Env,
        proposer: Address,
        description: soroban_sdk::String,
        action: Action,
        voting_period_seconds: u64,
        grace_period_seconds: u64,
    ) -> Result<u32, GovernanceError> {
        // Only admin or existing signers can propose
        // This check needs to be more robust later to verify against the stored signers
        proposer.require_auth();

        let admin: Address = env
            .storage()
            .instance()
            .get(&GovernanceDataKey::Admin)
            .ok_or(GovernanceError::NotInitialized)?;
        let signers: soroban_sdk::Vec<Address> = env
            .storage()
            .instance()
            .get(&GovernanceDataKey::Signers)
            .ok_or(GovernanceError::NotInitialized)?;

        if proposer != admin && !signers.contains(&proposer) {
            return Err(GovernanceError::Unauthorized);
        }

        let mut proposal_count: u32 = env
            .storage()
            .instance()
            .get(&GovernanceDataKey::ProposalCount)
            .ok_or(GovernanceError::NotInitialized)?;

        proposal_count = proposal_count
            .checked_add(1)
            .ok_or(GovernanceError::InvalidArguments)?; // Simple increment, consider overflow for very long-lived contracts

        let current_time = env.ledger().timestamp();
        let voting_ends_at = current_time
            .checked_add(voting_period_seconds)
            .ok_or(GovernanceError::InvalidArguments)?;
        let grace_period_ends_at = voting_ends_at
            .checked_add(grace_period_seconds)
            .ok_or(GovernanceError::InvalidArguments)?;

        let proposal = Proposal {
            proposer: proposer.clone(), // Clone here to fix the moved value error
            description: description.clone(),
            action,
            created_at: current_time,
            voting_ends_at,
            grace_period_ends_at,
            status: ProposalStatus::Pending, // Will become active after creation
            votes: 0,
            executed: false,
            voters: soroban_sdk::Vec::new(&env),
        };

        env.storage()
            .persistent()
            .set(&GovernanceDataKey::Proposal(proposal_count), &proposal);
        env.storage()
            .instance()
            .set(&GovernanceDataKey::ProposalCount, &proposal_count);

        // Emit event: ProposalCreated { proposal_id, proposer, description }
        env.events().publish(
            (Symbol::new(&env, "PROPOSAL_CREATED"), proposal_count),
            (proposer, description),
        );

        Ok(proposal_count)
    }

    /// Allows a signer to vote on an active proposal.
    ///
    /// # Arguments
    /// * `env` - The contract environment.
    /// * `voter` - The address casting the vote. Must be a signer.
    /// * `proposal_id` - The ID of the proposal to vote on.
    pub fn vote(env: Env, voter: Address, proposal_id: u32) -> Result<(), GovernanceError> {
        voter.require_auth();

        let signers: soroban_sdk::Vec<Address> = env
            .storage()
            .instance()
            .get(&GovernanceDataKey::Signers)
            .ok_or(GovernanceError::NotInitialized)?;

        if !signers.contains(&voter) {
            return Err(GovernanceError::Unauthorized);
        }

        let mut proposal: Proposal = env
            .storage()
            .persistent()
            .get(&GovernanceDataKey::Proposal(proposal_id))
            .ok_or(GovernanceError::NotFound)?;

        let current_time = env.ledger().timestamp();

        if current_time > proposal.voting_ends_at {
            // Proposal has expired for voting, but we don't change its status here.
            // The execute function or a separate cleanup function will handle this.
            return Err(GovernanceError::ProposalExpired);
        }

        if !(proposal.status == ProposalStatus::Pending
            || proposal.status == ProposalStatus::Active)
        {
            return Err(GovernanceError::InvalidArguments); // Can only vote on pending/active proposals
        }

        // For simplicity, for now, we'll just increment a counter. A more robust solution
        // would involve mapping voter address to proposal ID.

        // Check if voter already voted
        if proposal.voters.contains(&voter) {
            return Err(GovernanceError::VoteAlreadyCast);
        }

        // Add voter to the list
        proposal.voters.push_back(voter.clone());

        proposal.votes = proposal
            .votes
            .checked_add(1)
            .ok_or(GovernanceError::InvalidArguments)?;
        proposal.status = ProposalStatus::Active; // Ensure it's active after the first vote

        env.storage()
            .persistent()
            .set(&GovernanceDataKey::Proposal(proposal_id), &proposal);

        let threshold: u32 = env
            .storage()
            .instance()
            .get(&GovernanceDataKey::Threshold)
            .ok_or(GovernanceError::NotInitialized)?;

        if proposal.votes >= threshold {
            proposal.status = ProposalStatus::Approved;
            env.storage()
                .persistent()
                .set(&GovernanceDataKey::Proposal(proposal_id), &proposal);
            env.events().publish(
                (Symbol::new(&env, "PROPOSAL_APPROVED"), proposal_id),
                (proposal.votes,),
            );
        }

        // Emit event: VoteCast { proposal_id, voter }
        env.events()
            .publish((Symbol::new(&env, "VOTE_CAST"), proposal_id), (voter,));

        Ok(())
    }

    /// Executes an approved proposal.
    ///
    /// Can only be executed during its grace period and if it has enough votes.
    ///
    /// # Arguments
    /// * `env` - The contract environment.
    /// * `proposal_id` - The ID of the proposal to execute.
    pub fn execute(env: Env, proposal_id: u32) -> Result<(), GovernanceError> {
        let mut proposal: Proposal = env
            .storage()
            .persistent()
            .get(&GovernanceDataKey::Proposal(proposal_id))
            .ok_or(GovernanceError::NotFound)?;

        if proposal.executed {
            return Err(GovernanceError::ProposalAlreadyExecuted);
        }

        let current_time = env.ledger().timestamp();
        if current_time < proposal.voting_ends_at {
            return Err(GovernanceError::NotYetTime); // Not past voting period
        }
        if current_time > proposal.grace_period_ends_at {
            proposal.status = ProposalStatus::Expired;
            env.storage()
                .persistent()
                .set(&GovernanceDataKey::Proposal(proposal_id), &proposal);
            env.events().publish(
                (Symbol::new(&env, "PROPOSAL_EXPIRED"), proposal_id),
                (proposal.proposer,),
            );
            return Err(GovernanceError::ProposalExpired);
        }

        let threshold: u32 = env
            .storage()
            .instance()
            .get(&GovernanceDataKey::Threshold)
            .ok_or(GovernanceError::NotInitialized)?;

        if proposal.votes < threshold {
            proposal.status = ProposalStatus::Defeated;
            env.storage()
                .persistent()
                .set(&GovernanceDataKey::Proposal(proposal_id), &proposal);
            env.events()
                .publish((Symbol::new(&env, "PROPOSAL_DEFEATED"), proposal_id), ());
            return Err(GovernanceError::InsufficientVotes);
        }

        // Perform the action
        match proposal.action.clone() {
            Action::SetRiskParams(
                _min_collateral_ratio,
                _liquidation_threshold,
                _close_factor,
                _liquidation_incentive,
            ) => {
                env.events().publish(
                    (Symbol::new(&env, "ACTION_EXECUTED"), proposal_id),
                    Symbol::new(&env, "SetRiskParams"),
                );
            }
            Action::SetPauseSwitch(operation, paused) => {
                env.events().publish(
                    (Symbol::new(&env, "ACTION_EXECUTED"), proposal_id),
                    (Symbol::new(&env, "SetPauseSwitch"), operation, paused),
                );
            }
            Action::SetEmergencyPause(paused) => {
                env.events().publish(
                    (Symbol::new(&env, "ACTION_EXECUTED"), proposal_id),
                    (Symbol::new(&env, "SetEmergencyPause"), paused),
                );
            }
            Action::Call(contract_id, function, args) => {
                env.invoke_contract::<Val>(&contract_id, &function, args);
                env.events().publish(
                    (Symbol::new(&env, "ACTION_EXECUTED"), proposal_id),
                    (Symbol::new(&env, "Call"), contract_id, function),
                );
            }
            Action::Upgrade(new_wasm_hash) => {
                env.deployer()
                    .update_current_contract_wasm(new_wasm_hash.clone());
                env.events().publish(
                    (Symbol::new(&env, "ACTION_EXECUTED"), proposal_id),
                    (Symbol::new(&env, "Upgrade"), new_wasm_hash),
                );
            }
        }

        proposal.status = ProposalStatus::Executed;
        proposal.executed = true;
        env.storage()
            .persistent()
            .set(&GovernanceDataKey::Proposal(proposal_id), &proposal);

        env.events()
            .publish((Symbol::new(&env, "PROPOSAL_EXECUTED"), proposal_id), ());

        Ok(())
    }

    /// Retrieves a proposal by its ID.
    pub fn get_proposal(env: Env, proposal_id: u32) -> Result<Proposal, GovernanceError> {
        env.storage()
            .persistent()
            .get(&GovernanceDataKey::Proposal(proposal_id))
            .ok_or(GovernanceError::NotFound)
    }

    /// Adds a new signer to the multisig.
    ///
    /// Only the current admin can add signers.
    pub fn add_signer(
        env: Env,
        caller: Address,
        new_signer: Address,
    ) -> Result<(), GovernanceError> {
        caller.require_auth();

        let admin: Address = env
            .storage()
            .instance()
            .get(&GovernanceDataKey::Admin)
            .ok_or(GovernanceError::NotInitialized)?;

        if caller != admin {
            return Err(GovernanceError::Unauthorized);
        }

        let mut signers: soroban_sdk::Vec<Address> = env
            .storage()
            .instance()
            .get(&GovernanceDataKey::Signers)
            .ok_or(GovernanceError::NotInitialized)?;

        if signers.contains(&new_signer) {
            return Err(GovernanceError::SignerAlreadyExists);
        }

        signers.push_back(new_signer.clone());
        env.storage()
            .instance()
            .set(&GovernanceDataKey::Signers, &signers);

        env.events().publish(
            (Symbol::new(&env, "SIGNER_ADDED"), new_signer.clone()),
            (caller,),
        );

        Ok(())
    }

    /// Removes a signer from the multisig.
    ///
    /// Only the current admin can remove signers. A signer cannot remove themselves.
    pub fn remove_signer(
        env: Env,
        caller: Address,
        signer_to_remove: Address,
    ) -> Result<(), GovernanceError> {
        caller.require_auth();

        let admin: Address = env
            .storage()
            .instance()
            .get(&GovernanceDataKey::Admin)
            .ok_or(GovernanceError::NotInitialized)?;

        if caller != admin {
            return Err(GovernanceError::Unauthorized);
        }

        if caller == signer_to_remove {
            return Err(GovernanceError::CannotSelfRemove);
        }

        let signers: soroban_sdk::Vec<Address> = env
            .storage()
            .instance()
            .get(&GovernanceDataKey::Signers)
            .ok_or(GovernanceError::NotInitialized)?;

        let mut found = false;
        let mut new_signers = soroban_sdk::Vec::new(&env);
        for signer in signers.into_iter() {
            if signer != signer_to_remove {
                new_signers.push_back(signer);
            } else {
                found = true;
            }
        }

        if !found {
            return Err(GovernanceError::SignerNotFound);
        }

        env.storage()
            .instance()
            .set(&GovernanceDataKey::Signers, &new_signers);

        env.events().publish(
            (
                Symbol::new(&env, "SIGNER_REMOVED"),
                signer_to_remove.clone(),
            ),
            (caller,),
        );

        Ok(())
    }

    /// Sets the multisig threshold.
    ///
    /// Only the current admin can set the threshold.
    /// The threshold cannot be greater than the number of current signers.
    pub fn set_threshold(
        env: Env,
        caller: Address,
        new_threshold: u32,
    ) -> Result<(), GovernanceError> {
        caller.require_auth();

        let admin: Address = env
            .storage()
            .instance()
            .get(&GovernanceDataKey::Admin)
            .ok_or(GovernanceError::NotInitialized)?;

        if caller != admin {
            return Err(GovernanceError::Unauthorized);
        }

        let signers: soroban_sdk::Vec<Address> = env
            .storage()
            .instance()
            .get(&GovernanceDataKey::Signers)
            .ok_or(GovernanceError::NotInitialized)?;

        if new_threshold == 0 || new_threshold > signers.len() {
            return Err(GovernanceError::InvalidArguments);
        }

        env.storage()
            .instance()
            .set(&GovernanceDataKey::Threshold, &new_threshold);

        env.events().publish(
            (Symbol::new(&env, "THRESHOLD_SET"), new_threshold),
            (caller,),
        );

        Ok(())
    }

    /// Gets the current multisig signers.
    pub fn get_signers(env: Env) -> Result<soroban_sdk::Vec<Address>, GovernanceError> {
        env.storage()
            .instance()
            .get(&GovernanceDataKey::Signers)
            .ok_or(GovernanceError::NotInitialized)
    }

    /// Gets the current multisig threshold.
    pub fn get_threshold(env: Env) -> Result<u32, GovernanceError> {
        env.storage()
            .instance()
            .get(&GovernanceDataKey::Threshold)
            .ok_or(GovernanceError::NotInitialized)
    }

    /// Gets the current admin.
    pub fn get_admin(env: Env) -> Result<Address, GovernanceError> {
        env.storage()
            .instance()
            .get(&GovernanceDataKey::Admin)
            .ok_or(GovernanceError::NotInitialized)
    }

    /// Transfers admin ownership to a new address.
    ///
    /// # Arguments
    /// * `env` - The contract environment.
    /// * `caller` - The address initiating the transfer. Must be the current admin.
    /// * `new_admin` - The address of the new admin.
    pub fn transfer_admin(
        env: Env,
        caller: Address,
        new_admin: Address,
    ) -> Result<(), GovernanceError> {
        caller.require_auth();

        let admin: Address = env
            .storage()
            .instance()
            .get(&GovernanceDataKey::Admin)
            .ok_or(GovernanceError::NotInitialized)?;

        if caller != admin {
            return Err(GovernanceError::Unauthorized);
        }

        env.storage()
            .instance()
            .set(&GovernanceDataKey::Admin, &new_admin);

        env.events().publish(
            (Symbol::new(&env, "ADMIN_TRANSFERRED"), new_admin.clone()),
            (caller,),
        );

        Ok(())
    }
}
