#![no_std]
#![allow(unused_imports)]
use soroban_sdk::{
    contract, contractimpl, contracttype, Address, Env, IntoVal, Map, String, Symbol,
};

mod deposit;
mod governance;
mod risk_management;

use deposit::deposit_collateral;
use governance::{Action, GovernanceContractClient, Proposal};
use risk_management::{
    can_be_liquidated, get_close_factor, get_liquidation_incentive,
    get_liquidation_incentive_amount, get_liquidation_threshold, get_max_liquidatable_amount,
    get_min_collateral_ratio, initialize_risk_management, is_emergency_paused, is_operation_paused,
    require_min_collateral_ratio, set_emergency_pause, set_pause_switch, set_pause_switches,
    set_risk_params, RiskConfig, RiskManagementError,
};

mod withdraw;
use withdraw::withdraw_collateral;

mod repay;
use repay::repay_debt;

mod borrow;
use borrow::borrow_asset;

#[contracttype]
pub enum ContractDataKey {
    GovernanceAddress,
}
mod oracle;
use oracle::{configure_oracle, get_price, set_fallback_oracle, update_price_feed, OracleConfig};

mod flash_loan;
use flash_loan::{
    configure_flash_loan, execute_flash_loan, repay_flash_loan, set_flash_loan_fee, FlashLoanConfig,
};

#[contract]
pub struct HelloContract;

#[contractimpl]
impl HelloContract {
    pub fn hello(env: Env) -> String {
        String::from_str(&env, "Hello")
    }

    /// Initialize the contract with admin address and governance contract ID.
    ///
    /// Sets up the risk management system with default parameters.
    /// Must be called before any other operations.
    ///
    /// # Arguments
    /// * `admin` - The admin address
    /// * `governance_id` - The address of the deployed governance contract
    ///
    /// # Returns
    /// Returns Ok(()) on success
    pub fn initialize(
        env: Env,
        admin: Address,
        governance_id: Address,
    ) -> Result<(), RiskManagementError> {
        env.storage()
            .instance()
            .set(&ContractDataKey::GovernanceAddress, &governance_id);
        initialize_risk_management(&env, admin)
    }

    /// Deposit collateral into the protocol
    ///
    /// Allows users to deposit assets as collateral in the protocol.
    /// Supports multiple asset types including XLM (native) and token contracts (USDC, etc.).
    ///
    /// # Arguments
    /// * `user` - The address of the user depositing collateral
    /// * `asset` - The address of the asset contract to deposit (None for native XLM)
    /// * `amount` - The amount to deposit
    ///
    /// # Returns
    /// Returns the updated collateral balance for the user
    ///
    /// # Events
    /// Emits the following events:
    /// - `deposit`: Deposit transaction event
    /// - `position_updated`: User position update event
    /// - `analytics_updated`: Analytics update event
    /// - `user_activity_tracked`: User activity tracking event
    pub fn deposit_collateral(
        env: Env,
        user: Address,
        asset: Option<Address>,
        amount: i128,
    ) -> i128 {
        deposit_collateral(&env, user, asset, amount)
            .unwrap_or_else(|e| panic!("Deposit error: {:?}", e))
    }

    /// Set risk parameters (admin only, or via governance)
    ///
    /// Updates risk parameters with validation and change limits.
    /// Can optionally be submitted as a governance proposal.
    ///
    /// # Arguments
    /// * `caller` - The caller address (must be admin or proposer)
    /// * `min_collateral_ratio` - Optional new minimum collateral ratio (in basis points)
    /// * `liquidation_threshold` - Optional new liquidation threshold (in basis points)
    /// * `close_factor` - Optional new close factor (in basis points)
    /// * `liquidation_incentive` - Optional new liquidation incentive (in basis points)
    /// * `use_governance` - If true, this action is submitted as a governance proposal.
    ///
    /// # Returns
    /// Returns Ok(()) on success
    pub fn set_risk_params(
        env: Env,
        caller: Address,
        min_collateral_ratio: Option<i128>,
        liquidation_threshold: Option<i128>,
        close_factor: Option<i128>,
        liquidation_incentive: Option<i128>,
        use_governance: bool,
    ) -> Result<(), RiskManagementError> {
        if use_governance {
            let action = Action::SetRiskParams(
                min_collateral_ratio,
                liquidation_threshold,
                close_factor,
                liquidation_incentive,
            );
            // Default voting/grace periods for now. These would likely be configurable.
            let voting_period = 60 * 60 * 24 * 3; // 3 days
            let grace_period = 60 * 60 * 24; // 1 day
            Self::get_governance_client(&env)
                .try_propose(
                    &caller,
                    &soroban_sdk::String::from_str(&env, "Set risk parameters"),
                    &action,
                    &voting_period,
                    &grace_period,
                )
                .map_err(|_| RiskManagementError::GovernanceRequired)? // Note the ? here
                .map_err(|_| RiskManagementError::GovernanceRequired)?; // Convert ClientError to RiskManagementError
            Ok(())
        } else {
            set_risk_params(
                &env,
                caller,
                min_collateral_ratio,
                liquidation_threshold,
                close_factor,
                liquidation_incentive,
            )
        }
    }

    /// Set pause switch for an operation (admin only, or via governance)
    ///
    /// # Arguments
    /// * `caller` - The caller address (must be admin or proposer)
    /// * `operation` - The operation symbol (e.g., "pause_deposit", "pause_borrow")
    /// * `paused` - Whether to pause (true) or unpause (false)
    /// * `use_governance` - If true, this action is submitted as a governance proposal.
    ///
    /// # Returns
    /// Returns Ok(()) on success
    pub fn set_pause_switch(
        env: Env,
        caller: Address,
        operation: Symbol,
        paused: bool,
        use_governance: bool,
    ) -> Result<(), RiskManagementError> {
        if use_governance {
            let action = Action::SetPauseSwitch(operation.clone(), paused);
            let voting_period = 60 * 60 * 24 * 3; // 3 days
            let grace_period = 60 * 60 * 24; // 1 day
            let description_str = if paused {
                "Pause operation"
            } else {
                "Unpause operation"
            };
            Self::get_governance_client(&env)
                .try_propose(
                    &caller,
                    &soroban_sdk::String::from_str(&env, description_str),
                    &action,
                    &voting_period,
                    &grace_period,
                )
                .map_err(|_| RiskManagementError::GovernanceRequired)?
                .map_err(|_| RiskManagementError::GovernanceRequired)?;
            Ok(())
        } else {
            set_pause_switch(&env, caller, operation, paused)
        }
    }

    /// Set multiple pause switches at once (admin only, or via governance)
    ///
    /// # Arguments
    /// * `caller` - The caller address (must be admin or proposer)
    /// * `switches` - Map of operation symbols to pause states
    /// * `use_governance` - If true, this action is submitted as a governance proposal.
    ///
    /// # Returns
    /// Returns Ok(()) on success
    pub fn set_pause_switches(
        env: Env,
        caller: Address,
        switches: Map<Symbol, bool>,
        use_governance: bool,
    ) -> Result<(), RiskManagementError> {
        if use_governance {
            let action = Action::Call(
                env.current_contract_address(),
                Symbol::new(&env, "set_pause_switches_internal"),
                soroban_sdk::Vec::from_array(
                    &env,
                    [caller.into_val(&env), switches.into_val(&env)],
                ),
            );
            let voting_period = 60 * 60 * 24 * 3; // 3 days
            let grace_period = 60 * 60 * 24; // 1 day
            Self::get_governance_client(&env)
                .try_propose(
                    &caller,
                    &soroban_sdk::String::from_str(&env, "Set multiple pause switches"),
                    &action,
                    &voting_period,
                    &grace_period,
                )
                .map_err(|_| RiskManagementError::GovernanceRequired)?
                .map_err(|_| RiskManagementError::GovernanceRequired)?;
            Ok(())
        } else {
            set_pause_switches(&env, caller, switches)
        }
    }

    /// Set emergency pause (admin only, or via governance)
    ///
    /// Emergency pause stops all operations immediately.
    ///
    /// # Arguments
    /// * `caller` - The caller address (must be admin or proposer)
    /// * `paused` - Whether to enable (true) or disable (false) emergency pause
    /// * `use_governance` - If true, this action is submitted as a governance proposal.
    ///
    /// # Returns
    /// Returns Ok(()) on success
    pub fn set_emergency_pause(
        env: Env,
        caller: Address,
        paused: bool,
        use_governance: bool,
    ) -> Result<(), RiskManagementError> {
        if use_governance {
            let action = Action::SetEmergencyPause(paused);
            let voting_period = 60 * 60 * 24 * 3; // 3 days
            let grace_period = 60 * 60 * 24; // 1 day
            let description_str = if paused {
                "Enable emergency pause"
            } else {
                "Disable emergency pause"
            };
            Self::get_governance_client(&env)
                .try_propose(
                    &caller,
                    &soroban_sdk::String::from_str(&env, description_str),
                    &action,
                    &voting_period,
                    &grace_period,
                )
                .map_err(|_| RiskManagementError::GovernanceRequired)?
                .map_err(|_| RiskManagementError::GovernanceRequired)?;
            Ok(())
        } else {
            set_emergency_pause(&env, caller, paused)
        }
    }

    /// Internal function to set multiple pause switches. Designed to be called by governance.
    /// It's marked `pub(crate)` to be accessible within the crate but not externally.
    pub(crate) fn set_pause_switches_internal(
        env: Env,
        caller: Address,
        switches: Map<Symbol, bool>,
    ) -> Result<(), RiskManagementError> {
        set_pause_switches(&env, caller, switches)
    }

    /// Get current risk configuration
    ///
    /// # Returns
    /// Returns the current risk configuration or None if not initialized
    pub fn get_risk_config(env: Env) -> Option<RiskConfig> {
        risk_management::get_risk_config(&env)
    }

    /// Get minimum collateral ratio
    ///
    /// # Returns
    /// Returns the minimum collateral ratio in basis points
    pub fn get_min_collateral_ratio(env: Env) -> Result<i128, RiskManagementError> {
        get_min_collateral_ratio(&env)
    }

    /// Get liquidation threshold
    ///
    /// # Returns
    /// Returns the liquidation threshold in basis points
    pub fn get_liquidation_threshold(env: Env) -> Result<i128, RiskManagementError> {
        get_liquidation_threshold(&env)
    }

    /// Get close factor
    ///
    /// # Returns
    /// Returns the close factor in basis points
    pub fn get_close_factor(env: Env) -> Result<i128, RiskManagementError> {
        get_close_factor(&env)
    }

    /// Get liquidation incentive
    ///
    /// # Returns
    /// Returns the liquidation incentive in basis points
    pub fn get_liquidation_incentive(env: Env) -> Result<i128, RiskManagementError> {
        get_liquidation_incentive(&env)
    }

    /// Check if an operation is paused
    ///
    /// # Arguments
    /// * `operation` - The operation symbol to check
    ///
    /// # Returns
    /// Returns true if the operation is paused
    pub fn is_operation_paused(env: Env, operation: Symbol) -> bool {
        is_operation_paused(&env, operation)
    }

    /// Check if emergency pause is active
    ///
    /// # Returns
    /// Returns true if emergency pause is active
    pub fn is_emergency_paused(env: Env) -> bool {
        is_emergency_paused(&env)
    }

    /// Check if user meets minimum collateral ratio requirement
    ///
    /// # Arguments
    /// * `collateral_value` - Total collateral value (in base units)
    /// * `debt_value` - Total debt value (in base units)
    ///
    /// # Returns
    /// Returns Ok(()) if ratio is sufficient, Err otherwise
    pub fn require_min_collateral_ratio(
        env: Env,
        collateral_value: i128,
        debt_value: i128,
    ) -> Result<(), RiskManagementError> {
        require_min_collateral_ratio(&env, collateral_value, debt_value)
    }

    /// Check if position can be liquidated
    ///
    /// # Arguments
    /// * `collateral_value` - Total collateral value (in base units)
    /// * `debt_value` - Total debt value (in base units)
    ///
    /// # Returns
    /// Returns true if position can be liquidated
    pub fn can_be_liquidated(
        env: Env,
        collateral_value: i128,
        debt_value: i128,
    ) -> Result<bool, RiskManagementError> {
        can_be_liquidated(&env, collateral_value, debt_value)
    }

    /// Calculate maximum liquidatable amount
    ///
    /// # Arguments
    /// * `debt_value` - Total debt value (in base units)
    ///
    /// # Returns
    /// Maximum amount that can be liquidated
    pub fn get_max_liquidatable_amount(
        env: Env,
        debt_value: i128,
    ) -> Result<i128, RiskManagementError> {
        get_max_liquidatable_amount(&env, debt_value)
    }

    /// Calculate liquidation incentive amount
    ///
    /// # Arguments
    /// * `liquidated_amount` - Amount being liquidated (in base units)
    ///
    /// # Returns
    /// Liquidation incentive amount
    pub fn get_liquidation_incentive_amount(
        env: Env,
        liquidated_amount: i128,
    ) -> Result<i128, RiskManagementError> {
        get_liquidation_incentive_amount(&env, liquidated_amount)
    }

    /// Withdraw collateral from the protocol
    ///
    /// Allows users to withdraw their deposited collateral, subject to:
    /// - Sufficient collateral balance
    /// - Minimum collateral ratio requirements
    /// - Pause switch checks
    ///
    /// # Arguments
    /// * `user` - The address of the user withdrawing collateral
    /// * `asset` - The address of the asset contract to withdraw (None for native XLM)
    /// * `amount` - The amount to withdraw
    ///
    /// # Returns
    /// Returns the updated collateral balance for the user
    ///
    /// # Events
    /// Emits the following events:
    /// - `withdraw`: Withdraw transaction event
    /// - `position_updated`: User position update event
    /// - `analytics_updated`: Analytics update event
    /// - `user_activity_tracked`: User activity tracking event
    pub fn withdraw_collateral(
        env: Env,
        user: Address,
        asset: Option<Address>,
        amount: i128,
    ) -> i128 {
        withdraw_collateral(&env, user, asset, amount)
            .unwrap_or_else(|e| panic!("Withdraw error: {:?}", e))
    }

    /// Repay debt to the protocol
    ///
    /// Allows users to repay their borrowed assets, reducing debt and accrued interest.
    /// Supports both partial and full repayments.
    ///
    /// # Arguments
    /// * `user` - The address of the user repaying debt
    /// * `asset` - The address of the asset contract to repay (None for native XLM)
    /// * `amount` - The amount to repay
    ///
    /// # Returns
    /// Returns a tuple (remaining_debt, interest_paid, principal_paid)
    ///
    /// # Events
    /// Emits the following events:
    /// - `repay`: Repay transaction event
    /// - `position_updated`: User position update event
    /// - `analytics_updated`: Analytics update event
    /// - `user_activity_tracked`: User activity tracking event
    pub fn repay_debt(
        env: Env,
        user: Address,
        asset: Option<Address>,
        amount: i128,
    ) -> (i128, i128, i128) {
        repay_debt(&env, user, asset, amount).unwrap_or_else(|e| panic!("Repay error: {:?}", e))
    }

    /// Borrow assets from the protocol
    ///
    /// Allows users to borrow assets against their deposited collateral, subject to:
    /// - Sufficient collateral balance
    /// - Minimum collateral ratio requirements
    /// - Pause switch checks
    /// - Maximum borrow limits
    ///
    /// # Arguments
    /// * `user` - The address of the user borrowing assets
    /// * `asset` - The address of the asset contract to borrow (None for native XLM)
    /// * `amount` - The amount to borrow
    ///
    /// # Returns
    /// Returns the updated total debt (principal + interest) for the user
    ///
    /// # Events
    /// Emits the following events:
    /// - `borrow`: Borrow transaction event
    /// - `position_updated`: User position update event
    /// - `analytics_updated`: Analytics update event
    /// - `user_activity_tracked`: User activity tracking event
    pub fn borrow_asset(env: Env, user: Address, asset: Option<Address>, amount: i128) -> i128 {
        borrow_asset(&env, user, asset, amount).unwrap_or_else(|e| panic!("Borrow error: {:?}", e))
    }

    // --- Governance Proxy Functions ---

    fn get_governance_client(env: &Env) -> GovernanceContractClient<'static> {
        let governance_id: Address = env
            .storage()
            .instance()
            .get(&ContractDataKey::GovernanceAddress)
            .expect("Governance contract not initialized");
        GovernanceContractClient::new(env, &governance_id)
    }

    /// Proxies to GovernanceContract::propose
    pub fn gov_propose(
        env: Env,
        proposer: Address,
        description: soroban_sdk::String,
        action: Action,
        voting_period_seconds: u64,
        grace_period_seconds: u64,
    ) -> Result<u32, RiskManagementError> {
        // Return RiskManagementError for consistency
        Self::get_governance_client(&env)
            .try_propose(
                &proposer,
                &description,
                &action,
                &voting_period_seconds,
                &grace_period_seconds,
            )
            .map_err(|_| RiskManagementError::GovernanceRequired)?
            .map_err(|_| RiskManagementError::GovernanceRequired)
    }

    /// Proxies to GovernanceContract::vote
    pub fn gov_vote(env: Env, voter: Address, proposal_id: u32) -> Result<(), RiskManagementError> {
        Self::get_governance_client(&env)
            .try_vote(&voter, &proposal_id)
            .map_err(|_| RiskManagementError::GovernanceRequired)?
            .map_err(|_| RiskManagementError::GovernanceRequired)
    }

    /// Proxies to GovernanceContract::execute
    pub fn gov_execute(env: Env, proposal_id: u32) -> Result<(), RiskManagementError> {
        Self::get_governance_client(&env)
            .try_execute(&proposal_id)
            .map_err(|_| RiskManagementError::GovernanceRequired)?
            .map_err(|_| RiskManagementError::GovernanceRequired)
    }

    /// Proxies to GovernanceContract::get_proposal
    pub fn gov_get_proposal(env: Env, proposal_id: u32) -> Result<Proposal, RiskManagementError> {
        Self::get_governance_client(&env)
            .try_get_proposal(&proposal_id)
            .map_err(|_| RiskManagementError::GovernanceRequired)?
            .map_err(|_| RiskManagementError::GovernanceRequired)
    }

    /// Proxies to GovernanceContract::add_signer
    pub fn gov_add_signer(
        env: Env,
        caller: Address,
        new_signer: Address,
    ) -> Result<(), RiskManagementError> {
        Self::get_governance_client(&env)
            .try_add_signer(&caller, &new_signer)
            .map_err(|_| RiskManagementError::GovernanceRequired)?
            .map_err(|_| RiskManagementError::GovernanceRequired)
    }

    /// Proxies to GovernanceContract::remove_signer
    pub fn gov_remove_signer(
        env: Env,
        caller: Address,
        signer_to_remove: Address,
    ) -> Result<(), RiskManagementError> {
        Self::get_governance_client(&env)
            .try_remove_signer(&caller, &signer_to_remove)
            .map_err(|_| RiskManagementError::GovernanceRequired)?
            .map_err(|_| RiskManagementError::GovernanceRequired)
    }

    /// Proxies to GovernanceContract::set_threshold
    pub fn gov_set_threshold(
        env: Env,
        caller: Address,
        new_threshold: u32,
    ) -> Result<(), RiskManagementError> {
        Self::get_governance_client(&env)
            .try_set_threshold(&caller, &new_threshold)
            .map_err(|_| RiskManagementError::GovernanceRequired)?
            .map_err(|_| RiskManagementError::GovernanceRequired)
    }

    /// Proxies to GovernanceContract::get_signers
    pub fn gov_get_signers(env: Env) -> Result<soroban_sdk::Vec<Address>, RiskManagementError> {
        Self::get_governance_client(&env)
            .try_get_signers()
            .map_err(|_| RiskManagementError::GovernanceRequired)?
            .map_err(|_| RiskManagementError::GovernanceRequired)
    }

    /// Proxies to GovernanceContract::get_threshold
    pub fn gov_get_threshold(env: Env) -> Result<u32, RiskManagementError> {
        Self::get_governance_client(&env)
            .try_get_threshold()
            .map_err(|_| RiskManagementError::GovernanceRequired)?
            .map_err(|_| RiskManagementError::GovernanceRequired)
    }

    /// Proxies to GovernanceContract::get_admin
    pub fn gov_get_admin(env: Env) -> Result<Address, RiskManagementError> {
        Self::get_governance_client(&env)
            .try_get_admin()
            .map_err(|_| RiskManagementError::GovernanceRequired)?
            .map_err(|_| RiskManagementError::GovernanceRequired)
    }

    /// Proxies to GovernanceContract::transfer_admin
    pub fn gov_transfer_admin(
        env: Env,
        caller: Address,
        new_admin: Address,
    ) -> Result<(), RiskManagementError> {
        Self::get_governance_client(&env)
            .try_transfer_admin(&caller, &new_admin)
            .map_err(|_| RiskManagementError::GovernanceRequired)?
            .map_err(|_| RiskManagementError::GovernanceRequired)
    }
    /// Update price feed from oracle
    ///
    /// Updates the price for an asset from an oracle source with validation.
    ///
    /// # Arguments
    /// * `caller` - The address calling this function (must be admin or oracle)
    /// * `asset` - The asset address
    /// * `price` - The new price
    /// * `decimals` - Price decimals
    /// * `oracle` - The oracle address providing this price
    ///
    /// # Returns
    /// Returns the updated price
    ///
    /// # Events
    /// Emits `price_updated` event
    pub fn update_price_feed(
        env: Env,
        caller: Address,
        asset: Address,
        price: i128,
        decimals: u32,
        oracle: Address,
    ) -> i128 {
        update_price_feed(&env, caller, asset, price, decimals, oracle)
            .unwrap_or_else(|e| panic!("Oracle error: {:?}", e))
    }

    /// Get price for an asset
    ///
    /// Retrieves the current price for an asset, using cache or fallback if needed.
    ///
    /// # Arguments
    /// * `asset` - The asset address
    ///
    /// # Returns
    /// Returns the current price
    pub fn get_price(env: Env, asset: Address) -> i128 {
        get_price(&env, &asset).unwrap_or_else(|e| panic!("Oracle error: {:?}", e))
    }

    /// Set fallback oracle for an asset (admin only)
    ///
    /// # Arguments
    /// * `caller` - The caller address (must be admin)
    /// * `asset` - The asset address
    /// * `fallback_oracle` - The fallback oracle address
    pub fn set_fallback_oracle(
        env: Env,
        caller: Address,
        asset: Address,
        fallback_oracle: Address,
    ) {
        set_fallback_oracle(&env, caller, asset, fallback_oracle)
            .unwrap_or_else(|e| panic!("Oracle error: {:?}", e))
    }

    /// Configure oracle parameters (admin only)
    ///
    /// # Arguments
    /// * `caller` - The caller address (must be admin)
    /// * `config` - The new oracle configuration
    pub fn configure_oracle(env: Env, caller: Address, config: OracleConfig) {
        configure_oracle(&env, caller, config).unwrap_or_else(|e| panic!("Oracle error: {:?}", e))
    }

    /// Execute flash loan
    ///
    /// Allows users to borrow assets without collateral for a single transaction.
    /// The loan must be repaid (with fee) within the same transaction.
    ///
    /// # Arguments
    /// * `user` - The address borrowing the flash loan
    /// * `asset` - The address of the asset contract to borrow
    /// * `amount` - The amount to borrow
    /// * `callback` - The callback contract address that will handle repayment
    ///
    /// # Returns
    /// Returns the total amount to repay (principal + fee)
    ///
    /// # Events
    /// Emits `flash_loan_initiated` event
    pub fn execute_flash_loan(
        env: Env,
        user: Address,
        asset: Address,
        amount: i128,
        callback: Address,
    ) -> i128 {
        execute_flash_loan(&env, user, asset, amount, callback)
            .unwrap_or_else(|e| panic!("Flash loan error: {:?}", e))
    }

    /// Repay flash loan
    ///
    /// Must be called within the same transaction as the flash loan.
    /// Validates that the full amount (principal + fee) is repaid.
    ///
    /// # Arguments
    /// * `user` - The address repaying the flash loan
    /// * `asset` - The address of the asset contract
    /// * `amount` - The amount being repaid (should equal principal + fee)
    ///
    /// # Events
    /// Emits `flash_loan_repaid` event
    pub fn repay_flash_loan(env: Env, user: Address, asset: Address, amount: i128) {
        repay_flash_loan(&env, user, asset, amount)
            .unwrap_or_else(|e| panic!("Flash loan error: {:?}", e))
    }

    /// Set flash loan fee (admin only)
    ///
    /// # Arguments
    /// * `caller` - The caller address (must be admin)
    /// * `fee_bps` - The new fee in basis points
    pub fn set_flash_loan_fee(env: Env, caller: Address, fee_bps: i128) {
        set_flash_loan_fee(&env, caller, fee_bps)
            .unwrap_or_else(|e| panic!("Flash loan error: {:?}", e))
    }

    /// Configure flash loan parameters (admin only)
    ///
    /// # Arguments
    /// * `caller` - The caller address (must be admin)
    /// * `config` - The new flash loan configuration
    pub fn configure_flash_loan(env: Env, caller: Address, config: FlashLoanConfig) {
        configure_flash_loan(&env, caller, config)
            .unwrap_or_else(|e| panic!("Flash loan error: {:?}", e))
    }
}

#[cfg(test)]
mod governance_test;
#[cfg(test)]
mod test;
