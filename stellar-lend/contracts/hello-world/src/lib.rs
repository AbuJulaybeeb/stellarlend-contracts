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
