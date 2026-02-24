//! # Admin and Access Control Module
//!
//! Provides a centralized mechanism to manage the protocol's super-admin and role-based
//! access control for privileged operations.
//!
//! ## Features
//! - **Super Admin**: A single address with ultimate authority over the protocol.
//! - **Roles**: Optional multi-admin functionality via specific roles (e.g., "oracle_admin").
//! - **Events**: Emits events for critical admin actions (admin changes, role grants/revocations).

use soroban_sdk::{contracterror, contracttype, Address, Env, IntoVal, Symbol, Val, Vec};

/// Errors that can occur during admin operations
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum AdminError {
    /// Unauthorized access - caller is not admin or lacks required role
    Unauthorized = 1,
    /// Invalid parameter value
    InvalidParameter = 2,
    /// Admin has already been set
    AdminAlreadySet = 3,
}

/// Storage keys for Admin and Roles
#[contracttype]
#[derive(Clone)]
#[cfg_attr(test, derive(Debug, PartialEq))]
pub enum AdminDataKey {
    /// The super admin address
    Admin,
    /// Specific role assigned to an address: Role(RoleName, Address) -> bool
    Role(Symbol, Address),
}

/// Check if the super admin is set
pub fn has_admin(env: &Env) -> bool {
    env.storage().persistent().has(&AdminDataKey::Admin)
}

/// Get the super admin address
pub fn get_admin(env: &Env) -> Option<Address> {
    env.storage().persistent().get(&AdminDataKey::Admin)
}

/// Initialize the super admin. Can only be called once or by the existing admin.
///
/// # Arguments
/// * `env` - The Soroban environment
/// * `new_admin` - The new admin address
/// * `caller` - The caller address (must be the current admin if one exists)
pub fn set_admin(env: &Env, new_admin: Address, caller: Option<Address>) -> Result<(), AdminError> {
    if let Some(current_admin) = get_admin(env) {
        if let Some(ref c) = caller {
            if *c != current_admin {
                return Err(AdminError::Unauthorized);
            }
        } else {
            return Err(AdminError::Unauthorized);
        }
    }

    env.storage()
        .persistent()
        .set(&AdminDataKey::Admin, &new_admin);

    // Emit event
    let topics = (Symbol::new(env, "admin_changed"),);
    let mut data: Vec<Val> = Vec::new(env);
    data.push_back(Symbol::new(env, "new_admin").into_val(env));
    data.push_back(new_admin.into_val(env));
    if let Some(c) = caller {
        data.push_back(Symbol::new(env, "caller").into_val(env));
        data.push_back(c.into_val(env));
    }

    env.events().publish(topics, data);

    Ok(())
}

/// Require that the caller is the super admin
pub fn require_admin(env: &Env, caller: &Address) -> Result<(), AdminError> {
    let admin = get_admin(env).ok_or(AdminError::Unauthorized)?;
    if admin != *caller {
        return Err(AdminError::Unauthorized);
    }
    Ok(())
}

/// Grant a specific role to an address (admin only)
pub fn grant_role(
    env: &Env,
    caller: Address,
    role: Symbol,
    account: Address,
) -> Result<(), AdminError> {
    require_admin(env, &caller)?;

    let key = AdminDataKey::Role(role.clone(), account.clone());
    env.storage().persistent().set(&key, &true);

    // Emit event
    let topics = (
        Symbol::new(env, "role_granted"),
        caller.clone(),
        role.clone(),
    );
    let mut data: Vec<Val> = Vec::new(env);
    data.push_back(Symbol::new(env, "account").into_val(env));
    data.push_back(account.into_val(env));

    env.events().publish(topics, data);

    Ok(())
}

/// Revoke a specific role from an address (admin only)
pub fn revoke_role(
    env: &Env,
    caller: Address,
    role: Symbol,
    account: Address,
) -> Result<(), AdminError> {
    require_admin(env, &caller)?;

    let key = AdminDataKey::Role(role.clone(), account.clone());
    env.storage().persistent().remove(&key);

    // Emit event
    let topics = (
        Symbol::new(env, "role_revoked"),
        caller.clone(),
        role.clone(),
    );
    let mut data: Vec<Val> = Vec::new(env);
    data.push_back(Symbol::new(env, "account").into_val(env));
    data.push_back(account.into_val(env));

    env.events().publish(topics, data);

    Ok(())
}

/// Check if an address has a specific role
#[allow(dead_code)]
pub fn has_role(env: &Env, role: Symbol, account: Address) -> bool {
    let key = AdminDataKey::Role(role, account);
    env.storage().persistent().get(&key).unwrap_or(false)
}

/// Require that the caller is either the super admin or has the required role
#[allow(dead_code)]
pub fn require_role_or_admin(
    env: &Env,
    caller: &Address,
    required_role: Symbol,
) -> Result<(), AdminError> {
    if get_admin(env).map(|a| a == *caller).unwrap_or(false) {
        return Ok(());
    }

    if has_role(env, required_role, caller.clone()) {
        return Ok(());
    }

    Err(AdminError::Unauthorized)
}
