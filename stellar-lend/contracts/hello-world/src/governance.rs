//! # Governance Module
//!
//! Provides on-chain governance and multisig approval for the lending protocol.
//!
//! ## Proposal Lifecycle
//! 1. A multisig admin **creates** a proposal with a voting period and threshold.
//! 2. Voters **cast** votes (For / Against / Abstain) with voting power.
//! 3. If the For-votes meet the threshold, the proposal status becomes **Passed**.
//! 4. After the execution timelock expires, anyone can **execute** the proposal.
//!
//! ## Multisig
//! - A set of admin addresses and an approval threshold are maintained.
//! - Proposals require `threshold` approvals from distinct admins before execution.
//!
//! ## Defaults
//! - Voting period: 7 days
//! - Execution timelock: 2 days after voting ends
//! - Voting threshold: 50% of total voting power

#![allow(unused)]
use soroban_sdk::{contracterror, contracttype, Address, Env, IntoVal, Map, Symbol, Vec};

/// Errors that can occur during governance operations
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum GovernanceError {
    /// Unauthorized access - caller is not authorized
    Unauthorized = 1,
    /// Proposal not found
    ProposalNotFound = 2,
    /// Proposal already executed
    ProposalAlreadyExecuted = 3,
    /// Proposal already failed
    ProposalAlreadyFailed = 4,
    /// Proposal not ready for execution (timelock not expired)
    ProposalNotReady = 5,
    /// Voting threshold not met
    ThresholdNotMet = 6,
    /// Invalid proposal data
    InvalidProposal = 7,
    /// Invalid vote value
    InvalidVote = 8,
    /// Already voted
    AlreadyVoted = 9,
    /// Voting period ended
    VotingPeriodEnded = 10,
    /// Proposal execution failed
    ExecutionFailed = 11,
    /// Invalid multisig configuration
    InvalidMultisigConfig = 12,
    /// Not enough approvals
    InsufficientApprovals = 13,
    /// Proposal expired
    ProposalExpired = 14,
    /// Recovery already in progress
    RecoveryInProgress = 15,
    /// No recovery in progress
    NoRecoveryInProgress = 16,
    /// Invalid guardian configuration
    InvalidGuardianConfig = 17,
    /// Guardian already exists
    GuardianAlreadyExists = 18,
    /// Guardian not found
    GuardianNotFound = 19,
}

/// Storage keys for governance data
#[contracttype]
#[derive(Clone)]
#[cfg_attr(test, derive(Debug, PartialEq))]
pub enum GovernanceDataKey {
    /// Proposals: Map<u64, Proposal>
    Proposal(u64),
    /// Proposal counter
    ProposalCounter,
    /// Multisig admins: Vec<Address>
    MultisigAdmins,
    /// Multisig threshold (number of approvals required)
    MultisigThreshold,
    /// Proposal votes: Map<u64, Map<Address, Vote>>
    ProposalVotes(u64),
    /// Proposal approvals (for multisig): Map<u64, Vec<Address>>
    ProposalApprovals(u64),
    /// Guardians: Vec<Address>
    Guardians,
    /// Guardian threshold (number of approvals required for recovery)
    GuardianThreshold,
    /// Recovery request: Option<RecoveryRequest>
    RecoveryRequest,
    /// Recovery approvals: Vec<Address>
    RecoveryApprovals,
}

/// Proposal status
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProposalStatus {
    /// Proposal is active and accepting votes
    Active,
    /// Proposal passed voting threshold
    Passed,
    /// Proposal failed to meet threshold
    Failed,
    /// Proposal executed
    Executed,
    /// Proposal expired
    Expired,
}

/// Proposal type
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum ProposalType {
    /// Change minimum collateral ratio
    MinCollateralRatio(i128),
    /// Change risk parameters (min_cr, liq_threshold, close_factor, liq_incentive)
    RiskParams(Option<i128>, Option<i128>, Option<i128>, Option<i128>),
    /// Pause/unpause operation
    PauseSwitch(Symbol, bool),
    /// Emergency pause
    EmergencyPause(bool),
}

/// Vote type
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Vote {
    /// Vote in favor
    For,
    /// Vote against
    Against,
    /// Abstain
    Abstain,
}

/// Recovery request structure
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct RecoveryRequest {
    /// Old admin to be replaced
    pub old_admin: Address,
    /// New admin to be set
    pub new_admin: Address,
    /// Guardian who initiated the recovery
    pub initiator: Address,
    /// Timestamp when recovery was initiated
    pub initiated_at: u64,
    /// Expiration timestamp (recovery must be executed before this)
    pub expires_at: u64,
}

/// Proposal structure
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct Proposal {
    /// Proposal ID
    pub id: u64,
    /// Proposal creator
    pub proposer: Address,
    /// Proposal type
    pub proposal_type: ProposalType,
    /// Proposal description
    pub description: Symbol,
    /// Current status
    pub status: ProposalStatus,
    /// Voting start time
    pub voting_start: u64,
    /// Voting end time
    pub voting_end: u64,
    /// Execution timelock (when proposal can be executed after passing)
    pub execution_timelock: u64,
    /// Minimum voting threshold (in basis points, e.g., 5000 = 50%)
    pub voting_threshold: i128,
    /// Current votes for
    pub votes_for: i128,
    /// Current votes against
    pub votes_against: i128,
    /// Current votes abstain
    pub votes_abstain: i128,
    /// Total voting power
    pub total_voting_power: i128,
    /// Created timestamp
    pub created_at: u64,
}

/// Constants
const DEFAULT_VOTING_PERIOD: u64 = 7 * 24 * 60 * 60; // 7 days in seconds
const DEFAULT_EXECUTION_TIMELOCK: u64 = 2 * 24 * 60 * 60; // 2 days in seconds
const DEFAULT_VOTING_THRESHOLD: i128 = 5_000; // 50% in basis points
const BASIS_POINTS_SCALE: i128 = 10_000; // 100% = 10,000 basis points

/// Initialize the governance system.
///
/// Sets up the proposal counter, default multisig threshold (1), and adds
/// `admin` as the first multisig admin. No-ops if already initialized.
///
/// # Arguments
/// * `env` - The contract environment
/// * `admin` - The initial admin address added to the multisig set
///
/// # Errors
/// This function does not error; it silently returns `Ok` if already initialized.
pub fn initialize_governance(env: &Env, admin: Address) -> Result<(), GovernanceError> {
    let key = GovernanceDataKey::ProposalCounter;
    if env.storage().persistent().has(&key) {
        return Ok(()); // Already initialized
    }
    env.storage().persistent().set(&key, &0u64);

    // Set default multisig threshold
    let threshold_key = GovernanceDataKey::MultisigThreshold;
    env.storage().persistent().set(&threshold_key, &1u32); // Default: 1 approval required

    // Initialize multisig admins with the admin
    let admins_key = GovernanceDataKey::MultisigAdmins;
    let mut admins = Vec::new(env);
    admins.push_back(admin);
    env.storage().persistent().set(&admins_key, &admins);

    Ok(())
}

/// Create a new governance proposal.
///
/// Increments the proposal counter, initializes vote and approval maps, and
/// emits a `proposal_created` event. The proposal starts in `Active` status.
///
/// # Arguments
/// * `env` - The contract environment
/// * `proposer` - The address creating the proposal
/// * `proposal_type` - The action the proposal would execute
/// * `description` - Short description symbol
/// * `voting_period` - Custom voting window in seconds (default: 7 days)
/// * `execution_timelock` - Delay after passing before execution (default: 2 days)
/// * `voting_threshold` - Required For-vote percentage in basis points (default: 5000 = 50%)
///
/// # Returns
/// The new proposal's ID on success.
///
/// # Errors
/// * `InvalidProposal` - Voting threshold is out of range [0, 10000] or counter overflows
pub fn create_proposal(
    env: &Env,
    proposer: Address,
    proposal_type: ProposalType,
    description: Symbol,
    voting_period: Option<u64>,
    execution_timelock: Option<u64>,
    voting_threshold: Option<i128>,
) -> Result<u64, GovernanceError> {
    // Get and increment proposal counter
    let counter_key = GovernanceDataKey::ProposalCounter;
    let proposal_id: u64 = env
        .storage()
        .persistent()
        .get(&counter_key)
        .unwrap_or(0u64)
        .checked_add(1)
        .ok_or(GovernanceError::InvalidProposal)?;
    env.storage().persistent().set(&counter_key, &proposal_id);

    let now = env.ledger().timestamp();
    let voting_period = voting_period.unwrap_or(DEFAULT_VOTING_PERIOD);
    let execution_timelock = execution_timelock.unwrap_or(DEFAULT_EXECUTION_TIMELOCK);
    let voting_threshold = voting_threshold.unwrap_or(DEFAULT_VOTING_THRESHOLD);

    // Validate voting threshold
    if !(0..=BASIS_POINTS_SCALE).contains(&voting_threshold) {
        return Err(GovernanceError::InvalidProposal);
    }

    let proposal = Proposal {
        id: proposal_id,
        proposer: proposer.clone(),
        proposal_type,
        description,
        status: ProposalStatus::Active,
        voting_start: now,
        voting_end: now + voting_period,
        execution_timelock: now + voting_period + execution_timelock,
        voting_threshold,
        votes_for: 0,
        votes_against: 0,
        votes_abstain: 0,
        total_voting_power: 0,
        created_at: now,
    };

    let proposal_key = GovernanceDataKey::Proposal(proposal_id);
    env.storage().persistent().set(&proposal_key, &proposal);

    // Initialize votes map
    let votes_key = GovernanceDataKey::ProposalVotes(proposal_id);
    let votes_map: Map<Address, Vote> = Map::new(env);
    env.storage().persistent().set(&votes_key, &votes_map);

    // Initialize approvals map for multisig
    let approvals_key = GovernanceDataKey::ProposalApprovals(proposal_id);
    let approvals: Vec<Address> = Vec::new(env);
    env.storage().persistent().set(&approvals_key, &approvals);

    emit_proposal_created_event(env, &proposal_id, &proposer);

    Ok(proposal_id)
}

/// Cast a vote on an active proposal.
///
/// Records the voter's choice and updates the proposal's tally. If the
/// For-votes meet the threshold, the proposal status transitions to `Passed`.
/// If the voting period has expired, the proposal is marked `Expired`.
///
/// # Arguments
/// * `env` - The contract environment
/// * `voter` - The voter's address
/// * `proposal_id` - The proposal to vote on
/// * `vote` - The vote choice (`For`, `Against`, or `Abstain`)
/// * `voting_power` - The voter's voting weight (must be > 0)
///
/// # Errors
/// * `InvalidVote` - Voting power is zero or negative
/// * `ProposalNotFound` - Proposal does not exist or is not in Active/Passed status
/// * `VotingPeriodEnded` - The voting window has closed
/// * `AlreadyVoted` - The voter has already cast a vote on this proposal
pub fn vote(
    env: &Env,
    voter: Address,
    proposal_id: u64,
    vote: Vote,
    voting_power: i128,
) -> Result<(), GovernanceError> {
    if voting_power <= 0 {
        return Err(GovernanceError::InvalidVote);
    }

    let proposal_key = GovernanceDataKey::Proposal(proposal_id);
    let mut proposal: Proposal = env
        .storage()
        .persistent()
        .get(&proposal_key)
        .ok_or(GovernanceError::ProposalNotFound)?;

    // Check proposal status
    match proposal.status {
        ProposalStatus::Active | ProposalStatus::Passed => {}
        _ => return Err(GovernanceError::ProposalNotFound),
    }

    // Check voting period
    let now = env.ledger().timestamp();
    if now > proposal.voting_end {
        proposal.status = ProposalStatus::Expired;
        env.storage().persistent().set(&proposal_key, &proposal);
        return Err(GovernanceError::VotingPeriodEnded);
    }

    // Check if already voted
    let votes_key = GovernanceDataKey::ProposalVotes(proposal_id);
    let mut votes_map: Map<Address, Vote> = env
        .storage()
        .persistent()
        .get(&votes_key)
        .unwrap_or(Map::new(env));

    if votes_map.contains_key(voter.clone()) {
        return Err(GovernanceError::AlreadyVoted);
    }

    // Record vote
    votes_map.set(voter.clone(), vote.clone());
    env.storage().persistent().set(&votes_key, &votes_map);

    // Update proposal vote counts
    match vote {
        Vote::For => proposal.votes_for += voting_power,
        Vote::Against => proposal.votes_against += voting_power,
        Vote::Abstain => proposal.votes_abstain += voting_power,
    }
    proposal.total_voting_power += voting_power;

    // Check if threshold is met
    let threshold_votes =
        (proposal.total_voting_power * proposal.voting_threshold) / BASIS_POINTS_SCALE;
    if proposal.votes_for >= threshold_votes && proposal.status == ProposalStatus::Active {
        proposal.status = ProposalStatus::Passed;
    }

    env.storage().persistent().set(&proposal_key, &proposal);

    emit_vote_cast_event(env, &proposal_id, &voter, &vote, &voting_power);

    Ok(())
}

/// Execute a passed proposal after its timelock has expired.
///
/// Verifies the proposal has `Passed` status and the execution timelock has
/// elapsed, then marks it `Executed`. If the proposal is still `Active` but
/// meets the threshold, it transitions to `Passed` first.
///
/// # Arguments
/// * `env` - The contract environment
/// * `executor` - The address executing the proposal
/// * `proposal_id` - The proposal to execute
///
/// # Errors
/// * `ProposalNotFound` - Proposal does not exist
/// * `ProposalAlreadyExecuted` - Proposal was already executed
/// * `ProposalAlreadyFailed` - Proposal failed voting
/// * `ProposalExpired` - Proposal expired without execution
/// * `ThresholdNotMet` - Active proposal does not have enough For-votes
/// * `ProposalNotReady` - Execution timelock has not yet expired
pub fn execute_proposal(
    env: &Env,
    executor: Address,
    proposal_id: u64,
) -> Result<(), GovernanceError> {
    let proposal_key = GovernanceDataKey::Proposal(proposal_id);
    let mut proposal: Proposal = env
        .storage()
        .persistent()
        .get(&proposal_key)
        .ok_or(GovernanceError::ProposalNotFound)?;

    // Check proposal status
    match proposal.status {
        ProposalStatus::Passed => {}
        ProposalStatus::Executed => return Err(GovernanceError::ProposalAlreadyExecuted),
        ProposalStatus::Failed => return Err(GovernanceError::ProposalAlreadyFailed),
        ProposalStatus::Expired => return Err(GovernanceError::ProposalExpired),
        ProposalStatus::Active => {
            // Check if threshold is met
            let threshold_votes =
                (proposal.total_voting_power * proposal.voting_threshold) / BASIS_POINTS_SCALE;
            if proposal.votes_for < threshold_votes {
                proposal.status = ProposalStatus::Failed;
                env.storage().persistent().set(&proposal_key, &proposal);
                return Err(GovernanceError::ThresholdNotMet);
            }
            proposal.status = ProposalStatus::Passed;
        }
    }

    // Check timelock
    let now = env.ledger().timestamp();
    if now < proposal.execution_timelock {
        return Err(GovernanceError::ProposalNotReady);
    }

    // Mark as executed
    proposal.status = ProposalStatus::Executed;
    env.storage().persistent().set(&proposal_key, &proposal);

    emit_proposal_executed_event(env, &proposal_id, &executor);

    Ok(())
}

/// Finalize an active proposal whose voting period has ended.
///
/// If the For-votes meet the threshold the proposal transitions to `Passed`;
/// otherwise it is marked `Failed` and a `proposal_failed` event is emitted.
///
/// # Arguments
/// * `env` - The contract environment
/// * `proposal_id` - The proposal to finalize
///
/// # Errors
/// * `ProposalNotFound` - Proposal does not exist or is not `Active`
/// * `VotingPeriodEnded` - The voting period has **not** ended yet (still open)
pub fn mark_proposal_failed(env: &Env, proposal_id: u64) -> Result<(), GovernanceError> {
    let proposal_key = GovernanceDataKey::Proposal(proposal_id);
    let mut proposal: Proposal = env
        .storage()
        .persistent()
        .get(&proposal_key)
        .ok_or(GovernanceError::ProposalNotFound)?;

    if proposal.status != ProposalStatus::Active {
        return Err(GovernanceError::ProposalNotFound);
    }

    let now = env.ledger().timestamp();
    if now <= proposal.voting_end {
        return Err(GovernanceError::VotingPeriodEnded);
    }

    // Check if threshold was met
    let threshold_votes =
        (proposal.total_voting_power * proposal.voting_threshold) / BASIS_POINTS_SCALE;
    if proposal.votes_for < threshold_votes {
        proposal.status = ProposalStatus::Failed;
        env.storage().persistent().set(&proposal_key, &proposal);
        emit_proposal_failed_event(env, &proposal_id);
        Ok(())
    } else {
        proposal.status = ProposalStatus::Passed;
        env.storage().persistent().set(&proposal_key, &proposal);
        Ok(())
    }
}

/// Look up a proposal by ID.
///
/// # Arguments
/// * `env` - The contract environment
/// * `proposal_id` - The proposal ID to look up
///
/// # Returns
/// `Some(Proposal)` if found, `None` otherwise.
pub fn get_proposal(env: &Env, proposal_id: u64) -> Option<Proposal> {
    let proposal_key = GovernanceDataKey::Proposal(proposal_id);
    env.storage().persistent().get(&proposal_key)
}

/// Look up how a specific voter voted on a proposal.
///
/// # Arguments
/// * `env` - The contract environment
/// * `proposal_id` - The proposal ID
/// * `voter` - The voter's address
///
/// # Returns
/// `Some(Vote)` if the voter participated, `None` otherwise.
pub fn get_vote(env: &Env, proposal_id: u64, voter: Address) -> Option<Vote> {
    let votes_key = GovernanceDataKey::ProposalVotes(proposal_id);
    let votes_map: Map<Address, Vote> = env.storage().persistent().get(&votes_key)?;
    votes_map.get(voter)
}

// ============================================================================
// Multisig Operations
// ============================================================================

/// Replace the multisig admin set.
///
/// Only an existing multisig admin may call this. The new set must be non-empty.
///
/// # Arguments
/// * `env` - The contract environment
/// * `caller` - Must be a current multisig admin
/// * `admins` - The new admin address list (replaces existing)
///
/// # Errors
/// * `Unauthorized` - Caller is not a current admin or admin list is uninitialized
/// * `InvalidMultisigConfig` - Provided admin list is empty
pub fn set_multisig_admins(
    env: &Env,
    caller: Address,
    admins: Vec<Address>,
) -> Result<(), GovernanceError> {
    // Check if caller is current admin
    let admins_key = GovernanceDataKey::MultisigAdmins;
    let current_admins: Vec<Address> = env
        .storage()
        .persistent()
        .get(&admins_key)
        .ok_or(GovernanceError::Unauthorized)?;

    if !current_admins.contains(caller.clone()) {
        return Err(GovernanceError::Unauthorized);
    }

    if admins.is_empty() {
        return Err(GovernanceError::InvalidMultisigConfig);
    }

    env.storage().persistent().set(&admins_key, &admins);
    Ok(())
}

/// Set the number of admin approvals required to execute a multisig proposal.
///
/// Threshold must be in the range `[1, admins.len()]`.
///
/// # Arguments
/// * `env` - The contract environment
/// * `caller` - Must be a current multisig admin
/// * `threshold` - New approval threshold
///
/// # Errors
/// * `Unauthorized` - Caller is not a current admin
/// * `InvalidMultisigConfig` - Threshold is 0 or exceeds the admin count
pub fn set_multisig_threshold(
    env: &Env,
    caller: Address,
    threshold: u32,
) -> Result<(), GovernanceError> {
    let admins_key = GovernanceDataKey::MultisigAdmins;
    let admins: Vec<Address> = env
        .storage()
        .persistent()
        .get(&admins_key)
        .ok_or(GovernanceError::Unauthorized)?;

    if !admins.contains(caller.clone()) {
        return Err(GovernanceError::Unauthorized);
    }

    if threshold == 0 || threshold > admins.len() {
        return Err(GovernanceError::InvalidMultisigConfig);
    }

    let threshold_key = GovernanceDataKey::MultisigThreshold;
    env.storage().persistent().set(&threshold_key, &threshold);
    Ok(())
}

/// Create a proposal to change the minimum collateral ratio (multisig shortcut).
///
/// Only multisig admins may call this. Creates a `SetMinCollateralRatio`
/// proposal with default voting parameters.
///
/// # Arguments
/// * `env` - The contract environment
/// * `proposer` - Must be a current multisig admin
/// * `new_ratio` - Proposed collateral ratio in basis points
///
/// # Returns
/// The new proposal's ID on success.
///
/// # Errors
/// * `Unauthorized` - Proposer is not a multisig admin
/// * `InvalidProposal` - Proposal creation failed (see [`create_proposal`])
pub fn propose_set_min_collateral_ratio(
    env: &Env,
    proposer: Address,
    new_ratio: i128,
) -> Result<u64, GovernanceError> {
    // Check if proposer is multisig admin
    let admins_key = GovernanceDataKey::MultisigAdmins;
    let admins: Vec<Address> = env
        .storage()
        .persistent()
        .get(&admins_key)
        .ok_or(GovernanceError::Unauthorized)?;

    if !admins.contains(proposer.clone()) {
        return Err(GovernanceError::Unauthorized);
    }

    let proposal_type = ProposalType::MinCollateralRatio(new_ratio);
    let description = Symbol::new(env, "set_min_collateral_ratio");

    create_proposal(env, proposer, proposal_type, description, None, None, None)
}

/// Record a multisig admin's approval on a proposal.
///
/// Each admin may approve a proposal at most once. Once the number of approvals
/// meets the threshold, the proposal can be executed via [`execute_multisig_proposal`].
///
/// # Arguments
/// * `env` - The contract environment
/// * `approver` - Must be a current multisig admin
/// * `proposal_id` - The proposal to approve
///
/// # Errors
/// * `Unauthorized` - Approver is not a multisig admin
/// * `ProposalNotFound` - Proposal does not exist
/// * `AlreadyVoted` - Approver has already approved this proposal
pub fn approve_proposal(
    env: &Env,
    approver: Address,
    proposal_id: u64,
) -> Result<(), GovernanceError> {
    // Check if approver is multisig admin
    let admins_key = GovernanceDataKey::MultisigAdmins;
    let admins: Vec<Address> = env
        .storage()
        .persistent()
        .get(&admins_key)
        .ok_or(GovernanceError::Unauthorized)?;

    if !admins.contains(approver.clone()) {
        return Err(GovernanceError::Unauthorized);
    }

    // Check proposal exists
    let proposal_key = GovernanceDataKey::Proposal(proposal_id);
    let _proposal: Proposal = env
        .storage()
        .persistent()
        .get(&proposal_key)
        .ok_or(GovernanceError::ProposalNotFound)?;

    // Get approvals
    let approvals_key = GovernanceDataKey::ProposalApprovals(proposal_id);
    let mut approvals: Vec<Address> = env
        .storage()
        .persistent()
        .get(&approvals_key)
        .unwrap_or(Vec::new(env));

    // Check if already approved
    if approvals.contains(approver.clone()) {
        return Err(GovernanceError::AlreadyVoted);
    }

    // Add approval
    approvals.push_back(approver.clone());
    env.storage().persistent().set(&approvals_key, &approvals);

    emit_approval_event(env, &proposal_id, &approver);

    Ok(())
}

/// Execute a multisig proposal after sufficient approvals.
///
/// Verifies the executor is an admin and that the number of approvals meets
/// the multisig threshold, then delegates to [`execute_proposal`].
///
/// # Arguments
/// * `env` - The contract environment
/// * `executor` - Must be a current multisig admin
/// * `proposal_id` - The proposal to execute
///
/// # Errors
/// * `Unauthorized` - Executor is not a multisig admin
/// * `InsufficientApprovals` - Approval count is below the threshold
/// * Other errors from [`execute_proposal`]
pub fn execute_multisig_proposal(
    env: &Env,
    executor: Address,
    proposal_id: u64,
) -> Result<(), GovernanceError> {
    // Check if executor is multisig admin
    let admins_key = GovernanceDataKey::MultisigAdmins;
    let admins: Vec<Address> = env
        .storage()
        .persistent()
        .get(&admins_key)
        .ok_or(GovernanceError::Unauthorized)?;

    if !admins.contains(executor.clone()) {
        return Err(GovernanceError::Unauthorized);
    }

    // Get threshold
    let threshold_key = GovernanceDataKey::MultisigThreshold;
    let threshold: u32 = env
        .storage()
        .persistent()
        .get(&threshold_key)
        .unwrap_or(1u32);

    // Get approvals
    let approvals_key = GovernanceDataKey::ProposalApprovals(proposal_id);
    let approvals: Vec<Address> = env
        .storage()
        .persistent()
        .get(&approvals_key)
        .unwrap_or(Vec::new(env));

    if approvals.len() < threshold {
        return Err(GovernanceError::InsufficientApprovals);
    }

    // Execute the proposal
    execute_proposal(env, executor, proposal_id)
}

/// Return the current multisig admin set, or `None` if uninitialized.
pub fn get_multisig_admins(env: &Env) -> Option<Vec<Address>> {
    let admins_key = GovernanceDataKey::MultisigAdmins;
    env.storage().persistent().get(&admins_key)
}

/// Return the current multisig approval threshold (defaults to 1).
pub fn get_multisig_threshold(env: &Env) -> u32 {
    let threshold_key = GovernanceDataKey::MultisigThreshold;
    env.storage()
        .persistent()
        .get(&threshold_key)
        .unwrap_or(1u32)
}

/// Return the list of admins who have approved a proposal, or `None` if not found.
pub fn get_proposal_approvals(env: &Env, proposal_id: u64) -> Option<Vec<Address>> {
    let approvals_key = GovernanceDataKey::ProposalApprovals(proposal_id);
    env.storage().persistent().get(&approvals_key)
}

// ============================================================================
// Events
// ============================================================================

fn emit_proposal_created_event(env: &Env, proposal_id: &u64, proposer: &Address) {
    let topics = (
        Symbol::new(env, "proposal_created"),
        *proposal_id,
        proposer.clone(),
    );
    env.events().publish(topics, ());
}

fn emit_vote_cast_event(
    env: &Env,
    proposal_id: &u64,
    voter: &Address,
    vote: &Vote,
    voting_power: &i128,
) {
    let topics = (Symbol::new(env, "vote_cast"), *proposal_id, voter.clone());
    env.events().publish(topics, (vote.clone(), *voting_power));
}

fn emit_proposal_executed_event(env: &Env, proposal_id: &u64, executor: &Address) {
    let topics = (
        Symbol::new(env, "proposal_executed"),
        *proposal_id,
        executor.clone(),
    );
    env.events().publish(topics, ());
}

fn emit_proposal_failed_event(env: &Env, proposal_id: &u64) {
    let topics = (Symbol::new(env, "proposal_failed"), *proposal_id);
    env.events().publish(topics, ());
}

fn emit_approval_event(env: &Env, proposal_id: &u64, approver: &Address) {
    let topics = (
        Symbol::new(env, "proposal_approved"),
        *proposal_id,
        approver.clone(),
    );
    env.events().publish(topics, ());
}

// ============================================================================
// Social Recovery Operations
// ============================================================================

const DEFAULT_RECOVERY_PERIOD: u64 = 3 * 24 * 60 * 60; // 3 days in seconds

/// Add a guardian to the recovery system.
///
/// Only multisig admins can add guardians. Guardians can initiate and approve
/// recovery requests to change the admin in case of key loss or compromise.
///
/// # Arguments
/// * `env` - The contract environment
/// * `caller` - Must be a current multisig admin
/// * `guardian` - The guardian address to add
///
/// # Errors
/// * `Unauthorized` - Caller is not a multisig admin
/// * `GuardianAlreadyExists` - Guardian is already in the list
pub fn add_guardian(env: &Env, caller: Address, guardian: Address) -> Result<(), GovernanceError> {
    let admins_key = GovernanceDataKey::MultisigAdmins;
    let admins: Vec<Address> = env
        .storage()
        .persistent()
        .get(&admins_key)
        .ok_or(GovernanceError::Unauthorized)?;

    if !admins.contains(caller.clone()) {
        return Err(GovernanceError::Unauthorized);
    }

    let guardians_key = GovernanceDataKey::Guardians;
    let mut guardians: Vec<Address> = env
        .storage()
        .persistent()
        .get(&guardians_key)
        .unwrap_or(Vec::new(env));

    if guardians.contains(guardian.clone()) {
        return Err(GovernanceError::GuardianAlreadyExists);
    }

    guardians.push_back(guardian.clone());
    env.storage().persistent().set(&guardians_key, &guardians);

    emit_guardian_added_event(env, &guardian);
    Ok(())
}

/// Remove a guardian from the recovery system.
///
/// Only multisig admins can remove guardians.
///
/// # Arguments
/// * `env` - The contract environment
/// * `caller` - Must be a current multisig admin
/// * `guardian` - The guardian address to remove
///
/// # Errors
/// * `Unauthorized` - Caller is not a multisig admin
/// * `GuardianNotFound` - Guardian is not in the list
pub fn remove_guardian(
    env: &Env,
    caller: Address,
    guardian: Address,
) -> Result<(), GovernanceError> {
    let admins_key = GovernanceDataKey::MultisigAdmins;
    let admins: Vec<Address> = env
        .storage()
        .persistent()
        .get(&admins_key)
        .ok_or(GovernanceError::Unauthorized)?;

    if !admins.contains(caller.clone()) {
        return Err(GovernanceError::Unauthorized);
    }

    let guardians_key = GovernanceDataKey::Guardians;
    let mut guardians: Vec<Address> = env
        .storage()
        .persistent()
        .get(&guardians_key)
        .ok_or(GovernanceError::GuardianNotFound)?;

    let mut found = false;
    let mut new_guardians = Vec::new(env);
    for g in guardians.iter() {
        if g != guardian {
            new_guardians.push_back(g);
        } else {
            found = true;
        }
    }

    if !found {
        return Err(GovernanceError::GuardianNotFound);
    }

    env.storage()
        .persistent()
        .set(&guardians_key, &new_guardians);

    emit_guardian_removed_event(env, &guardian);
    Ok(())
}

/// Set the guardian threshold for recovery approvals.
///
/// Only multisig admins can set the threshold. Threshold must be in range [1, guardians.len()].
///
/// # Arguments
/// * `env` - The contract environment
/// * `caller` - Must be a current multisig admin
/// * `threshold` - Number of guardian approvals required for recovery
///
/// # Errors
/// * `Unauthorized` - Caller is not a multisig admin
/// * `InvalidGuardianConfig` - Threshold is 0 or exceeds guardian count
pub fn set_guardian_threshold(
    env: &Env,
    caller: Address,
    threshold: u32,
) -> Result<(), GovernanceError> {
    let admins_key = GovernanceDataKey::MultisigAdmins;
    let admins: Vec<Address> = env
        .storage()
        .persistent()
        .get(&admins_key)
        .ok_or(GovernanceError::Unauthorized)?;

    if !admins.contains(caller.clone()) {
        return Err(GovernanceError::Unauthorized);
    }

    let guardians_key = GovernanceDataKey::Guardians;
    let guardians: Vec<Address> = env
        .storage()
        .persistent()
        .get(&guardians_key)
        .unwrap_or(Vec::new(env));

    if threshold == 0 || threshold > guardians.len() {
        return Err(GovernanceError::InvalidGuardianConfig);
    }

    let threshold_key = GovernanceDataKey::GuardianThreshold;
    env.storage().persistent().set(&threshold_key, &threshold);
    Ok(())
}

/// Start a recovery process to change the admin.
///
/// Only guardians can initiate recovery. Creates a recovery request that must be
/// approved by the threshold number of guardians before execution.
///
/// # Arguments
/// * `env` - The contract environment
/// * `initiator` - Must be a guardian
/// * `old_admin` - The current admin to be replaced
/// * `new_admin` - The new admin to be set
///
/// # Errors
/// * `Unauthorized` - Initiator is not a guardian
/// * `RecoveryInProgress` - A recovery is already in progress
pub fn start_recovery(
    env: &Env,
    initiator: Address,
    old_admin: Address,
    new_admin: Address,
) -> Result<(), GovernanceError> {
    let guardians_key = GovernanceDataKey::Guardians;
    let guardians: Vec<Address> = env
        .storage()
        .persistent()
        .get(&guardians_key)
        .ok_or(GovernanceError::Unauthorized)?;

    if !guardians.contains(initiator.clone()) {
        return Err(GovernanceError::Unauthorized);
    }

    let recovery_key = GovernanceDataKey::RecoveryRequest;
    if env.storage().persistent().has(&recovery_key) {
        return Err(GovernanceError::RecoveryInProgress);
    }

    let now = env.ledger().timestamp();
    let recovery = RecoveryRequest {
        old_admin: old_admin.clone(),
        new_admin: new_admin.clone(),
        initiator: initiator.clone(),
        initiated_at: now,
        expires_at: now + DEFAULT_RECOVERY_PERIOD,
    };

    env.storage().persistent().set(&recovery_key, &recovery);

    let approvals_key = GovernanceDataKey::RecoveryApprovals;
    let mut approvals = Vec::new(env);
    approvals.push_back(initiator.clone());
    env.storage().persistent().set(&approvals_key, &approvals);

    emit_recovery_started_event(env, &old_admin, &new_admin, &initiator);
    Ok(())
}

/// Approve a recovery request.
///
/// Only guardians can approve. Each guardian can approve once. Once threshold
/// is met, recovery can be executed.
///
/// # Arguments
/// * `env` - The contract environment
/// * `approver` - Must be a guardian
///
/// # Errors
/// * `Unauthorized` - Approver is not a guardian
/// * `NoRecoveryInProgress` - No recovery request exists
/// * `AlreadyVoted` - Guardian has already approved
/// * `ProposalExpired` - Recovery request has expired
pub fn approve_recovery(env: &Env, approver: Address) -> Result<(), GovernanceError> {
    let guardians_key = GovernanceDataKey::Guardians;
    let guardians: Vec<Address> = env
        .storage()
        .persistent()
        .get(&guardians_key)
        .ok_or(GovernanceError::Unauthorized)?;

    if !guardians.contains(approver.clone()) {
        return Err(GovernanceError::Unauthorized);
    }

    let recovery_key = GovernanceDataKey::RecoveryRequest;
    let recovery: RecoveryRequest = env
        .storage()
        .persistent()
        .get(&recovery_key)
        .ok_or(GovernanceError::NoRecoveryInProgress)?;

    let now = env.ledger().timestamp();
    if now > recovery.expires_at {
        env.storage().persistent().remove(&recovery_key);
        return Err(GovernanceError::ProposalExpired);
    }

    let approvals_key = GovernanceDataKey::RecoveryApprovals;
    let mut approvals: Vec<Address> = env
        .storage()
        .persistent()
        .get(&approvals_key)
        .unwrap_or(Vec::new(env));

    if approvals.contains(approver.clone()) {
        return Err(GovernanceError::AlreadyVoted);
    }

    approvals.push_back(approver.clone());
    env.storage().persistent().set(&approvals_key, &approvals);

    emit_recovery_approved_event(env, &approver);
    Ok(())
}

/// Execute a recovery request after sufficient approvals.
///
/// Anyone can execute once threshold is met. Changes the admin to the new address
/// and clears the recovery request.
///
/// # Arguments
/// * `env` - The contract environment
/// * `executor` - Any address (no authorization required)
///
/// # Errors
/// * `NoRecoveryInProgress` - No recovery request exists
/// * `InsufficientApprovals` - Not enough guardian approvals
/// * `ProposalExpired` - Recovery request has expired
pub fn execute_recovery(env: &Env, executor: Address) -> Result<(), GovernanceError> {
    let recovery_key = GovernanceDataKey::RecoveryRequest;
    let recovery: RecoveryRequest = env
        .storage()
        .persistent()
        .get(&recovery_key)
        .ok_or(GovernanceError::NoRecoveryInProgress)?;

    let now = env.ledger().timestamp();
    if now > recovery.expires_at {
        env.storage().persistent().remove(&recovery_key);
        return Err(GovernanceError::ProposalExpired);
    }

    let threshold_key = GovernanceDataKey::GuardianThreshold;
    let threshold: u32 = env
        .storage()
        .persistent()
        .get(&threshold_key)
        .unwrap_or(1u32);

    let approvals_key = GovernanceDataKey::RecoveryApprovals;
    let approvals: Vec<Address> = env
        .storage()
        .persistent()
        .get(&approvals_key)
        .unwrap_or(Vec::new(env));

    if approvals.len() < threshold {
        return Err(GovernanceError::InsufficientApprovals);
    }

    // Update admin in multisig admins
    let admins_key = GovernanceDataKey::MultisigAdmins;
    let mut admins: Vec<Address> = env
        .storage()
        .persistent()
        .get(&admins_key)
        .unwrap_or(Vec::new(env));

    let mut new_admins = Vec::new(env);
    for admin in admins.iter() {
        if admin != recovery.old_admin {
            new_admins.push_back(admin);
        }
    }
    new_admins.push_back(recovery.new_admin.clone());
    env.storage().persistent().set(&admins_key, &new_admins);

    // Clear recovery state
    env.storage().persistent().remove(&recovery_key);
    env.storage().persistent().remove(&approvals_key);

    emit_recovery_executed_event(env, &recovery.old_admin, &recovery.new_admin, &executor);
    Ok(())
}

/// Get the list of guardians.
pub fn get_guardians(env: &Env) -> Option<Vec<Address>> {
    let guardians_key = GovernanceDataKey::Guardians;
    env.storage().persistent().get(&guardians_key)
}

/// Get the guardian threshold.
pub fn get_guardian_threshold(env: &Env) -> u32 {
    let threshold_key = GovernanceDataKey::GuardianThreshold;
    env.storage()
        .persistent()
        .get(&threshold_key)
        .unwrap_or(1u32)
}

/// Get the current recovery request.
pub fn get_recovery_request(env: &Env) -> Option<RecoveryRequest> {
    let recovery_key = GovernanceDataKey::RecoveryRequest;
    env.storage().persistent().get(&recovery_key)
}

/// Get recovery approvals.
pub fn get_recovery_approvals(env: &Env) -> Option<Vec<Address>> {
    let approvals_key = GovernanceDataKey::RecoveryApprovals;
    env.storage().persistent().get(&approvals_key)
}

// ============================================================================
// Recovery Events
// ============================================================================

fn emit_guardian_added_event(env: &Env, guardian: &Address) {
    let topics = (Symbol::new(env, "guardian_added"), guardian.clone());
    env.events().publish(topics, ());
}

fn emit_guardian_removed_event(env: &Env, guardian: &Address) {
    let topics = (Symbol::new(env, "guardian_removed"), guardian.clone());
    env.events().publish(topics, ());
}

fn emit_recovery_started_event(
    env: &Env,
    old_admin: &Address,
    new_admin: &Address,
    initiator: &Address,
) {
    let topics = (
        Symbol::new(env, "recovery_started"),
        old_admin.clone(),
        new_admin.clone(),
    );
    env.events().publish(topics, initiator.clone());
}

fn emit_recovery_approved_event(env: &Env, approver: &Address) {
    let topics = (Symbol::new(env, "recovery_approved"), approver.clone());
    env.events().publish(topics, ());
}

fn emit_recovery_executed_event(
    env: &Env,
    old_admin: &Address,
    new_admin: &Address,
    executor: &Address,
) {
    let topics = (
        Symbol::new(env, "recovery_executed"),
        old_admin.clone(),
        new_admin.clone(),
    );
    env.events().publish(topics, executor.clone());
}
