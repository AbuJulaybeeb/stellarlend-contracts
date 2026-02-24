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

use crate::pause::{self, PauseType};
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

/// Storage keys for protocol-wide data.
#[contracttype]
#[derive(Clone)]
#[allow(clippy::enum_variant_names)]
pub enum BorrowDataKey {
    /// Protocol admin address
    ProtocolAdmin,
    /// Per-user debt position
    BorrowUserDebt(Address),
    /// Per-user collateral position
    BorrowUserCollateral(Address),
    /// Aggregate protocol debt
    BorrowTotalDebt,
    /// Maximum total debt allowed
    BorrowDebtCeiling,
    /// Interest rate configuration
    BorrowInterestRate,
    /// Collateral ratio configuration
    BorrowCollateralRatio,
    /// Minimum borrow amount
    BorrowMinAmount,
    /// Oracle contract address for price feeds (optional)
    OracleAddress,
    /// Liquidation threshold in basis points (e.g. 8000 = 80%)
    LiquidationThresholdBps,
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
pub struct BorrowCollateral {
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
pub fn borrow(
    env: &Env,
    user: Address,
    asset: Address,
    amount: i128,
    collateral_asset: Address,
    collateral_amount: i128,
) -> Result<(), BorrowError> {
    user.require_auth();

    if pause::is_paused(env, PauseType::Borrow) {
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

fn validate_collateral_ratio(collateral: i128, borrow: i128) -> Result<(), BorrowError> {
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
        .get(&BorrowDataKey::BorrowUserDebt(user.clone()))
        .unwrap_or(DebtPosition {
            borrowed_amount: 0,
            interest_accrued: 0,
            last_update: env.ledger().timestamp(),
            asset: user.clone(),
        })
}

fn save_debt_position(env: &Env, user: &Address, position: &DebtPosition) {
    env.storage()
        .persistent()
        .set(&BorrowDataKey::BorrowUserDebt(user.clone()), position);
}

fn get_collateral_position(env: &Env, user: &Address) -> BorrowCollateral {
    env.storage()
        .persistent()
        .get(&BorrowDataKey::BorrowUserCollateral(user.clone()))
        .unwrap_or(BorrowCollateral {
            amount: 0,
            asset: user.clone(),
        })
}

fn save_collateral_position(env: &Env, user: &Address, position: &BorrowCollateral) {
    env.storage()
        .persistent()
        .set(&BorrowDataKey::BorrowUserCollateral(user.clone()), position);
}

fn get_total_debt(env: &Env) -> i128 {
    env.storage()
        .persistent()
        .get(&BorrowDataKey::BorrowTotalDebt)
        .unwrap_or(0)
}

fn set_total_debt(env: &Env, amount: i128) {
    env.storage()
        .persistent()
        .set(&BorrowDataKey::BorrowTotalDebt, &amount);
}

fn get_debt_ceiling(env: &Env) -> i128 {
    env.storage()
        .persistent()
        .get(&BorrowDataKey::BorrowDebtCeiling)
        .unwrap_or(i128::MAX)
}

fn get_min_borrow_amount(env: &Env) -> i128 {
    env.storage()
        .persistent()
        .get(&BorrowDataKey::BorrowMinAmount)
        .unwrap_or(1000)
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

pub fn initialize_borrow_settings(
    env: &Env,
    debt_ceiling: i128,
    min_borrow_amount: i128,
) -> Result<(), BorrowError> {
    // Note: ProtocolAdmin check should be performed by the caller (lib.rs)
    env.storage()
        .persistent()
        .set(&BorrowDataKey::BorrowDebtCeiling, &debt_ceiling);
    env.storage()
        .persistent()
        .set(&BorrowDataKey::BorrowMinAmount, &min_borrow_amount);
    Ok(())
}

pub fn get_user_debt(env: &Env, user: &Address) -> DebtPosition {
    let mut position = get_debt_position(env, user);
    let accrued = calculate_interest(env, &position);
    position.interest_accrued = position.interest_accrued.saturating_add(accrued);
    position
}

pub fn get_user_collateral(env: &Env, user: &Address) -> BorrowCollateral {
    get_collateral_position(env, user)
}

pub fn set_admin(env: &Env, admin: &Address) {
    env.storage()
        .persistent()
        .set(&BorrowDataKey::ProtocolAdmin, admin);
}

pub fn get_admin(env: &Env) -> Option<Address> {
    env.storage()
        .persistent()
        .get(&BorrowDataKey::ProtocolAdmin)
}

/// Returns the oracle address if configured. Used by views for collateral/debt valuation.
pub fn get_oracle(env: &Env) -> Option<Address> {
    env.storage()
        .persistent()
        .get(&BorrowDataKey::OracleAddress)
}

/// Returns liquidation threshold in basis points (e.g. 8000 = 80%). Default 8000 if not set.
pub fn get_liquidation_threshold_bps(env: &Env) -> i128 {
    env.storage()
        .persistent()
        .get(&BorrowDataKey::LiquidationThresholdBps)
        .unwrap_or(8000)
}

/// Set oracle address for price feeds (admin only). Caller must be admin and authorize.
pub fn set_oracle(env: &Env, admin: &Address, oracle: Address) -> Result<(), BorrowError> {
    let current = get_admin(env).ok_or(BorrowError::Unauthorized)?;
    if *admin != current {
        return Err(BorrowError::Unauthorized);
    }
    admin.require_auth();
    env.storage()
        .persistent()
        .set(&BorrowDataKey::OracleAddress, &oracle);
    Ok(())
}

/// Set liquidation threshold in basis points (admin only). E.g. 8000 = 80%.
pub fn set_liquidation_threshold_bps(
    env: &Env,
    admin: &Address,
    bps: i128,
) -> Result<(), BorrowError> {
    let current = get_admin(env).ok_or(BorrowError::Unauthorized)?;
    if *admin != current {
        return Err(BorrowError::Unauthorized);
    }
    admin.require_auth();
    if bps <= 0 || bps > 10000 {
        return Err(BorrowError::InvalidAmount);
    }
    env.storage()
        .persistent()
        .set(&BorrowDataKey::LiquidationThresholdBps, &bps);
    Ok(())
}
