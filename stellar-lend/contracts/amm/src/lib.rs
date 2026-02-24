//! # StellarLend AMM Integration Contract
//!
//! Provides Automated Market Maker (AMM) integration for the lending protocol,
//! enabling token swaps, liquidity provision, and collateral optimization.
//!
//! ## Features
//! - Multi-protocol AMM support with pluggable protocol configs
//! - Slippage protection with configurable tolerances
//! - Auto-swap for collateral optimization during lending operations
//! - Callback validation with nonce-based replay protection
//! - Swap and liquidity operation history for analytics

#![no_std]
#![allow(clippy::too_many_arguments)]
use soroban_sdk::{contract, contractimpl, Address, Env, Map};

mod amm;
use amm::{
    add_amm_protocol, add_liquidity, auto_swap_for_collateral, execute_swap,
    initialize_amm_settings, remove_liquidity, update_amm_settings, validate_amm_callback,
    AmmCallbackData, AmmError, AmmProtocolConfig, AmmSettings, LiquidityParams, SwapParams,
};

#[contract]
pub struct AmmContract;

#[contractimpl]
impl AmmContract {
    /// Initialize AMM settings (admin only)
    ///
    /// Sets up AMM integration parameters including slippage tolerances and thresholds.
    ///
    /// # Arguments
    /// * `admin` - The admin address
    /// * `default_slippage` - Default slippage tolerance in basis points (e.g., 100 = 1%)
    /// * `max_slippage` - Maximum allowed slippage in basis points
    /// * `auto_swap_threshold` - Minimum amount for auto-swap operations
    ///
    /// # Returns
    /// Returns Ok(()) on success
    pub fn initialize_amm_settings(
        env: Env,
        admin: Address,
        default_slippage: i128,
        max_slippage: i128,
        auto_swap_threshold: i128,
    ) -> Result<(), AmmError> {
        initialize_amm_settings(
            &env,
            admin,
            default_slippage,
            max_slippage,
            auto_swap_threshold,
        )
    }

    /// Add AMM protocol (admin only)
    ///
    /// Registers a new AMM protocol for swap and liquidity operations.
    ///
    /// # Arguments
    /// * `admin` - The admin address
    /// * `protocol_config` - Configuration for the AMM protocol
    ///
    /// # Returns
    /// Returns Ok(()) on success
    pub fn add_amm_protocol(
        env: Env,
        admin: Address,
        protocol_config: AmmProtocolConfig,
    ) -> Result<(), AmmError> {
        add_amm_protocol(&env, admin, protocol_config)
    }

    /// Update AMM settings (admin only)
    ///
    /// Updates AMM operation settings.
    ///
    /// # Arguments
    /// * `admin` - The admin address
    /// * `settings` - New AMM settings
    ///
    /// # Returns
    /// Returns Ok(()) on success
    pub fn update_amm_settings(
        env: Env,
        admin: Address,
        settings: AmmSettings,
    ) -> Result<(), AmmError> {
        update_amm_settings(&env, admin, settings)
    }

    /// Execute swap through AMM
    ///
    /// Performs token swaps using configured AMM protocols with slippage protection.
    /// Can be used within lending operations for collateral optimization.
    ///
    /// # Arguments
    /// * `user` - The user performing the swap
    /// * `params` - Swap parameters including tokens, amounts, and slippage tolerance
    ///
    /// # Returns
    /// Returns the actual amount received from the swap
    ///
    /// # Events
    /// Emits the following events:
    /// - `swap_executed`: Swap transaction details
    /// - `amm_operation`: AMM operation tracking
    pub fn execute_swap(env: Env, user: Address, params: SwapParams) -> Result<i128, AmmError> {
        execute_swap(&env, user, params)
    }

    /// Add liquidity to AMM pool
    ///
    /// Adds liquidity to AMM pools for earning fees and supporting protocol operations.
    ///
    /// # Arguments
    /// * `user` - The user adding liquidity
    /// * `params` - Liquidity parameters including tokens and amounts
    ///
    /// # Returns
    /// Returns the amount of LP tokens received
    ///
    /// # Events
    /// Emits the following events:
    /// - `liquidity_added`: Liquidity addition details
    /// - `amm_operation`: AMM operation tracking
    pub fn add_liquidity(
        env: Env,
        user: Address,
        params: LiquidityParams,
    ) -> Result<i128, AmmError> {
        add_liquidity(&env, user, params)
    }

    /// Remove liquidity from AMM pool
    ///
    /// Removes liquidity from AMM pools and returns underlying tokens.
    ///
    /// # Arguments
    /// * `user` - The user removing liquidity
    /// * `protocol` - AMM protocol address
    /// * `token_a` - First token address (None for native XLM)
    /// * `token_b` - Second token address (None for native XLM)
    /// * `lp_tokens` - Amount of LP tokens to burn
    /// * `min_amount_a` - Minimum amount of token A to receive
    /// * `min_amount_b` - Minimum amount of token B to receive
    /// * `deadline` - Operation deadline timestamp
    ///
    /// # Returns
    /// Returns tuple (amount_a, amount_b) received
    ///
    /// # Events
    /// Emits the following events:
    /// - `liquidity_removed`: Liquidity removal details
    /// - `amm_operation`: AMM operation tracking
    #[allow(clippy::too_many_arguments)]
    pub fn remove_liquidity(
        env: Env,
        user: Address,
        protocol: Address,
        token_a: Option<Address>,
        token_b: Option<Address>,
        lp_tokens: i128,
        min_amount_a: i128,
        min_amount_b: i128,
        deadline: u64,
    ) -> Result<(i128, i128), AmmError> {
        remove_liquidity(
            &env,
            user,
            protocol,
            token_a,
            token_b,
            lp_tokens,
            min_amount_a,
            min_amount_b,
            deadline,
        )
    }

    /// Validate AMM callback
    ///
    /// Validates callbacks from AMM protocols to ensure they are legitimate
    /// and prevent replay attacks. This is called by AMM protocols during operations.
    ///
    /// # Arguments
    /// * `caller` - The AMM protocol making the callback
    /// * `callback_data` - The callback data to validate
    ///
    /// # Returns
    /// Returns Ok(()) if callback is valid
    ///
    /// # Events
    /// Emits callback_validated event
    pub fn validate_amm_callback(
        env: Env,
        caller: Address,
        callback_data: AmmCallbackData,
    ) -> Result<(), AmmError> {
        validate_amm_callback(&env, caller, callback_data)
    }

    /// Auto-swap for collateral optimization
    ///
    /// Automatically swaps assets to optimize collateral ratios during lending operations.
    /// This is typically called internally during borrow/liquidation operations.
    ///
    /// # Arguments
    /// * `user` - The user whose collateral to optimize
    /// * `target_token` - The token to swap to (None for native XLM)
    /// * `amount` - Amount to swap
    ///
    /// # Returns
    /// Returns the amount received from the swap
    ///
    /// # Events
    /// Emits swap_executed and amm_operation events
    pub fn auto_swap_for_collateral(
        env: Env,
        user: Address,
        target_token: Option<Address>,
        amount: i128,
    ) -> Result<i128, AmmError> {
        auto_swap_for_collateral(&env, user, target_token, amount)
    }

    /// Get AMM settings
    ///
    /// Returns the current AMM configuration settings.
    ///
    /// # Returns
    /// Returns the current AMM settings
    pub fn get_amm_settings(env: Env) -> Option<AmmSettings> {
        amm::get_amm_settings(&env).ok()
    }

    /// Get supported AMM protocols
    ///
    /// Returns a list of all registered AMM protocols.
    ///
    /// # Returns
    /// Returns a map of protocol addresses to their configurations
    pub fn get_amm_protocols(env: Env) -> Option<Map<Address, AmmProtocolConfig>> {
        amm::get_amm_protocols(&env).ok()
    }

    /// Get swap history
    ///
    /// Returns recent swap operations for analytics.
    ///
    /// # Arguments
    /// * `user` - Optional user address to filter by
    /// * `limit` - Maximum number of records to return
    ///
    /// # Returns
    /// Returns a vector of swap records
    pub fn get_swap_history(
        env: Env,
        user: Option<Address>,
        limit: u32,
    ) -> Option<soroban_sdk::Vec<amm::SwapRecord>> {
        amm::get_swap_history(&env, user, limit).ok()
    }

    /// Get liquidity history
    ///
    /// Returns recent liquidity operations for analytics.
    ///
    /// # Arguments
    /// * `user` - Optional user address to filter by
    /// * `limit` - Maximum number of records to return
    ///
    /// # Returns
    /// Returns a vector of liquidity records
    pub fn get_liquidity_history(
        env: Env,
        user: Option<Address>,
        limit: u32,
    ) -> Option<soroban_sdk::Vec<amm::LiquidityRecord>> {
        amm::get_liquidity_history(&env, user, limit).ok()
    }
}

// Liquidation integration tests require lending crate; enable with feature "liquidate_integration"
// when lending is available as a dependency.
#[cfg(all(test, feature = "liquidate_integration"))]
mod liquidate_test;
#[cfg(test)]
mod test;
