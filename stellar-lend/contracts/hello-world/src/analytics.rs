#![allow(unused)]
use soroban_sdk::{contracterror, contracttype, Address, Env, Map, Symbol, Vec};

use crate::deposit::{
    DepositDataKey, Position, ProtocolAnalytics as DepositProtocolAnalytics,
    UserAnalytics as DepositUserAnalytics,
};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum AnalyticsError {
    NotInitialized = 1,
    InvalidParameter = 2,
    Overflow = 3,
    DataNotFound = 4,
}

#[contracttype]
#[derive(Clone)]
#[cfg_attr(test, derive(Debug, PartialEq))]
pub enum AnalyticsDataKey {
    ProtocolMetrics,
    UserMetrics(Address),
    ActivityLog,
    TotalUsers,
    TotalTransactions,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ProtocolMetrics {
    pub total_value_locked: i128,
    pub total_deposits: i128,
    pub total_borrows: i128,
    pub utilization_rate: i128,
    pub average_borrow_rate: i128,
    pub total_users: u64,
    pub total_transactions: u64,
    pub last_update: u64,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct UserMetrics {
    pub collateral: i128,
    pub debt: i128,
    pub health_factor: i128,
    pub total_deposits: i128,
    pub total_borrows: i128,
    pub total_withdrawals: i128,
    pub total_repayments: i128,
    pub activity_score: i128,
    pub risk_level: i128,
    pub transaction_count: u64,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ActivityEntry {
    pub user: Address,
    pub activity_type: Symbol,
    pub amount: i128,
    pub asset: Option<Address>,
    pub timestamp: u64,
    pub metadata: Map<Symbol, i128>,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ProtocolReport {
    pub metrics: ProtocolMetrics,
    pub timestamp: u64,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct UserReport {
    pub user: Address,
    pub metrics: UserMetrics,
    pub position: Position,
    pub recent_activities: Vec<ActivityEntry>,
    pub timestamp: u64,
}

const BASIS_POINTS: i128 = 10_000;
const MAX_ACTIVITY_LOG_SIZE: u32 = 10_000;

pub fn get_total_value_locked(env: &Env) -> Result<i128, AnalyticsError> {
    let protocol_analytics = env
        .storage()
        .persistent()
        .get::<DepositDataKey, DepositProtocolAnalytics>(&DepositDataKey::ProtocolAnalytics)
        .unwrap_or(DepositProtocolAnalytics {
            total_deposits: 0,
            total_borrows: 0,
            total_value_locked: 0,
        });

    Ok(protocol_analytics.total_value_locked)
}

pub fn get_protocol_utilization(env: &Env) -> Result<i128, AnalyticsError> {
    let protocol_analytics = env
        .storage()
        .persistent()
        .get::<DepositDataKey, DepositProtocolAnalytics>(&DepositDataKey::ProtocolAnalytics)
        .unwrap_or(DepositProtocolAnalytics {
            total_deposits: 0,
            total_borrows: 0,
            total_value_locked: 0,
        });

    if protocol_analytics.total_deposits == 0 {
        return Ok(0);
    }

    let utilization = (protocol_analytics.total_borrows * BASIS_POINTS)
        .checked_div(protocol_analytics.total_deposits)
        .ok_or(AnalyticsError::Overflow)?;

    Ok(utilization)
}

pub fn calculate_weighted_avg_interest_rate(env: &Env) -> Result<i128, AnalyticsError> {
    let protocol_analytics = env
        .storage()
        .persistent()
        .get::<DepositDataKey, DepositProtocolAnalytics>(&DepositDataKey::ProtocolAnalytics)
        .unwrap_or(DepositProtocolAnalytics {
            total_deposits: 0,
            total_borrows: 0,
            total_value_locked: 0,
        });

    if protocol_analytics.total_borrows == 0 {
        return Ok(0);
    }

    let utilization = get_protocol_utilization(env)?;
    let base_rate = 200;
    let rate = base_rate + (utilization * 10) / BASIS_POINTS;

    Ok(rate)
}

pub fn update_protocol_metrics(env: &Env) -> Result<ProtocolMetrics, AnalyticsError> {
    let tvl = get_total_value_locked(env)?;
    let utilization = get_protocol_utilization(env)?;
    let avg_rate = calculate_weighted_avg_interest_rate(env)?;

    let protocol_analytics = env
        .storage()
        .persistent()
        .get::<DepositDataKey, DepositProtocolAnalytics>(&DepositDataKey::ProtocolAnalytics)
        .unwrap_or(DepositProtocolAnalytics {
            total_deposits: 0,
            total_borrows: 0,
            total_value_locked: 0,
        });

    let total_users = env
        .storage()
        .persistent()
        .get::<AnalyticsDataKey, u64>(&AnalyticsDataKey::TotalUsers)
        .unwrap_or(0);

    let total_transactions = env
        .storage()
        .persistent()
        .get::<AnalyticsDataKey, u64>(&AnalyticsDataKey::TotalTransactions)
        .unwrap_or(0);

    let metrics = ProtocolMetrics {
        total_value_locked: tvl,
        total_deposits: protocol_analytics.total_deposits,
        total_borrows: protocol_analytics.total_borrows,
        utilization_rate: utilization,
        average_borrow_rate: avg_rate,
        total_users,
        total_transactions,
        last_update: env.ledger().timestamp(),
    };

    env.storage()
        .persistent()
        .set(&AnalyticsDataKey::ProtocolMetrics, &metrics);

    Ok(metrics)
}

pub fn get_protocol_stats(env: &Env) -> Result<ProtocolMetrics, AnalyticsError> {
    let cached_metrics = env
        .storage()
        .persistent()
        .get::<AnalyticsDataKey, ProtocolMetrics>(&AnalyticsDataKey::ProtocolMetrics);

    if let Some(metrics) = cached_metrics {
        Ok(metrics)
    } else {
        update_protocol_metrics(env)
    }
}

pub fn get_user_position_summary(env: &Env, user: &Address) -> Result<Position, AnalyticsError> {
    let position = env
        .storage()
        .persistent()
        .get::<DepositDataKey, Position>(&DepositDataKey::Position(user.clone()))
        .ok_or(AnalyticsError::DataNotFound)?;

    Ok(position)
}

pub fn calculate_health_factor(env: &Env, user: &Address) -> Result<i128, AnalyticsError> {
    let position = get_user_position_summary(env, user)?;

    if position.debt == 0 {
        return Ok(i128::MAX);
    }

    let health_factor = (position.collateral * BASIS_POINTS)
        .checked_div(position.debt)
        .ok_or(AnalyticsError::Overflow)?;

    Ok(health_factor)
}

pub fn calculate_user_risk_level(health_factor: i128) -> i128 {
    if health_factor >= 15_000 {
        1
    } else if health_factor >= 12_000 {
        2
    } else if health_factor >= 11_000 {
        3
    } else if health_factor >= 10_500 {
        4
    } else {
        5
    }
}

pub fn get_user_activity_summary(env: &Env, user: &Address) -> Result<UserMetrics, AnalyticsError> {
    let user_analytics = env
        .storage()
        .persistent()
        .get::<DepositDataKey, DepositUserAnalytics>(&DepositDataKey::UserAnalytics(user.clone()))
        .ok_or(AnalyticsError::DataNotFound)?;

    let position = get_user_position_summary(env, user).unwrap_or(Position {
        collateral: 0,
        debt: 0,
        borrow_interest: 0,
        last_accrual_time: 0,
    });

    let health_factor = calculate_health_factor(env, user).unwrap_or(i128::MAX);
    let risk_level = calculate_user_risk_level(health_factor);

    let activity_score = (user_analytics.transaction_count as i128)
        .saturating_mul(100)
        .saturating_add(user_analytics.total_deposits / 1000);

    let metrics = UserMetrics {
        collateral: position.collateral,
        debt: position.debt,
        health_factor,
        total_deposits: user_analytics.total_deposits,
        total_borrows: user_analytics.total_borrows,
        total_withdrawals: user_analytics.total_withdrawals,
        total_repayments: user_analytics.total_repayments,
        activity_score,
        risk_level,
        transaction_count: user_analytics.transaction_count,
    };

    Ok(metrics)
}

pub fn update_user_metrics(env: &Env, user: &Address) -> Result<UserMetrics, AnalyticsError> {
    let metrics = get_user_activity_summary(env, user)?;

    env.storage()
        .persistent()
        .set(&AnalyticsDataKey::UserMetrics(user.clone()), &metrics);

    Ok(metrics)
}

pub fn record_activity(
    env: &Env,
    user: &Address,
    activity_type: Symbol,
    amount: i128,
    asset: Option<Address>,
) -> Result<(), AnalyticsError> {
    let mut activity_log = env
        .storage()
        .persistent()
        .get::<AnalyticsDataKey, Vec<ActivityEntry>>(&AnalyticsDataKey::ActivityLog)
        .unwrap_or_else(|| Vec::new(env));

    let entry = ActivityEntry {
        user: user.clone(),
        activity_type,
        amount,
        asset,
        timestamp: env.ledger().timestamp(),
        metadata: Map::new(env),
    };

    activity_log.push_back(entry);

    if activity_log.len() > MAX_ACTIVITY_LOG_SIZE {
        activity_log.pop_front();
    }

    env.storage()
        .persistent()
        .set(&AnalyticsDataKey::ActivityLog, &activity_log);

    let total_transactions = env
        .storage()
        .persistent()
        .get::<AnalyticsDataKey, u64>(&AnalyticsDataKey::TotalTransactions)
        .unwrap_or(0);

    env.storage().persistent().set(
        &AnalyticsDataKey::TotalTransactions,
        &(total_transactions + 1),
    );

    Ok(())
}

pub fn get_recent_activity(
    env: &Env,
    limit: u32,
    offset: u32,
) -> Result<Vec<ActivityEntry>, AnalyticsError> {
    let activity_log = env
        .storage()
        .persistent()
        .get::<AnalyticsDataKey, Vec<ActivityEntry>>(&AnalyticsDataKey::ActivityLog)
        .unwrap_or_else(|| Vec::new(env));

    let total_len = activity_log.len();
    if offset >= total_len {
        return Ok(Vec::new(env));
    }

    let mut result = Vec::new(env);
    let start = total_len.saturating_sub(offset + limit);
    let end = total_len.saturating_sub(offset);

    for i in (start..end).rev() {
        if let Some(entry) = activity_log.get(i) {
            result.push_back(entry);
        }
    }

    Ok(result)
}

pub fn get_user_activity_feed(
    env: &Env,
    user: &Address,
    limit: u32,
    offset: u32,
) -> Result<Vec<ActivityEntry>, AnalyticsError> {
    let activity_log = env
        .storage()
        .persistent()
        .get::<AnalyticsDataKey, Vec<ActivityEntry>>(&AnalyticsDataKey::ActivityLog)
        .unwrap_or_else(|| Vec::new(env));

    let mut user_activities = Vec::new(env);

    for i in (0..activity_log.len()).rev() {
        if let Some(entry) = activity_log.get(i) {
            if entry.user == *user {
                user_activities.push_back(entry);
            }
        }
    }

    let total_len = user_activities.len();
    if offset >= total_len {
        return Ok(Vec::new(env));
    }

    let mut result = Vec::new(env);
    let end = total_len.saturating_sub(offset);
    let start = end.saturating_sub(limit);

    for i in start..end {
        if let Some(entry) = user_activities.get(i) {
            result.push_back(entry);
        }
    }

    Ok(result)
}

pub fn get_activity_by_type(
    env: &Env,
    activity_type: Symbol,
    limit: u32,
) -> Result<Vec<ActivityEntry>, AnalyticsError> {
    let activity_log = env
        .storage()
        .persistent()
        .get::<AnalyticsDataKey, Vec<ActivityEntry>>(&AnalyticsDataKey::ActivityLog)
        .unwrap_or_else(|| Vec::new(env));

    let mut filtered = Vec::new(env);
    let mut count = 0u32;

    for i in (0..activity_log.len()).rev() {
        if count >= limit {
            break;
        }

        if let Some(entry) = activity_log.get(i) {
            if entry.activity_type == activity_type {
                filtered.push_back(entry);
                count += 1;
            }
        }
    }

    Ok(filtered)
}

pub fn generate_protocol_report(env: &Env) -> Result<ProtocolReport, AnalyticsError> {
    let metrics = update_protocol_metrics(env)?;

    let report = ProtocolReport {
        metrics,
        timestamp: env.ledger().timestamp(),
    };

    Ok(report)
}

pub fn generate_user_report(env: &Env, user: &Address) -> Result<UserReport, AnalyticsError> {
    let metrics = get_user_activity_summary(env, user)?;
    let position = get_user_position_summary(env, user)?;
    let recent_activities = get_user_activity_feed(env, user, 10, 0)?;

    let report = UserReport {
        user: user.clone(),
        metrics,
        position,
        recent_activities,
        timestamp: env.ledger().timestamp(),
    };

    Ok(report)
}
