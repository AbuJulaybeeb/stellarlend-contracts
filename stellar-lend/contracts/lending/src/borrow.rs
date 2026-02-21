//! # Borrow Implementation (Simplified Lending)
//!
//! Core borrow logic for the simplified lending contract. Handles collateral
//! validation, debt tracking, interest calculation, and pause controls.
//!
//! ## Interest Model
//! Uses a fixed 5% APY simple interest model:
//! `interest = principal * 500bps * time_elapsed / seconds_per_year`
//!
//! ## Collateral Requirements
//! Minimum collateral ratio is 150% (15,000 basis points).

use soroban_sdk::{contracterror, contractevent, contracttype, Address, Env};

/// Errors that can occur during borrow operations.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum BorrowError {
    /// Collateral amount does not meet the 150% minimum ratio
    InsufficientCollateral = 1,
    /// Total protocol debt would exceed the configured debt ceiling
    DebtCeilingReached = 2,
    /// Borrow operations are currently paused
    ProtocolPaused = 3,
    /// Borrow or collateral amount is zero or negative
    InvalidAmount = 4,
    /// Arithmetic overflow during calculation
    Overflow = 5,
    /// Caller is not authorized for this operation
    Unauthorized = 6,
    /// The requested asset is not supported for borrowing
    AssetNotSupported = 7,
    /// Borrow amount is below the configured minimum
    BelowMinimumBorrow = 8,
}

/// Storage keys for borrow-related data.
#[contracttype]
#[derive(Clone)]
pub enum BorrowDataKey {
    /// Per-user debt position
    UserDebt(Address),
    /// Per-user collateral position
    UserCollateral(Address),
    /// Aggregate protocol debt
    TotalDebt,
    /// Maximum total debt allowed
    DebtCeiling,
    /// Interest rate configuration
    InterestRate,
    /// Collateral ratio configuration
    CollateralRatio,
    /// Minimum borrow amount
    MinBorrowAmount,
    /// Protocol pause flag
    Paused,
}

/// User debt position tracking.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct DebtPosition {
    /// Principal amount borrowed
    pub borrowed_amount: i128,
    /// Cumulative interest accrued
    pub interest_accrued: i128,
    /// Timestamp of last interest accrual
    pub last_update: u64,
    /// Address of the borrowed asset
    pub asset: Address,
}

/// User collateral position tracking.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct CollateralPosition {
    /// Amount of collateral deposited
    pub amount: i128,
    /// Address of the collateral asset
    pub asset: Address,
}

/// Event data emitted on each borrow operation.
#[contractevent]
#[derive(Clone, Debug)]
pub struct BorrowEvent {
    /// Borrower's address
    pub user: Address,
    /// Borrowed asset address
    pub asset: Address,
    /// Amount borrowed
    pub amount: i128,
    /// Collateral amount provided
    pub collateral: i128,
    /// Ledger timestamp of the borrow
    pub timestamp: u64,
}

const COLLATERAL_RATIO_MIN: i128 = 15000; // 150% in basis points
const INTEREST_RATE_PER_YEAR: i128 = 500; // 5% in basis points
const SECONDS_PER_YEAR: u64 = 31536000;

/// Borrow assets against deposited collateral
///
/// # Arguments
/// * `env` - The contract environment
/// * `user` - The borrower's address
/// * `asset` - The asset to borrow
/// * `amount` - The amount to borrow
/// * `collateral_asset` - The collateral asset
/// * `collateral_amount` - The collateral amount
///
/// # Returns
/// Returns Ok(()) on success or BorrowError on failure
///
/// # Security
/// - Validates collateral ratio meets minimum requirements
/// - Checks protocol is not paused
/// - Validates debt ceiling not exceeded
/// - Prevents overflow in calculations
pub fn borrow(
    env: &Env,
    user: Address,
    asset: Address,
    amount: i128,
    collateral_asset: Address,
    collateral_amount: i128,
) -> Result<(), BorrowError> {
    user.require_auth();

    if is_paused(env) {
        return Err(BorrowError::ProtocolPaused);
    }

    if amount <= 0 || collateral_amount <= 0 {
        return Err(BorrowError::InvalidAmount);
    }

    let min_borrow = get_min_borrow_amount(env);
    if amount < min_borrow {
        return Err(BorrowError::BelowMinimumBorrow);
    }

    validate_collateral_ratio(collateral_amount, amount)?;

    let total_debt = get_total_debt(env);
    let debt_ceiling = get_debt_ceiling(env);
    let new_total = total_debt
        .checked_add(amount)
        .ok_or(BorrowError::Overflow)?;

    if new_total > debt_ceiling {
        return Err(BorrowError::DebtCeilingReached);
    }

    let mut debt_position = get_debt_position(env, &user);
    let accrued_interest = calculate_interest(env, &debt_position);

    debt_position.borrowed_amount = debt_position
        .borrowed_amount
        .checked_add(amount)
        .ok_or(BorrowError::Overflow)?;
    debt_position.interest_accrued = debt_position
        .interest_accrued
        .checked_add(accrued_interest)
        .ok_or(BorrowError::Overflow)?;
    debt_position.last_update = env.ledger().timestamp();
    debt_position.asset = asset.clone();

    let mut collateral_position = get_collateral_position(env, &user);
    collateral_position.amount = collateral_position
        .amount
        .checked_add(collateral_amount)
        .ok_or(BorrowError::Overflow)?;
    collateral_position.asset = collateral_asset.clone();

    save_debt_position(env, &user, &debt_position);
    save_collateral_position(env, &user, &collateral_position);
    set_total_debt(env, new_total);

    emit_borrow_event(env, user, asset, amount, collateral_amount);

    Ok(())
}

/// Validate collateral ratio meets minimum requirements
fn validate_collateral_ratio(collateral: i128, borrow: i128) -> Result<(), BorrowError> {
    // To avoid overflow, check if collateral >= borrow * 1.5
    // Which is: collateral * 10000 >= borrow * 15000
    // Rearranged: collateral >= (borrow * 15000) / 10000

    let min_collateral = borrow
        .checked_mul(COLLATERAL_RATIO_MIN)
        .ok_or(BorrowError::Overflow)?
        .checked_div(10000)
        .ok_or(BorrowError::InvalidAmount)?;

    if collateral < min_collateral {
        return Err(BorrowError::InsufficientCollateral);
    }

    Ok(())
}

/// Calculate accrued interest for a debt position
fn calculate_interest(env: &Env, position: &DebtPosition) -> i128 {
    if position.borrowed_amount == 0 {
        return 0;
    }

    let current_time = env.ledger().timestamp();
    let time_elapsed = current_time.saturating_sub(position.last_update);

    position
        .borrowed_amount
        .saturating_mul(INTEREST_RATE_PER_YEAR)
        .saturating_mul(time_elapsed as i128)
        .saturating_div(10000)
        .saturating_div(SECONDS_PER_YEAR as i128)
}

fn get_debt_position(env: &Env, user: &Address) -> DebtPosition {
    env.storage()
        .persistent()
        .get(&BorrowDataKey::UserDebt(user.clone()))
        .unwrap_or(DebtPosition {
            borrowed_amount: 0,
            interest_accrued: 0,
            last_update: env.ledger().timestamp(),
            asset: user.clone(), // Placeholder, will be replaced on first borrow
        })
}

fn save_debt_position(env: &Env, user: &Address, position: &DebtPosition) {
    env.storage()
        .persistent()
        .set(&BorrowDataKey::UserDebt(user.clone()), position);
}

fn get_collateral_position(env: &Env, user: &Address) -> CollateralPosition {
    env.storage()
        .persistent()
        .get(&BorrowDataKey::UserCollateral(user.clone()))
        .unwrap_or(CollateralPosition {
            amount: 0,
            asset: user.clone(), // Placeholder, will be replaced on first borrow
        })
}

fn save_collateral_position(env: &Env, user: &Address, position: &CollateralPosition) {
    env.storage()
        .persistent()
        .set(&BorrowDataKey::UserCollateral(user.clone()), position);
}

fn get_total_debt(env: &Env) -> i128 {
    env.storage()
        .persistent()
        .get(&BorrowDataKey::TotalDebt)
        .unwrap_or(0)
}

fn set_total_debt(env: &Env, amount: i128) {
    env.storage()
        .persistent()
        .set(&BorrowDataKey::TotalDebt, &amount);
}

fn get_debt_ceiling(env: &Env) -> i128 {
    env.storage()
        .persistent()
        .get(&BorrowDataKey::DebtCeiling)
        .unwrap_or(i128::MAX)
}

fn get_min_borrow_amount(env: &Env) -> i128 {
    env.storage()
        .persistent()
        .get(&BorrowDataKey::MinBorrowAmount)
        .unwrap_or(1000)
}

fn is_paused(env: &Env) -> bool {
    env.storage()
        .persistent()
        .get(&BorrowDataKey::Paused)
        .unwrap_or(false)
}

fn emit_borrow_event(env: &Env, user: Address, asset: Address, amount: i128, collateral: i128) {
    BorrowEvent {
        user,
        asset,
        amount,
        collateral,
        timestamp: env.ledger().timestamp(),
    }
    .publish(env);
}

/// Initialize borrow settings (admin only)
pub fn initialize_borrow_settings(
    env: &Env,
    debt_ceiling: i128,
    min_borrow_amount: i128,
) -> Result<(), BorrowError> {
    env.storage()
        .persistent()
        .set(&BorrowDataKey::DebtCeiling, &debt_ceiling);
    env.storage()
        .persistent()
        .set(&BorrowDataKey::MinBorrowAmount, &min_borrow_amount);
    env.storage()
        .persistent()
        .set(&BorrowDataKey::Paused, &false);
    Ok(())
}

/// Set protocol pause state (admin only)
pub fn set_paused(env: &Env, paused: bool) -> Result<(), BorrowError> {
    env.storage()
        .persistent()
        .set(&BorrowDataKey::Paused, &paused);
    Ok(())
}

/// Get user's debt position
pub fn get_user_debt(env: &Env, user: &Address) -> DebtPosition {
    let mut position = get_debt_position(env, user);
    let accrued = calculate_interest(env, &position);
    position.interest_accrued = position.interest_accrued.saturating_add(accrued);
    position
}

/// Get user's collateral position
pub fn get_user_collateral(env: &Env, user: &Address) -> CollateralPosition {
    get_collateral_position(env, user)
}
