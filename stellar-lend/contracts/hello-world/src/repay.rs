#![allow(unused)]
use soroban_sdk::{contracterror, Address, Env, IntoVal, Map, Symbol, Val, Vec};

use crate::deposit::{
    add_activity_log, emit_analytics_updated_event, emit_position_updated_event,
    emit_user_activity_tracked_event, update_protocol_analytics, update_user_analytics, Activity,
    DepositDataKey, Position, ProtocolAnalytics, UserAnalytics,
};
use crate::events::{log_repay, RepayEvent};

/// Errors that can occur during repay operations
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum RepayError {
    /// Repay amount must be greater than zero
    InvalidAmount = 1,
    /// Asset address is invalid
    InvalidAsset = 2,
    /// Insufficient balance to repay
    InsufficientBalance = 3,
    /// Repay operations are currently paused
    RepayPaused = 4,
    /// No debt to repay
    NoDebt = 5,
    /// Overflow occurred during calculation
    Overflow = 6,
    /// Reentrancy detected
    Reentrancy = 7,
}

/// Calculate interest accrued since last accrual time
fn calculate_accrued_interest(
    env: &Env,
    principal: i128,
    last_accrual_time: u64,
    current_time: u64,
) -> Result<i128, RepayError> {
    if principal == 0 {
        return Ok(0);
    }
    if current_time <= last_accrual_time {
        return Ok(0);
    }
    let rate_bps = crate::interest_rate::calculate_borrow_rate(env).map_err(|_| RepayError::Overflow)?;
    crate::interest_rate::calculate_accrued_interest(
        principal,
        last_accrual_time,
        current_time,
        rate_bps,
    ).map_err(|_| RepayError::Overflow)
}

/// Accrue interest on a position
fn accrue_interest(env: &Env, position: &mut Position) -> Result<(), RepayError> {
    let current_time = env.ledger().timestamp();
    if position.debt == 0 {
        position.borrow_interest = 0;
        position.last_accrual_time = current_time;
        return Ok(());
    }
    let new_interest = calculate_accrued_interest(env, position.debt, position.last_accrual_time, current_time)?;
    position.borrow_interest = position.borrow_interest.checked_add(new_interest).ok_or(RepayError::Overflow)?;
    position.last_accrual_time = current_time;
    Ok(())
}

/// Repay debt function
pub fn repay_debt(
    env: &Env,
    user: Address,
    asset: Option<Address>,
    amount: i128,
) -> Result<(i128, i128, i128), RepayError> {
    if amount <= 0 {
        return Err(RepayError::InvalidAmount);
    }

    let pause_switches_key = DepositDataKey::PauseSwitches;
    if let Some(pause_map) = env.storage().persistent().get::<DepositDataKey, Map<Symbol, bool>>(&pause_switches_key) {
        if let Some(paused) = pause_map.get(Symbol::new(env, "pause_repay")) {
            if paused { return Err(RepayError::RepayPaused); }
        }
    }

    let timestamp = env.ledger().timestamp();

    if let Some(ref asset_addr) = asset {
        if asset_addr == &env.current_contract_address() {
            return Err(RepayError::InvalidAsset);
        }
    }

    let reserve_factor = if let Some(asset_addr) = asset.as_ref() {
        let params_key = DepositDataKey::AssetParams(asset_addr.clone());
        if let Some(params) = env.storage().persistent().get::<DepositDataKey, crate::deposit::AssetParams>(&params_key) {
            1000 // Default 10%
        } else {
            1000
        }
    } else {
        1000
    };

    let position_key = DepositDataKey::Position(user.clone());
    let mut position = env.storage().persistent().get::<DepositDataKey, Position>(&position_key).ok_or(RepayError::NoDebt)?;

    if position.debt == 0 && position.borrow_interest == 0 {
        return Err(RepayError::NoDebt);
    }

    accrue_interest(env, &mut position)?;

    let total_debt = position.debt.checked_add(position.borrow_interest).ok_or(RepayError::Overflow)?;
    let repay_amount = if amount >= total_debt { total_debt } else { amount };

    if let Some(ref asset_addr) = asset {
        #[cfg(not(test))]
        {
            let token_client = soroban_sdk::token::Client::new(env, asset_addr);
            let user_balance = token_client.balance(&user);
            if user_balance < repay_amount {
                return Err(RepayError::InsufficientBalance);
            }
            token_client.transfer_from(&env.current_contract_address(), &user, &env.current_contract_address(), &repay_amount);
        }
    }

    let interest_paid = if repay_amount <= position.borrow_interest { repay_amount } else { position.borrow_interest };
    let principal_paid = repay_amount.checked_sub(interest_paid).ok_or(RepayError::Overflow)?;

    position.borrow_interest = position.borrow_interest.checked_sub(interest_paid).unwrap_or(0);
    position.debt = position.debt.checked_sub(principal_paid).unwrap_or(0);
    position.last_accrual_time = timestamp;

    env.storage().persistent().set(&position_key, &position);

    if interest_paid > 0 {
        let reserve_amount = interest_paid.checked_mul(reserve_factor).ok_or(RepayError::Overflow)?.checked_div(10000).ok_or(RepayError::Overflow)?;
        if reserve_amount > 0 {
            let reserve_key = DepositDataKey::ProtocolReserve(asset.clone());
            let current_reserve = env.storage().persistent().get::<DepositDataKey, i128>(&reserve_key).unwrap_or(0);
            env.storage().persistent().set(&reserve_key, &(current_reserve.checked_add(reserve_amount).ok_or(RepayError::Overflow)?));
        }
    }

    update_user_analytics_repay(env, &user, repay_amount, timestamp)?;
    update_protocol_analytics_repay(env, repay_amount)?;
    add_activity_log(env, &user, Symbol::new(env, "repay"), repay_amount, asset.clone(), timestamp).map_err(|e| RepayError::Overflow)?;
    log_repay(env, RepayEvent { user: user.clone(), asset: asset.clone(), amount: repay_amount, timestamp });
    emit_position_updated_event(env, &user, &position);
    emit_analytics_updated_event(env, &user, "repay", repay_amount, timestamp);
    emit_user_activity_tracked_event(env, &user, Symbol::new(env, "repay"), repay_amount, timestamp);

    let remaining_debt = position.debt.checked_add(position.borrow_interest).unwrap_or(0);
    Ok((remaining_debt, interest_paid, principal_paid))
}

fn update_user_analytics_repay(env: &Env, user: &Address, amount: i128, timestamp: u64) -> Result<(), RepayError> {
    let analytics_key = DepositDataKey::UserAnalytics(user.clone());
    let mut analytics = env.storage().persistent().get::<DepositDataKey, UserAnalytics>(&analytics_key)
        .unwrap_or_else(|| UserAnalytics {
            total_deposits: 0, total_borrows: 0, total_withdrawals: 0, total_repayments: 0,
            collateral_value: 0, debt_value: 0, collateralization_ratio: 0, activity_score: 0,
            transaction_count: 0, first_interaction: timestamp, last_activity: timestamp,
            risk_level: 0, loyalty_tier: 0,
        });

    analytics.total_repayments = analytics.total_repayments.checked_add(amount).ok_or(RepayError::Overflow)?;
    analytics.debt_value = analytics.debt_value.checked_sub(amount).unwrap_or(0);

    if analytics.debt_value > 0 && analytics.collateral_value > 0 {
        analytics.collateralization_ratio = analytics.collateral_value.checked_mul(10000)
            .and_then(|v| v.checked_div(analytics.debt_value)).unwrap_or(0);
    } else {
        analytics.collateralization_ratio = 0;
    }

    analytics.transaction_count = analytics.transaction_count.saturating_add(1);
    analytics.last_activity = timestamp;

    env.storage().persistent().set(&analytics_key, &analytics);
    Ok(())
}

fn update_protocol_analytics_repay(env: &Env, amount: i128) -> Result<(), RepayError> {
    let analytics_key = DepositDataKey::ProtocolAnalytics;
    let mut analytics = env.storage().persistent().get::<DepositDataKey, ProtocolAnalytics>(&analytics_key)
        .unwrap_or(ProtocolAnalytics { total_deposits: 0, total_borrows: 0, total_value_locked: 0 });
    env.storage().persistent().set(&analytics_key, &analytics);
    Ok(())
}
