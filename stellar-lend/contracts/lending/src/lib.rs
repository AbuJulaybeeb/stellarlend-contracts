//! # StellarLend Simplified Lending Contract
//!
//! A streamlined lending contract that provides basic borrow functionality
//! with collateral requirements, debt ceilings, and interest accrual.
//!
//! This contract is a simplified version of the main lending protocol,
//! suitable for single-asset lending scenarios with a fixed 5% APY
//! interest rate and 150% minimum collateral ratio.

#![no_std]
#![allow(deprecated)]
use soroban_sdk::{contract, contractimpl, Address, Env};

mod borrow;
use borrow::{
    borrow, get_user_collateral, get_user_debt, initialize_borrow_settings, set_paused,
    BorrowError, CollateralPosition, DebtPosition,
};

mod deposit;
use deposit::{
    deposit, get_user_collateral as get_deposit_collateral, initialize_deposit_settings,
    set_paused as set_deposit_paused, CollateralPosition as DepositCollateralPosition,
    DepositError,
};

mod withdraw;
use withdraw::{initialize_withdraw_settings, set_withdraw_paused, WithdrawError};

#[cfg(test)]
mod borrow_test;

#[cfg(test)]
mod deposit_test;

#[cfg(test)]
mod withdraw_test;

#[contract]
pub struct LendingContract;

#[contractimpl]
impl LendingContract {
    pub fn borrow(
        env: Env,
        user: Address,
        asset: Address,
        amount: i128,
        collateral_asset: Address,
        collateral_amount: i128,
    ) -> Result<(), BorrowError> {
        borrow(
            &env,
            user,
            asset,
            amount,
            collateral_asset,
            collateral_amount,
        )
    }

    pub fn initialize_borrow_settings(
        env: Env,
        debt_ceiling: i128,
        min_borrow_amount: i128,
    ) -> Result<(), BorrowError> {
        initialize_borrow_settings(&env, debt_ceiling, min_borrow_amount)
    }

    pub fn set_paused(env: Env, paused: bool) -> Result<(), BorrowError> {
        set_paused(&env, paused)
    }

    pub fn get_user_debt(env: Env, user: Address) -> DebtPosition {
        get_user_debt(&env, &user)
    }

    pub fn get_user_collateral(env: Env, user: Address) -> CollateralPosition {
        get_user_collateral(&env, &user)
    }

    /// Deposit collateral into the protocol
    ///
    /// Allows users to deposit assets as collateral. Supports configured collateral
    /// assets (XLM, USDC, etc.). Validates amounts and emits events.
    ///
    /// # Arguments
    /// * `user` - The depositor's address (must authorize)
    /// * `asset` - The collateral asset address
    /// * `amount` - The amount to deposit
    ///
    /// # Returns
    /// Returns the updated collateral balance
    ///
    /// # Errors
    /// - `InvalidAmount` - Amount is zero, negative, or below minimum
    /// - `DepositPaused` - Deposit operations are paused
    /// - `ExceedsDepositCap` - Protocol deposit cap would be exceeded
    /// - `Overflow` - Arithmetic overflow occurred
    pub fn deposit(
        env: Env,
        user: Address,
        asset: Address,
        amount: i128,
    ) -> Result<i128, DepositError> {
        deposit(&env, user, asset, amount)
    }

    /// Initialize deposit settings (admin only)
    ///
    /// Sets up the protocol's deposit cap and minimum deposit amount.
    ///
    /// # Arguments
    /// * `deposit_cap` - Maximum total deposits allowed
    /// * `min_deposit_amount` - Minimum amount that can be deposited
    pub fn initialize_deposit_settings(
        env: Env,
        deposit_cap: i128,
        min_deposit_amount: i128,
    ) -> Result<(), DepositError> {
        initialize_deposit_settings(&env, deposit_cap, min_deposit_amount)
    }

    /// Set deposit pause state (admin only)
    ///
    /// Pauses or unpauses the deposit functionality.
    ///
    /// # Arguments
    /// * `paused` - True to pause, false to unpause
    pub fn set_deposit_paused(env: Env, paused: bool) -> Result<(), DepositError> {
        set_deposit_paused(&env, paused)
    }

    /// Get user's deposit collateral position
    ///
    /// Returns the user's current deposit collateral position.
    ///
    /// # Arguments
    /// * `user` - The user's address
    /// * `asset` - The asset address
    ///
    /// # Returns
    /// DepositCollateralPosition with amount, asset, and last deposit time
    pub fn get_user_collateral_deposit(
        env: Env,
        user: Address,
        asset: Address,
    ) -> DepositCollateralPosition {
        get_deposit_collateral(&env, &user, &asset)
    }

    /// Withdraw collateral from the protocol
    ///
    /// Allows users to withdraw deposited collateral. Validates amounts,
    /// checks pause state, ensures sufficient balance, and enforces
    /// minimum collateral ratio if user has outstanding debt.
    ///
    /// # Arguments
    /// * `user` - The withdrawer's address (must authorize)
    /// * `asset` - The collateral asset address
    /// * `amount` - The amount to withdraw
    ///
    /// # Returns
    /// Returns the remaining collateral balance
    ///
    /// # Errors
    /// - `InvalidAmount` - Amount is zero, negative, or below minimum
    /// - `WithdrawPaused` - Withdraw operations are paused
    /// - `InsufficientCollateral` - User balance too low
    /// - `InsufficientCollateralRatio` - Would violate 150% ratio
    /// - `Overflow` - Arithmetic overflow occurred
    pub fn withdraw(
        env: Env,
        user: Address,
        asset: Address,
        amount: i128,
    ) -> Result<i128, WithdrawError> {
        withdraw::withdraw(&env, user, asset, amount)
    }

    /// Initialize withdraw settings (admin only)
    ///
    /// Sets up the minimum withdraw amount and unpauses withdrawals.
    ///
    /// # Arguments
    /// * `min_withdraw_amount` - Minimum amount that can be withdrawn
    pub fn initialize_withdraw_settings(
        env: Env,
        min_withdraw_amount: i128,
    ) -> Result<(), WithdrawError> {
        initialize_withdraw_settings(&env, min_withdraw_amount)
    }

    /// Set withdraw pause state (admin only)
    ///
    /// Pauses or unpauses the withdraw functionality.
    ///
    /// # Arguments
    /// * `paused` - True to pause, false to unpause
    pub fn set_withdraw_paused(env: Env, paused: bool) -> Result<(), WithdrawError> {
        set_withdraw_paused(&env, paused)
    }
}
