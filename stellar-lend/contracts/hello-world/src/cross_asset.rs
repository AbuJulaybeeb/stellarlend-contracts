//! # Cross-Asset Lending Module
//!
//! Extends the lending protocol with multi-asset support, allowing users to
//! deposit collateral and borrow across different asset types simultaneously.
//!
//! ## Features
//! - Per-asset configuration: collateral factor, borrow factor, reserve factor, caps
//! - Oracle-based price feeds for cross-asset value calculation
//! - Unified position summary with health factor across all assets
//! - Supply and borrow cap enforcement per asset
//!
//! ## Health Factor
//! Computed as `weighted_collateral_value / weighted_debt_value * 10000`.
//! A health factor below 10,000 (1.0x) makes the position liquidatable.
//!
//! ## Invariants
//! - Withdrawals and borrows are rejected if they would lower health factor below 1.0.
//! - Prices must not be stale (> 1 hour old) for position calculations.

#![allow(dead_code)]
use soroban_sdk::{contracterror, contracttype, symbol_short, Address, Env, Map, Symbol, Vec};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetConfig {
    /// Asset contract address (None for native XLM)
    pub asset: Option<Address>,
    /// Collateral factor (LTV) in basis points (e.g., 7500 = 75%)
    /// Maximum percentage of collateral value that can be borrowed
    pub collateral_factor: i128,
    /// Liquidation threshold in basis points (e.g., 8000 = 80%)
    /// Health factor below this triggers liquidation
    pub liquidation_threshold: i128,
    /// Reserve factor in basis points (e.g., 1000 = 10%)
    pub reserve_factor: i128,
    /// Maximum supply cap (0 = unlimited)
    pub max_supply: i128,
    /// Maximum borrow cap / debt ceiling (0 = unlimited)
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

/// Errors that can occur during cross-asset lending operations.
#[contracterror]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CrossAssetError {
    /// The specified asset has no configuration registered
    AssetNotConfigured = 1,
    /// The asset is configured but disabled for the requested operation
    AssetDisabled = 2,
    /// Insufficient collateral for the requested withdrawal or borrow
    InsufficientCollateral = 3,
    /// Borrow would exceed the user's remaining borrow capacity
    ExceedsBorrowCapacity = 4,
    /// Operation would result in a health factor below 1.0
    UnhealthyPosition = 5,
    /// Deposit would exceed the asset's supply cap
    SupplyCapExceeded = 6,
    /// Borrow would exceed the asset's borrow cap
    BorrowCapExceeded = 7,
    /// Price is zero or negative
    InvalidPrice = 8,
    /// Asset price is older than the staleness threshold (1 hour)
    PriceStale = 9,
    /// Caller is not authorized (not admin)
    NotAuthorized = 10,
}

// Storage keys - using Symbol for type-safe storage keys
const ASSET_CONFIGS: Symbol = symbol_short!("configs");
const USER_POSITIONS: Symbol = symbol_short!("positions");
const TOTAL_SUPPLIES: Symbol = symbol_short!("supplies");
const TOTAL_BORROWS: Symbol = symbol_short!("borrows");
const ASSET_LIST: Symbol = symbol_short!("assets");
const ADMIN: Symbol = symbol_short!("admin");

/// Initialize the cross-asset lending module.
///
/// Sets the admin address. Can only be called once; subsequent calls return
/// `NotAuthorized`.
///
/// # Arguments
/// * `admin` - The admin address (must authorize the transaction)
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

/// Register a new asset with the cross-asset lending module.
///
/// Validates the configuration (factors in basis-point range, positive price)
/// and appends the asset to the global asset list if not already present.
///
/// # Arguments
/// * `env` - The contract environment
/// * `asset` - Asset to configure (`None` for native XLM)
/// * `config` - Full asset configuration (factors, caps, price)
///
/// # Errors
/// * `NotAuthorized` - Caller is not the admin
/// * `AssetNotConfigured` - A basis-point field is out of [0, 10000]
/// * `InvalidPrice` - Price is zero or negative
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

/// Selectively update an existing asset's configuration.
///
/// Only the provided `Some` fields are updated; `None` fields keep their
/// current values. Factor fields are validated to be in [0, 10000] bps.
///
/// # Arguments
/// * `env` - The contract environment
/// * `asset` - Asset to update (`None` for XLM)
/// * `collateral_factor` - Optional new collateral factor/LTV (basis points)
/// * `liquidation_threshold` - Optional new liquidation threshold (basis points)
/// * `max_supply` - Optional new supply cap
/// * `max_borrow` - Optional new borrow cap/debt ceiling
/// * `can_collateralize` - Optional flag to enable/disable as collateral
/// * `can_borrow` - Optional flag to enable/disable borrowing
///
/// # Errors
/// * `NotAuthorized` - Caller is not the admin
/// * `AssetNotConfigured` - Asset has not been initialized or factor out of range
#[allow(clippy::too_many_arguments)]
pub fn update_asset_config(
    env: &Env,
    asset: Option<Address>,
    collateral_factor: Option<i128>,
    liquidation_threshold: Option<i128>,
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

    if let Some(lt) = liquidation_threshold {
        require_valid_basis_points(lt)?;
        config.liquidation_threshold = lt;
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

/// Update the oracle price for an asset.
///
/// Records the new price and the current ledger timestamp for staleness checks.
///
/// # Arguments
/// * `env` - The contract environment
/// * `asset` - Asset to update price for (`None` for XLM)
/// * `price` - New price in base units (7 decimals, must be > 0)
///
/// # Errors
/// * `NotAuthorized` - Caller is not the admin
/// * `InvalidPrice` - Price is zero or negative
/// * `AssetNotConfigured` - Asset has not been initialized
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

/// Calculate a unified position summary across all registered assets.
///
/// Iterates over all configured assets, aggregates collateral and debt values
/// weighted by their respective factors, and computes the health factor.
/// Prices older than 1 hour are rejected.
///
/// # Arguments
/// * `env` - The contract environment
/// * `user` - User address
///
/// # Returns
/// [`UserPositionSummary`] with health factor, liquidation status, and borrow capacity.
///
/// # Errors
/// * `PriceStale` - Any asset with a non-zero position has a price older than 1 hour
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
                weighted_collateral_value +=
                    (collateral_value * config.liquidation_threshold) / 10_000;
            }

            let total_debt = position.debt_principal + position.accrued_interest;
            let debt_value = (total_debt * config.price) / 10_000_000;
            total_debt_value += debt_value;

            weighted_debt_value += debt_value;
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

/// Deposit collateral for a specific asset.
///
/// Requires user authorization. Validates the asset is enabled for collateral
/// and that the deposit does not exceed the supply cap.
///
/// # Arguments
/// * `env` - The contract environment
/// * `user` - User depositing collateral (must authorize)
/// * `asset` - Asset to deposit (`None` for XLM)
/// * `amount` - Amount to deposit
///
/// # Returns
/// Updated [`AssetPosition`] after the deposit.
///
/// # Errors
/// * `AssetNotConfigured` - Asset is not registered
/// * `AssetDisabled` - Asset is not enabled for collateral
/// * `SupplyCapExceeded` - Deposit would exceed the asset's supply cap
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

/// Withdraw collateral for a specific asset.
///
/// Requires user authorization. Checks that the user has sufficient collateral
/// and that the withdrawal does not bring the health factor below 1.0. If the
/// health check fails, the withdrawal is rolled back.
///
/// # Arguments
/// * `env` - The contract environment
/// * `user` - User withdrawing collateral (must authorize)
/// * `asset` - Asset to withdraw (`None` for XLM)
/// * `amount` - Amount to withdraw
///
/// # Returns
/// Updated [`AssetPosition`] after the withdrawal.
///
/// # Errors
/// * `InsufficientCollateral` - User's collateral balance is below `amount`
/// * `UnhealthyPosition` - Withdrawal would drop health factor below 1.0
/// * `PriceStale` - Stale price prevents health factor calculation
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

/// Borrow a specific asset against cross-asset collateral.
///
/// Requires user authorization. Validates the asset is enabled for borrowing,
/// checks the borrow cap, and verifies the post-borrow health factor stays
/// above 1.0. If the health check fails, the borrow is rolled back.
///
/// # Arguments
/// * `env` - The contract environment
/// * `user` - User borrowing (must authorize)
/// * `asset` - Asset to borrow (`None` for XLM)
/// * `amount` - Amount to borrow
///
/// # Returns
/// Updated [`AssetPosition`] after the borrow.
///
/// # Errors
/// * `AssetNotConfigured` - Asset is not registered
/// * `AssetDisabled` - Asset is not enabled for borrowing
/// * `BorrowCapExceeded` - Borrow would exceed the asset's borrow cap
/// * `ExceedsBorrowCapacity` - Health factor would drop below 1.0
/// * `PriceStale` - Stale price prevents health factor calculation
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

/// Repay debt for a specific asset.
///
/// Requires user authorization. Repayment is capped at the total outstanding
/// debt (principal + accrued interest). Interest is paid first, then principal.
///
/// # Arguments
/// * `env` - The contract environment
/// * `user` - User repaying debt (must authorize)
/// * `asset` - Asset to repay (`None` for XLM)
/// * `amount` - Amount to repay (capped at total debt)
///
/// # Returns
/// Updated [`AssetPosition`] after the repayment.
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

/// Return the list of all registered asset keys.
///
/// Returns an empty vector if no assets have been configured.
pub fn get_asset_list(env: &Env) -> Vec<AssetKey> {
    env.storage()
        .persistent()
        .get(&ASSET_LIST)
        .unwrap_or(Vec::new(env))
}

/// Look up the configuration for a specific asset by address.
///
/// # Arguments
/// * `env` - The contract environment
/// * `asset` - Asset address (`None` for native XLM)
///
/// # Returns
/// The [`AssetConfig`] for the requested asset.
///
/// # Errors
/// * `AssetNotConfigured` - No configuration exists for this asset
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
    require_valid_basis_points(config.liquidation_threshold)?;
    require_valid_basis_points(config.reserve_factor)?;

    if config.price <= 0 {
        return Err(CrossAssetError::InvalidPrice);
    }

    // Liquidation threshold must be >= collateral factor (LTV)
    if config.liquidation_threshold < config.collateral_factor {
        return Err(CrossAssetError::AssetNotConfigured);
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
    /// Convert an `Option<Address>` into an `AssetKey` (`None` → `Native`).
    pub fn from_option(asset: Option<Address>) -> Self {
        match asset {
            Some(addr) => AssetKey::Token(addr),
            None => AssetKey::Native,
        }
    }

    /// Convert back to `Option<Address>` (`Native` → `None`).
    pub fn to_option(&self) -> Option<Address> {
        match self {
            AssetKey::Native => None,
            AssetKey::Token(addr) => Some(addr.clone()),
        }
    }
}
