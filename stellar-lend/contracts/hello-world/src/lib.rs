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
//!
//! ## Invariants
//! - All positions must maintain the minimum collateral ratio or face liquidation.
//! - Interest accrues continuously based on protocol utilization.
//! - Only the admin can modify risk parameters, oracle config, and pause switches.
//! - Emergency pause halts all operations immediately.

#![allow(clippy::too_many_arguments)]
#![allow(deprecated)]
#![no_std]
use soroban_sdk::{contract, contractimpl, Address, Env, Map, String, Symbol};

mod borrow;
mod deposit;
mod events;
mod repay;
mod risk_management;
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
#[allow(unused_imports)]
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

mod flash_loan;
use flash_loan::{
    configure_flash_loan, execute_flash_loan, repay_flash_loan, set_flash_loan_fee, FlashLoanConfig,
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

    /// Initialize the contract with admin address and governance contract ID.
    ///
    /// Sets up the risk management system and interest rate model with default parameters.
    /// Must be called before any other operations.
    ///
    /// # Arguments
    /// * `admin` - The admin address
    /// * `governance_id` - The address of the deployed governance contract
    ///
    /// # Returns
    /// Returns Ok(()) on success
    pub fn initialize(env: Env, admin: Address) -> Result<(), RiskManagementError> {
        initialize_risk_management(&env, admin.clone())?;
        // Initialize interest rate config with default parameters
        initialize_interest_rate_config(&env, admin.clone())
            .map_err(|_| RiskManagementError::Unauthorized)?;
        // initialize_governance(&env, admin).map_err(|_| RiskManagementError::Unauthorized)?;
        Ok(())
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

    // ============================================================================
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod flash_loan_test;
