//! # StellarLend Simplified Lending Contract
//!
//! A streamlined lending contract that provides basic borrow functionality
//! with collateral requirements, debt ceilings, and interest accrual.
//!
//! This contract is a simplified version of the main lending protocol,
//! suitable for single-asset lending scenarios with a fixed 5% APY
//! interest rate and 150% minimum collateral ratio.

#![no_std]
use soroban_sdk::{contract, contractimpl, Address, Env, Val, Vec};

mod borrow;
use borrow::{
    borrow, deposit, get_user_collateral, get_user_debt, initialize_borrow_settings, repay,
    set_paused, BorrowError, CollateralPosition, DebtPosition,
};

mod token_receiver;
use token_receiver::receive;

#[cfg(test)]
mod borrow_test;

#[cfg(test)]
mod token_receiver_test;

#[contract]
pub struct LendingContract;

#[contractimpl]
impl LendingContract {
    /// Borrow assets against deposited collateral
    ///
    /// Allows users to borrow assets by providing collateral. The collateral ratio
    /// must meet minimum requirements (150%). Interest accrues over time at 5% APY.
    ///
    /// # Arguments
    /// * `user` - The borrower's address (must authorize)
    /// * `asset` - The asset to borrow
    /// * `amount` - The amount to borrow
    /// * `collateral_asset` - The collateral asset
    /// * `collateral_amount` - The collateral amount
    ///
    /// # Returns
    /// Returns Ok(()) on success
    ///
    /// # Errors
    /// - `InsufficientCollateral` - Collateral ratio below 150%
    /// - `DebtCeilingReached` - Protocol debt ceiling exceeded
    /// - `ProtocolPaused` - Protocol is paused
    /// - `InvalidAmount` - Amount or collateral is zero or negative
    /// - `BelowMinimumBorrow` - Amount below minimum borrow threshold
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

    /// Initialize borrow settings (admin only)
    ///
    /// Sets up the protocol's debt ceiling and minimum borrow amount.
    ///
    /// # Arguments
    /// * `debt_ceiling` - Maximum total debt allowed in the protocol
    /// * `min_borrow_amount` - Minimum amount that can be borrowed
    pub fn initialize_borrow_settings(
        env: Env,
        debt_ceiling: i128,
        min_borrow_amount: i128,
    ) -> Result<(), BorrowError> {
        initialize_borrow_settings(&env, debt_ceiling, min_borrow_amount)
    }

    /// Set protocol pause state (admin only)
    ///
    /// Pauses or unpauses the borrow functionality.
    ///
    /// # Arguments
    /// * `paused` - True to pause, false to unpause
    pub fn set_paused(env: Env, paused: bool) -> Result<(), BorrowError> {
        set_paused(&env, paused)
    }

    /// Get user's debt position
    ///
    /// Returns the user's current debt including accrued interest.
    ///
    /// # Arguments
    /// * `user` - The user's address
    ///
    /// # Returns
    /// DebtPosition with borrowed amount, interest, and last update time
    pub fn get_user_debt(env: Env, user: Address) -> DebtPosition {
        get_user_debt(&env, &user)
    }

    /// Get user's collateral position
    ///
    /// Returns the user's current collateral.
    ///
    /// # Arguments
    /// * `user` - The user's address
    ///
    /// # Returns
    /// CollateralPosition with amount and asset
    pub fn get_user_collateral(env: Env, user: Address) -> CollateralPosition {
        get_user_collateral(&env, &user)
    }

    /// Deposit collateral
    pub fn deposit(env: Env, user: Address, asset: Address, amount: i128) -> Result<(), BorrowError> {
        user.require_auth();
        deposit(&env, user, asset, amount)
    }

    /// Repay debt
    pub fn repay(env: Env, user: Address, asset: Address, amount: i128) -> Result<(), BorrowError> {
        user.require_auth();
        repay(&env, user, asset, amount)
    }

    /// Token receiver hook
    pub fn receive(
        env: Env,
        token_asset: Address,
        from: Address,
        amount: i128,
        payload: Vec<Val>,
    ) -> Result<(), BorrowError> {
        receive(env, token_asset, from, amount, payload)
    }
}
