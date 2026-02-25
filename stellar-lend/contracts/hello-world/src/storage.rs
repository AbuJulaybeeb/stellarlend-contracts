use soroban_sdk::{contracttype, Address, Vec};

#[derive(Clone)]
#[contracttype]
pub enum GovernanceDataKey {
    Admin,
    Config,
    NextProposalId,
    MultisigConfig,
    GuardianConfig,

    Proposal(u64),
    Vote(u64, Address),
    ProposalApprovals(u64),
    UserProposals(Address, u64),

    RecoveryRequest,
    RecoveryApprovals,
}

#[derive(Clone)]
#[contracttype]
pub struct GuardianConfig {
    pub guardians: Vec<Address>,
    pub threshold: u32,
}
