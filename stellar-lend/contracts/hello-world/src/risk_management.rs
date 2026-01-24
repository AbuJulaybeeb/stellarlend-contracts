#![allow(unused)]
use soroban_sdk::{contracterror, contracttype, Address, Env, IntoVal, Map, Symbol, Val, Vec};

/// Errors that can occur during risk management operations
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum RiskManagementError {
    /// Unauthorized access - caller is not admin
    Unauthorized = 1,
    /// Invalid parameter value
    InvalidParameter = 2,
    /// Parameter change exceeds maximum allowed change
    ParameterChangeTooLarge = 3,
    /// Minimum collateral ratio not met
    InsufficientCollateralRatio = 4,
    /// Operation is paused
    OperationPaused = 5,
    /// Emergency pause is active
    EmergencyPaused = 6,
    /// Invalid collateral ratio (must be >= liquidation threshold)
    InvalidCollateralRatio = 7,
    /// Invalid liquidation threshold (must be <= collateral ratio)
    InvalidLiquidationThreshold = 8,
    /// Close factor out of valid range (0-100%)
    InvalidCloseFactor = 9,
    /// Liquidation incentive out of valid range (0-50%)
    InvalidLiquidationIncentive = 10,
    /// Overflow occurred during calculation
    Overflow = 11,
}

/// Storage keys for risk management data
#[contracttype]
#[derive(Clone)]
#[cfg_attr(test, derive(Debug, PartialEq))]
pub enum RiskDataKey {
    /// Risk configuration parameters
    RiskConfig,
    /// Admin address
    Admin,
    /// Emergency pause flag
    EmergencyPause,
    /// Parameter change timelock (for safety)
    ParameterChangeTimelock,
}

/// Risk configuration parameters
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct RiskConfig {
    /// Minimum collateral ratio (in basis points, e.g., 11000 = 110%)
    /// Users must maintain this ratio or face liquidation
    pub min_collateral_ratio: i128,
    /// Liquidation threshold (in basis points, e.g., 10500 = 105%)
    /// When collateral ratio falls below this, liquidation is allowed
    pub liquidation_threshold: i128,
    /// Close factor (in basis points, e.g., 5000 = 50%)
    /// Maximum percentage of debt that can be liquidated in a single transaction
    pub close_factor: i128,
    /// Liquidation incentive (in basis points, e.g., 1000 = 10%)
    /// Bonus given to liquidators
    pub liquidation_incentive: i128,
    /// Pause switches for different operations
    pub pause_switches: Map<Symbol, bool>,
    /// Last update timestamp
    pub last_update: u64,
}

/// Pause switch operation types
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum PauseOperation {
    /// Pause deposit operations
    Deposit,
    /// Pause withdraw operations
    Withdraw,
    /// Pause borrow operations
    Borrow,
    /// Pause repay operations
    Repay,
    /// Pause liquidation operations
    Liquidate,
    /// Pause all operations (emergency)
    All,
}

/// Constants for parameter validation
const BASIS_POINTS_SCALE: i128 = 10_000; // 100% = 10,000 basis points
const MIN_COLLATERAL_RATIO_MIN: i128 = 10_000; // 100% minimum
const MIN_COLLATERAL_RATIO_MAX: i128 = 50_000; // 500% maximum
const LIQUIDATION_THRESHOLD_MIN: i128 = 10_000; // 100% minimum
const LIQUIDATION_THRESHOLD_MAX: i128 = 50_000; // 500% maximum
const CLOSE_FACTOR_MIN: i128 = 0; // 0% minimum
const CLOSE_FACTOR_MAX: i128 = BASIS_POINTS_SCALE; // 100% maximum
const LIQUIDATION_INCENTIVE_MIN: i128 = 0; // 0% minimum
const LIQUIDATION_INCENTIVE_MAX: i128 = 5_000; // 50% maximum (safety limit)
const MAX_PARAMETER_CHANGE_BPS: i128 = 1_000; // 10% maximum change per update

/// Initialize risk management system
///
/// Sets up default risk parameters and admin address.
/// Should be called during contract initialization.
///
/// # Arguments
/// * `env` - The Soroban environment
/// * `admin` - The admin address
///
/// # Returns
/// Returns Ok(()) on success
///
/// # Errors
/// * `RiskManagementError::InvalidParameter` - If default parameters are invalid
pub fn initialize_risk_management(env: &Env, admin: Address) -> Result<(), RiskManagementError> {
    // Set admin
    let admin_key = RiskDataKey::Admin;
    env.storage().persistent().set(&admin_key, &admin);

    // Initialize default risk config
    let default_config = RiskConfig {
        min_collateral_ratio: 11_000,  // 110% default
        liquidation_threshold: 10_500, // 105% default
        close_factor: 5_000,           // 50% default
        liquidation_incentive: 1_000,  // 10% default
        pause_switches: create_default_pause_switches(env),
        last_update: env.ledger().timestamp(),
    };

    // Validate default config
    validate_risk_config(&default_config)?;

    let config_key = RiskDataKey::RiskConfig;
    env.storage().persistent().set(&config_key, &default_config);

    // Initialize emergency pause as false
    let emergency_key = RiskDataKey::EmergencyPause;
    env.storage().persistent().set(&emergency_key, &false);

    Ok(())
}

/// Create default pause switches map
fn create_default_pause_switches(env: &Env) -> Map<Symbol, bool> {
    let mut switches = Map::new(env);
    switches.set(Symbol::new(env, "pause_deposit"), false);
    switches.set(Symbol::new(env, "pause_withdraw"), false);
    switches.set(Symbol::new(env, "pause_borrow"), false);
    switches.set(Symbol::new(env, "pause_repay"), false);
    switches.set(Symbol::new(env, "pause_liquidate"), false);
    switches
}

/// Get the admin address
pub fn get_admin(env: &Env) -> Option<Address> {
    let admin_key = RiskDataKey::Admin;
    env.storage()
        .persistent()
        .get::<RiskDataKey, Address>(&admin_key)
}

/// Check if caller is admin
pub fn require_admin(env: &Env, caller: &Address) -> Result<(), RiskManagementError> {
    let admin = get_admin(env).ok_or(RiskManagementError::Unauthorized)?;
    if admin != *caller {
        return Err(RiskManagementError::Unauthorized);
    }
    Ok(())
}

/// Get current risk configuration
pub fn get_risk_config(env: &Env) -> Option<RiskConfig> {
    let config_key = RiskDataKey::RiskConfig;
    env.storage()
        .persistent()
        .get::<RiskDataKey, RiskConfig>(&config_key)
}

/// Set risk parameters (admin only)
///
/// Updates risk parameters with validation and change limits.
///
/// # Arguments
/// * `env` - The Soroban environment
/// * `caller` - The caller address (must be admin)
/// * `min_collateral_ratio` - New minimum collateral ratio (in basis points)
/// * `liquidation_threshold` - New liquidation threshold (in basis points)
/// * `close_factor` - New close factor (in basis points)
/// * `liquidation_incentive` - New liquidation incentive (in basis points)
///
/// # Returns
/// Returns Ok(()) on success
///
/// # Errors
/// * `RiskManagementError::Unauthorized` - If caller is not admin
/// * `RiskManagementError::InvalidParameter` - If parameters are invalid
/// * `RiskManagementError::ParameterChangeTooLarge` - If change exceeds maximum allowed
pub fn set_risk_params(
    env: &Env,
    caller: Address,
    min_collateral_ratio: Option<i128>,
    liquidation_threshold: Option<i128>,
    close_factor: Option<i128>,
    liquidation_incentive: Option<i128>,
) -> Result<(), RiskManagementError> {
    // Check admin
    require_admin(env, &caller)?;

    // Check emergency pause
    check_emergency_pause(env)?;

    // Get current config
    let mut config = get_risk_config(env).ok_or(RiskManagementError::InvalidParameter)?;

    // Update parameters if provided
    if let Some(mcr) = min_collateral_ratio {
        validate_parameter_change(config.min_collateral_ratio, mcr)?;
        config.min_collateral_ratio = mcr;
    }

    if let Some(lt) = liquidation_threshold {
        validate_parameter_change(config.liquidation_threshold, lt)?;
        config.liquidation_threshold = lt;
    }

    if let Some(cf) = close_factor {
        validate_parameter_change(config.close_factor, cf)?;
        config.close_factor = cf;
    }

    if let Some(li) = liquidation_incentive {
        validate_parameter_change(config.liquidation_incentive, li)?;
        config.liquidation_incentive = li;
    }

    // Validate the updated config
    validate_risk_config(&config)?;

    // Update timestamp
    config.last_update = env.ledger().timestamp();

    // Save config
    let config_key = RiskDataKey::RiskConfig;
    env.storage().persistent().set(&config_key, &config);

    // Emit event
    emit_risk_params_updated_event(env, &caller, &config);

    Ok(())
}

/// Validate risk configuration
fn validate_risk_config(config: &RiskConfig) -> Result<(), RiskManagementError> {
    // Validate min collateral ratio
    if config.min_collateral_ratio < MIN_COLLATERAL_RATIO_MIN
        || config.min_collateral_ratio > MIN_COLLATERAL_RATIO_MAX
    {
        return Err(RiskManagementError::InvalidParameter);
    }

    // Validate liquidation threshold
    if config.liquidation_threshold < LIQUIDATION_THRESHOLD_MIN
        || config.liquidation_threshold > LIQUIDATION_THRESHOLD_MAX
    {
        return Err(RiskManagementError::InvalidLiquidationThreshold);
    }

    // Validate that min collateral ratio >= liquidation threshold
    if config.min_collateral_ratio < config.liquidation_threshold {
        return Err(RiskManagementError::InvalidCollateralRatio);
    }

    // Validate close factor
    if config.close_factor < CLOSE_FACTOR_MIN || config.close_factor > CLOSE_FACTOR_MAX {
        return Err(RiskManagementError::InvalidCloseFactor);
    }

    // Validate liquidation incentive
    if config.liquidation_incentive < LIQUIDATION_INCENTIVE_MIN
        || config.liquidation_incentive > LIQUIDATION_INCENTIVE_MAX
    {
        return Err(RiskManagementError::InvalidLiquidationIncentive);
    }

    Ok(())
}

/// Validate parameter change doesn't exceed maximum allowed change
fn validate_parameter_change(old_value: i128, new_value: i128) -> Result<(), RiskManagementError> {
    let change = if new_value > old_value {
        new_value - old_value
    } else {
        old_value - new_value
    };

    // Calculate maximum allowed change (10% of old value)
    let max_change = (old_value * MAX_PARAMETER_CHANGE_BPS) / BASIS_POINTS_SCALE;

    if change > max_change {
        return Err(RiskManagementError::ParameterChangeTooLarge);
    }

    Ok(())
}

/// Set pause switches (admin only)
///
/// Updates pause switches for different operations.
///
/// # Arguments
/// * `env` - The Soroban environment
/// * `caller` - The caller address (must be admin)
/// * `operation` - The operation to pause/unpause (as Symbol)
/// * `paused` - Whether to pause (true) or unpause (false)
///
/// # Returns
/// Returns Ok(()) on success
///
/// # Errors
/// * `RiskManagementError::Unauthorized` - If caller is not admin
pub fn set_pause_switch(
    env: &Env,
    caller: Address,
    operation: Symbol,
    paused: bool,
) -> Result<(), RiskManagementError> {
    // Check admin
    require_admin(env, &caller)?;

    // Get current config
    let mut config = get_risk_config(env).ok_or(RiskManagementError::InvalidParameter)?;

    // Update pause switch
    config.pause_switches.set(operation.clone(), paused);

    // Update timestamp
    config.last_update = env.ledger().timestamp();

    // Save config
    let config_key = RiskDataKey::RiskConfig;
    env.storage().persistent().set(&config_key, &config);

    // Emit event
    emit_pause_switch_updated_event(env, &caller, &operation, paused);

    Ok(())
}

/// Set multiple pause switches at once (admin only)
///
/// # Arguments
/// * `env` - The Soroban environment
/// * `caller` - The caller address (must be admin)
/// * `switches` - Map of operation symbols to pause states
///
/// # Returns
/// Returns Ok(()) on success
pub fn set_pause_switches(
    env: &Env,
    caller: Address,
    switches: Map<Symbol, bool>,
) -> Result<(), RiskManagementError> {
    // Check admin
    require_admin(env, &caller)?;

    // Get current config
    let mut config = get_risk_config(env).ok_or(RiskManagementError::InvalidParameter)?;

    // Update all pause switches
    for (op, paused) in switches.iter() {
        config.pause_switches.set(op, paused);
    }

    // Update timestamp
    config.last_update = env.ledger().timestamp();

    // Save config
    let config_key = RiskDataKey::RiskConfig;
    env.storage().persistent().set(&config_key, &config);

    // Emit event
    emit_pause_switches_updated_event(env, &caller, &switches);

    Ok(())
}

/// Check if an operation is paused
pub fn is_operation_paused(env: &Env, operation: Symbol) -> bool {
    if let Some(config) = get_risk_config(env) {
        config.pause_switches.get(operation).unwrap_or(false)
    } else {
        false
    }
}

/// Require that an operation is not paused
pub fn require_operation_not_paused(
    env: &Env,
    operation: Symbol,
) -> Result<(), RiskManagementError> {
    if is_operation_paused(env, operation.clone()) {
        return Err(RiskManagementError::OperationPaused);
    }
    Ok(())
}

/// Check if operation is paused (public helper for other modules)
/// This is a convenience function that can be called from other modules
pub fn check_operation_paused(env: &Env, operation: Symbol) -> bool {
    // First check emergency pause
    if is_emergency_paused(env) {
        return true;
    }
    // Then check specific operation pause
    is_operation_paused(env, operation)
}

/// Set emergency pause (admin only)
///
/// Emergency pause stops all operations immediately.
///
/// # Arguments
/// * `env` - The Soroban environment
/// * `caller` - The caller address (must be admin)
/// * `paused` - Whether to enable (true) or disable (false) emergency pause
///
/// # Returns
/// Returns Ok(()) on success
pub fn set_emergency_pause(
    env: &Env,
    caller: Address,
    paused: bool,
) -> Result<(), RiskManagementError> {
    // Check admin
    require_admin(env, &caller)?;

    // Set emergency pause
    let emergency_key = RiskDataKey::EmergencyPause;
    env.storage().persistent().set(&emergency_key, &paused);

    // Emit event
    emit_emergency_pause_event(env, &caller, paused);

    Ok(())
}

/// Check if emergency pause is active
pub fn is_emergency_paused(env: &Env) -> bool {
    let emergency_key = RiskDataKey::EmergencyPause;
    env.storage()
        .persistent()
        .get::<RiskDataKey, bool>(&emergency_key)
        .unwrap_or(false)
}

/// Require that emergency pause is not active
pub fn check_emergency_pause(env: &Env) -> Result<(), RiskManagementError> {
    if is_emergency_paused(env) {
        return Err(RiskManagementError::EmergencyPaused);
    }
    Ok(())
}

/// Check if user meets minimum collateral ratio requirement
///
/// # Arguments
/// * `env` - The Soroban environment
/// * `collateral_value` - Total collateral value (in base units)
/// * `debt_value` - Total debt value (in base units)
///
/// # Returns
/// Returns Ok(()) if ratio is sufficient, Err otherwise
pub fn require_min_collateral_ratio(
    env: &Env,
    collateral_value: i128,
    debt_value: i128,
) -> Result<(), RiskManagementError> {
    let config = get_risk_config(env).ok_or(RiskManagementError::InvalidParameter)?;

    // If no debt, ratio is infinite (always valid)
    if debt_value == 0 {
        return Ok(());
    }

    // Calculate collateral ratio: (collateral / debt) * 10000 (basis points)
    let ratio = (collateral_value * BASIS_POINTS_SCALE)
        .checked_div(debt_value)
        .ok_or(RiskManagementError::Overflow)?;

    // Check if ratio meets minimum
    if ratio < config.min_collateral_ratio {
        return Err(RiskManagementError::InsufficientCollateralRatio);
    }

    Ok(())
}

/// Check if position can be liquidated
///
/// Returns true if collateral ratio is below liquidation threshold.
///
/// # Arguments
/// * `env` - The Soroban environment
/// * `collateral_value` - Total collateral value (in base units)
/// * `debt_value` - Total debt value (in base units)
///
/// # Returns
/// Returns true if position can be liquidated
pub fn can_be_liquidated(
    env: &Env,
    collateral_value: i128,
    debt_value: i128,
) -> Result<bool, RiskManagementError> {
    let config = get_risk_config(env).ok_or(RiskManagementError::InvalidParameter)?;

    // If no debt, cannot be liquidated
    if debt_value == 0 {
        return Ok(false);
    }

    // Calculate collateral ratio
    let ratio = (collateral_value * BASIS_POINTS_SCALE)
        .checked_div(debt_value)
        .ok_or(RiskManagementError::Overflow)?;

    // Can be liquidated if ratio < liquidation threshold
    Ok(ratio < config.liquidation_threshold)
}

/// Calculate maximum liquidatable amount
///
/// Uses close factor to determine maximum debt that can be liquidated.
///
/// # Arguments
/// * `env` - The Soroban environment
/// * `debt_value` - Total debt value (in base units)
///
/// # Returns
/// Maximum amount that can be liquidated
pub fn get_max_liquidatable_amount(
    env: &Env,
    debt_value: i128,
) -> Result<i128, RiskManagementError> {
    let config = get_risk_config(env).ok_or(RiskManagementError::InvalidParameter)?;

    // Calculate: debt * close_factor / BASIS_POINTS_SCALE
    let max_amount = (debt_value * config.close_factor)
        .checked_div(BASIS_POINTS_SCALE)
        .ok_or(RiskManagementError::Overflow)?;

    Ok(max_amount)
}

/// Calculate liquidation incentive amount
///
/// Returns the bonus amount for liquidators.
///
/// # Arguments
/// * `env` - The Soroban environment
/// * `liquidated_amount` - Amount being liquidated (in base units)
///
/// # Returns
/// Liquidation incentive amount
pub fn get_liquidation_incentive_amount(
    env: &Env,
    liquidated_amount: i128,
) -> Result<i128, RiskManagementError> {
    let config = get_risk_config(env).ok_or(RiskManagementError::InvalidParameter)?;

    // Calculate: amount * liquidation_incentive / BASIS_POINTS_SCALE
    let incentive = (liquidated_amount * config.liquidation_incentive)
        .checked_div(BASIS_POINTS_SCALE)
        .ok_or(RiskManagementError::Overflow)?;

    Ok(incentive)
}

/// Get minimum collateral ratio
pub fn get_min_collateral_ratio(env: &Env) -> Result<i128, RiskManagementError> {
    let config = get_risk_config(env).ok_or(RiskManagementError::InvalidParameter)?;
    Ok(config.min_collateral_ratio)
}

/// Get liquidation threshold
pub fn get_liquidation_threshold(env: &Env) -> Result<i128, RiskManagementError> {
    let config = get_risk_config(env).ok_or(RiskManagementError::InvalidParameter)?;
    Ok(config.liquidation_threshold)
}

/// Get close factor
pub fn get_close_factor(env: &Env) -> Result<i128, RiskManagementError> {
    let config = get_risk_config(env).ok_or(RiskManagementError::InvalidParameter)?;
    Ok(config.close_factor)
}

/// Get liquidation incentive
pub fn get_liquidation_incentive(env: &Env) -> Result<i128, RiskManagementError> {
    let config = get_risk_config(env).ok_or(RiskManagementError::InvalidParameter)?;
    Ok(config.liquidation_incentive)
}

/// Emit risk parameters updated event
fn emit_risk_params_updated_event(env: &Env, caller: &Address, config: &RiskConfig) {
    let topics = (Symbol::new(env, "risk_params_updated"), caller.clone());
    let mut data: Vec<Val> = Vec::new(env);
    data.push_back(Symbol::new(env, "caller").into_val(env));
    data.push_back(caller.clone().into_val(env));
    data.push_back(Symbol::new(env, "min_collateral_ratio").into_val(env));
    data.push_back(config.min_collateral_ratio.into_val(env));
    data.push_back(Symbol::new(env, "liquidation_threshold").into_val(env));
    data.push_back(config.liquidation_threshold.into_val(env));
    data.push_back(Symbol::new(env, "close_factor").into_val(env));
    data.push_back(config.close_factor.into_val(env));
    data.push_back(Symbol::new(env, "liquidation_incentive").into_val(env));
    data.push_back(config.liquidation_incentive.into_val(env));
    data.push_back(Symbol::new(env, "timestamp").into_val(env));
    data.push_back(config.last_update.into_val(env));

    env.events().publish(topics, data);
}

/// Emit pause switch updated event
fn emit_pause_switch_updated_event(env: &Env, caller: &Address, operation: &Symbol, paused: bool) {
    let topics = (Symbol::new(env, "pause_switch_updated"), caller.clone());
    let mut data: Vec<Val> = Vec::new(env);
    data.push_back(Symbol::new(env, "caller").into_val(env));
    data.push_back(caller.clone().into_val(env));
    data.push_back(Symbol::new(env, "operation").into_val(env));
    data.push_back(operation.clone().into_val(env));
    data.push_back(Symbol::new(env, "paused").into_val(env));
    data.push_back(paused.into_val(env));

    env.events().publish(topics, data);
}

/// Emit pause switches updated event
fn emit_pause_switches_updated_event(env: &Env, caller: &Address, switches: &Map<Symbol, bool>) {
    let topics = (Symbol::new(env, "pause_switches_updated"), caller.clone());
    let mut data: Vec<Val> = Vec::new(env);
    data.push_back(Symbol::new(env, "caller").into_val(env));
    data.push_back(caller.clone().into_val(env));
    data.push_back(Symbol::new(env, "switches").into_val(env));
    data.push_back(switches.clone().into_val(env));

    env.events().publish(topics, data);
}

/// Emit emergency pause event
fn emit_emergency_pause_event(env: &Env, caller: &Address, paused: bool) {
    let topics = (Symbol::new(env, "emergency_pause"), caller.clone());
    let mut data: Vec<Val> = Vec::new(env);
    data.push_back(Symbol::new(env, "caller").into_val(env));
    data.push_back(caller.clone().into_val(env));
    data.push_back(Symbol::new(env, "paused").into_val(env));
    data.push_back(paused.into_val(env));

    env.events().publish(topics, data);
}
