use soroban_sdk::{contracterror, contracttype, Address, Env, Symbol};

/// Errors that can occur during deposit operations
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum DepositError {
    InvalidAmount = 1,
    DepositPaused = 2,
    Overflow = 3,
    AssetNotSupported = 4,
    ExceedsDepositCap = 5,
}

/// Storage keys for deposit-related data
#[contracttype]
#[derive(Clone)]
pub enum DepositDataKey {
    UserCollateral(Address),
    TotalDeposits,
    DepositCap,
    MinDepositAmount,
    Paused,
}

/// User collateral position
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct CollateralPosition {
    pub amount: i128,
    pub asset: Address,
    pub last_deposit_time: u64,
}

/// Deposit event data
#[contracttype]
#[derive(Clone, Debug)]
pub struct DepositEvent {
    pub user: Address,
    pub asset: Address,
    pub amount: i128,
    pub new_balance: i128,
    pub timestamp: u64,
}

/// Deposit collateral into the protocol
///
/// # Arguments
/// * `env` - The contract environment
/// * `user` - The depositor's address
/// * `asset` - The collateral asset address
/// * `amount` - The amount to deposit
///
/// # Returns
/// Returns the updated collateral balance on success
pub fn deposit(
    env: &Env,
    user: Address,
    asset: Address,
    amount: i128,
) -> Result<i128, DepositError> {
    user.require_auth();

    if is_paused(env) {
        return Err(DepositError::DepositPaused);
    }

    if amount <= 0 {
        return Err(DepositError::InvalidAmount);
    }

    let min_deposit = get_min_deposit_amount(env);
    if amount < min_deposit {
        return Err(DepositError::InvalidAmount);
    }

    let total_deposits = get_total_deposits(env);
    let deposit_cap = get_deposit_cap(env);
    let new_total = total_deposits
        .checked_add(amount)
        .ok_or(DepositError::Overflow)?;

    if new_total > deposit_cap {
        return Err(DepositError::ExceedsDepositCap);
    }

    let mut position = get_collateral_position(env, &user, &asset);
    position.amount = position
        .amount
        .checked_add(amount)
        .ok_or(DepositError::Overflow)?;
    position.last_deposit_time = env.ledger().timestamp();
    position.asset = asset.clone();

    save_collateral_position(env, &user, &position);
    set_total_deposits(env, new_total);
    emit_deposit_event(env, user, asset, amount, position.amount);

    Ok(position.amount)
}

/// Initialize deposit settings
pub fn initialize_deposit_settings(
    env: &Env,
    deposit_cap: i128,
    min_deposit_amount: i128,
) -> Result<(), DepositError> {
    env.storage()
        .persistent()
        .set(&DepositDataKey::DepositCap, &deposit_cap);
    env.storage()
        .persistent()
        .set(&DepositDataKey::MinDepositAmount, &min_deposit_amount);
    env.storage()
        .persistent()
        .set(&DepositDataKey::Paused, &false);
    Ok(())
}

/// Set deposit pause state
pub fn set_paused(env: &Env, paused: bool) -> Result<(), DepositError> {
    env.storage()
        .persistent()
        .set(&DepositDataKey::Paused, &paused);
    Ok(())
}

/// Get user's collateral position
pub fn get_user_collateral(env: &Env, user: &Address, asset: &Address) -> CollateralPosition {
    get_collateral_position(env, user, asset)
}

fn get_collateral_position(env: &Env, user: &Address, asset: &Address) -> CollateralPosition {
    env.storage()
        .persistent()
        .get(&DepositDataKey::UserCollateral(user.clone()))
        .unwrap_or(CollateralPosition {
            amount: 0,
            asset: asset.clone(),
            last_deposit_time: env.ledger().timestamp(),
        })
}

fn save_collateral_position(env: &Env, user: &Address, position: &CollateralPosition) {
    env.storage()
        .persistent()
        .set(&DepositDataKey::UserCollateral(user.clone()), position);
}

fn get_total_deposits(env: &Env) -> i128 {
    env.storage()
        .persistent()
        .get(&DepositDataKey::TotalDeposits)
        .unwrap_or(0)
}

fn set_total_deposits(env: &Env, amount: i128) {
    env.storage()
        .persistent()
        .set(&DepositDataKey::TotalDeposits, &amount);
}

fn get_deposit_cap(env: &Env) -> i128 {
    env.storage()
        .persistent()
        .get(&DepositDataKey::DepositCap)
        .unwrap_or(i128::MAX)
}

fn get_min_deposit_amount(env: &Env) -> i128 {
    env.storage()
        .persistent()
        .get(&DepositDataKey::MinDepositAmount)
        .unwrap_or(0)
}

fn is_paused(env: &Env) -> bool {
    env.storage()
        .persistent()
        .get(&DepositDataKey::Paused)
        .unwrap_or(false)
}

fn emit_deposit_event(env: &Env, user: Address, asset: Address, amount: i128, new_balance: i128) {
    let event = DepositEvent {
        user,
        asset,
        amount,
        new_balance,
        timestamp: env.ledger().timestamp(),
    };
    env.events().publish((Symbol::new(env, "deposit"),), event);
}
