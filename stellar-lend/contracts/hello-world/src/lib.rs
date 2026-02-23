//! # StellarLend Core Contract
//!
//! The main entrypoint for the StellarLend lending protocol on Soroban.
//!
//! This contract orchestrates all protocol operations including:
//! - **Collateral management**: deposit and withdraw collateral assets
//! - **Borrowing**: borrow assets against deposited collateral
//! - **Repayment**: repay debt (partial or full) with interest
//! - **Liquidation**: liquidate undercollateralized positions
//! - **Risk management**: configurable risk parameters and pause controls
//! - **Interest rates**: dynamic kink-based interest rate model
//! - **Oracle integration**: price feeds with staleness checks and fallbacks
//! - **Flash loans**: uncollateralized single-transaction loans
//! - **Analytics**: protocol and user reporting
//! - **Governance**: on-chain proposal voting and execution

#![allow(clippy::too_many_arguments)]
#![allow(deprecated)]
#![allow(unused_variables)]
#![no_std]

use soroban_sdk::{contract, contractimpl, Address, Env, Map, String, Symbol, Vec};

mod admin;
mod borrow;
mod deposit;
mod errors;
mod events;
mod repay;
mod risk_management;
mod storage;
mod types;
mod withdraw;

use borrow::borrow_asset;
use deposit::deposit_collateral;
use repay::repay_debt;

use risk_management::{
    can_be_liquidated, get_close_factor, get_liquidation_incentive,
    get_liquidation_incentive_amount, get_liquidation_threshold, get_max_liquidatable_amount,
    get_min_collateral_ratio, initialize_risk_management, is_emergency_paused, is_operation_paused,
    require_min_collateral_ratio, set_emergency_pause, set_pause_switch, set_pause_switches,
    set_risk_params, RiskConfig, RiskManagementError,
};
use withdraw::withdraw_collateral;

mod analytics;

use analytics::{
    generate_protocol_report, generate_user_report, get_recent_activity, get_user_activity_feed,
    AnalyticsError, ProtocolReport, UserReport,
};

mod cross_asset;
use cross_asset::{
    cross_asset_borrow, cross_asset_deposit, cross_asset_repay, cross_asset_withdraw,
    get_asset_config_by_address, get_asset_list, get_user_asset_position,
    get_user_position_summary, initialize, initialize_asset, update_asset_config,
    update_asset_price, AssetConfig, AssetKey, AssetPosition, CrossAssetError, UserPositionSummary,
};

mod oracle;
use oracle::{
    configure_oracle, get_price, set_fallback_oracle, set_primary_oracle, update_price_feed,
    OracleConfig,
};

mod config;
use config::{config_backup, config_get, config_restore, config_set, ConfigError};

mod flash_loan;
use flash_loan::{
    configure_flash_loan, execute_flash_loan, repay_flash_loan, set_flash_loan_fee, FlashLoanConfig,
};

mod bridge;
#[allow(unused_imports)]
use bridge::{
    bridge_deposit, bridge_withdraw, get_bridge_config, list_bridges, register_bridge,
    set_bridge_fee, BridgeConfig, BridgeError,
};

mod liquidate;
use liquidate::liquidate;

mod interest_rate;
#[allow(unused_imports)]
use interest_rate::{
    get_current_borrow_rate, get_current_supply_rate, get_current_utilization,
    initialize_interest_rate_config, set_emergency_rate_adjustment, update_interest_rate_config,
    InterestRateError,
};

mod governance;

use storage::GuardianConfig;

// Governance module
use crate::types::{
    GovernanceConfig, MultisigConfig, Proposal, ProposalOutcome, ProposalType, RecoveryRequest,
    VoteInfo, VoteType,
};
// use crate::governance::self;

/// The StellarLend core contract.
///
/// Provides the public API for all lending protocol operations. Each method
/// delegates to the corresponding module implementation and converts internal
/// errors into panics for Soroban's contract-call semantics.
#[contract]
pub struct HelloContract;

#[contractimpl]
impl HelloContract {
    /// Health-check endpoint.
    ///
    /// Returns the string `"Hello"` to verify the contract is deployed and callable.
    pub fn hello(env: Env) -> String {
        String::from_str(&env, "Hello")
    }

    /// Initialize the contract with admin address.
    ///
    /// Sets up the risk management system and interest rate model with default parameters.
    /// Must be called before any other operations.
    ///
    /// # Arguments
    /// * `admin` - The admin address
    ///
    /// # Returns
    /// Returns Ok(()) on success
    pub fn initialize(env: Env, admin: Address) -> Result<(), RiskManagementError> {
        // Prevent double initialization
        if crate::admin::has_admin(&env) {
            return Err(RiskManagementError::Unauthorized); // or a specific error
        }

        crate::admin::set_admin(&env, admin.clone(), None)
            .map_err(|_| RiskManagementError::Unauthorized)?;
        initialize_risk_management(&env, admin.clone())?;
        initialize_interest_rate_config(&env, admin).map_err(|e| {
            if e == InterestRateError::AlreadyInitialized {
                RiskManagementError::AlreadyInitialized
            } else {
                RiskManagementError::Unauthorized
            }
        })?;
        Ok(())
    }

    /// Transfer super admin rights
    ///
    /// # Arguments
    /// * `caller` - The current admin
    /// * `new_admin` - The new admin
    pub fn transfer_admin(
        env: Env,
        caller: Address,
        new_admin: Address,
    ) -> Result<(), crate::admin::AdminError> {
        crate::admin::set_admin(&env, new_admin, Some(caller))
    }

    /// Grant a role to an address (admin only)
    pub fn grant_role(
        env: Env,
        caller: Address,
        role: Symbol,
        account: Address,
    ) -> Result<(), crate::admin::AdminError> {
        crate::admin::grant_role(&env, caller, role, account)
    }

    /// Revoke a role from an address (admin only)
    pub fn revoke_role(
        env: Env,
        caller: Address,
        role: Symbol,
        account: Address,
    ) -> Result<(), crate::admin::AdminError> {
        crate::admin::revoke_role(&env, caller, role, account)
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

    /// Set native asset address (admin only). Required before using asset = None for deposit/borrow/repay.
    pub fn set_native_asset_address(
        env: Env,
        caller: Address,
        native_asset: Address,
    ) -> Result<(), deposit::DepositError> {
        deposit::set_native_asset_address(&env, caller, native_asset)
    }

    /// Set risk parameters (admin only)
    ///
    /// Updates risk parameters with validation and change limits.
    ///
    /// # Arguments
    /// * `caller` - The caller address (must be admin)
    /// * `min_collateral_ratio` - Optional new minimum collateral ratio (in basis points)
    /// * `liquidation_threshold` - Optional new liquidation threshold (in basis points)
    /// * `close_factor` - Optional new close factor (in basis points)
    /// * `liquidation_incentive` - Optional new liquidation incentive (in basis points)
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
    ) -> Result<(), RiskManagementError> {
        set_risk_params(
            &env,
            caller,
            min_collateral_ratio,
            liquidation_threshold,
            close_factor,
            liquidation_incentive,
        )
    }

    /// Set pause switch for an operation (admin only)
    ///
    /// # Arguments
    /// * `caller` - The caller address (must be admin)
    /// * `operation` - The operation symbol (e.g., "pause_deposit", "pause_borrow")
    /// * `paused` - Whether to pause (true) or unpause (false)
    ///
    /// # Returns
    /// Returns Ok(()) on success
    pub fn set_pause_switch(
        env: Env,
        caller: Address,
        operation: Symbol,
        paused: bool,
    ) -> Result<(), RiskManagementError> {
        set_pause_switch(&env, caller, operation, paused)
    }

    /// Set multiple pause switches at once (admin only)
    ///
    /// # Arguments
    /// * `caller` - The caller address (must be admin)
    /// * `switches` - Map of operation symbols to pause states
    ///
    /// # Returns
    /// Returns Ok(()) on success
    pub fn set_pause_switches(
        env: Env,
        caller: Address,
        switches: Map<Symbol, bool>,
    ) -> Result<(), RiskManagementError> {
        set_pause_switches(&env, caller, switches)
    }

    /// Set emergency pause (admin only)
    ///
    /// Emergency pause stops all operations immediately.
    ///
    /// # Arguments
    /// * `caller` - The caller address (must be admin)
    /// * `paused` - Whether to enable (true) or disable (false) emergency pause
    ///
    /// # Returns
    /// Returns Ok(()) on success
    pub fn set_emergency_pause(
        env: Env,
        caller: Address,
        paused: bool,
    ) -> Result<(), RiskManagementError> {
        set_emergency_pause(&env, caller, paused)
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

    /// Generate a comprehensive protocol report.
    ///
    /// Aggregates TVL, utilization, average borrow rate, and user/transaction counts
    /// into a single [`ProtocolReport`] snapshot.
    ///
    /// # Returns
    /// A `ProtocolReport` containing current protocol metrics and timestamp.
    ///
    /// # Errors
    /// Returns `AnalyticsError` if protocol data is not initialized or computation overflows.
    pub fn get_protocol_report(env: Env) -> Result<ProtocolReport, AnalyticsError> {
        generate_protocol_report(&env)
    }

    /// Generate a comprehensive report for a specific user.
    ///
    /// Includes the user's position, health factor, risk level, activity history,
    /// and cumulative transaction metrics.
    ///
    /// # Arguments
    /// * `user` - The address of the user to report on
    ///
    /// # Returns
    /// A `UserReport` with the user's metrics, position, and recent activities.
    ///
    /// # Errors
    /// Returns `AnalyticsError::DataNotFound` if the user has no recorded activity.
    pub fn get_user_report(env: Env, user: Address) -> Result<UserReport, AnalyticsError> {
        generate_user_report(&env, &user)
    }

    /// Retrieve recent protocol activity entries.
    ///
    /// Returns a paginated list of the most recent protocol activities in
    /// reverse chronological order.
    ///
    /// # Arguments
    /// * `limit` - Maximum number of entries to return
    /// * `offset` - Number of entries to skip from the most recent
    ///
    /// # Returns
    /// A vector of `ActivityEntry` records.
    pub fn get_recent_activity(
        env: Env,
        limit: u32,
        offset: u32,
    ) -> Result<soroban_sdk::Vec<analytics::ActivityEntry>, AnalyticsError> {
        get_recent_activity(&env, limit, offset)
    }

    /// Retrieve activity entries for a specific user.
    ///
    /// Returns a paginated list of the user's activities in reverse
    /// chronological order.
    ///
    /// # Arguments
    /// * `user` - The address of the user
    /// * `limit` - Maximum number of entries to return
    /// * `offset` - Number of entries to skip from the most recent
    ///
    /// # Returns
    /// A vector of `ActivityEntry` records for the specified user.
    pub fn get_user_activity(
        env: Env,
        user: Address,
        limit: u32,
        offset: u32,
    ) -> Result<soroban_sdk::Vec<analytics::ActivityEntry>, AnalyticsError> {
        get_user_activity_feed(&env, &user, limit, offset)
    }

    /// Update price feed from oracle
    ///
    /// Updates the price for an asset from an oracle source with validation.
    ///
    /// # Arguments
    /// * `caller` - The caller address (must be admin or oracle)
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

    /// Set primary oracle for an asset (admin only)
    ///
    /// # Arguments
    /// * `caller` - The caller address (must be admin)
    /// * `asset` - The asset address
    /// * `primary_oracle` - The primary oracle address
    pub fn set_primary_oracle(env: Env, caller: Address, asset: Address, primary_oracle: Address) {
        set_primary_oracle(&env, caller, asset, primary_oracle)
            .unwrap_or_else(|e| panic!("Oracle error: {:?}", e))
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

    /// Liquidate an undercollateralized position
    ///
    /// Allows liquidators to liquidate undercollateralized positions by:
    /// - Repaying debt on behalf of the borrower
    /// - Receiving collateral plus a liquidation incentive
    ///
    /// # Arguments
    /// * `liquidator` - The address of the liquidator
    /// * `borrower` - The address of the borrower being liquidated
    /// * `debt_asset` - The address of the debt asset to repay (None for native XLM)
    /// * `collateral_asset` - The address of the collateral asset to receive (None for native XLM)
    /// * `debt_amount` - The amount of debt to liquidate
    ///
    /// # Returns
    /// Returns a tuple (debt_liquidated, collateral_seized, incentive_amount)
    ///
    /// # Events
    /// Emits the following events:
    /// - `liquidation`: Liquidation transaction event
    /// - `position_updated`: Borrower position update event
    /// - `analytics_updated`: Analytics update event
    /// - `user_activity_tracked`: User activity tracking event
    pub fn liquidate(
        env: Env,
        liquidator: Address,
        borrower: Address,
        debt_asset: Option<Address>,
        collateral_asset: Option<Address>,
        debt_amount: i128,
    ) -> (i128, i128, i128) {
        liquidate(
            &env,
            liquidator,
            borrower,
            debt_asset,
            collateral_asset,
            debt_amount,
        )
        .unwrap_or_else(|e| panic!("Liquidation error: {:?}", e))
    }

    /// Get current utilization rate
    ///
    /// Returns the current protocol utilization (borrows / deposits) in basis points.
    ///
    /// # Returns
    /// Utilization rate in basis points (0-10000)
    pub fn get_utilization(env: Env) -> i128 {
        get_current_utilization(&env).unwrap_or_else(|e| panic!("Interest rate error: {:?}", e))
    }

    /// Get current borrow interest rate
    ///
    /// Returns the current borrow interest rate based on utilization.
    ///
    /// # Returns
    /// Borrow rate in basis points (annual)
    pub fn get_borrow_rate(env: Env) -> i128 {
        get_current_borrow_rate(&env).unwrap_or_else(|e| panic!("Interest rate error: {:?}", e))
    }

    /// Get current supply interest rate
    ///
    /// Returns the current supply interest rate (borrow rate - spread).
    ///
    /// # Returns
    /// Supply rate in basis points (annual)
    pub fn get_supply_rate(env: Env) -> i128 {
        get_current_supply_rate(&env).unwrap_or_else(|e| panic!("Interest rate error: {:?}", e))
    }

    /// Update interest rate configuration (admin only)
    ///
    /// Updates interest rate model parameters with validation.
    ///
    /// # Arguments
    /// * `caller` - The caller address (must be admin)
    /// * `base_rate_bps` - Optional new base rate (in basis points)
    /// * `kink_utilization_bps` - Optional new kink utilization (in basis points)
    /// * `multiplier_bps` - Optional new multiplier (in basis points)
    /// * `jump_multiplier_bps` - Optional new jump multiplier (in basis points)
    /// * `rate_floor_bps` - Optional new rate floor (in basis points)
    /// * `rate_ceiling_bps` - Optional new rate ceiling (in basis points)
    /// * `spread_bps` - Optional new spread (in basis points)
    ///
    /// # Returns
    /// Returns Ok(()) on success
    #[allow(clippy::too_many_arguments)]
    pub fn update_interest_rate_config(
        env: Env,
        caller: Address,
        base_rate_bps: Option<i128>,
        kink_utilization_bps: Option<i128>,
        multiplier_bps: Option<i128>,
        jump_multiplier_bps: Option<i128>,
        rate_floor_bps: Option<i128>,
        rate_ceiling_bps: Option<i128>,
        spread_bps: Option<i128>,
    ) -> Result<(), InterestRateError> {
        update_interest_rate_config(
            &env,
            caller,
            base_rate_bps,
            kink_utilization_bps,
            multiplier_bps,
            jump_multiplier_bps,
            rate_floor_bps,
            rate_ceiling_bps,
            spread_bps,
        )
    }

    /// Set emergency rate adjustment (admin only)
    ///
    /// Allows admin to make emergency adjustments to interest rates.
    ///
    /// # Arguments
    /// * `caller` - The caller address (must be admin)
    /// * `adjustment_bps` - Emergency adjustment in basis points (can be negative)
    ///
    /// # Returns
    /// Returns Ok(()) on success
    pub fn set_emergency_rate_adjustment(
        env: Env,
        caller: Address,
        adjustment_bps: i128,
    ) -> Result<(), InterestRateError> {
        set_emergency_rate_adjustment(&env, caller, adjustment_bps)
    }

    /// Set a configuration value (admin only)
    ///
    /// # Arguments
    /// * `caller` - The caller address (must be admin)
    /// * `key` - The configuration key
    /// * `value` - The configuration value
    ///
    /// # Returns
    /// Returns Ok(()) on success
    pub fn config_set(
        env: Env,
        caller: Address,
        key: soroban_sdk::Symbol,
        value: soroban_sdk::Val,
    ) -> Result<(), ConfigError> {
        config_set(&env, caller, key, value)
    }

    /// Get a configuration value
    ///
    /// # Arguments
    /// * `key` - The configuration key
    ///
    /// # Returns
    /// Returns Some(value) if the key exists, None otherwise
    pub fn config_get(env: Env, key: soroban_sdk::Symbol) -> Option<soroban_sdk::Val> {
        config_get(&env, key)
    }

    /// Backup configuration parameters (admin only)
    ///
    /// # Arguments
    /// * `caller` - The caller address (must be admin)
    /// * `keys` - A vector of configuration keys to backup
    ///
    /// # Returns
    /// Returns a vector of key-value pairs representing the backup
    pub fn config_backup(
        env: Env,
        caller: Address,
        keys: soroban_sdk::Vec<soroban_sdk::Symbol>,
    ) -> Result<soroban_sdk::Vec<(soroban_sdk::Symbol, soroban_sdk::Val)>, ConfigError> {
        config_backup(&env, caller, keys)
    }

    /// Restore configuration parameters (admin only)
    ///
    /// # Arguments
    /// * `caller` - The caller address (must be admin)
    /// * `backup` - A vector of key-value pairs to restore
    ///
    /// # Returns
    /// Returns Ok(()) on success
    pub fn config_restore(
        env: Env,
        caller: Address,
        backup: soroban_sdk::Vec<(soroban_sdk::Symbol, soroban_sdk::Val)>,
    ) -> Result<(), ConfigError> {
        config_restore(&env, caller, backup)
    }

    // ============================================================================
    // Cross-Asset / Asset Configuration Methods
    // ============================================================================

    /// Initialize cross-asset lending module (admin only)
    ///
    /// Sets up the cross-asset lending system with an admin.
    /// Must be called before any asset configuration.
    ///
    /// # Arguments
    /// * `admin` - The admin address
    ///
    /// # Returns
    /// Returns Ok(()) on success
    pub fn initialize_ca(env: Env, admin: Address) -> Result<(), CrossAssetError> {
        initialize(&env, admin)
    }

    /// Initialize/register a new asset with configuration
    ///
    /// Registers an asset with its collateral factor (LTV), liquidation threshold,
    /// debt ceiling, and other parameters. Admin only.
    ///
    /// # Arguments
    /// * `asset` - Asset address (None for native XLM)
    /// * `config` - Complete asset configuration
    ///
    /// # Returns
    /// Returns Ok(()) on success
    pub fn initialize_asset(
        env: Env,
        asset: Option<Address>,
        config: AssetConfig,
    ) -> Result<(), CrossAssetError> {
        initialize_asset(&env, asset, config)
    }

    /// Update asset configuration (admin only)
    ///
    /// Updates asset parameters including LTV, liquidation threshold, debt ceiling,
    /// and collateral/borrow enablement. Only provided fields are updated.
    ///
    /// # Arguments
    /// * `asset` - Asset address (None for XLM)
    /// * `collateral_factor` - Optional new LTV in basis points
    /// * `liquidation_threshold` - Optional new liquidation threshold in basis points
    /// * `max_supply` - Optional new supply cap
    /// * `max_borrow` - Optional new debt ceiling
    /// * `can_collateralize` - Optional flag to enable/disable as collateral
    /// * `can_borrow` - Optional flag to enable/disable borrowing
    ///
    /// # Returns
    /// Returns Ok(()) on success
    #[allow(clippy::too_many_arguments)]
    pub fn update_asset_config(
        env: Env,
        asset: Option<Address>,
        collateral_factor: Option<i128>,
        liquidation_threshold: Option<i128>,
        max_supply: Option<i128>,
        max_borrow: Option<i128>,
        can_collateralize: Option<bool>,
        can_borrow: Option<bool>,
    ) -> Result<(), CrossAssetError> {
        update_asset_config(
            &env,
            asset,
            collateral_factor,
            liquidation_threshold,
            max_supply,
            max_borrow,
            can_collateralize,
            can_borrow,
        )
    }

    /// Update asset price (admin/oracle only)
    ///
    /// Updates the price for an asset used in health factor calculations.
    ///
    /// # Arguments
    /// * `asset` - Asset address (None for XLM)
    /// * `price` - New price in base units (7 decimals)
    ///
    /// # Returns
    /// Returns Ok(()) on success
    pub fn update_asset_price(
        env: Env,
        asset: Option<Address>,
        price: i128,
    ) -> Result<(), CrossAssetError> {
        update_asset_price(&env, asset, price)
    }

    /// Get asset configuration
    ///
    /// Returns the configuration for a specific asset including LTV,
    /// liquidation threshold, debt ceiling, etc.
    ///
    /// # Arguments
    /// * `asset` - Asset address (None for XLM)
    ///
    /// # Returns
    /// Asset configuration
    pub fn get_asset_config(
        env: Env,
        asset: Option<Address>,
    ) -> Result<AssetConfig, CrossAssetError> {
        get_asset_config_by_address(&env, asset)
    }

    /// Get list of all configured assets
    ///
    /// Returns all assets that have been registered in the system.
    ///
    /// # Returns
    /// Vector of asset keys
    pub fn get_asset_list(env: Env) -> soroban_sdk::Vec<AssetKey> {
        get_asset_list(&env)
    }

    /// Deposit collateral for cross-asset lending
    ///
    /// Deposits collateral that can be used across multiple assets.
    /// Asset must be enabled for collateral.
    ///
    /// # Arguments
    /// * `user` - User address
    /// * `asset` - Asset address (None for XLM)
    /// * `amount` - Amount to deposit
    ///
    /// # Returns
    /// Updated asset position
    pub fn cross_asset_deposit(
        env: Env,
        user: Address,
        asset: Option<Address>,
        amount: i128,
    ) -> Result<AssetPosition, CrossAssetError> {
        cross_asset_deposit(&env, user, asset, amount)
    }

    /// Withdraw collateral from cross-asset lending
    ///
    /// Withdraws collateral while maintaining healthy position.
    ///
    /// # Arguments
    /// * `user` - User address
    /// * `asset` - Asset address (None for XLM)
    /// * `amount` - Amount to withdraw
    ///
    /// # Returns
    /// Updated asset position
    pub fn cross_asset_withdraw(
        env: Env,
        user: Address,
        asset: Option<Address>,
        amount: i128,
    ) -> Result<AssetPosition, CrossAssetError> {
        cross_asset_withdraw(&env, user, asset, amount)
    }

    /// Borrow asset in cross-asset lending
    ///
    /// Borrows against cross-asset collateral. Respects LTV and debt ceiling.
    ///
    /// # Arguments
    /// * `user` - User address
    /// * `asset` - Asset address (None for XLM)
    /// * `amount` - Amount to borrow
    ///
    /// # Returns
    /// Updated asset position
    pub fn cross_asset_borrow(
        env: Env,
        user: Address,
        asset: Option<Address>,
        amount: i128,
    ) -> Result<AssetPosition, CrossAssetError> {
        cross_asset_borrow(&env, user, asset, amount)
    }

    /// Repay borrowed asset
    ///
    /// Repays debt for a specific asset.
    ///
    /// # Arguments
    /// * `user` - User address
    /// * `asset` - Asset address (None for XLM)
    /// * `amount` - Amount to repay
    ///
    /// # Returns
    /// Updated asset position
    pub fn cross_asset_repay(
        env: Env,
        user: Address,
        asset: Option<Address>,
        amount: i128,
    ) -> Result<AssetPosition, CrossAssetError> {
        cross_asset_repay(&env, user, asset, amount)
    }

    /// Get user's position for a specific asset
    ///
    /// Returns collateral and debt for a user in a specific asset.
    ///
    /// # Arguments
    /// * `user` - User address
    /// * `asset` - Asset address (None for XLM)
    ///
    /// # Returns
    /// Asset position
    pub fn get_user_asset_position(
        env: Env,
        user: Address,
        asset: Option<Address>,
    ) -> AssetPosition {
        get_user_asset_position(&env, &user, asset)
    }

    /// Get user's unified position summary across all assets
    ///
    /// Calculates health factor, liquidation status, and borrow capacity
    /// across all assets.
    ///
    /// # Arguments
    /// * `user` - User address
    ///
    /// # Returns
    /// Position summary with health factor
    pub fn get_user_position_summary(
        env: Env,
        user: Address,
    ) -> Result<UserPositionSummary, CrossAssetError> {
        get_user_position_summary(&env, &user)
    }

    // ============================================================================
    // Governance Entrypoints
    // ============================================================================

    /// Initialize governance module
    ///
    /// Sets up the governance system with voting token and parameters.
    /// Must be called after contract initialization.
    ///
    /// # Arguments
    /// * `admin` - The admin address
    /// * `vote_token` - Token contract address used for voting
    /// * `voting_period` - Optional voting duration in seconds
    /// * `execution_delay` - Optional delay before execution in seconds
    /// * `quorum_bps` - Optional quorum requirement in basis points
    /// * `proposal_threshold` - Optional minimum tokens to create proposal
    /// * `timelock_duration` - Optional timelock duration in seconds
    /// * `default_voting_threshold` - Optional default voting threshold in basis points
    ///
    /// # Returns
    /// Returns Ok(()) on success
    pub fn gov_initialize(
        env: Env,
        admin: Address,
        vote_token: Address,
        voting_period: Option<u64>,
        execution_delay: Option<u64>,
        quorum_bps: Option<u32>,
        proposal_threshold: Option<i128>,
        timelock_duration: Option<u64>,
        default_voting_threshold: Option<i128>,
    ) -> Result<(), errors::GovernanceError> {
        governance::initialize(
            &env,
            admin,
            vote_token,
            voting_period,
            execution_delay,
            quorum_bps,
            proposal_threshold,
            timelock_duration,
            default_voting_threshold,
        )
    }

    /// Create a new governance proposal
    ///
    /// # Arguments
    /// * `proposer` - Address creating the proposal
    /// * `proposal_type` - Type of proposal (parameter change, pause, etc.)
    /// * `description` - Description of the proposal
    /// * `voting_threshold` - Optional custom voting threshold
    ///
    /// # Returns
    /// Returns the new proposal ID
    pub fn gov_create_proposal(
        env: Env,
        proposer: Address,
        proposal_type: ProposalType,
        description: String,
        voting_threshold: Option<i128>,
    ) -> Result<u64, errors::GovernanceError> {
        governance::create_proposal(&env, proposer, proposal_type, description, voting_threshold)
    }

    /// Cast a vote on a proposal
    ///
    /// # Arguments
    /// * `voter` - Address casting the vote
    /// * `proposal_id` - ID of the proposal to vote on
    /// * `vote_type` - Vote choice (For, Against, Abstain)
    ///
    /// # Returns
    /// Returns Ok(()) on success
    pub fn gov_vote(
        env: Env,
        voter: Address,
        proposal_id: u64,
        vote_type: VoteType,
    ) -> Result<(), errors::GovernanceError> {
        governance::vote(&env, voter, proposal_id, vote_type)
    }

    /// Queue a successful proposal for execution
    ///
    /// # Arguments
    /// * `caller` - Address queuing the proposal
    /// * `proposal_id` - ID of the proposal to queue
    ///
    /// # Returns
    /// Returns the proposal outcome
    pub fn gov_queue_proposal(
        env: Env,
        caller: Address,
        proposal_id: u64,
    ) -> Result<ProposalOutcome, errors::GovernanceError> {
        governance::queue_proposal(&env, caller, proposal_id)
    }

    /// Execute a queued proposal
    ///
    /// # Arguments
    /// * `executor` - Address executing the proposal
    /// * `proposal_id` - ID of the proposal to execute
    ///
    /// # Returns
    /// Returns Ok(()) on success
    pub fn gov_execute_proposal(
        env: Env,
        executor: Address,
        proposal_id: u64,
    ) -> Result<(), errors::GovernanceError> {
        governance::execute_proposal(&env, executor, proposal_id)
    }

    /// Cancel a proposal
    ///
    /// Only proposer or admin can cancel.
    ///
    /// # Arguments
    /// * `caller` - Address cancelling the proposal
    /// * `proposal_id` - ID of the proposal to cancel
    ///
    /// # Returns
    /// Returns Ok(()) on success
    pub fn gov_cancel_proposal(
        env: Env,
        caller: Address,
        proposal_id: u64,
    ) -> Result<(), errors::GovernanceError> {
        governance::cancel_proposal(&env, caller, proposal_id)
    }

    /// Approve a proposal as multisig admin
    ///
    /// # Arguments
    /// * `approver` - Admin address approving the proposal
    /// * `proposal_id` - ID of the proposal to approve
    ///
    /// # Returns
    /// Returns Ok(()) on success
    pub fn gov_approve_proposal(
        env: Env,
        approver: Address,
        proposal_id: u64,
    ) -> Result<(), errors::GovernanceError> {
        governance::approve_proposal(&env, approver, proposal_id)
    }

    /// Set multisig configuration
    ///
    /// # Arguments
    /// * `caller` - Caller address (must be admin)
    /// * `admins` - Vector of admin addresses
    /// * `threshold` - Number of approvals required
    ///
    /// # Returns
    /// Returns Ok(()) on success
    pub fn gov_set_multisig_config(
        env: Env,
        caller: Address,
        admins: Vec<Address>,
        threshold: u32,
    ) -> Result<(), errors::GovernanceError> {
        governance::set_multisig_config(&env, caller, admins, threshold)
    }

    /// Add a guardian
    ///
    /// # Arguments
    /// * `caller` - Caller address (must be admin)
    /// * `guardian` - Guardian address to add
    ///
    /// # Returns
    /// Returns Ok(()) on success
    pub fn gov_add_guardian(
        env: Env,
        caller: Address,
        guardian: Address,
    ) -> Result<(), errors::GovernanceError> {
        governance::add_guardian(&env, caller, guardian)
    }

    /// Remove a guardian
    ///
    /// # Arguments
    /// * `caller` - Caller address (must be admin)
    /// * `guardian` - Guardian address to remove
    ///
    /// # Returns
    /// Returns Ok(()) on success
    pub fn gov_remove_guardian(
        env: Env,
        caller: Address,
        guardian: Address,
    ) -> Result<(), errors::GovernanceError> {
        governance::remove_guardian(&env, caller, guardian)
    }

    /// Set guardian threshold
    ///
    /// # Arguments
    /// * `caller` - Caller address (must be admin)
    /// * `threshold` - Number of guardian approvals required for recovery
    ///
    /// # Returns
    /// Returns Ok(()) on success
    pub fn gov_set_guardian_threshold(
        env: Env,
        caller: Address,
        threshold: u32,
    ) -> Result<(), errors::GovernanceError> {
        governance::set_guardian_threshold(&env, caller, threshold)
    }

    /// Start recovery process
    ///
    /// # Arguments
    /// * `initiator` - Guardian initiating recovery
    /// * `old_admin` - Current admin to replace
    /// * `new_admin` - New admin address
    ///
    /// # Returns
    /// Returns Ok(()) on success
    pub fn gov_start_recovery(
        env: Env,
        initiator: Address,
        old_admin: Address,
        new_admin: Address,
    ) -> Result<(), errors::GovernanceError> {
        governance::start_recovery(&env, initiator, old_admin, new_admin)
    }

    /// Approve recovery
    ///
    /// # Arguments
    /// * `approver` - Guardian approving recovery
    ///
    /// # Returns
    /// Returns Ok(()) on success
    pub fn gov_approve_recovery(
        env: Env,
        approver: Address,
    ) -> Result<(), errors::GovernanceError> {
        governance::approve_recovery(&env, approver)
    }

    /// Execute recovery
    ///
    /// # Arguments
    /// * `executor` - Address executing recovery
    ///
    /// # Returns
    /// Returns Ok(()) on success
    pub fn gov_execute_recovery(
        env: Env,
        executor: Address,
    ) -> Result<(), errors::GovernanceError> {
        governance::execute_recovery(&env, executor)
    }

    // ============================================================================
    // Governance Query Functions
    // ============================================================================

    /// Get proposal by ID
    pub fn gov_get_proposal(env: Env, proposal_id: u64) -> Option<Proposal> {
        governance::get_proposal(&env, proposal_id)
    }

    /// Get vote information
    pub fn gov_get_vote(env: Env, proposal_id: u64, voter: Address) -> Option<VoteInfo> {
        governance::get_vote(&env, proposal_id, voter)
    }

    /// Get governance configuration
    pub fn gov_get_config(env: Env) -> Option<GovernanceConfig> {
        governance::get_config(&env)
    }

    /// Get governance admin
    pub fn gov_get_admin(env: Env) -> Option<Address> {
        governance::get_admin(&env)
    }

    /// Get multisig configuration
    pub fn gov_get_multisig_config(env: Env) -> Option<MultisigConfig> {
        governance::get_multisig_config(&env)
    }

    /// Get guardian configuration
    pub fn gov_get_guardian_config(env: Env) -> Option<GuardianConfig> {
        governance::get_guardian_config(&env)
    }

    /// Get proposal approvals
    pub fn gov_get_proposal_approvals(env: Env, proposal_id: u64) -> Option<Vec<Address>> {
        governance::get_proposal_approvals(&env, proposal_id)
    }

    /// Get current recovery request
    pub fn gov_get_recovery_request(env: Env) -> Option<RecoveryRequest> {
        governance::get_recovery_request(&env)
    }

    /// Get recovery approvals
    pub fn gov_get_recovery_approvals(env: Env) -> Option<Vec<Address>> {
        governance::get_recovery_approvals(&env)
    }

    /// Get paginated list of proposals
    pub fn gov_get_proposals(env: Env, start_id: u64, limit: u32) -> Vec<Proposal> {
        governance::get_proposals(&env, start_id, limit)
    }

    /// Check if an address can vote on a proposal
    pub fn gov_can_vote(env: Env, voter: Address, proposal_id: u64) -> bool {
        governance::can_vote(&env, voter, proposal_id)
    }

    // --- Bridge ---

    /// Register a new bridge (admin only)
    pub fn register_bridge(
        env: Env,
        caller: Address,
        network_id: u32,
        bridge: Address,
        fee_bps: i128,
    ) -> Result<(), BridgeError> {
        register_bridge(&env, caller, network_id, bridge, fee_bps)
    }

    /// Set fee for a bridge (admin only)
    pub fn set_bridge_fee(
        env: Env,
        caller: Address,
        network_id: u32,
        fee_bps: i128,
    ) -> Result<(), BridgeError> {
        set_bridge_fee(&env, caller, network_id, fee_bps)
    }

    /// List all registered bridges
    pub fn list_bridges(env: Env) -> Map<u32, BridgeConfig> {
        list_bridges(&env)
    }

    /// Get configuration for a bridge by network id
    pub fn get_bridge_config(env: Env, network_id: u32) -> Result<BridgeConfig, BridgeError> {
        get_bridge_config(&env, network_id)
    }

    /// Deposit into protocol via a bridge
    pub fn bridge_deposit(
        env: Env,
        user: Address,
        network_id: u32,
        asset: Option<Address>,
        amount: i128,
    ) -> Result<i128, BridgeError> {
        bridge_deposit(&env, user, network_id, asset, amount)
    }

    /// Withdraw from protocol via a bridge
    pub fn bridge_withdraw(
        env: Env,
        user: Address,
        network_id: u32,
        asset: Option<Address>,
        amount: i128,
    ) -> Result<i128, BridgeError> {
        bridge_withdraw(&env, user, network_id, asset, amount)
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod flash_loan_test;
