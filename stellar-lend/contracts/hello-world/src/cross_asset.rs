use soroban_sdk::{contracterror, contracttype, symbol_short, Address, Env, Map, Symbol, Vec};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetConfig {
    /// Asset contract address (None for native XLM)
    pub asset: Option<Address>,
    /// Collateral factor in basis points (e.g., 7500 = 75%)
    pub collateral_factor: i128,
    /// Borrow factor in basis points (e.g., 8000 = 80%)
    pub borrow_factor: i128,
    /// Reserve factor in basis points (e.g., 1000 = 10%)
    pub reserve_factor: i128,
    /// Maximum supply cap (0 = unlimited)
    pub max_supply: i128,
    /// Maximum borrow cap (0 = unlimited)
    pub max_borrow: i128,
    /// Whether asset is enabled for collateral
    pub can_collateralize: bool,
    /// Whether asset is enabled for borrowing
    pub can_borrow: bool,
    /// Asset price in base units (normalized to 7 decimals)
    pub price: i128,
    /// Last price update timestamp
    pub price_updated_at: u64,
}

/// User position across a single asset
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetPosition {
    /// Collateral balance in asset's native units
    pub collateral: i128,
    /// Debt principal in asset's native units
    pub debt_principal: i128,
    /// Accrued interest in asset's native units
    pub accrued_interest: i128,
    /// Last update timestamp
    pub last_updated: u64,
}

/// Unified user position summary across all assets
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserPositionSummary {
    /// Total collateral value in USD (7 decimals)
    pub total_collateral_value: i128,
    /// Total weighted collateral (considering collateral factors)
    pub weighted_collateral_value: i128,
    /// Total debt value in USD (7 decimals)
    pub total_debt_value: i128,
    /// Total weighted debt (considering borrow factors)
    pub weighted_debt_value: i128,
    /// Current health factor (scaled by 10000, e.g., 15000 = 1.5)
    pub health_factor: i128,
    /// Whether position can be liquidated
    pub is_liquidatable: bool,
    /// Maximum additional borrow capacity in USD
    pub borrow_capacity: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AssetKey {
    Native,
    Token(Address),
}

#[contracterror]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CrossAssetError {
    AssetNotConfigured = 1,
    AssetDisabled = 2,
    InsufficientCollateral = 3,
    ExceedsBorrowCapacity = 4,
    UnhealthyPosition = 5,
    SupplyCapExceeded = 6,
    BorrowCapExceeded = 7,
    InvalidPrice = 8,
    PriceStale = 9,
    NotAuthorized = 10,
}

// Storage keys - using Symbol for type-safe storage keys
const ASSET_CONFIGS: Symbol = symbol_short!("configs");
const USER_POSITIONS: Symbol = symbol_short!("positions");
const TOTAL_SUPPLIES: Symbol = symbol_short!("supplies");
const TOTAL_BORROWS: Symbol = symbol_short!("borrows");
const ASSET_LIST: Symbol = symbol_short!("assets");
const ADMIN: Symbol = symbol_short!("admin");

pub fn initialize(env: &Env, admin: Address) -> Result<(), CrossAssetError> {
    if env.storage().persistent().has(&ADMIN) {
        return Err(CrossAssetError::NotAuthorized);
    }

    admin.require_auth();

    env.storage().persistent().set(&ADMIN, &admin);

    Ok(())
}

fn require_admin(env: &Env) -> Result<(), CrossAssetError> {
    let admin: Address = env
        .storage()
        .persistent()
        .get(&ADMIN)
        .ok_or(CrossAssetError::NotAuthorized)?;

    admin.require_auth();

    Ok(())
}

/// Initialize asset configuration
///
/// # Arguments
/// * `env` - The contract environment
/// * `admin` - Admin address for authorization
/// * `asset` - Asset to configure (None for XLM)
/// * `config` - Asset configuration parameters
pub fn initialize_asset(
    env: &Env,
    asset: Option<Address>,
    config: AssetConfig,
) -> Result<(), CrossAssetError> {
    require_admin(env)?;

    require_valid_config(&config)?;

    let asset_key = AssetKey::from_option(asset.clone());
    let mut configs: Map<AssetKey, AssetConfig> = env
        .storage()
        .persistent()
        .get(&ASSET_CONFIGS)
        .unwrap_or(Map::new(env));

    configs.set(asset_key.clone(), config);
    env.storage().persistent().set(&ASSET_CONFIGS, &configs);

    let mut asset_list: Vec<AssetKey> = env
        .storage()
        .persistent()
        .get(&ASSET_LIST)
        .unwrap_or(Vec::new(env));

    if !asset_list.contains(&asset_key) {
        asset_list.push_back(asset_key);
        env.storage().persistent().set(&ASSET_LIST, &asset_list);
    }

    Ok(())
}

/// Update asset configuration parameters
///
/// # Arguments
/// * `env` - The contract environment
/// * `admin` - Admin address for authorization
/// * `asset` - Asset to update (None for XLM)
/// * `collateral_factor` - Optional new collateral factor
/// * `borrow_factor` - Optional new borrow factor
/// * `max_supply` - Optional new supply cap
/// * `max_borrow` - Optional new borrow cap
/// * `can_collateralize` - Optional collateral enablement
/// * `can_borrow` - Optional borrow enablement
#[allow(clippy::too_many_arguments)]
pub fn update_asset_config(
    env: &Env,
    asset: Option<Address>,
    collateral_factor: Option<i128>,
    borrow_factor: Option<i128>,
    max_supply: Option<i128>,
    max_borrow: Option<i128>,
    can_collateralize: Option<bool>,
    can_borrow: Option<bool>,
) -> Result<(), CrossAssetError> {
    require_admin(env)?;

    let asset_key = AssetKey::from_option(asset);
    let mut config = get_asset_config(env, &asset_key)?;

    if let Some(cf) = collateral_factor {
        require_valid_basis_points(cf)?;
        config.collateral_factor = cf;
    }

    if let Some(bf) = borrow_factor {
        require_valid_basis_points(bf)?;
        config.borrow_factor = bf;
    }

    if let Some(ms) = max_supply {
        config.max_supply = ms;
    }

    if let Some(mb) = max_borrow {
        config.max_borrow = mb;
    }

    if let Some(cc) = can_collateralize {
        config.can_collateralize = cc;
    }

    if let Some(cb) = can_borrow {
        config.can_borrow = cb;
    }

    // Update storage
    let mut configs: Map<AssetKey, AssetConfig> = env
        .storage()
        .persistent()
        .get(&ASSET_CONFIGS)
        .unwrap_or(Map::new(env));

    configs.set(asset_key, config);
    env.storage().persistent().set(&ASSET_CONFIGS, &configs);

    Ok(())
}

/// Update asset price (oracle integration point)
///
/// # Arguments
/// * `env` - The contract environment
/// * `admin` - Admin address for authorization
/// * `asset` - Asset to update price for (None for XLM)
/// * `price` - New price in base units (7 decimals)
pub fn update_asset_price(
    env: &Env,
    asset: Option<Address>,
    price: i128,
) -> Result<(), CrossAssetError> {
    require_admin(env)?;

    if price <= 0 {
        return Err(CrossAssetError::InvalidPrice);
    }

    let asset_key = AssetKey::from_option(asset);
    let mut config = get_asset_config(env, &asset_key)?;
    config.price = price;
    config.price_updated_at = env.ledger().timestamp();

    let mut configs: Map<AssetKey, AssetConfig> = env
        .storage()
        .persistent()
        .get(&ASSET_CONFIGS)
        .unwrap_or(Map::new(env));

    configs.set(asset_key, config);
    env.storage().persistent().set(&ASSET_CONFIGS, &configs);

    Ok(())
}

/// Get user's position for a specific asset
///
/// # Arguments
/// * `env` - The contract environment
/// * `user` - User address
/// * `asset` - Asset address (None for XLM)
///
/// # Returns
/// Asset position or default empty position
pub fn get_user_asset_position(env: &Env, user: &Address, asset: Option<Address>) -> AssetPosition {
    let key = UserAssetKey::new(user.clone(), asset);
    let positions: Map<UserAssetKey, AssetPosition> = env
        .storage()
        .persistent()
        .get(&USER_POSITIONS)
        .unwrap_or(Map::new(env));

    positions.get(key).unwrap_or(AssetPosition {
        collateral: 0,
        debt_principal: 0,
        accrued_interest: 0,
        last_updated: env.ledger().timestamp(),
    })
}

/// Update user's position for a specific asset
///
/// # Arguments
/// * `env` - The contract environment
/// * `user` - User address
/// * `asset` - Asset address (None for XLM)
/// * `position` - Updated position data
fn set_user_asset_position(
    env: &Env,
    user: &Address,
    asset: Option<Address>,
    position: AssetPosition,
) {
    let key = UserAssetKey::new(user.clone(), asset);
    let mut positions: Map<UserAssetKey, AssetPosition> = env
        .storage()
        .persistent()
        .get(&USER_POSITIONS)
        .unwrap_or(Map::new(env));

    positions.set(key, position);
    env.storage().persistent().set(&USER_POSITIONS, &positions);
}

/// Calculate unified position summary across all assets
///
/// # Arguments
/// * `env` - The contract environment
/// * `user` - User address
///
/// # Returns
/// Comprehensive position summary
pub fn get_user_position_summary(
    env: &Env,
    user: &Address,
) -> Result<UserPositionSummary, CrossAssetError> {
    let asset_list: Vec<AssetKey> = env
        .storage()
        .persistent()
        .get(&ASSET_LIST)
        .unwrap_or(Vec::new(env));

    let configs: Map<AssetKey, AssetConfig> = env
        .storage()
        .persistent()
        .get(&ASSET_CONFIGS)
        .unwrap_or(Map::new(env));

    let mut total_collateral_value: i128 = 0;
    let mut weighted_collateral_value: i128 = 0;
    let mut total_debt_value: i128 = 0;
    let mut weighted_debt_value: i128 = 0;

    for i in 0..asset_list.len() {
        let asset_key = asset_list.get(i).unwrap();

        if let Some(config) = configs.get(asset_key.clone()) {
            let asset_option = asset_key.to_option();
            let position = get_user_asset_position(env, user, asset_option);

            if position.collateral == 0 && position.debt_principal == 0 {
                continue;
            }

            let current_time = env.ledger().timestamp();
            if current_time > config.price_updated_at
                && current_time - config.price_updated_at > 3600
            {
                return Err(CrossAssetError::PriceStale);
            }

            let collateral_value = (position.collateral * config.price) / 10_000_000;
            total_collateral_value += collateral_value;

            if config.can_collateralize {
                weighted_collateral_value += (collateral_value * config.collateral_factor) / 10_000;
            }

            let total_debt = position.debt_principal + position.accrued_interest;
            let debt_value = (total_debt * config.price) / 10_000_000;
            total_debt_value += debt_value;

            if config.can_borrow {
                weighted_debt_value += (debt_value * config.borrow_factor) / 10_000;
            }
        }
    }

    // Calculate health factor (weighted_collateral / weighted_debt * 10000)
    // Health factor of 1.0 = 10000, below 1.0 can be liquidated
    let health_factor = if weighted_debt_value > 0 {
        (weighted_collateral_value * 10_000) / weighted_debt_value
    } else {
        i128::MAX // No debt = infinite health
    };

    // Position is liquidatable if health factor < 1.0 (10000)
    let is_liquidatable = health_factor < 10_000 && weighted_debt_value > 0;

    // Calculate remaining borrow capacity
    let borrow_capacity = if weighted_collateral_value > weighted_debt_value {
        weighted_collateral_value - weighted_debt_value
    } else {
        0
    };

    Ok(UserPositionSummary {
        total_collateral_value,
        weighted_collateral_value,
        total_debt_value,
        weighted_debt_value,
        health_factor,
        is_liquidatable,
        borrow_capacity,
    })
}

/// Cross-asset deposit operation
///
/// # Arguments
/// * `env` - The contract environment
/// * `user` - User depositing collateral
/// * `asset` - Asset to deposit (None for XLM)
/// * `amount` - Amount to deposit
///
/// # Returns
/// Updated asset position
pub fn cross_asset_deposit(
    env: &Env,
    user: Address,
    asset: Option<Address>,
    amount: i128,
) -> Result<AssetPosition, CrossAssetError> {
    user.require_auth();

    let asset_key = AssetKey::from_option(asset.clone());
    let config = get_asset_config(env, &asset_key)?;

    if !config.can_collateralize {
        return Err(CrossAssetError::AssetDisabled);
    }

    if config.max_supply > 0 {
        let total_supply = get_total_supply(env, &asset_key);
        if total_supply + amount > config.max_supply {
            return Err(CrossAssetError::SupplyCapExceeded);
        }
    }

    let mut position = get_user_asset_position(env, &user, asset.clone());

    position.collateral += amount;
    position.last_updated = env.ledger().timestamp();

    set_user_asset_position(env, &user, asset, position.clone());
    update_total_supply(env, &asset_key, amount);

    Ok(position)
}

/// Cross-asset withdraw operation
///
/// # Arguments
/// * `env` - The contract environment
/// * `user` - User withdrawing collateral
/// * `asset` - Asset to withdraw (None for XLM)
/// * `amount` - Amount to withdraw
///
/// # Returns
/// Updated asset position
pub fn cross_asset_withdraw(
    env: &Env,
    user: Address,
    asset: Option<Address>,
    amount: i128,
) -> Result<AssetPosition, CrossAssetError> {
    user.require_auth();

    let asset_key = AssetKey::from_option(asset.clone());

    let mut position = get_user_asset_position(env, &user, asset.clone());

    if position.collateral < amount {
        return Err(CrossAssetError::InsufficientCollateral);
    }

    position.collateral -= amount;
    position.last_updated = env.ledger().timestamp();

    set_user_asset_position(env, &user, asset.clone(), position.clone());

    let summary = get_user_position_summary(env, &user)?;

    if summary.total_debt_value > 0 && summary.health_factor < 10_000 {
        position.collateral += amount;
        set_user_asset_position(env, &user, asset, position);
        return Err(CrossAssetError::UnhealthyPosition);
    }

    update_total_supply(env, &asset_key, -amount);

    Ok(position)
}

/// Cross-asset borrow operation
///
/// # Arguments
/// * `env` - The contract environment
/// * `user` - User borrowing
/// * `asset` - Asset to borrow (None for XLM)
/// * `amount` - Amount to borrow
///
/// # Returns
/// Updated asset position
pub fn cross_asset_borrow(
    env: &Env,
    user: Address,
    asset: Option<Address>,
    amount: i128,
) -> Result<AssetPosition, CrossAssetError> {
    user.require_auth();

    let asset_key = AssetKey::from_option(asset.clone());
    let config = get_asset_config(env, &asset_key)?;

    if !config.can_borrow {
        return Err(CrossAssetError::AssetDisabled);
    }

    if config.max_borrow > 0 {
        let total_borrow = get_total_borrow(env, &asset_key);
        if total_borrow + amount > config.max_borrow {
            return Err(CrossAssetError::BorrowCapExceeded);
        }
    }

    let mut position = get_user_asset_position(env, &user, asset.clone());

    position.debt_principal += amount;
    position.last_updated = env.ledger().timestamp();

    set_user_asset_position(env, &user, asset.clone(), position.clone());

    let summary = get_user_position_summary(env, &user)?;

    if summary.health_factor < 10_000 {
        position.debt_principal -= amount;
        set_user_asset_position(env, &user, asset, position);
        return Err(CrossAssetError::ExceedsBorrowCapacity);
    }

    update_total_borrow(env, &asset_key, amount);

    Ok(position)
}

/// Cross-asset repay operation
///
/// # Arguments
/// * `env` - The contract environment
/// * `user` - User repaying debt
/// * `asset` - Asset to repay (None for XLM)
/// * `amount` - Amount to repay
///
/// # Returns
/// Updated asset position
pub fn cross_asset_repay(
    env: &Env,
    user: Address,
    asset: Option<Address>,
    amount: i128,
) -> Result<AssetPosition, CrossAssetError> {
    user.require_auth();

    let asset_key = AssetKey::from_option(asset.clone());

    // Get current position
    let mut position = get_user_asset_position(env, &user, asset.clone());

    let total_debt = position.debt_principal + position.accrued_interest;
    let repay_amount = amount.min(total_debt);

    // Pay interest first, then principal
    if repay_amount <= position.accrued_interest {
        position.accrued_interest -= repay_amount;
    } else {
        let remaining = repay_amount - position.accrued_interest;
        position.accrued_interest = 0;
        position.debt_principal -= remaining;
    }

    position.last_updated = env.ledger().timestamp();

    // Update storage
    set_user_asset_position(env, &user, asset, position.clone());
    update_total_borrow(env, &asset_key, -repay_amount);

    Ok(position)
}

/// Get list of all configured assets
pub fn get_asset_list(env: &Env) -> Vec<AssetKey> {
    env.storage()
        .persistent()
        .get(&ASSET_LIST)
        .unwrap_or(Vec::new(env))
}

/// Get configuration for a specific asset
pub fn get_asset_config_by_address(
    env: &Env,
    asset: Option<Address>,
) -> Result<AssetConfig, CrossAssetError> {
    let asset_key = AssetKey::from_option(asset);
    get_asset_config(env, &asset_key)
}

// Helper functions

fn get_asset_config(env: &Env, asset_key: &AssetKey) -> Result<AssetConfig, CrossAssetError> {
    let configs: Map<AssetKey, AssetConfig> = env
        .storage()
        .persistent()
        .get(&ASSET_CONFIGS)
        .unwrap_or(Map::new(env));

    configs
        .get(asset_key.clone())
        .ok_or(CrossAssetError::AssetNotConfigured)
}

fn require_valid_config(config: &AssetConfig) -> Result<(), CrossAssetError> {
    require_valid_basis_points(config.collateral_factor)?;
    require_valid_basis_points(config.borrow_factor)?;
    require_valid_basis_points(config.reserve_factor)?;

    if config.price <= 0 {
        return Err(CrossAssetError::InvalidPrice);
    }

    Ok(())
}

fn require_valid_basis_points(value: i128) -> Result<(), CrossAssetError> {
    if !(0..=10_000).contains(&value) {
        return Err(CrossAssetError::AssetNotConfigured);
    }
    Ok(())
}

fn get_total_supply(env: &Env, asset_key: &AssetKey) -> i128 {
    let supplies: Map<AssetKey, i128> = env
        .storage()
        .persistent()
        .get(&TOTAL_SUPPLIES)
        .unwrap_or(Map::new(env));

    supplies.get(asset_key.clone()).unwrap_or(0)
}

fn update_total_supply(env: &Env, asset_key: &AssetKey, delta: i128) {
    let mut supplies: Map<AssetKey, i128> = env
        .storage()
        .persistent()
        .get(&TOTAL_SUPPLIES)
        .unwrap_or(Map::new(env));

    let current = supplies.get(asset_key.clone()).unwrap_or(0);
    supplies.set(asset_key.clone(), current + delta);
    env.storage().persistent().set(&TOTAL_SUPPLIES, &supplies);
}

fn get_total_borrow(env: &Env, asset_key: &AssetKey) -> i128 {
    let borrows: Map<AssetKey, i128> = env
        .storage()
        .persistent()
        .get(&TOTAL_BORROWS)
        .unwrap_or(Map::new(env));

    borrows.get(asset_key.clone()).unwrap_or(0)
}

fn update_total_borrow(env: &Env, asset_key: &AssetKey, delta: i128) {
    let mut borrows: Map<AssetKey, i128> = env
        .storage()
        .persistent()
        .get(&TOTAL_BORROWS)
        .unwrap_or(Map::new(env));

    let current = borrows.get(asset_key.clone()).unwrap_or(0);
    borrows.set(asset_key.clone(), current + delta);
    env.storage().persistent().set(&TOTAL_BORROWS, &borrows);
}

/// Combined key for user-asset position lookups
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserAssetKey {
    pub user: Address,
    pub asset: AssetKey,
}

impl UserAssetKey {
    pub fn new(user: Address, asset: Option<Address>) -> Self {
        Self {
            user,
            asset: AssetKey::from_option(asset),
        }
    }
}

impl AssetKey {
    pub fn from_option(asset: Option<Address>) -> Self {
        match asset {
            Some(addr) => AssetKey::Token(addr),
            None => AssetKey::Native,
        }
    }

    pub fn to_option(&self) -> Option<Address> {
        match self {
            AssetKey::Native => None,
            AssetKey::Token(addr) => Some(addr.clone()),
        }
    }
}
