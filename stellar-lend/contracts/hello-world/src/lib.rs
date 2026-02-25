use soroban_sdk::{contract, contractimpl, Address, Env, Map, Symbol, Vec};

pub mod analytics;
pub mod borrow;
pub mod cross_asset;
pub mod deposit;
pub mod events;
pub mod flash_loan;
pub mod governance;
pub mod interest_rate;
pub mod liquidate;
pub mod oracle;
pub mod repay;
pub mod risk_management;
pub mod withdraw;

#[cfg(test)]
mod tests;

use crate::deposit::{AssetParams, DepositDataKey, ProtocolAnalytics};
use crate::oracle::OracleConfig;
use crate::risk_management::{RiskConfig, RiskManagementError};

/// Helper function to require admin authorization
fn require_admin(env: &Env, caller: &Address) -> Result<(), RiskManagementError> {
    caller.require_auth();
    let admin_key = DepositDataKey::Admin;
    let admin = env
        .storage()
        .persistent()
        .get::<DepositDataKey, Address>(&admin_key)
        .ok_or(RiskManagementError::Unauthorized)?;

    if caller != &admin {
        return Err(RiskManagementError::Unauthorized);
    }
    Ok(())
}
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

mod admin;
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

pub mod governance;

/// The StellarLend core contract.
///
/// Provides the public API for all lending protocol operations. Each method
/// delegates to the corresponding module implementation and converts internal
/// errors into panics for Soroban's contract-call semantics.
#[contract]
pub struct HelloContract;

#[contractimpl]
impl HelloContract {
    /// Initialize the contract with an admin address
    pub fn initialize(env: Env, admin: Address) {
        let admin_key = DepositDataKey::Admin;
        if env.storage().persistent().has(&admin_key) {
            panic!("Already initialized");
        }
        env.storage().persistent().set(&admin_key, &admin);

        // Initialize protocol analytics
        let analytics_key = DepositDataKey::ProtocolAnalytics;
        let analytics = ProtocolAnalytics {
            total_deposits: 0,
            total_borrows: 0,
            total_value_locked: 0,
        };
        env.storage().persistent().set(&analytics_key, &analytics);

        // Initialize other modules
        interest_rate::initialize_interest_rate_config(&env, admin.clone()).unwrap();
        risk_management::initialize_risk_management(&env, admin).unwrap();
    }

    /// Deposit assets into the protocol
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
        // Prevent double initialization
        if crate::admin::has_admin(&env) {
            return Err(RiskManagementError::Unauthorized); // or a specific error
        }

        crate::admin::set_admin(&env, admin.clone(), None)
            .map_err(|_| RiskManagementError::Unauthorized)?;
        initialize_risk_management(&env, admin.clone())?;
        // Initialize interest rate config with default parameters
        initialize_interest_rate_config(&env, admin.clone()).map_err(|e| {
            if e == InterestRateError::AlreadyInitialized {
                RiskManagementError::AlreadyInitialized
            } else {
                RiskManagementError::Unauthorized
            }
        })?;
        // initialize_governance(&env, admin).map_err(|_| RiskManagementError::Unauthorized)?;
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
    ) -> Result<i128, crate::deposit::DepositError> {
        deposit::deposit_collateral(&env, user, asset, amount)
    }

    /// Withdraw assets from the protocol
    pub fn withdraw_asset(
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
        user: Address,
        asset: Option<Address>,
        amount: i128,
    ) -> Result<i128, crate::withdraw::WithdrawError> {
        withdraw::withdraw_collateral(&env, user, asset, amount)
    }

    /// Borrow assets from the protocol
    pub fn borrow_asset(
        env: Env,
        user: Address,
        asset: Option<Address>,
        amount: i128,
    ) -> Result<i128, crate::borrow::BorrowError> {
        borrow::borrow_asset(&env, user, asset, amount)
    }

    /// Repay borrowed assets
    pub fn repay_debt(
        env: Env,
        user: Address,
        asset: Option<Address>,
        amount: i128,
    ) -> Result<(i128, i128, i128), crate::repay::RepayError> {
        repay::repay_debt(&env, user, asset, amount)
    }

    /// Liquidate an undercollateralized position
    pub fn liquidate(
        env: Env,
        liquidator: Address,
        borrower: Address,
        debt_asset: Option<Address>,
        collateral_asset: Option<Address>,
        debt_amount: i128,
    ) -> (i128, i128, i128) {
        liquidate::liquidate(&env, liquidator, borrower, debt_asset, collateral_asset, debt_amount)
            .expect("Liquidation error")
    }

    /// Update asset parameters (admin only)
    pub fn update_asset_params(
        env: Env,
        admin: Address,
        asset: Address,
        params: AssetParams,
    ) -> Result<(), RiskManagementError> {
        require_admin(&env, &admin)?;

        let asset_params_key = DepositDataKey::AssetParams(asset);
        env.storage().persistent().set(&asset_params_key, &params);
        Ok(())
    }

    /// Update pause switches (admin only)
    pub fn update_pause_switches(
        env: Env,
        admin: Address,
        switches: Map<Symbol, bool>,
    ) -> Result<(), RiskManagementError> {
        require_admin(&env, &admin)?;

        let pause_switches_key = DepositDataKey::PauseSwitches;
        env.storage().persistent().set(&pause_switches_key, &switches);
        Ok(())
    }

    /// Get current borrow rate (in basis points)
    pub fn get_borrow_rate(env: Env) -> i128 {
        interest_rate::calculate_borrow_rate(&env).unwrap_or(0)
    }

    /// Get current supply rate (in basis points)
    pub fn get_supply_rate(env: Env) -> i128 {
        interest_rate::calculate_supply_rate(&env).unwrap_or(0)
    }

    /// Update interest rate model configuration (admin only)
    #[allow(clippy::too_many_arguments)]
    pub fn update_interest_rate_config(
        env: Env,
        admin: Address,
        base_rate: Option<i128>,
        kink: Option<i128>,
        multiplier: Option<i128>,
        jump_multiplier: Option<i128>,
        rate_floor: Option<i128>,
        rate_ceiling: Option<i128>,
        spread: Option<i128>,
    ) -> Result<(), RiskManagementError> {
        require_admin(&env, &admin)?;

        interest_rate::update_interest_rate_config(
            &env,
            admin,
            base_rate,
            kink,
            multiplier,
            jump_multiplier,
            rate_floor,
            rate_ceiling,
            spread,
        )
        .map_err(|_| RiskManagementError::InvalidParameter)
    }

    /// Manual emergency interest rate adjustment (admin only)
    pub fn set_emergency_rate_adjustment(
        env: Env,
        admin: Address,
        adjustment_bps: i128,
    ) -> Result<(), RiskManagementError> {
        require_admin(&env, &admin)?;

        interest_rate::set_emergency_rate_adjustment(&env, admin, adjustment_bps)
            .map_err(|_| RiskManagementError::InvalidParameter)
    }

    /// Get protocol utilization (in basis points)
    pub fn get_utilization(env: Env) -> i128 {
        interest_rate::calculate_utilization(&env).unwrap_or(0)
    }

    /// Refresh analytics for a user
    pub fn refresh_user_analytics(_env: Env, _user: Address) -> Result<(), RiskManagementError> {
        Ok(())
    }

    /// Claim accumulated protocol reserves (admin only)
    pub fn claim_reserves(env: Env, caller: Address, asset: Option<Address>, to: Address, amount: i128) -> Result<(), RiskManagementError> {
        require_admin(&env, &caller)?;
        
        let reserve_key = DepositDataKey::ProtocolReserve(asset.clone());
        let mut reserve_balance = env.storage().persistent()
            .get::<DepositDataKey, i128>(&reserve_key)
            .unwrap_or(0);
            
        if amount > reserve_balance {
            return Err(RiskManagementError::InvalidParameter);
        }
        
        if let Some(_asset_addr) = asset {
            #[cfg(not(test))]
            {
                let token_client = soroban_sdk::token::Client::new(&env, &_asset_addr);
                token_client.transfer(&env.current_contract_address(), &to, &amount);
            }
        }
        
        reserve_balance -= amount;
        env.storage().persistent().set(&reserve_key, &reserve_balance);
        Ok(())
    }

    /// Get current protocol reserve balance for an asset
    pub fn get_reserve_balance(env: Env, asset: Option<Address>) -> i128 {
        let reserve_key = DepositDataKey::ProtocolReserve(asset);
        env.storage().persistent()
            .get::<DepositDataKey, i128>(&reserve_key)
            .unwrap_or(0)
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
    pub fn update_price_feed(
        env: Env,
        caller: Address,
        asset: Address,
        price: i128,
        decimals: u32,
        oracle: Address,
    ) -> i128 {
        oracle::update_price_feed(&env, caller, asset, price, decimals, oracle)
            .expect("Oracle error")
    }

    /// Get current price for an asset
    pub fn get_price(env: Env, asset: Address) -> i128 {
        oracle::get_price(&env, &asset).expect("Oracle error")
    }

    /// Configure oracle parameters (admin only)
    pub fn configure_oracle(
        env: Env,
        caller: Address,
        config: OracleConfig,
    ) {
        oracle::configure_oracle(&env, caller, config).expect("Oracle error")
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
    pub fn set_fallback_oracle(
        env: Env,
        caller: Address,
        asset: Address,
        fallback_oracle: Address,
    ) {
        oracle::set_fallback_oracle(&env, caller, asset, fallback_oracle).expect("Oracle error")
    }

    /// Get recent activity from analytics
    pub fn get_recent_activity(env: Env, limit: u32, offset: u32) -> Result<Vec<crate::analytics::ActivityEntry>, crate::analytics::AnalyticsError> {
        analytics::get_recent_activity(&env, limit, offset)
    }

    /// Initialize risk management (admin only)
    pub fn initialize_risk_management(env: Env, admin: Address) -> Result<(), RiskManagementError> {
        risk_management::initialize_risk_management(&env, admin)
    }

    /// Get current risk configuration
    pub fn get_risk_config(env: Env) -> Option<RiskConfig> {
        risk_management::get_risk_config(&env)
    }

    /// Set risk management parameters (admin only)
    pub fn set_risk_params(
        env: Env, 
        admin: Address, 
        min_collateral_ratio: Option<i128>,
        liquidation_threshold: Option<i128>,
        close_factor: Option<i128>,
        liquidation_incentive: Option<i128>,
    ) -> Result<(), RiskManagementError> {
        risk_management::set_risk_params(&env, admin, min_collateral_ratio, liquidation_threshold, close_factor, liquidation_incentive)
    }

    /// Set a pause switch for an operation (admin only)
    pub fn set_pause_switch(env: Env, admin: Address, operation: Symbol, paused: bool) -> Result<(), RiskManagementError> {
        risk_management::set_pause_switch(&env, admin, operation, paused)
    }

    /// Check if an operation is paused
    pub fn is_operation_paused(env: Env, operation: Symbol) -> bool {
        risk_management::is_operation_paused(&env, operation)
    }

    /// Check if emergency pause is active
    pub fn is_emergency_paused(env: Env) -> bool {
        risk_management::is_emergency_paused(&env)
    }

    /// Set emergency pause (admin only)
    pub fn set_emergency_pause(env: Env, admin: Address, paused: bool) -> Result<(), RiskManagementError> {
        risk_management::set_emergency_pause(&env, admin, paused)
    }

    /// Get user analytics metrics
    pub fn get_user_analytics(env: Env, user: Address) -> Result<crate::analytics::UserMetrics, crate::analytics::AnalyticsError> {
        analytics::get_user_activity_summary(&env, &user)
    }

    /// Get protocol analytics metrics
    pub fn get_protocol_analytics(env: Env) -> Result<crate::analytics::ProtocolMetrics, crate::analytics::AnalyticsError> {
        analytics::get_protocol_stats(&env)
    }
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
